//! Required reservation persistence around real reasoning-loop provider calls.
#![cfg(unix)]
use async_trait::async_trait;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use symbi_runtime::{
    reasoning::{
        circuit_breaker::CircuitBreakerRegistry, context_manager::DefaultContextManager,
        conversation::Conversation, executor::DefaultActionExecutor, inference::*, loop_types::*,
        policy_bridge::DefaultPolicyGate, protected_journal::ProtectedJournal,
        reasoning_loop::ReasoningLoopRunner, recovery::inspect_run, run_audit::RunAuditReference,
    },
    types::AgentId,
};

struct Provider {
    calls: Arc<AtomicUsize>,
    entered: Arc<tokio::sync::Notify>,
    hold: bool,
    audit: RunAuditReference,
}
#[async_trait]
impl InferenceProvider for Provider {
    fn input_token_reservation(
        &self,
        _: &Conversation,
        _: &InferenceOptions,
    ) -> Result<u32, InferenceError> {
        Ok(10)
    }
    async fn complete(
        &self,
        conversation: &Conversation,
        options: &InferenceOptions,
    ) -> Result<InferenceResponse, InferenceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let key: [u8; 32] = hex::decode(&self.audit.public_key)
            .unwrap()
            .try_into()
            .unwrap();
        let entries =
            ProtectedJournal::verify_run(&self.audit.path, &key, self.audit.run_id).unwrap();
        let LoopEvent::BudgetReservationStarted { reservation } = &entries.last().unwrap().event
        else {
            panic!("provider called before durable reservation")
        };
        assert_eq!(reservation.input_tokens, 10);
        assert_eq!(reservation.output_tokens, options.max_tokens);
        assert_eq!(options.max_tokens, 90);
        let expected = symbi_runtime::reasoning::prepared::digest_json(
            &serde_json::json!({"conversation":conversation,"options":options}),
        )
        .unwrap();
        assert_eq!(reservation.request_hash, expected);
        self.entered.notify_one();
        if self.hold {
            std::future::pending::<()>().await;
        }
        Ok(InferenceResponse {
            content: "useful result".into(),
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
        false
    }
}

struct Writer {
    inner: Arc<ProtectedJournal>,
    fail_start: bool,
    fail_finish: bool,
}
#[async_trait]
impl JournalWriter for Writer {
    fn audit_reference(&self) -> Option<RunAuditReference> {
        self.inner.audit_reference()
    }
    async fn append(&self, entry: JournalEntry) -> Result<(), JournalError> {
        if (self.fail_start && matches!(entry.event, LoopEvent::BudgetReservationStarted { .. }))
            || (self.fail_finish
                && matches!(entry.event, LoopEvent::BudgetReservationFinished { .. }))
        {
            return Err(JournalError::WriteFailed("fixture storage outage".into()));
        }
        self.inner.append(entry).await
    }
    async fn next_sequence(&self) -> u64 {
        self.inner.next_sequence().await
    }
}

fn fixture(
    path: &std::path::Path,
    hold: bool,
    fail_start: bool,
    fail_finish: bool,
) -> (
    ReasoningLoopRunner,
    AgentId,
    RunAuditReference,
    Arc<AtomicUsize>,
    Arc<tokio::sync::Notify>,
) {
    let principal = AgentId::new();
    let writer = Arc::new(
        ProtectedJournal::create_run(&path.join("audit"), principal, uuid::Uuid::new_v4()).unwrap(),
    );
    let audit = writer.audit_reference().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let runner = ReasoningLoopRunner {
        provider: Arc::new(Provider {
            calls: calls.clone(),
            entered: entered.clone(),
            hold,
            audit: audit.clone(),
        }),
        executor: Arc::new(DefaultActionExecutor::default()),
        policy_gate: Arc::new(DefaultPolicyGate::permissive_for_dev_only()),
        context_manager: Arc::new(DefaultContextManager::default()),
        circuit_breakers: Arc::new(CircuitBreakerRegistry::default()),
        journal: Arc::new(Writer {
            inner: writer,
            fail_start,
            fail_finish,
        }),
        knowledge_bridge: None,
        delegation: None,
    };
    (runner, principal, audit, calls, entered)
}
fn config() -> LoopConfig {
    LoopConfig {
        max_total_tokens: 100,
        ..Default::default()
    }
}
fn inspect(audit: &RunAuditReference) -> symbi_runtime::reasoning::recovery::RecoveryReport {
    let key: [u8; 32] = hex::decode(&audit.public_key).unwrap().try_into().unwrap();
    inspect_run(&audit.path, &key, audit.run_id).unwrap()
}

#[tokio::test]
async fn useful_provider_output_has_durable_exact_reservation_and_settlement() {
    let root = tempfile::tempdir().unwrap();
    let (runner, principal, audit, calls, _) = fixture(root.path(), false, false, false);
    let result = runner.run(principal, Conversation::new(), config()).await;
    assert!(matches!(
        result.termination_reason,
        TerminationReason::Completed
    ));
    assert_eq!(result.output, "useful result");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let report = inspect(&audit);
    assert!(!report.requires_reconciliation);
    let recovered = report.recovered_budget.unwrap();
    assert_eq!(recovered.scopes[0].usage.total_tokens, 2);
    assert_eq!(recovered.scopes[0].available_tokens, 98);
    assert_eq!(recovered.scopes[0].uncertain_tokens, 0);
}

#[tokio::test]
async fn missing_required_reservation_refuses_provider_transport() {
    let root = tempfile::tempdir().unwrap();
    let (runner, principal, _, calls, _) = fixture(root.path(), false, true, false);
    let result = runner.run(principal, Conversation::new(), config()).await;
    assert!(matches!(
        result.termination_reason,
        TerminationReason::Error { .. }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(result.output.is_empty());
}

#[tokio::test]
async fn missing_required_settlement_discards_response_and_recovers_full_charge() {
    let root = tempfile::tempdir().unwrap();
    let (runner, principal, audit, calls, _) = fixture(root.path(), false, false, true);
    let result = runner.run(principal, Conversation::new(), config()).await;
    assert!(matches!(
        result.termination_reason,
        TerminationReason::Error { .. }
    ));
    assert!(result.output.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let report = inspect(&audit);
    assert!(report.requires_reconciliation);
    let budget = report.recovered_budget.unwrap();
    assert_eq!(budget.scopes[0].uncertain_tokens, 100);
    assert_eq!(budget.scopes[0].available_tokens, 0);
}

#[tokio::test]
async fn dropping_running_owner_preserves_unsettled_usage_in_signed_storage() {
    let root = tempfile::tempdir().unwrap();
    let (runner, principal, audit, calls, entered) = fixture(root.path(), true, false, false);
    let task =
        tokio::spawn(async move { runner.run(principal, Conversation::new(), config()).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let report = inspect(&audit);
    assert!(!report.journal_complete);
    assert!(report.requires_reconciliation);
    let recovered = report.recovered_budget.unwrap();
    assert_eq!(recovered.scopes[0].usage.total_tokens, 0);
    assert_eq!(recovered.scopes[0].uncertain_tokens, 100);
    assert_eq!(recovered.scopes[0].available_tokens, 0);
}
