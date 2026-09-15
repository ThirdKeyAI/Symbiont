use super::{
    circuit_breaker::CircuitBreakerRegistry,
    conversation::Conversation,
    delegation::{DelegationContext, DelegationError, DelegationExecutor},
    effect_journal::ToolEffect,
    executor::ActionExecutor,
    governed_session::{BrokerToolCall, GovernedToolSession},
    inference::*,
    loop_types::*,
    policy_bridge::{DefaultPolicyGate, ReasoningPolicyGate},
    prepared::AuthorizedAction,
    protected_journal::ProtectedJournal,
    reasoning_loop::ReasoningLoopRunner,
};
use crate::types::AgentId;
use async_trait::async_trait;
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct Provider {
    calls: AtomicUsize,
    actions: Vec<ToolCallRequest>,
}

#[async_trait]
impl InferenceProvider for Provider {
    async fn complete(
        &self,
        _: &Conversation,
        _: &InferenceOptions,
    ) -> Result<InferenceResponse, InferenceError> {
        let first = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
        Ok(InferenceResponse {
            content: if first {
                String::new()
            } else {
                "finished".into()
            },
            tool_calls: if first { self.actions.clone() } else { vec![] },
            finish_reason: if first {
                FinishReason::ToolCalls
            } else {
                FinishReason::Stop
            },
            usage: Usage::default(),
            model: "fixture".into(),
        })
    }
    fn provider_name(&self) -> &str {
        "fixture"
    }
    fn default_model(&self) -> &str {
        "fixture"
    }
    fn supports_native_tools(&self) -> bool {
        true
    }
    fn supports_structured_output(&self) -> bool {
        true
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Outcome {
    Known,
    Error,
    MissingReceipt,
    FailedReceipt,
}

struct Executor {
    outcome: Outcome,
    calls: AtomicUsize,
}

fn start(id: &str) -> ToolEffect {
    ToolEffect::NetworkRequestStarted {
        request_id: id.into(),
        method: "POST".into(),
        url: "https://fixture.invalid/effect".into(),
        request_hash: "request".into(),
        request_bytes: 0,
    }
}
fn finish(id: &str, known: bool) -> ToolEffect {
    ToolEffect::NetworkRequestFinished {
        request_id: id.into(),
        status: known.then_some(200),
        response_hash: known.then(|| "response".into()),
        response_headers_hash: known.then(|| "headers".into()),
        response_bytes: 0,
        error: (!known).then(|| "response was lost".into()),
    }
}

#[async_trait]
impl ActionExecutor for Executor {
    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        vec![
            ToolDefinition {
                name: "fixture".into(),
                description: "effect fixture".into(),
                parameters: json!({"type":"object"}),
            },
            ToolDefinition {
                name: "delegate".into(),
                description: "Delegate fixture work".into(),
                parameters: json!({"type":"object", "required":["agent","task"], "properties":{"agent":{"type":"string"}, "task":{"type":"string"}}}),
            },
        ]
    }
    async fn execute_actions(
        &self,
        _: &[ProposedAction],
        _: &LoopConfig,
        _: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        panic!("authorized execution required")
    }
    async fn execute_authorized(
        &self,
        grants: Vec<AuthorizedAction>,
        _: &LoopConfig,
        _: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        let mut results = Vec::new();
        for grant in grants {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let audit = grant.effect_journal().unwrap();
            if self.outcome != Outcome::Error {
                audit.append(start("first")).await.unwrap();
                if self.outcome == Outcome::FailedReceipt {
                    audit.append(start("already-in-flight")).await.unwrap();
                    audit.append(finish("first", false)).await.unwrap();
                    assert!(audit.check_live().is_err());
                    assert!(audit.append(start("forbidden-retry")).await.is_err());
                    // Failure closes new authority but does not discard receipts
                    // from requests that started before it became known.
                    audit
                        .append(finish("already-in-flight", true))
                        .await
                        .unwrap();
                } else if self.outcome == Outcome::Known {
                    audit.append(finish("first", true)).await.unwrap();
                }
            }
            let ProposedAction::ToolCall { name, call_id, .. } = grant.action() else {
                panic!("tool")
            };
            let mut observation = if self.outcome == Outcome::Error {
                Observation::tool_error(name, "failed after effect")
            } else {
                Observation::tool_result(name, "backend claims success")
            }
            .with_call_id(call_id);
            // Backend-supplied claims must not override runtime evidence.
            observation
                .metadata
                .insert("effect_outcome".into(), "result_recorded".into());
            results.push(observation);
        }
        results
    }
}

struct DenyTools;
#[async_trait]
impl ReasoningPolicyGate for DenyTools {
    async fn evaluate_action(
        &self,
        _: &AgentId,
        action: &ProposedAction,
        _: &LoopState,
    ) -> LoopDecision {
        if matches!(action, ProposedAction::ToolCall { .. }) {
            LoopDecision::Deny {
                reason: "fixture denied before dispatch".into(),
            }
        } else {
            LoopDecision::Allow
        }
    }
}

#[tokio::test]
async fn uncertainty_stops_inference_but_predispatch_denial_is_recoverable() {
    for (outcome, denied) in [
        (Outcome::Known, false),
        (Outcome::Error, false),
        (Outcome::MissingReceipt, false),
        (Outcome::FailedReceipt, false),
        (Outcome::Error, true),
    ] {
        let root = tempfile::tempdir().unwrap();
        let principal = AgentId::new();
        let run_id = uuid::Uuid::new_v4();
        let journal = Arc::new(
            ProtectedJournal::create_run(&root.path().join("audit"), principal, run_id).unwrap(),
        );
        let provider = Arc::new(Provider {
            calls: AtomicUsize::new(0),
            actions: vec![ToolCallRequest {
                id: "first".into(),
                name: "fixture".into(),
                arguments: "{}".into(),
            }],
        });
        let executor = Arc::new(Executor {
            outcome,
            calls: AtomicUsize::new(0),
        });
        let gate: Arc<dyn ReasoningPolicyGate> = if denied {
            Arc::new(DenyTools)
        } else {
            Arc::new(DefaultPolicyGate::permissive_for_dev_only())
        };
        let runner = ReasoningLoopRunner::builder()
            .provider(provider.clone())
            .executor(executor.clone())
            .policy_gate(gate)
            .journal(journal.clone())
            .build();
        let result = runner
            .run(principal, Conversation::new(), LoopConfig::default())
            .await;
        let unknown = outcome != Outcome::Known && !denied;
        assert!(
            matches!(
                (&result.termination_reason, unknown),
                (TerminationReason::UnconfirmedEffects, true)
                    | (TerminationReason::Completed, false)
            ),
            "{outcome:?} denied={denied}: {result:?}"
        );
        assert_eq!(
            provider.calls.load(Ordering::SeqCst),
            if unknown { 1 } else { 2 }
        );
        assert_eq!(executor.calls.load(Ordering::SeqCst), usize::from(!denied));
        let entries =
            ProtectedJournal::verify_run(journal.path(), &journal.public_key(), run_id).unwrap();
        assert!(matches!(
            entries.last().unwrap().event,
            LoopEvent::Terminated { .. }
        ));
        if unknown {
            assert!(result.output.is_empty());
            assert!(entries.iter().any(|e| matches!(
                e.event,
                LoopEvent::ToolDispatchFinished { is_error: true, .. }
            )));
            // Even an embedding that ignores the loop result cannot reuse the
            // required writer to authorize a fresh action after this outcome.
            let mut retry = entries
                .iter()
                .find(|e| matches!(e.event, LoopEvent::ToolDispatchStarted { .. }))
                .unwrap()
                .clone();
            retry.iteration += 1;
            assert!(journal
                .append(retry)
                .await
                .unwrap_err()
                .to_string()
                .contains("reconciliation"));
            assert_eq!(
                ProtectedJournal::verify_run(journal.path(), &journal.public_key(), run_id)
                    .unwrap()
                    .len(),
                entries.len()
            );
        }
    }
}

#[tokio::test]
async fn managed_session_refuses_a_new_call_identity_after_uncertainty() {
    let root = tempfile::tempdir().unwrap();
    let principal = AgentId::new();
    let journal = Arc::new(
        ProtectedJournal::create_run(&root.path().join("audit"), principal, uuid::Uuid::new_v4())
            .unwrap(),
    );
    let executor = Arc::new(Executor {
        outcome: Outcome::Error,
        calls: AtomicUsize::new(0),
    });
    let session = GovernedToolSession::start(
        executor.clone(),
        Arc::new(DefaultPolicyGate::permissive_for_dev_only()),
        journal,
        LoopState::new(principal, Conversation::new()),
        LoopConfig::default(),
    )
    .await
    .unwrap();
    let call = |id: &str| BrokerToolCall {
        call_id: id.into(),
        name: "fixture".into(),
        arguments: json!({}),
    };
    assert!(session
        .call(call("first"))
        .await
        .unwrap()
        .has_unconfirmed_effect());
    assert!(session.call(call("different-id")).await.is_err());
    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    assert!(session.close().await.is_err());
}

struct Child {
    calls: AtomicUsize,
}
#[async_trait]
impl DelegationExecutor for Child {
    async fn delegate(
        &self,
        _: &str,
        _: &str,
        _: DelegationContext,
    ) -> Result<String, DelegationError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(DelegationError::Unconfirmed(
            "child stopped after an effect".into(),
        ))
    }
}

