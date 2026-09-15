//! Retain selected source rules through normalization, authorization and audit.

use super::{
    circuit_breaker::CircuitBreakerRegistry,
    executor::ActionExecutor,
    inference::ToolDefinition,
    loop_types::{LoopConfig, LoopState, Observation, ProposedAction},
    prepared::{digest_json, AuthorizedAction, PreparedAction},
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc, time::Instant};

pub(crate) struct BoundSourcePolicy {
    policy: dsl::ExecutionPolicy,
    metadata: Value,
}

impl BoundSourcePolicy {
    pub(crate) fn metadata(&self) -> &Value {
        &self.metadata
    }
    pub(crate) fn check(&self, prepared: &PreparedAction, state: &LoopState) -> Result<(), String> {
        let effect = match prepared.action() {
            ProposedAction::ToolCall { name, .. } => name.clone(),
            ProposedAction::Delegate { target, .. } => format!("delegate::{target}"),
            // Permit reporting a denial or ending the run. The external gate
            // still evaluates these actions and the journal still records them.
            ProposedAction::Respond { .. } | ProposedAction::Terminate { .. } => return Ok(()),
        };
        let mut context = state.trusted_context.clone();
        context.remove("has_human_approval");
        context.remove("approved_fingerprint");
        self.policy.evaluate(
            &effect,
            &json!({
                "principal": state.agent_id.to_string(),
                "invocation": prepared.policy_context(),
                "context": context,
            }),
        )
    }
}

/// An executor wrapper that freezes source rules independently of gate choice.
/// Construct after tool/knowledge/delegation wrappers so every effect carries
/// the same source contract. It does not interpret executable DSL statements.
pub struct SourcePolicyExecutor {
    inner: Arc<dyn ActionExecutor>,
    policy: Arc<BoundSourcePolicy>,
}

impl SourcePolicyExecutor {
    pub fn new(inner: Arc<dyn ActionExecutor>, policy: dsl::ExecutionPolicy) -> Self {
        let metadata = json!({
            "semantics": "inline_effect_policy_v1",
            "source_hash": digest_json(&json!(policy.source())).expect("source is JSON serializable"),
            "declaration_hash": digest_json(&json!(policy.settings().agent_source)).expect("source is JSON serializable"),
            "agent_name": policy.settings().agent_name,
            "policies": policy.names(),
        });
        Self {
            inner,
            policy: Arc::new(BoundSourcePolicy { policy, metadata }),
        }
    }
}

