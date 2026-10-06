//! Real reasoning/delegation and protected-journal integration with a
//! deterministic provider. No provider credentials or external effects.
#![cfg(unix)]

use async_trait::async_trait;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use symbi_runtime::{
    reasoning::{
        circuit_breaker::CircuitBreakerRegistry,
        context_manager::DefaultContextManager,
        conversation::Conversation,
        delegation_executor::SubLoopDelegationExecutor,
        executor::DefaultActionExecutor,
        inference::{
            FinishReason, InferenceError, InferenceOptions, InferenceProvider, InferenceResponse,
            ToolCallRequest, ToolDefinition, Usage,
        },
        loop_types::{LoopConfig, LoopEvent, TerminationReason},
        policy_bridge::DefaultPolicyGate,
        protected_journal::ProtectedJournal,
        reasoning_loop::ReasoningLoopRunner,
        run_audit::{open_run_journal, RunAuditReference},
    },
    types::AgentId,
};

struct Provider {
    calls: Mutex<Vec<u32>>,
    child_entered: tokio::sync::Notify,
    wait_for_cancellation: bool,
}

#[async_trait]
impl InferenceProvider for Provider {
    fn input_token_reservation(
        &self,
        _: &Conversation,
        _: &InferenceOptions,
    ) -> Result<u32, InferenceError> {
        Ok(10) // Exact synthetic input contract.
    }

