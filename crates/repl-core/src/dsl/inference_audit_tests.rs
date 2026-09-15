use super::*;
use crate::dsl::{
    agent_composition::*, evaluator::DslValue, pattern_builtins::builtin_chain,
    reasoning_builtins::builtin_llm_call,
};
use std::os::unix::fs::PermissionsExt;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use symbi_runtime::communication::policy_gate::{
    CommunicationCondition, CommunicationEffect, CommunicationPolicyGate, CommunicationPolicyRule,
};
use symbi_runtime::reasoning::{
    agent_registry::AgentRegistry,
    inference::{FinishReason, InferenceError, InferenceProvider},
    loop_types::{BufferedJournal, JournalEntry, JournalError, JournalWriter},
    protected_journal::ProtectedJournal,
};

#[derive(Default)]
struct Provider {
    requests: Mutex<Vec<serde_json::Value>>,
    active: Arc<AtomicUsize>,
    release: tokio::sync::Notify,
    audit_log: Mutex<Option<Arc<AuditReferenceLog>>>,
}

struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl InferenceProvider for Provider {
    async fn complete(
        &self,
        conversation: &Conversation,
        options: &InferenceOptions,
    ) -> std::result::Result<InferenceResponse, InferenceError> {
        if let Some(log) = self.audit_log.lock().unwrap().as_ref() {
            let references = log.snapshot().unwrap();
            if !references.entries.is_empty() {
                let expected = digest_json(
                    &serde_json::json!({"conversation": conversation, "options": options}),
                )
                .unwrap();
                assert!(references.entries.iter().any(|reference| read_records(reference).is_ok_and(|entries| entries.iter().any(|entry| matches!(&entry.event, LoopEvent::DirectInferenceRequested { request_hash, .. } if request_hash == &expected)))), "provider reached before its signed request record");
            }
        }
        self.requests
            .lock()
            .unwrap()
            .push(serde_json::json!({"conversation": conversation, "options": options}));
        self.active.fetch_add(1, Ordering::SeqCst);
        let _active = Active(self.active.clone());
        let message = &conversation.messages().last().unwrap().content;
        match message.as_str() {
            "wait" => std::future::pending::<()>().await,
            "slow" => self.release.notified().await,
            "fail" => return Err(InferenceError::Provider("synthetic failure".into())),
            _ => {}
        }
        Ok(InferenceResponse {
            content: if message == "huge" {
                "x".repeat(MAX_RESPONSE_BYTES)
            } else {
                "synthetic response".into()
            },
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
            usage: Usage {
                prompt_tokens: 2,
                completion_tokens: 3,
                total_tokens: 5,
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
        false
    }
    fn supports_structured_output(&self) -> bool {
        false
    }
}

fn context() -> (tempfile::TempDir, Arc<Provider>, ReasoningBuiltinContext) {
    let project = tempfile::tempdir().unwrap();
    let provider = Arc::new(Provider::default());
    let context = ReasoningBuiltinContext {
        provider: Some(provider.clone()),
        sender_agent_id: Some(AgentId::new()),
        agent_registry: Some(Arc::new(AgentRegistry::new())),
        comm_policy: Some(Arc::new(CommunicationPolicyGate::permissive())),
        ..Default::default()
    }
    .with_project_root(project.path())
    .unwrap();
    *provider.audit_log.lock().unwrap() = Some(context.audit_references.clone());
    (project, provider, context)
}

fn read_records(
    reference: &InferenceAuditReference,
) -> std::result::Result<Vec<JournalEntry>, JournalError> {
    let key: [u8; 32] = (0..32)
        .map(|i| u8::from_str_radix(&reference.audit.public_key[i * 2..i * 2 + 2], 16).unwrap())
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    ProtectedJournal::verify_run(&reference.audit.path, &key, reference.audit.run_id)
}

fn verified(reference: &InferenceAuditReference) -> Vec<JournalEntry> {
    let entries = read_records(reference).unwrap();
    let key: [u8; 32] = (0..32)
        .map(|i| u8::from_str_radix(&reference.audit.public_key[i * 2..i * 2 + 2], 16).unwrap())
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    assert!(
        ProtectedJournal::verify_run(&reference.audit.path, &key, uuid::Uuid::new_v4()).is_err()
    );
    assert!(entries
        .iter()
        .all(|entry| entry.agent_id == reference.agent_id));
    assert_eq!(
        std::fs::metadata(&reference.audit.path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    entries
}

async fn wait_for_calls(provider: &Provider, count: usize) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while provider.requests.lock().unwrap().len() < count {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

async fn settled(ctx: &ReasoningBuiltinContext, count: usize) -> Vec<Vec<JournalEntry>> {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let snapshot = ctx.audit_references.snapshot().unwrap();
            if snapshot.entries.len() == count {
                let entries: Option<Vec<_>> = snapshot
                    .entries
                    .iter()
                    .map(|reference| read_records(reference).ok())
                    .collect();
                if entries.is_some_and(|entries| {
                    entries.iter().all(|entries| {
                        matches!(entries.last().unwrap().event, LoopEvent::Terminated { .. })
                    })
                }) {
                    break snapshot.entries.iter().map(verified).collect();
                }
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn direct_calls_and_chain_hash_actual_normalized_inputs_and_keep_principal() {
    let (_project, provider, mut ctx) = context();
    ctx.reasoning_config = Some(symbi_runtime::reasoning::LoopConfig {
        max_output_tokens: 16,
        ..Default::default()
    });
    let value = builtin_llm_call(&[DslValue::String("request".into())], &ctx)
        .await
        .unwrap();
    assert_eq!(value, DslValue::String("synthetic response".into()));
    builtin_chain(
        &[DslValue::List(vec![
            DslValue::String("first".into()),
            DslValue::String("second".into()),
        ])],
        &ctx,
    )
    .await
    .unwrap();
    let records = settled(&ctx, 3).await;
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    let references = ctx.audit_references.snapshot().unwrap();
    assert_eq!(
        references
            .entries
            .iter()
            .map(|r| r.audit.run_id)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        3
    );
    for ((entries, request), reference) in
        records.iter().zip(requests.iter()).zip(references.entries)
    {
        assert_eq!(reference.agent_id, ctx.sender_agent_id.unwrap());
        assert_eq!(request["options"]["max_tokens"], 16);
        let expected = digest_json(request).unwrap();
        assert!(entries.iter().any(|entry| matches!(&entry.event, LoopEvent::DirectInferenceRequested { request_hash, .. } if request_hash == &expected)));
        assert!(matches!(
            entries.last().unwrap().event,
            LoopEvent::Terminated {
                reason: TerminationReason::Completed,
                ..
            }
        ));
    }
}

struct FailingJournal {
    at: usize,
    writes: AtomicUsize,
    inner: BufferedJournal,
}
#[async_trait::async_trait]
impl JournalWriter for FailingJournal {
    async fn append(&self, entry: JournalEntry) -> std::result::Result<(), JournalError> {
        if self.writes.fetch_add(1, Ordering::SeqCst) + 1 == self.at {
            return Err(JournalError::WriteFailed("required audit fixture".into()));
        }
        self.inner.append(entry).await
    }
    async fn next_sequence(&self) -> u64 {
        self.inner.next_sequence().await
    }
}

#[tokio::test]
async fn storage_and_each_required_append_failure_prevent_false_success() {
    let (project, provider, mut ctx) = context();
    let audit = project.path().join(".symbiont/governed");
    std::fs::create_dir_all(&audit).unwrap();
    std::fs::set_permissions(&audit, std::fs::Permissions::from_mode(0o777)).unwrap();
    assert!(
        builtin_llm_call(&[DslValue::String("request".into())], &ctx)
            .await
            .is_err()
    );
    assert!(provider.requests.lock().unwrap().is_empty());
    assert_eq!(std::fs::read_dir(&audit).unwrap().count(), 0);
    for at in 1..=5 {
        let before = provider.requests.lock().unwrap().len();
        ctx.reasoning_journal = Some(Arc::new(FailingJournal {
            at,
            writes: AtomicUsize::new(0),
            inner: BufferedJournal::new(16),
        }));
        let error = builtin_llm_call(&[DslValue::String("request".into())], &ctx)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("required audit fixture"),
            "{error}"
        );
        assert_eq!(
            provider.requests.lock().unwrap().len() - before,
            usize::from(at > 2)
        );
    }
}

#[tokio::test]
async fn cancellation_settles_the_owned_call_and_signed_terminal_record() {
    let (_project, provider, ctx) = context();
    let owned = ctx.clone();
    let task =
        tokio::spawn(
            async move { builtin_llm_call(&[DslValue::String("wait".into())], &owned).await },
        );
    wait_for_calls(&provider, 1).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let entries = settled(&ctx, 1).await.remove(0);
    assert!(
        matches!(&entries.last().unwrap().event, LoopEvent::Terminated { reason: TerminationReason::Error { message }, .. } if message == "caller cancelled")
    );
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn timeouts_and_oversized_responses_have_honest_signed_outcomes() {
    let (_project, provider, mut ctx) = context();
    ctx.reasoning_config = Some(symbi_runtime::reasoning::LoopConfig {
        timeout: Duration::from_millis(100),
        ..Default::default()
    });
    assert!(builtin_llm_call(&[DslValue::String("wait".into())], &ctx)
        .await
        .unwrap_err()
        .to_string()
        .contains("deadline expired"));
    assert!(matches!(
        settled(&ctx, 1).await[0].last().unwrap().event,
        LoopEvent::Terminated {
            reason: TerminationReason::Timeout,
            ..
        }
    ));
    ctx.reasoning_config = None;
    assert!(builtin_llm_call(&[DslValue::String("huge".into())], &ctx)
        .await
        .is_err());
    assert!(matches!(
        settled(&ctx, 2).await[1].last().unwrap().event,
        LoopEvent::Terminated {
            reason: TerminationReason::Error { .. },
            ..
        }
    ));
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
}

struct PausedJournal {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    inner: BufferedJournal,
}
#[async_trait::async_trait]
impl JournalWriter for PausedJournal {
    async fn append(&self, entry: JournalEntry) -> std::result::Result<(), JournalError> {
        if matches!(entry.event, LoopEvent::DirectInferenceRequested { .. }) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        self.inner.append(entry).await
    }
    async fn next_sequence(&self) -> u64 {
        self.inner.next_sequence().await
    }
}

#[tokio::test]
async fn cancellation_during_startup_retains_journal_owner_without_provider_effects() {
    let (_project, provider, mut ctx) = context();
    let journal = Arc::new(PausedJournal {
        entered: Default::default(),
        release: Default::default(),
        inner: BufferedJournal::new(16),
    });
    ctx.reasoning_journal = Some(journal.clone());
    let task =
        tokio::spawn(
            async move { builtin_llm_call(&[DslValue::String("request".into())], &ctx).await },
        );
    tokio::time::timeout(Duration::from_secs(3), journal.entered.notified())
        .await
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    journal.release.notify_one();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let entries = journal.inner.entries().await;
            if matches!(entries.last().map(|entry| &entry.event), Some(LoopEvent::Terminated { reason: TerminationReason::Error { message }, .. }) if message == "caller cancelled") {
                assert_eq!(entries.len(), 4);
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }).await.unwrap();
    assert!(provider.requests.lock().unwrap().is_empty());
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn replacing_a_registered_name_cannot_change_an_authorized_recipient() {
    let (_project, provider, mut ctx) = context();
    let registry = ctx.agent_registry.as_ref().unwrap().clone();
    let original = registry
        .spawn_agent("worker", "original prompt", vec![], None)
        .await;
    ctx.comm_policy = Some(Arc::new(CommunicationPolicyGate::new(vec![
        CommunicationPolicyRule {
            id: "exact".into(),
            name: "exact recipient".into(),
            priority: 1,
            condition: CommunicationCondition::All(vec![
                CommunicationCondition::SenderIs(ctx.sender_agent_id.unwrap()),
                CommunicationCondition::RecipientIs(original),
            ]),
            effect: CommunicationEffect::Allow,
        },
    ])));
    let journal = Arc::new(PausedJournal {
        entered: Default::default(),
        release: Default::default(),
        inner: BufferedJournal::new(16),
    });
    ctx.reasoning_journal = Some(journal.clone());
    let owned = ctx.clone();
    let task = tokio::spawn(async move { governed_ask(&owned, "worker", "request", None).await });
    tokio::time::timeout(Duration::from_secs(3), journal.entered.notified())
        .await
        .unwrap();
    let replacement = registry
        .spawn_agent("worker", "replacement prompt", vec![], None)
        .await;
    assert_ne!(original, replacement);
    journal.release.notify_one();
    task.await.unwrap().unwrap();
    let requests = provider.requests.lock().unwrap().clone();
    assert_eq!(
        requests[0]["conversation"]["messages"][0]["content"],
        "original prompt"
    );
    assert!(journal.inner.entries().await.iter().any(|entry| matches!(entry.event, LoopEvent::DirectInferenceRequested { recipient: Some(id), .. } if id == original)));
    assert!(governed_ask(&ctx, "worker", "request", None).await.is_err());
    assert_eq!(provider.requests.lock().unwrap().len(), 1);
}

fn tasks(messages: &[(&str, &str)]) -> Vec<DslValue> {
    vec![DslValue::List(
        messages
            .iter()
            .map(|(agent, message)| {
                DslValue::Map(std::collections::HashMap::from([
                    ("agent".into(), DslValue::String((*agent).into())),
                    ("message".into(), DslValue::String((*message).into())),
                ]))
            })
            .collect(),
    )]
}

#[tokio::test]
async fn parallel_cancellation_does_not_detach_inference_calls() {
    let (_project, provider, ctx) = context();
    for name in ["one", "two"] {
        ctx.agent_registry
            .as_ref()
            .unwrap()
            .spawn_agent(name, "worker", vec![], None)
            .await;
    }
    let owned = ctx.clone();
    let task = tokio::spawn(async move {
        builtin_parallel(&tasks(&[("one", "wait"), ("two", "wait")]), &owned).await
    });
    wait_for_calls(&provider, 2).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    for entries in settled(&ctx, 2).await {
        assert!(
            matches!(&entries.last().unwrap().event, LoopEvent::Terminated { reason: TerminationReason::Error { message }, .. } if message == "caller cancelled")
        );
    }
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn race_waits_for_success_and_audits_cancelled_losers() {
    let (_project, provider, ctx) = context();
    for name in ["one", "two", "three"] {
        ctx.agent_registry
            .as_ref()
            .unwrap()
            .spawn_agent(name, "worker", vec![], None)
            .await;
    }
    let owned = ctx.clone();
    let task = tokio::spawn(async move {
        builtin_race(
            &tasks(&[("one", "fail"), ("two", "slow"), ("three", "wait")]),
            &owned,
        )
        .await
    });
    wait_for_calls(&provider, 3).await;
    assert!(
        !task.is_finished(),
        "an earlier error must not end the race"
    );
    provider.release.notify_one();
    assert_eq!(
        task.await.unwrap().unwrap(),
        DslValue::String("synthetic response".into())
    );
    let records = settled(&ctx, 3).await;
    assert_eq!(
        records
            .iter()
            .filter(|entries| matches!(
                entries.last().unwrap().event,
                LoopEvent::Terminated {
                    reason: TerminationReason::Completed,
                    ..
                }
            ))
            .count(),
        1
    );
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn background_send_requires_durable_acceptance_and_records_later_timeout() {
    let (_project, provider, mut ctx) = context();
    ctx.agent_registry
        .as_ref()
        .unwrap()
        .spawn_agent("worker", "worker", vec![], None)
        .await;
    ctx.reasoning_journal = Some(Arc::new(FailingJournal {
        at: 2,
        writes: AtomicUsize::new(0),
        inner: BufferedJournal::new(16),
    }));
    let args = vec![
        DslValue::String("worker".into()),
        DslValue::String("wait".into()),
    ];
    assert!(builtin_send_to(&args, &ctx).await.is_err());
    assert!(provider.requests.lock().unwrap().is_empty());
    ctx.reasoning_journal = None;
    ctx.reasoning_config = Some(symbi_runtime::reasoning::LoopConfig {
        timeout: Duration::from_millis(150),
        ..Default::default()
    });
    assert_eq!(builtin_send_to(&args, &ctx).await.unwrap(), DslValue::Null);
    let reference = ctx.audit_references.snapshot().unwrap().entries[0].clone();
    assert!(verified(&reference)
        .iter()
        .any(|entry| matches!(entry.event, LoopEvent::DirectInferenceRequested { .. })));
    assert!(matches!(
        settled(&ctx, 1).await[0].last().unwrap().event,
        LoopEvent::Terminated {
            reason: TerminationReason::Timeout,
            ..
        }
    ));
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn absent_communication_policy_and_unknown_delegation_never_invoke_provider() {
    let (_project, provider, mut ctx) = context();
    ctx.agent_registry
        .as_ref()
        .unwrap()
        .spawn_agent("worker", "worker", vec![], None)
        .await;
    ctx.comm_policy = None;
    assert!(governed_ask(&ctx, "worker", "request", None)
        .await
        .unwrap_err()
        .to_string()
        .contains("configured policy gate"));
    let error = crate::dsl::reasoning_builtins::builtin_delegate(
        &[
            DslValue::String("unknown".into()),
            DslValue::String("request".into()),
        ],
        &ctx,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("Unknown agent"));
    assert!(provider.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn communication_enqueue_failure_and_audit_failure_cannot_report_success() {
    use symbi_runtime::communication::{
        CommunicationBus, CommunicationConfig, DefaultCommunicationBus,
    };
    let (_project, provider, mut ctx) = context();
    let recipient = ctx
        .agent_registry
        .as_ref()
        .unwrap()
        .spawn_agent("worker", "worker", vec![], None)
        .await;
    let sender = ctx.sender_agent_id.unwrap();
    let bus = Arc::new(
        DefaultCommunicationBus::new(CommunicationConfig {
            max_message_size: 8,
            ..Default::default()
        })
        .await
        .unwrap(),
    );
    bus.register_agent(sender).await.unwrap();
    bus.register_agent(recipient).await.unwrap();
    ctx.comm_bus = Some(bus.clone());
    // The request fits; the provider response does not. The response record must
    // already be durable, while the failed enqueue cannot become a success.
    assert!(governed_ask(&ctx, "worker", "ok", None)
        .await
        .unwrap_err()
        .to_string()
        .contains("response enqueue failed"));
    let entries = settled(&ctx, 1).await.remove(0);
    assert!(entries.iter().any(|entry| matches!(
        entry.event,
        LoopEvent::DirectInferenceResponseReceived { .. }
    )));
    assert!(matches!(
        entries.last().unwrap().event,
        LoopEvent::Terminated {
            reason: TerminationReason::Error { .. },
            ..
        }
    ));
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let messages = bus.receive_messages(recipient).await.unwrap();
            if !messages.is_empty() {
                assert_eq!(messages.len(), 1);
                assert_eq!(messages[0].payload.data.as_ref(), b"ok");
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(bus.receive_messages(sender).await.unwrap().is_empty());
    let mut conversation = Conversation::new();
    conversation.push(
        symbi_runtime::reasoning::conversation::ConversationMessage::user(
            "private threaded history exceeding the communication queue limit",
        ),
    );
    governed_ask_conversation(&ctx, "worker", &conversation)
        .await
        .unwrap();
    let threaded = settled(&ctx, 2).await.remove(1);
    assert!(threaded.iter().any(|entry| matches!(&entry.event,
        LoopEvent::DirectInferenceRequested { communication: Some(metadata), .. }
        if metadata["enqueue_request"] == false && metadata["message_hash"].is_null()
    )));
    assert!(bus.receive_messages(sender).await.unwrap().is_empty());
    assert!(bus.receive_messages(recipient).await.unwrap().is_empty());
    ctx.reasoning_journal = Some(Arc::new(FailingJournal {
        at: 1,
        writes: AtomicUsize::new(0),
        inner: BufferedJournal::new(16),
    }));
    assert!(governed_ask(&ctx, "worker", "again", None).await.is_err());
    assert_eq!(provider.requests.lock().unwrap().len(), 2);
    assert!(bus.receive_messages(recipient).await.unwrap().is_empty());
    bus.shutdown().await.unwrap();
}