#[tokio::test]
async fn uncertain_child_stops_parent_and_remaining_delegations() {
    let provider = Arc::new(Provider {
        calls: AtomicUsize::new(0),
        actions: ["first", "second"]
            .into_iter()
            .map(|id| ToolCallRequest {
                id: id.into(),
                name: "delegate".into(),
                arguments: json!({"agent":"child", "task":"perform work"}).to_string(),
            })
            .collect(),
    });
    let child = Arc::new(Child {
        calls: AtomicUsize::new(0),
    });
    let root = tempfile::tempdir().unwrap();
    let principal = AgentId::new();
    let journal = Arc::new(
        ProtectedJournal::create_run(&root.path().join("audit"), principal, uuid::Uuid::new_v4())
            .unwrap(),
    );
    let runner = ReasoningLoopRunner::builder()
        .provider(provider.clone())
        .executor(Arc::new(Executor {
            outcome: Outcome::Known,
            calls: AtomicUsize::new(0),
        }))
        .policy_gate(Arc::new(DefaultPolicyGate::permissive_for_dev_only()))
        .journal(journal)
        .delegation(child.clone())
        .build();
    let config = LoopConfig {
        tool_definitions: vec![ToolDefinition {
            name: "delegate".into(),
            description: "Delegate fixture work".into(),
            parameters: json!({"type":"object", "required":["agent","task"], "properties":{"agent":{"type":"string"}, "task":{"type":"string"}}}),
        }],
        ..LoopConfig::default()
    };
    let result = runner.run(principal, Conversation::new(), config).await;
    assert!(
        matches!(
            result.termination_reason,
            TerminationReason::UnconfirmedEffects
        ),
        "{result:?}"
    );
    assert_eq!(child.calls.load(Ordering::SeqCst), 1);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert!(result
        .conversation
        .messages()
        .iter()
        .any(|m| m.content.contains("delegation not started")));
}
