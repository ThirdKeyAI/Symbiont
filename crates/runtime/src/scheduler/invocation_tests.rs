use super::*;
use crate::{
    reasoning::{conversation::Conversation, inference::*, CedarPolicy, CedarPolicyGate},
    scheduler::{execution::GovernedAgentExecutor, AgentScheduler, SchedulerConfig},
    types::{AgentId, ExecutionMode, ResourceLimits, SecurityTier},
};
use std::{
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

struct Provider {
    calls: AtomicUsize,
    wait: bool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
#[async_trait::async_trait]
impl InferenceProvider for Provider {
    async fn complete(
        &self,
        _: &Conversation,
        _: &InferenceOptions,
    ) -> Result<InferenceResponse, InferenceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        if self.wait {
            self.release.notified().await;
        }
        Ok(InferenceResponse {
            content: "sum is 10".into(),
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
fn provider(wait: bool) -> Arc<Provider> {
    Arc::new(Provider {
        calls: AtomicUsize::new(0),
        wait,
        entered: Default::default(),
        release: Default::default(),
    })
}
fn config() -> AgentConfig {
    AgentConfig {
        id: AgentId::new(),
        name: "fixture".into(),
        dsl_source: "agent fixture() { with sandbox = \"docker\", timeout = 10.seconds {} }".into(),
        execution_mode: ExecutionMode::Ephemeral,
        security_tier: SecurityTier::Tier1,
        resource_limits: ResourceLimits::default(),
        capabilities: vec![],
        policies: vec![],
        metadata: Default::default(),
        priority: Default::default(),
    }
}
async fn scheduler(root: &Path, provider: Arc<Provider>, capacity: usize) -> DefaultAgentScheduler {
    let gate = Arc::new(CedarPolicyGate::deny_by_default());
    gate.add_policy(CedarPolicy {
        name: "fixture".into(),
        active: true,
        source: "permit(principal, action, resource);".into(),
    })
    .await;
    DefaultAgentScheduler::new_with_executor(
        SchedulerConfig {
            max_concurrent_agents: capacity,
            task_timeout: Duration::from_secs(15),
            ..Default::default()
        },
        None,
        Arc::new(
            GovernedAgentExecutor::new(root)
                .unwrap()
                .with_provider(provider)
                .with_policy_gate(gate),
        ),
    )
    .await
    .unwrap()
}
fn identity() -> InvocationIdentity {
    InvocationIdentity {
        id: Uuid::new_v4(),
        context: json!({"caller":"fixture"}),
    }
}
fn queued(value: Admission) -> TaskHandle {
    match value {
        Admission::Queued { handle, .. } => handle,
        other => panic!("expected admission, got {other:?}"),
    }
}
async fn wait(handle: &TaskHandle) -> TaskCompletion {
    tokio::time::timeout(Duration::from_secs(12), handle.wait())
        .await
        .unwrap()
}
#[tokio::test]
async fn persistent_scheduler_identity_excludes_duplicate_owners_and_survives_restart() {
    let root = tempfile::tempdir().unwrap();
    let provider = provider(true);
    let runtime = scheduler(root.path(), provider.clone(), 1).await;
    let config = config();
    let id = identity();
    let (a, b) = tokio::join!(
        runtime.schedule_identified_invocation(config.clone(), json!("calculate"), id.clone()),
        runtime.schedule_identified_invocation(config.clone(), json!("calculate"), id.clone())
    );
    let (handle, existing) = match (a.unwrap(), b.unwrap()) {
        (Admission::Queued { handle, .. }, other) | (other, Admission::Queued { handle, .. }) => {
            (handle, other)
        }
        other => panic!("expected exactly one queue owner: {other:?}"),
    };
    assert!(matches!(
        existing,
        Admission::Existing(ExistingInvocation::InProgress)
    ));
    tokio::time::timeout(Duration::from_secs(5), provider.entered.notified())
        .await
        .unwrap();
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    provider.release.notify_one();
    let result = wait(&handle).await;
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
    assert_eq!(result.output.as_deref(), Some("sum is 10"));
    assert_eq!(result.total_usage.as_ref().unwrap().total_tokens, 2);
    let audit = result.audit.unwrap();
    let entries = crate::reasoning::protected_journal::ProtectedJournal::verify_run(
        &audit.path,
        &hex::decode(audit.public_key).unwrap().try_into().unwrap(),
        result.run_id,
    )
    .unwrap();
    let started = serde_json::to_value(&entries[0].event).unwrap();
    assert_eq!(
        started["Started"]["execution_context"]["invocation"]["id"],
        id.id.to_string()
    );
    runtime.shutdown().await.unwrap();
    let restarted = scheduler(root.path(), provider.clone(), 1).await;
    let cached = restarted
        .schedule_identified_invocation(config.clone(), json!("calculate"), id.clone())
        .await
        .unwrap();
    match cached {
        Admission::Existing(ExistingInvocation::Recorded {
            audit,
            result: payload,
        }) => {
            let saved = recorded_completion(&audit, payload).unwrap();
            assert_eq!(saved.run_id, result.run_id);
            assert_eq!(saved.output.as_deref(), Some("sum is 10"));
        }
        other => panic!("expected saved completion: {other:?}"),
    }
    assert!(restarted
        .schedule_identified_invocation(config.clone(), json!("changed"), id.clone())
        .await
        .unwrap_err()
        .contains("conflicts"));
    let mut other = id.clone();
    other.context = json!({"caller":"different"});
    assert!(restarted
        .schedule_identified_invocation(config.clone(), json!("calculate"), other)
        .await
        .unwrap_err()
        .contains("conflicts"));
    let mut other = config;
    other.resource_limits.memory_mb += 1;
    assert!(restarted
        .schedule_identified_invocation(other, json!("calculate"), id)
        .await
        .unwrap_err()
        .contains("conflicts"));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    restarted.shutdown().await.unwrap();
}
#[tokio::test]
async fn cancelled_queued_claim_is_retained_without_provider_execution() {
    let root = tempfile::tempdir().unwrap();
    let provider = provider(false);
    let runtime = scheduler(root.path(), provider.clone(), 0).await;
    let config = config();
    let id = identity();
    let handle = queued(
        runtime
            .schedule_identified_invocation(config.clone(), Value::Null, id.clone())
            .await
            .unwrap(),
    );
    handle.cancel();
    let result = wait(&handle).await;
    assert_eq!(result.status, TaskStatus::Unresolved);
    let audit = result.audit.unwrap();
    assert_eq!(std::fs::metadata(audit.path).unwrap().len(), 0);
    assert!(matches!(
        runtime
            .schedule_identified_invocation(config, Value::Null, id)
            .await
            .unwrap(),
        Admission::Existing(ExistingInvocation::Unresolved { .. })
    ));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    runtime.shutdown().await.unwrap();
}
#[tokio::test]
async fn failed_setup_cannot_execute_when_configuration_later_becomes_available() {
    let root = tempfile::tempdir().unwrap();
    let provider = provider(false);
    let runtime = scheduler(root.path(), provider.clone(), 1).await;
    let config = config();
    let id = identity();
    std::fs::write(root.path().join("symbiont.toml"), "[malformed").unwrap();
    let handle = queued(
        runtime
            .schedule_identified_invocation(config.clone(), Value::Null, id.clone())
            .await
            .unwrap(),
    );
    let result = wait(&handle).await;
    assert_eq!(result.status, TaskStatus::Unresolved);
    std::fs::write(
        root.path().join("symbiont.toml"),
        "[sandbox]\ntier='docker'\n",
    )
    .unwrap();
    assert!(matches!(
        runtime
            .schedule_identified_invocation(config, Value::Null, id)
            .await
            .unwrap(),
        Admission::Existing(ExistingInvocation::Unresolved { .. })
    ));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    runtime.shutdown().await.unwrap();
}

#[tokio::test]
async fn lookup_does_not_create_a_claim_and_rejects_a_linked_store() {
    let root = tempfile::tempdir().unwrap();
    let p = provider(false);
    let scheduler = scheduler(root.path(), p.clone(), 1).await;
    let identity = identity();
    let config = config();
    assert!(scheduler
        .lookup_identified_invocation(&config, &json!(null), &identity)
        .await
        .unwrap()
        .is_none());
    assert!(!root.path().join(".symbiont/invocations").exists());
    std::fs::create_dir(root.path().join(".symbiont")).unwrap();
    std::os::unix::fs::symlink(
        root.path().join("missing"),
        root.path().join(".symbiont/invocations"),
    )
    .unwrap();
    assert!(scheduler
        .lookup_identified_invocation(&config, &json!(null), &identity)
        .await
        .is_err());
    assert_eq!(p.calls.load(Ordering::SeqCst), 0);
    scheduler.shutdown().await.unwrap();
}