    async fn complete(
        &self,
        _: &Conversation,
        options: &InferenceOptions,
    ) -> Result<InferenceResponse, InferenceError> {
        let call = {
            let mut calls = self.calls.lock().unwrap();
            calls.push(options.max_tokens);
            calls.len()
        };
        if call == 2 {
            self.child_entered.notify_one();
            if self.wait_for_cancellation {
                std::future::pending::<()>().await;
            }
        }
        let output = match call {
            1 => 10,
            2 => 20,
            3 => 5,
            _ => panic!("unexpected provider call"),
        };
        assert!(output <= options.max_tokens);
        Ok(InferenceResponse {
            content: if call == 1 {
                String::new()
            } else {
                "useful result".into()
            },
            tool_calls: if call == 1 {
                vec![ToolCallRequest {
                    id: "child-call".into(),
                    name: "delegate".into(),
                    arguments: r#"{"agent":"reviewer","task":"check the calculation"}"#.into(),
                }]
            } else {
                Vec::new()
            },
            finish_reason: if call == 1 {
                FinishReason::ToolCalls
            } else {
                FinishReason::Stop
            },
            usage: Usage {
                prompt_tokens: 10,
                completion_tokens: output,
                total_tokens: 10 + output,
            },
            model: "synthetic-budget-provider".into(),
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

async fn fixture(
    project: &std::path::Path,
    provider: Arc<dyn InferenceProvider>,
) -> (ReasoningLoopRunner, AgentId, RunAuditReference) {
    let agent = AgentId::new();
    let executor = Arc::new(DefaultActionExecutor::default());
    let policy = Arc::new(DefaultPolicyGate::permissive_for_dev_only());
    let manager = Arc::new(DefaultContextManager::default());
    let breakers = Arc::new(CircuitBreakerRegistry::default());
    let delegation = SubLoopDelegationExecutor::new_protected(
        provider.clone(),
        executor.clone(),
        policy.clone(),
        manager.clone(),
        breakers.clone(),
        Ok(project.to_owned()),
        HashMap::from([("reviewer".into(), "Child reviewer".into())]),
        3,
    );
    let (journal, audit) = open_run_journal(project, agent).await.unwrap();
    (
        ReasoningLoopRunner {
            provider,
            executor,
            policy_gate: policy,
            context_manager: manager,
            circuit_breakers: breakers,
            journal,
            knowledge_bridge: None,
            delegation: Some(delegation),
        },
        agent,
        audit,
    )
}

fn config(limit: u32) -> LoopConfig {
    LoopConfig {
        max_total_tokens: limit,
        tool_definitions: vec![ToolDefinition {
            name: "delegate".into(),
            description: "Ask the registered reviewer".into(),
            parameters: serde_json::json!({"type":"object","properties":{"agent":{"type":"string"},"task":{"type":"string"}},"required":["agent","task"]}),
        }],
        ..Default::default()
    }
}

fn entries(audit: &RunAuditReference) -> Vec<symbi_runtime::reasoning::loop_types::JournalEntry> {
    let key: [u8; 32] = hex::decode(&audit.public_key).unwrap().try_into().unwrap();
    ProtectedJournal::verify_run(&audit.path, &key, audit.run_id).unwrap()
}

#[tokio::test]
async fn protected_child_usage_is_deducted_and_reported_in_the_parent() {
    for (limit, expected_calls, expected_usage) in [(100, 3, 65), (60, 2, 50)] {
        let project = tempfile::tempdir().unwrap();
        let provider = Arc::new(Provider {
            calls: Mutex::new(Vec::new()),
            child_entered: Default::default(),
            wait_for_cancellation: false,
        });
        let (runner, agent, audit) = fixture(project.path(), provider.clone()).await;
        let result = runner
            .run(
                agent,
                Conversation::with_system("Parent coordinator"),
                config(limit),
            )
            .await;
        assert_eq!(provider.calls.lock().unwrap().len(), expected_calls);
        assert_eq!(result.total_usage.total_tokens, expected_usage);
        assert_eq!(
            result.budget.as_ref().unwrap().available_tokens,
            limit - expected_usage
        );
        assert!(
            matches!(
                (&result.termination_reason, limit),
                (TerminationReason::Completed, 100) | (TerminationReason::MaxTokens, 60)
            ),
            "{result:?}"
        );
        if limit == 100 {
            assert_eq!(result.output, "useful result");
            assert_eq!(*provider.calls.lock().unwrap(), vec![90, 70, 40]);
        }
        let root_entries = entries(&audit);
        let recovered = symbi_runtime::reasoning::budget::journal::recover(&root_entries)
            .unwrap()
            .unwrap();
        assert_eq!(recovered.scopes[0].usage.total_tokens, expected_usage);
        assert_eq!(recovered.scopes[0].available_tokens, limit - expected_usage);
        assert_eq!(recovered.reservations.len(), expected_calls);
        assert!(root_entries.iter().any(|entry| matches!(&entry.event, LoopEvent::BudgetUpdated { budget } if budget.usage.total_tokens == expected_usage)));
        assert!(
            matches!(&root_entries.last().unwrap().event, LoopEvent::Terminated { total_usage, .. } if total_usage.total_tokens == expected_usage)
        );
        let child_audit = root_entries
            .iter()
            .find_map(|entry| match &entry.event {
                LoopEvent::DelegationStarted { audit, .. } => Some(audit),
                _ => None,
            })
            .expect("protected parent/child link");
        let child_entries = entries(child_audit);
        assert!(child_entries.iter().any(|entry|matches!(&entry.event,LoopEvent::BudgetScopeLinked {root_audit:Some(root),..} if root.run_id==audit.run_id)));
        assert!(
            matches!(&child_entries.last().unwrap().event, LoopEvent::Terminated { total_usage, .. } if total_usage.total_tokens == 30)
        );
    }
}

#[tokio::test]
async fn parent_cancellation_retains_unknown_child_spend_after_cleanup() {
    let project = tempfile::tempdir().unwrap();
    let provider = Arc::new(Provider {
        calls: Mutex::new(Vec::new()),
        child_entered: Default::default(),
        wait_for_cancellation: true,
    });
    let (runner, agent, audit) = fixture(project.path(), provider.clone()).await;
    let cancellation = tokio_util::sync::CancellationToken::new();
    let child_token = cancellation.clone();
    let task = tokio::spawn(async move {
        runner
            .run_cancellable(
                agent,
                Conversation::with_system("Parent coordinator"),
                config(100),
                child_token,
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), provider.child_entered.notified())
        .await
        .unwrap();
    cancellation.cancel();
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        result.termination_reason,
        TerminationReason::Error { .. }
    ));
    assert_eq!(result.total_usage.total_tokens, 20);
    let snapshot = result.budget.unwrap();
    assert_eq!(snapshot.reserved_tokens, 0);
    assert_eq!(snapshot.uncertain_tokens, 80);
    assert_eq!(snapshot.available_tokens, 0);
    let root_entries = entries(&audit);
    assert!(
        matches!(&root_entries.last().unwrap().event, LoopEvent::Terminated { total_usage, .. } if total_usage.total_tokens == 20)
    );
    let child = root_entries
        .iter()
        .find_map(|entry| match &entry.event {
            LoopEvent::DelegationStarted { audit, .. } => Some(audit),
            _ => None,
        })
        .unwrap();
    assert!(matches!(
        &entries(child).last().unwrap().event,
        LoopEvent::Terminated {
            reason: TerminationReason::Error { .. },
            ..
        }
    ));
}

#[derive(Default)]
struct GatedProvider {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

#[async_trait]
impl InferenceProvider for GatedProvider {
    fn input_token_reservation(
        &self,
        _: &Conversation,
        _: &InferenceOptions,
    ) -> Result<u32, InferenceError> {
        Ok(1)
    }
    async fn complete(
        &self,
        _: &Conversation,
        options: &InferenceOptions,
    ) -> Result<InferenceResponse, InferenceError> {
        assert_eq!(options.max_tokens, 10);
        self.entered.notify_one();
        self.release.notified().await;
        Ok(InferenceResponse {
            content: "sum is 2".into(),
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
            usage: Usage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: 2,
            },
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

#[tokio::test]
async fn child_timeout_preserves_live_parent_and_sibling_signed_reservations() {
    use symbi_runtime::reasoning::budget::{journal::recover, SharedBudget};
    let project = tempfile::tempdir().unwrap();
    let root_budget = SharedBudget::new(1000);
    let timed_budget = root_budget.child(100).unwrap();
    let sibling_budget = root_budget.child(100).unwrap();
    let root_provider = Arc::new(GatedProvider::default());
    let timed_provider = Arc::new(GatedProvider::default());
    let sibling_provider = Arc::new(GatedProvider::default());
    let run_config = |budget, limit, timeout| LoopConfig {
        shared_budget: Some(budget),
        max_total_tokens: limit,
        max_output_tokens: 10,
        timeout,
        ..Default::default()
    };
    let (root, agent, root_audit) = fixture(project.path(), root_provider.clone()).await;
    let task = tokio::spawn(async move {
        root.run(
            agent,
            Conversation::with_system("Root"),
            run_config(root_budget, 1000, Duration::from_secs(10)),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(5), root_provider.entered.notified())
        .await
        .unwrap();
    let (sibling, agent, sibling_audit) = fixture(project.path(), sibling_provider.clone()).await;
    let sibling_task = tokio::spawn(async move {
        sibling
            .run(
                agent,
                Conversation::with_system("Sibling"),
                run_config(sibling_budget, 100, Duration::from_secs(10)),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), sibling_provider.entered.notified())
        .await
        .unwrap();
    let (timed, agent, timed_audit) = fixture(project.path(), timed_provider.clone()).await;
    let timed_task = tokio::spawn(async move {
        timed
            .run(
                agent,
                Conversation::with_system("Timeout"),
                run_config(timed_budget, 100, Duration::from_secs(1)),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), timed_provider.entered.notified())
        .await
        .unwrap();
    let timed_result = timed_task.await.unwrap();
    assert!(matches!(
        timed_result.termination_reason,
        TerminationReason::Timeout
    ));
    let partial = recover(&entries(&root_audit)).unwrap().unwrap();
    assert_eq!(partial.reservations.len(), 3);
    let finished: Vec<_> = partial
        .reservations
        .iter()
        .filter(|r| r.finish_sequence.is_some())
        .collect();
    assert_eq!(finished.len(), 1);
    assert_eq!(
        finished[0].reservation.audit.as_ref().unwrap().run_id,
        timed_audit.run_id
    );
    sibling_provider.release.notify_one();
    assert!(matches!(
        sibling_task.await.unwrap().termination_reason,
        TerminationReason::Completed
    ));
    root_provider.release.notify_one();
    let result = task.await.unwrap();
    assert!(matches!(
        result.termination_reason,
        TerminationReason::Completed
    ));
    assert_eq!(result.output, "sum is 2");
    let recovered = recover(&entries(&root_audit)).unwrap().unwrap();
    assert!(recovered
        .reservations
        .iter()
        .all(|r| r.finish_sequence.is_some()));
    assert_eq!(recovered.scopes[0].usage.total_tokens, 4);
    assert_eq!(recovered.scopes[0].uncertain_tokens, 11);
    assert_eq!(recovered.scopes[0].available_tokens, 985);
    assert_eq!(result.budget.unwrap().available_tokens, 985);
    for (audit, timeout) in [(&timed_audit, true), (&sibling_audit, false)] {
        let verified = entries(audit);
        assert!(
            matches!(&verified.last().unwrap().event, LoopEvent::Terminated { reason, .. }
            if matches!(reason, TerminationReason::Timeout) == timeout)
        );
    }
}
