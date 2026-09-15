use super::*;
use crate::reasoning::{
    inference::{FinishReason, ToolCallRequest, Usage},
    protected_journal::ProtectedJournal,
    CedarPolicy, CedarPolicyGate,
};
use std::{
    io::Write,
    os::unix::fs::PermissionsExt,
    sync::atomic::{AtomicUsize, Ordering},
};

#[derive(Clone, Copy)]
enum Mode {
    Answer,
    Damage,
    Tool,
    Stall,
}
struct Provider {
    project: PathBuf,
    key: [u8; 32],
    calls: AtomicUsize,
    mode: Mode,
    entered: tokio::sync::Notify,
}
impl Provider {
    fn records(&self) -> Vec<JournalEntry> {
        let paths: Vec<_> = std::fs::read_dir(self.project.join(".symbiont/governed"))
            .unwrap()
            .map(Result::unwrap)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
            .collect();
        assert_eq!(paths.len(), 1);
        // This runs from inside the provider, so the run is still appending to
        // the journal. Verification requires a file that does not change while
        // it is read, so copy it first and verify that snapshot -- exactly what
        // the runtime tells callers to do for an in-flight run. The copy lives
        // outside .symbiont/governed so it is not picked up as a second journal.
        let snapshot = self.project.join("journal-snapshot.jsonl");
        std::fs::copy(&paths[0], &snapshot).unwrap();
        ProtectedJournal::verify(&snapshot, &self.key).unwrap()
    }
}
#[async_trait]
impl InferenceProvider for Provider {
    async fn complete(
        &self,
        conversation: &Conversation,
        options: &InferenceOptions,
    ) -> Result<InferenceResponse, InferenceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let records = self.records();
        let expected =
            digest_json(&json!({"conversation":conversation,"options":options})).unwrap();
        assert!(
            matches!(&records.last().unwrap().event,LoopEvent::DirectInferenceRequested {request_hash,..} if request_hash==&expected)
        );
        assert!(records
            .iter()
            .all(|record| record.agent_id == response_agent_id("declared")));
        assert!(options.tool_definitions.is_empty());
        if matches!(self.mode, Mode::Stall) {
            self.entered.notify_one();
            std::future::pending::<()>().await;
        }
        if matches!(self.mode, Mode::Damage) {
            let path = std::fs::read_dir(self.project.join(".symbiont/governed"))
                .unwrap()
                .map(Result::unwrap)
                .map(|entry| entry.path())
                .find(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
                .unwrap();
            std::fs::OpenOptions::new()
                .append(true)
                .open(path)
                .unwrap()
                .write_all(b"injected-storage-fault")
                .unwrap();
        }
        Ok(InferenceResponse {
            content: "private model response".into(),
            tool_calls: if matches!(self.mode, Mode::Tool) {
                vec![ToolCallRequest {
                    id: "call".into(),
                    name: "run_command".into(),
                    arguments: r#"{"command":"touch marker"}"#.into(),
                }]
            } else {
                vec![]
            },
            finish_reason: if matches!(self.mode, Mode::Tool) {
                FinishReason::ToolCalls
            } else {
                FinishReason::Stop
            },
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

async fn fixture(mode: Mode, deny: bool) -> (tempfile::TempDir, Arc<Provider>, ResponseRequest) {
    let root = tempfile::tempdir().unwrap();
    let storage = root.path().join(".symbiont/governed");
    std::fs::create_dir_all(&storage).unwrap();
    std::fs::set_permissions(&storage, std::fs::Permissions::from_mode(0o700)).unwrap();
    let key = ed25519_dalek::SigningKey::from_bytes(&[45; 32]);
    std::fs::write(storage.join("audit-signing.key"), key.to_bytes()).unwrap();
    std::fs::set_permissions(
        storage.join("audit-signing.key"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let provider = Arc::new(Provider {
        project: root.path().into(),
        key: key.verifying_key().to_bytes(),
        calls: AtomicUsize::new(0),
        mode,
        entered: tokio::sync::Notify::new(),
    });
    let gate = CedarPolicyGate::deny_by_default();
    gate.add_policy(CedarPolicy {
        name: "response".into(),
        source: if deny {
            "forbid(principal, action, resource);"
        } else {
            "permit(principal, action == Action::\"respond\", resource);"
        }
        .into(),
        active: true,
    })
    .await;
    let request = ResponseRequest {
        project: root.path().into(),
        agent: dsl::ConversationalAgent::parse("agent declared {}", "declared").unwrap(),
        surface: "mcp-server".into(),
        input: "fixture input".into(),
        caller_instructions: Some("caller guidance".into()),
        provider: provider.clone(),
        gate: Arc::new(gate),
        cancellation: tokio_util::sync::CancellationToken::new(),
    };
    (root, provider, request)
}

#[tokio::test]
async fn response_requires_durable_request_policy_and_terminal_records() {
    let (_root, provider, request) = fixture(Mode::Answer, false).await;
    let result = run_response(request).await;
    assert_eq!(result.result.unwrap(), "private model response");
    let audit = result.audit.unwrap();
    let records = ProtectedJournal::verify_run(&audit.path, &provider.key, audit.run_id).unwrap();
    assert!(matches!(
        records.last().unwrap().event,
        LoopEvent::Terminated {
            reason: TerminationReason::Completed,
            ..
        }
    ));
    assert!(records.iter().any(|entry| matches!(
        entry.event,
        LoopEvent::PolicyEvaluated {
            denied_count: 0,
            ..
        }
    )));
    let LoopEvent::Started {
        execution_context, ..
    } = &records[0].event
    else {
        panic!("missing startup")
    };
    assert_eq!(execution_context["agent_definition"]["name"], "declared");
    assert_eq!(
        execution_context["agent_definition"]["tools_enabled"],
        false
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn unsafe_storage_prevents_response_inference() {
    let (root, provider, request) = fixture(Mode::Answer, false).await;
    std::fs::set_permissions(
        root.path().join(".symbiont/governed"),
        std::fs::Permissions::from_mode(0o777),
    )
    .unwrap();
    assert!(run_response(request).await.result.is_err());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn denied_response_and_unadvertised_tools_are_withheld() {
    for (mode, deny) in [(Mode::Answer, true), (Mode::Tool, false)] {
        let (_root, provider, request) = fixture(mode, deny).await;
        let result = run_response(request).await;
        assert!(result.result.is_err());
        let audit = result.audit.unwrap();
        let records =
            ProtectedJournal::verify_run(&audit.path, &provider.key, audit.run_id).unwrap();
        assert!(!matches!(
            records.last().unwrap().event,
            LoopEvent::Terminated {
                reason: TerminationReason::Completed,
                ..
            }
        ));
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn required_response_append_failure_withholds_model_text() {
    let (_root, provider, request) = fixture(Mode::Damage, false).await;
    let result = run_response(request).await;
    assert!(result.result.is_err());
    let audit = result.audit.unwrap();
    assert!(ProtectedJournal::verify_run(&audit.path, &provider.key, audit.run_id).is_err());
}

#[tokio::test]
async fn dropped_response_caller_retains_terminal_cancellation() {
    let (_root, provider, request) = fixture(Mode::Stall, false).await;
    let task = tokio::spawn(run_response(request));
    tokio::time::timeout(Duration::from_secs(2), provider.entered.notified())
        .await
        .unwrap();
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if matches!(
                provider.records().last().unwrap().event,
                LoopEvent::Terminated {
                    reason: TerminationReason::Error { .. },
                    ..
                }
            ) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[derive(Clone, Copy)]
enum DeliveryMode {
    Confirm,
    Reject,
    Damage,
    Stall,
}
struct Destination {
    provider: Arc<Provider>,
    mode: DeliveryMode,
    calls: AtomicUsize,
    entered: tokio::sync::Notify,
}
#[async_trait]
impl super::super::response_delivery::ResponseDestination for Destination {
    fn context(&self) -> Value {
        json!({"channel":"C1"})
    }
    fn validate(&self) -> Result<(), String> {
        Ok(())
    }
    fn prepare(&self, content: &str) -> Result<Value, String> {
        Ok(json!({"channel":"C1", "formatted":{"content":content,"header":"declared"}}))
    }
    async fn send(
        &self,
        request: &Value,
    ) -> Result<super::super::response_delivery::DeliveryReceipt, String> {
        let records = self.provider.records();
        let LoopEvent::ResponseDeliveryStarted {
            fingerprint,
            request_hash,
            ..
        } = &records.last().unwrap().event
        else {
            panic!("send without durable start")
        };
        assert_eq!(request_hash, &digest_json(request).unwrap());
        assert!(records.iter().any(|record| matches!(&record.event,
            LoopEvent::PolicyEvaluated { approved_calls, .. } if approved_calls.iter().any(|call|
                call["fingerprint"] == *fingerprint
                && call["resolved"]["response_delivery"] == *request))));
        assert_eq!(request["formatted"]["content"], "private model response");
        self.calls.fetch_add(1, Ordering::SeqCst);
        if matches!(self.mode, DeliveryMode::Stall) {
            self.entered.notify_one();
            std::future::pending::<()>().await;
        }
        if matches!(self.mode, DeliveryMode::Damage) {
            let path = std::fs::read_dir(self.provider.project.join(".symbiont/governed"))
                .unwrap()
                .map(Result::unwrap)
                .map(|e| e.path())
                .find(|p| p.extension().is_some_and(|e| e == "jsonl"))
                .unwrap();
            std::fs::OpenOptions::new()
                .append(true)
                .open(path)
                .unwrap()
                .write_all(b"receipt-storage-fault")
                .unwrap();
        }
        let confirmed = !matches!(self.mode, DeliveryMode::Reject);
        Ok(super::super::response_delivery::DeliveryReceipt {
            receipt: json!({"channel":"C1", "success":confirmed}),
            confirmed,
        })
    }
}
fn destination(provider: Arc<Provider>, mode: DeliveryMode) -> Arc<Destination> {
    Arc::new(Destination {
        provider,
        mode,
        calls: AtomicUsize::new(0),
        entered: tokio::sync::Notify::new(),
    })
}

#[tokio::test]
async fn delivery_requires_exact_formatted_policy_and_durable_receipt_before_completion() {
    let (_root, provider, mut request) = fixture(Mode::Answer, false).await;
    let gate = CedarPolicyGate::deny_by_default();
    gate.add_policy(CedarPolicy {
        name: "destination".into(), active: true,
        source: r#"permit(principal, action == Action::"respond", resource) when { context.invocation.resolved.response_delivery.channel == "C1" };"#.into(),
    }).await;
    request.gate = Arc::new(gate);
    let destination = destination(provider.clone(), DeliveryMode::Confirm);
    let outcome = run_response_to(request, destination.clone()).await;
    assert!(outcome.result.is_ok(), "{:?}", outcome.result);
    assert_eq!(destination.calls.load(Ordering::SeqCst), 1);
    let records = provider.records();
    assert!(records.iter().any(|entry| matches!(
        entry.event,
        LoopEvent::ResponseDeliveryFinished {
            confirmed: true,
            ..
        }
    )));
    assert!(matches!(
        records.last().unwrap().event,
        LoopEvent::Terminated {
            reason: TerminationReason::Completed,
            ..
        }
    ));
}

#[tokio::test]
async fn delivery_policy_checks_destination_and_audit_failure_prevents_send() {
    for damage in [false, true] {
        let (_root, provider, mut request) =
            fixture(if damage { Mode::Damage } else { Mode::Answer }, false).await;
        let gate = CedarPolicyGate::deny_by_default();
        gate.add_policy(CedarPolicy {name:"destination".into(), active:true, source:r#"permit(principal, action == Action::"respond", resource) when { context.invocation.resolved.response_delivery.channel == "C2" };"#.into()}).await;
        request.gate = Arc::new(gate);
        let destination = destination(provider, DeliveryMode::Confirm);
        assert!(run_response_to(request, destination.clone())
            .await
            .result
            .is_err());
        assert_eq!(destination.calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn rejected_or_unrecorded_delivery_never_reports_success() {
    for mode in [DeliveryMode::Reject, DeliveryMode::Damage] {
        let (_root, provider, request) = fixture(Mode::Answer, false).await;
        let destination = destination(provider.clone(), mode);
        assert!(run_response_to(request, destination.clone())
            .await
            .result
            .is_err());
        assert_eq!(destination.calls.load(Ordering::SeqCst), 1);
        if matches!(mode, DeliveryMode::Reject) {
            assert!(provider.records().iter().any(|entry| matches!(
                entry.event,
                LoopEvent::ResponseDeliveryFinished {
                    confirmed: false,
                    ..
                }
            )));
        }
    }
}

#[tokio::test]
async fn cancellation_during_send_retains_terminal_audit_and_unconfirmed_delivery() {
    let (_root, provider, request) = fixture(Mode::Answer, false).await;
    let destination = destination(provider.clone(), DeliveryMode::Stall);
    let task = tokio::spawn(run_response_to(request, destination.clone()));
    tokio::time::timeout(Duration::from_secs(2), destination.entered.notified())
        .await
        .unwrap();
    task.abort();
    assert!(matches!(task.await,Err(error) if error.is_cancelled()));
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let records = provider.records();
            if matches!(
                records.last().unwrap().event,
                LoopEvent::Terminated {
                    reason: TerminationReason::Error { .. },
                    ..
                }
            ) {
                assert!(!records.iter().any(|entry| matches!(
                    entry.event,
                    LoopEvent::ResponseDeliveryFinished { .. }
                )));
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