#[async_trait]
impl ActionExecutor for SourcePolicyExecutor {
    fn execution_context(&self) -> HashMap<String, Value> {
        let mut context = self.inner.execution_context();
        context.insert("source_policy".into(), self.policy.metadata.clone());
        context
    }
    fn validate_configuration(&self) -> Result<(), String> {
        self.inner.validate_configuration()
    }
    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.inner.tool_definitions()
    }
    fn prepare_action(
        &self,
        action: &ProposedAction,
        config: &LoopConfig,
    ) -> Result<PreparedAction, String> {
        self.inner
            .prepare_action(action, config)?
            .with_source_policy(self.policy.clone())
    }
    fn cancel_run(&self, run: &str, deadline: Instant) {
        self.inner.cancel_run(run, deadline);
    }
    async fn close_run(&self, run: &str, deadline: Instant) -> Result<(), String> {
        self.inner.close_run(run, deadline).await
    }
    async fn execute_actions(
        &self,
        actions: &[ProposedAction],
        _config: &LoopConfig,
        _breakers: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        actions
            .iter()
            .filter_map(|action| match action {
                ProposedAction::ToolCall { name, call_id, .. } => Some(
                    Observation::tool_error(
                        name,
                        "source-bound execution requires a prepared authorization grant",
                    )
                    .with_call_id(call_id),
                ),
                _ => None,
            })
            .collect()
    }
    async fn execute_authorized(
        &self,
        actions: Vec<AuthorizedAction>,
        config: &LoopConfig,
        breakers: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        let mut approved = Vec::new();
        let mut observations = Vec::new();
        for grant in actions {
            if grant
                .prepared()
                .source_policy
                .as_ref()
                .is_some_and(|policy| Arc::ptr_eq(policy, &self.policy))
            {
                approved.push(grant);
            } else if let ProposedAction::ToolCall { name, call_id, .. } = grant.action() {
                observations.push(
                    Observation::tool_error(
                        name,
                        "authorization belongs to another source policy executor",
                    )
                    .with_call_id(call_id),
                );
            }
        }
        observations.extend(
            self.inner
                .execute_authorized(approved, config, breakers)
                .await,
        );
        observations
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        conversation::Conversation,
        dispatch::authorize_action,
        loop_types::LoopDecision,
        policy_bridge::{DefaultPolicyGate, ReasoningPolicyGate},
    };
    use super::*;
    use crate::types::AgentId;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Counter(AtomicUsize);
    #[async_trait]
    impl ActionExecutor for Counter {
        fn tool_definitions(&self) -> Vec<ToolDefinition> {
            vec![ToolDefinition {
                name: "write".into(),
                description: "fixture".into(),
                parameters: json!({"type":"object", "properties":{"path":{"type":"string"}}, "required":["path"], "additionalProperties":false}),
            }]
        }
        async fn execute_actions(
            &self,
            _: &[ProposedAction],
            _: &LoopConfig,
            _: &CircuitBreakerRegistry,
        ) -> Vec<Observation> {
            self.0.fetch_add(1, Ordering::SeqCst);
            vec![Observation::tool_result("write", "recorded")]
        }
    }
    fn fixture(
        rules: &str,
    ) -> (
        SourcePolicyExecutor,
        Arc<Counter>,
        LoopState,
        LoopConfig,
        ProposedAction,
    ) {
        let inner = Arc::new(Counter(AtomicUsize::new(0)));
        let policy = dsl::ExecutionPolicy::parse(
            &format!("agent writer() {{ policy files {{ {rules} }} }}"),
            "writer",
        )
        .unwrap();
        let executor = SourcePolicyExecutor::new(inner.clone(), policy);
        let mut state = LoopState::new(AgentId::new(), Conversation::new());
        state.trusted_context = executor.execution_context();
        let config = LoopConfig {
            tool_definitions: executor.tool_definitions(),
            ..Default::default()
        };
        let action = ProposedAction::ToolCall {
            name: "write".into(),
            call_id: "call".into(),
            arguments: r#"{"path":"allowed"}"#.into(),
        };
        (executor, inner, state, config, action)
    }

    struct CountGate {
        approvals: AtomicUsize,
        evaluations: AtomicUsize,
    }
    #[async_trait]
    impl ReasoningPolicyGate for CountGate {
        async fn approve_prepared(
            &self,
            _: &PreparedAction,
            _: &LoopState,
            _: &LoopConfig,
        ) -> Result<Option<super::super::prepared::ApprovalReceipt>, String> {
            self.approvals.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        }
        async fn evaluate_action(
            &self,
            _: &AgentId,
            _: &ProposedAction,
            _: &LoopState,
        ) -> LoopDecision {
            self.evaluations.fetch_add(1, Ordering::SeqCst);
            LoopDecision::Allow
        }
    }

    #[tokio::test]
    async fn denies_before_approval_even_with_permissive_gate_and_direct_grant_issue() {
        let (executor, inner, state, config, action) = fixture("deny: true");
        let gate = CountGate {
            approvals: AtomicUsize::new(0),
            evaluations: AtomicUsize::new(0),
        };
        let error = authorize_action(&action, &state, &config, &executor, &gate)
            .await
            .err()
            .unwrap();
        assert!(error.contains("inline policy files"));
        assert_eq!(gate.approvals.load(Ordering::SeqCst), 0);
        assert_eq!(gate.evaluations.load(Ordering::SeqCst), 0);
        let prepared = executor.prepare_action(&action, &config).unwrap();
        assert!(AuthorizedAction::issue(prepared, &state, &config, None).is_err());
        assert!(
            executor
                .execute_actions(&[action], &config, &CircuitBreakerRegistry::default())
                .await[0]
                .is_error
        );
        assert_eq!(inner.0.load(Ordering::SeqCst), 0);
        for action in [
            ProposedAction::Respond {
                content: "denied".into(),
            },
            ProposedAction::Terminate {
                reason: "stop".into(),
                output: String::new(),
            },
        ] {
            assert!(authorize_action(&action, &state, &config, &executor, &gate)
                .await
                .is_ok());
        }
    }

    #[tokio::test]
    async fn retains_source_through_legacy_executor_recheck_and_rejects_other_owner() {
        let (executor, inner, state, config, action) =
            fixture(r#"allow: "write" if invocation.arguments.path == "allowed""#);
        let gate = DefaultPolicyGate::permissive_for_dev_only();
        let grant = authorize_action(&action, &state, &config, &executor, &gate)
            .await
            .unwrap();
        assert_eq!(
            grant.audit_context()["source_policy"],
            state.trusted_context["source_policy"]
        );
        let observations = executor
            .execute_authorized(vec![grant], &config, &CircuitBreakerRegistry::default())
            .await;
        assert!(!observations[0].is_error, "{observations:?}");
        assert_eq!(inner.0.load(Ordering::SeqCst), 1);
        let grant = authorize_action(&action, &state, &config, &executor, &gate)
            .await
            .unwrap();
        let other = SourcePolicyExecutor::new(inner.clone(), executor.policy.policy.clone());
        assert!(
            other
                .execute_authorized(vec![grant], &config, &CircuitBreakerRegistry::default())
                .await[0]
                .is_error
        );
        assert_eq!(inner.0.load(Ordering::SeqCst), 1);
        assert!(authorize_action(
            &action,
            &state,
            &config,
            &executor,
            &DefaultPolicyGate::new()
        )
        .await
        .is_err());
    }

    struct Modify;
    #[async_trait]
    impl ReasoningPolicyGate for Modify {
        async fn evaluate_action(
            &self,
            _: &AgentId,
            action: &ProposedAction,
            _: &LoopState,
        ) -> LoopDecision {
            let mut modified = action.clone();
            if let ProposedAction::ToolCall { arguments, .. } = &mut modified {
                *arguments = r#"{"path":"blocked"}"#.into();
            }
            LoopDecision::Modify {
                modified_action: Box::new(modified),
                reason: "fixture replacement".into(),
            }
        }
    }
    #[tokio::test]
    async fn modifications_and_principal_substitution_cannot_evade_source_rules() {
        let (executor, inner, mut state, config, action) = fixture(
            r#"allow: "write" if invocation.arguments.path == "allowed" && principal == context.expected_principal"#,
        );
        state.trusted_context.insert(
            "expected_principal".into(),
            json!(state.agent_id.to_string()),
        );
        assert!(
            authorize_action(&action, &state, &config, &executor, &Modify)
                .await
                .is_err()
        );
        let prepared = executor.prepare_action(&action, &config).unwrap();
        state.agent_id = AgentId::new();
        assert!(AuthorizedAction::issue(prepared, &state, &config, None).is_err());
        assert_eq!(inner.0.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn inline_allow_cannot_waive_manifest_approval() {
        use crate::toolclad::{executor::ToolCladExecutor, manifest::Manifest};
        let mut manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "write"
version = "1"
description = "approval fixture"
binary = "/usr/bin/touch"
[args.path]
position = 1
required = true
type = "string"
[command]
template = "/usr/bin/touch {path}"
[output]
format = "text"
"#,
        )
        .unwrap();
        manifest.tool.human_approval = true;
        let inner = Arc::new(
            ToolCladExecutor::new(vec![("write".into(), manifest)])
                .with_development_host_execution(),
        );
        let policy =
            dsl::ExecutionPolicy::parse("agent a() { policy p { allow: true } }", "a").unwrap();
        let executor = SourcePolicyExecutor::new(inner, policy);
        let config = LoopConfig {
            tool_definitions: executor.tool_definitions(),
            ..Default::default()
        };
        let state = LoopState::new(AgentId::new(), Conversation::new());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("effect");
        let action = ProposedAction::ToolCall {
            name: "write".into(),
            call_id: "call".into(),
            arguments: json!({"path":path}).to_string(),
        };
        let error = authorize_action(
            &action,
            &state,
            &config,
            &executor,
            &DefaultPolicyGate::permissive_for_dev_only(),
        )
        .await
        .err()
        .unwrap();
        assert!(error.contains("approval relay"), "{error}");
        assert!(!path.exists());
    }
}
