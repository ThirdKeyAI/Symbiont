//! Actual WebSocket ingress, governed coordinator turns and signed audit.
//! Local provider/runtime fixtures expose no external service or tool process.
use super::{
    api_keys::{ApiKeyRecord, ApiKeyStore},
    coordinator::CoordinatorState,
    traits::RuntimeApiProvider,
    types::*,
    ws_handler::ws_chat_handler,
    ws_types::ServerMessage,
};
use crate::{
    reasoning::{
        conversation::{Conversation, MessageRole},
        inference::*,
        loop_types::{JournalEntry, LoopEvent, TerminationReason},
        protected_journal::ProtectedJournal,
        run_audit::RunAuditReference,
        CedarPolicy, CedarPolicyGate,
    },
    types::{AgentId, RuntimeError},
};
use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use serde_json::json;
use std::{
    io::Write,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::Notify,
};
use tokio_tungstenite::{tungstenite::Message, MaybeTlsStream, WebSocketStream};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy)]
enum Mode {
    Normal,
    DamageJournal,
    Stall,
    Delegate,
    DelegateStall,
    DelegateUnsafeStorage,
    DelegateDamageJournal,
    DelegateInlineDenied,
}
impl Mode {
    fn delegates(self) -> bool {
        matches!(
            self,
            Self::Delegate
                | Self::DelegateStall
                | Self::DelegateUnsafeStorage
                | Self::DelegateDamageJournal
                | Self::DelegateInlineDenied
        )
    }
}
struct Probe {
    project: PathBuf,
    key: [u8; 32],
    mode: Mode,
    inference_calls: AtomicUsize,
    api_calls: AtomicUsize,
    entered: Notify,
    cancelled: AtomicBool,
}
impl Probe {
    fn active_journal(&self) -> (PathBuf, Vec<JournalEntry>) {
        let active: Vec<_> = std::fs::read_dir(self.project.join(".symbiont/governed"))
            .unwrap()
            .map(Result::unwrap)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
            .filter_map(|path| {
                let entries = ProtectedJournal::verify(&path, &self.key).unwrap();
                (!matches!(entries.last().unwrap().event, LoopEvent::Terminated { .. }))
                    .then_some((path, entries))
            })
            .collect();
        if active.len() == 2 && self.mode.delegates() {
            let child = crate::reasoning::delegation_executor::delegated_agent_id("reviewer");
            return active
                .into_iter()
                .find(|(_, entries)| entries[0].agent_id == child)
                .unwrap();
        }
        assert_eq!(
            active.len(),
            1,
            "one active signed invocation or a linked parent and child"
        );
        active.into_iter().next().unwrap()
    }
}
struct Cancelled(Arc<Probe>);
impl Drop for Cancelled {
    fn drop(&mut self) {
        self.0.cancelled.store(true, Ordering::SeqCst);
    }
}
struct Provider(Arc<Probe>);
#[async_trait]
impl InferenceProvider for Provider {
    async fn complete(
        &self,
        conversation: &Conversation,
        _: &InferenceOptions,
    ) -> Result<InferenceResponse, InferenceError> {
        self.0.inference_calls.fetch_add(1, Ordering::SeqCst);
        let (path, entries) = self.0.active_journal();
        let is_child = conversation
            .messages()
            .first()
            .unwrap()
            .content
            .starts_with("You are agent 'reviewer'.");
        assert!(
            matches!(entries[0].event, LoopEvent::Started { .. }),
            "startup must be durable before inference"
        );
        if is_child {
            let LoopEvent::Started {
                execution_context, ..
            } = &entries[0].event
            else {
                panic!("child start")
            };
            assert_eq!(execution_context["agent_definition"]["name"], "reviewer");
            assert_eq!(execution_context["source_policy"]["agent_name"], "reviewer");
            let link = &execution_context["delegation"];
            assert_eq!(link["target"], "reviewer");
            let parent = std::fs::read_dir(self.0.project.join(".symbiont/governed"))
                .unwrap()
                .map(Result::unwrap)
                .map(|entry| entry.path())
                .find(|candidate| {
                    candidate.extension().is_some_and(|ext| ext == "jsonl")
                        && candidate
                            .file_name()
                            .unwrap()
                            .to_str()
                            .unwrap()
                            .starts_with(link["parent_agent_id"].as_str().unwrap())
                        && ProtectedJournal::verify(candidate, &self.0.key)
                            .unwrap()
                            .last()
                            .is_some_and(|entry| {
                                !matches!(entry.event, LoopEvent::Terminated { .. })
                            })
                })
                .expect("parent link persisted before child inference");
            let parent_entries = ProtectedJournal::verify(&parent, &self.0.key).unwrap();
            let parent_index = parent_entries
                .iter()
                .rposition(|entry| matches!(entry.event, LoopEvent::DelegationStarted { .. }))
                .unwrap();
            let parent_link = serde_json::to_value(&parent_entries[parent_index].event).unwrap();
            for (key, value) in parent_link["DelegationStarted"].as_object().unwrap() {
                assert_eq!(&link[key], value);
            }
            assert!(matches!(
                parent_entries[parent_index - 1].event,
                LoopEvent::PolicyEvaluated { .. }
            ));
        }
        if matches!(self.0.mode, Mode::Stall)
            || (is_child && matches!(self.0.mode, Mode::DelegateStall))
        {
            let _cancelled = Cancelled(self.0.clone());
            self.0.entered.notify_one();
            std::future::pending::<()>().await;
        }
        if matches!(self.0.mode, Mode::DamageJournal)
            || (is_child && matches!(self.0.mode, Mode::DelegateDamageJournal))
        {
            std::fs::OpenOptions::new()
                .append(true)
                .open(path)
                .unwrap()
                .write_all(b"injected-storage-fault")
                .unwrap();
        }
        if !is_child && matches!(self.0.mode, Mode::DelegateUnsafeStorage) {
            std::fs::set_permissions(
                self.0.project.join(".symbiont/governed"),
                std::fs::Permissions::from_mode(0o777),
            )
            .unwrap();
        }
        let tool = conversation
            .messages()
            .iter()
            .rev()
            .find(|message| message.role == MessageRole::Tool);
        Ok(InferenceResponse {
            content: tool
                .map(|message| message.content.clone())
                .unwrap_or_default(),
            tool_calls: if tool.is_none() {
                vec![ToolCallRequest {
                    id: if self.0.mode.delegates() && !is_child {
                        "delegate-call"
                    } else {
                        "health-call"
                    }
                    .into(),
                    name: if self.0.mode.delegates() && !is_child {
                        "delegate"
                    } else {
                        "system_health"
                    }
                    .into(),
                    arguments: if self.0.mode.delegates() && !is_child {
                        r#"{"agent":"alias","task":"health"}"#
                    } else {
                        "{}"
                    }
                    .into(),
                }]
            } else {
                vec![]
            },
            finish_reason: if tool.is_none() {
                FinishReason::ToolCalls
            } else {
                FinishReason::Stop
            },
            usage: Usage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: 2,
            },
            model: "local-coordinator-fixture".into(),
        })
    }
    fn provider_name(&self) -> &str {
        "local-fixture"
    }
    fn default_model(&self) -> &str {
        "local-fixture"
    }
    fn supports_native_tools(&self) -> bool {
        true
    }
    fn supports_structured_output(&self) -> bool {
        false
    }
}

struct FixtureRuntime(Arc<Probe>);
macro_rules! fixture_runtime {
    ($($name:ident($($arg:ty),*) -> $ret:ty;)*) => {
        #[async_trait]
        impl RuntimeApiProvider for FixtureRuntime {
            async fn get_system_health(&self) -> Result<serde_json::Value, RuntimeError> {
                let (_, entries) = self.0.active_journal();
                let dispatch = entries.len()-1;
                assert!(matches!(&entries[dispatch].event, LoopEvent::ToolDispatchStarted {tool_name, call_id, ..} if tool_name=="system_health" && call_id=="health-call"), "exact dispatch must be durable before the API effect: {:?}",entries[dispatch].event);
                assert!(entries[..dispatch].iter().any(|entry| matches!(entry.event, LoopEvent::PolicyEvaluated { action_count:1, denied_count:0, .. })), "policy must be durable before the API effect");
                self.0.api_calls.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"status":"fixture-healthy"}))
            }
            $(async fn $name(&self, $(_: $arg),*) -> Result<$ret, RuntimeError> { panic!("unexpected runtime API: {}", stringify!($name)); })*
        }
    }
}
fixture_runtime! {
    execute_workflow(WorkflowExecutionRequest) -> serde_json::Value;
    get_agent_status(AgentId) -> AgentStatusResponse;
    list_agents() -> Vec<AgentId>;
    shutdown_agent(AgentId) -> ();
    get_metrics() -> serde_json::Value;
    create_agent(CreateAgentRequest) -> CreateAgentResponse;
    update_agent(AgentId, UpdateAgentRequest) -> UpdateAgentResponse;
    delete_agent(AgentId) -> DeleteAgentResponse;
    execute_agent(AgentId, ExecuteAgentRequest) -> ExecuteAgentResponse;
    get_agent_history(AgentId) -> GetAgentHistoryResponse;
    list_schedules() -> Vec<ScheduleSummary>;
    create_schedule(CreateScheduleRequest) -> CreateScheduleResponse;
    get_schedule(&str) -> ScheduleDetail;
    update_schedule(&str, UpdateScheduleRequest) -> ScheduleDetail;
    delete_schedule(&str) -> DeleteScheduleResponse;
    pause_schedule(&str) -> ScheduleActionResponse;
    resume_schedule(&str) -> ScheduleActionResponse;
    trigger_schedule(&str) -> ScheduleActionResponse;
    get_schedule_history(&str, usize) -> ScheduleHistoryResponse;
    get_schedule_next_runs(&str, usize) -> NextRunsResponse;
    get_scheduler_health() -> SchedulerHealthResponse;
    list_channels() -> Vec<ChannelSummary>;
    register_channel(RegisterChannelRequest) -> RegisterChannelResponse;
    get_channel(&str) -> ChannelDetail;
    update_channel(&str, UpdateChannelRequest) -> ChannelDetail;
    delete_channel(&str) -> DeleteChannelResponse;
    start_channel(&str) -> ChannelActionResponse;
    stop_channel(&str) -> ChannelActionResponse;
    get_channel_health(&str) -> ChannelHealthResponse;
    list_channel_mappings(&str) -> Vec<IdentityMappingEntry>;
    add_channel_mapping(&str, AddIdentityMappingRequest) -> IdentityMappingEntry;
    remove_channel_mapping(&str, &str) -> ();
    get_channel_audit(&str, usize) -> ChannelAuditResponse;
    update_agent_heartbeat(AgentId, HeartbeatRequest) -> ();
    push_agent_event(AgentId, PushEventRequest) -> ();
    send_agent_message(AgentId, SendMessageRequest) -> SendMessageResponse;
    receive_agent_messages(AgentId) -> ReceiveMessagesResponse;
    get_message_status(&str) -> MessageStatusResponse;
}

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
struct Fixture {
    _root: tempfile::TempDir,
    state: Arc<CoordinatorState>,
    probe: Arc<Probe>,
    address: std::net::SocketAddr,
    stop: CancellationToken,
    server: tokio::task::JoinHandle<std::io::Result<()>>,
}
impl Fixture {
    async fn start(mode: Mode, unsafe_storage: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let audit = root.path().join(".symbiont/governed");
        std::fs::create_dir_all(&audit).unwrap();
        std::fs::set_permissions(
            &audit,
            std::fs::Permissions::from_mode(if unsafe_storage { 0o777 } else { 0o700 }),
        )
        .unwrap();
        // Synthetic fixture key, pinned independently of the WebSocket reply.
        let key = ed25519_dalek::SigningKey::from_bytes(&[31u8; 32]);
        let key_path = audit.join("audit-signing.key");
        std::fs::write(&key_path, key.to_bytes()).unwrap();
        std::fs::set_permissions(key_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let probe = Arc::new(Probe {
            project: root.path().into(),
            key: key.verifying_key().to_bytes(),
            mode,
            inference_calls: AtomicUsize::new(0),
            api_calls: AtomicUsize::new(0),
            entered: Notify::new(),
            cancelled: AtomicBool::new(false),
        });
        let gate = CedarPolicyGate::deny_by_default();
        gate.add_policy(CedarPolicy {
            name: "health".into(),
            source: "permit(principal, action in [Action::\"tool_call::system_health\", Action::\"respond\"], resource);".into(),
            active: true,
        }).await;
        if mode.delegates() {
            gate.add_policy(CedarPolicy {
                name: "delegate".into(),
                source: "permit(principal, action == Action::\"delegate::reviewer\", resource);"
                    .into(),
                active: true,
            })
            .await;
        }
        let mut state = CoordinatorState::new(
            Arc::new(Provider(probe.clone())),
            Arc::new(gate),
            Arc::new(FixtureRuntime(probe.clone())),
        )
        .with_audit_project(root.path());
        if mode.delegates() {
            let source = if matches!(mode, Mode::DelegateInlineDenied) {
                "agent reviewer { policy restricted { deny: \"system_health\" } }"
            } else {
                "agent reviewer {}"
            };
            state = state.with_registered_delegation(
                crate::reasoning::delegation_executor::RegisteredDelegationRegistry::from_sources(
                    vec![("alias.symbi".into(), source.into())],
                )
                .unwrap(),
            );
        }
        state.loop_config.timeout = Duration::from_secs(10);
        let state = Arc::new(state);
        let keys = root.path().join("api-keys.json");
        let record = ApiKeyRecord {
            key_id: "fixture".into(),
            key_hash: ApiKeyStore::hash_key("test-only-secret").unwrap(),
            agent_scope: None,
            description: "Local fixture".into(),
            created_at: "2026-01-01".into(),
            revoked: false,
        };
        let scoped = ApiKeyRecord {
            key_id: "scoped".into(),
            agent_scope: Some(vec![AgentId::new().to_string()]),
            ..record.clone()
        };
        let other = ApiKeyRecord {
            key_id: "other".into(),
            ..record.clone()
        };
        std::fs::write(
            &keys,
            serde_json::to_vec(&vec![record, scoped, other]).unwrap(),
        )
        .unwrap();
        std::fs::set_permissions(&keys, std::fs::Permissions::from_mode(0o600)).unwrap();
        let store = Arc::new(ApiKeyStore::load_from_file(&keys).unwrap());
        let router = axum::Router::new()
            .route("/ws/chat", axum::routing::get(ws_chat_handler))
            .with_state(state.clone())
            .layer(axum::Extension(store));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let stop = CancellationToken::new();
        let stopped = stop.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(stopped.cancelled_owned())
                .await
        });
        Self {
            _root: root,
            state,
            probe,
            address,
            stop,
            server,
        }
    }
    async fn connect(&self) -> Socket {
        tokio_tungstenite::connect_async(format!(
            "ws://{}/ws/chat?token=fixture.test-only-secret",
            self.address
        ))
        .await
        .unwrap()
        .0
    }
    async fn finish(self) {
        self.stop.cancel();
        tokio::time::timeout(Duration::from_secs(2), self.server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while Arc::strong_count(&self.state) != 1 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("connection and session ownership released");
    }
}
async fn send(socket: &mut Socket) {
    socket
        .send(Message::Text(
            json!({"type":"ChatSend","id":uuid::Uuid::new_v4().to_string(),"content":"Inspect system health"})
                .to_string(),
        ))
        .await
        .unwrap();
}
async fn next(socket: &mut Socket) -> ServerMessage {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Message::Text(text) = message {
            let message = serde_json::from_str(&text).unwrap();
            if !matches!(message, ServerMessage::Pong) {
                return message;
            }
        }
    }
}
async fn audit(socket: &mut Socket, fixture: &Fixture) -> RunAuditReference {
    let ServerMessage::AuditOpened { audit, .. } = next(socket).await else {
        panic!("audit reference before turn output");
    };
    assert_eq!(audit.public_key, hex::encode(fixture.probe.key));
    assert!(audit.path.starts_with(&fixture.probe.project));
    audit
}

#[tokio::test]
async fn websocket_turns_persist_before_effects_and_release_connection_ownership() {
    let fixture = Fixture::start(Mode::Normal, false).await;
    for token in ["", "scoped.test-only-secret", "fixture.wrong-secret"] {
        let denied = tokio_tungstenite::connect_async(format!(
            "ws://{}/ws/chat?token={token}",
            fixture.address
        ))
        .await;
        assert!(
            matches!(denied,Err(tokio_tungstenite::tungstenite::Error::Http(response)) if response.status()==401)
        );
    }
    let mut socket = fixture.connect().await;
    let mut runs = Vec::new();
    for _ in 0..2 {
        send(&mut socket).await;
        let audit = audit(&mut socket, &fixture).await;
        loop {
            match next(&mut socket).await {
                ServerMessage::ChatChunk { content, done, .. } => {
                    assert!(done && content.contains("fixture-healthy"));
                    break;
                }
                ServerMessage::Error { message, .. } => panic!("{message}"),
                _ => {}
            }
        }
        let entries =
            ProtectedJournal::verify_run(&audit.path, &fixture.probe.key, audit.run_id).unwrap();
        assert!(entries
            .iter()
            .all(|entry| entry.agent_id == fixture.state.knowledge_agent_id));
        assert!(matches!(
            entries.last().unwrap().event,
            LoopEvent::Terminated {
                reason: TerminationReason::Completed,
                ..
            }
        ));
        runs.push(audit.run_id);
    }
    assert_ne!(runs[0], runs[1]);
    assert_eq!(fixture.probe.api_calls.load(Ordering::SeqCst), 2);
    socket.close(None).await.unwrap();
    drop(socket);
    fixture.finish().await;
}

#[tokio::test]
async fn unsafe_storage_prevents_websocket_inference() {
    let fixture = Fixture::start(Mode::Normal, true).await;
    let mut socket = fixture.connect().await;
    send(&mut socket).await;
    assert!(
        matches!(next(&mut socket).await,ServerMessage::Error {code,..} if code=="AUDIT_UNAVAILABLE")
    );
    assert_eq!(fixture.probe.inference_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.probe.api_calls.load(Ordering::SeqCst), 0);
    socket.close(None).await.unwrap();
    drop(socket);
    fixture.finish().await;
}

#[tokio::test]
async fn required_append_failure_prevents_the_proposed_runtime_action() {
    let fixture = Fixture::start(Mode::DamageJournal, false).await;
    let mut socket = fixture.connect().await;
    send(&mut socket).await;
    let audit = audit(&mut socket, &fixture).await;
    assert!(
        matches!(next(&mut socket).await,ServerMessage::Error {code,..} if code=="INVOCATION_UNRESOLVED")
    );
    assert_eq!(fixture.probe.inference_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.probe.api_calls.load(Ordering::SeqCst), 0);
    assert!(ProtectedJournal::verify_run(&audit.path, &fixture.probe.key, audit.run_id).is_err());
    socket.close(None).await.unwrap();
    drop(socket);
    fixture.finish().await;
}

#[tokio::test]
async fn websocket_disconnect_cancels_inference_and_retains_terminal_audit() {
    let fixture = Fixture::start(Mode::Stall, false).await;
    let mut socket = fixture.connect().await;
    send(&mut socket).await;
    let audit = audit(&mut socket, &fixture).await;
    tokio::time::timeout(Duration::from_secs(5), fixture.probe.entered.notified())
        .await
        .unwrap();
    socket
        .send(Message::Text(
            json!({"type":"ChatSend","id":"large","content":"x".repeat(64*1024+1)}).to_string(),
        ))
        .await
        .unwrap();
    assert!(
        matches!(next(&mut socket).await,ServerMessage::Error {code,..} if code=="MESSAGE_TOO_LARGE")
    );
    send(&mut socket).await; // One bounded queued request is durably admitted.
    assert!(matches!(
        next(&mut socket).await,
        ServerMessage::AuditOpened { .. }
    ));
    send(&mut socket).await;
    assert!(
        matches!(next(&mut socket).await,ServerMessage::Error {code,..} if code=="SESSION_BUSY")
    );
    socket.close(None).await.unwrap();
    drop(socket);
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let entries =
                ProtectedJournal::verify_run(&audit.path, &fixture.probe.key, audit.run_id)
                    .unwrap();
            if matches!(
                entries.last().unwrap().event,
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
    .expect("disconnect writes a terminal cancellation before the 10-second run deadline");
    assert!(fixture.probe.cancelled.load(Ordering::SeqCst));
    assert_eq!(
        fixture.probe.inference_calls.load(Ordering::SeqCst),
        1,
        "queued turn cannot start after disconnect"
    );
    assert_eq!(fixture.probe.api_calls.load(Ordering::SeqCst), 0);
    fixture.finish().await;
}

#[tokio::test]
async fn delegated_turns_link_distinct_child_runs_before_inference() {
    let fixture = Fixture::start(Mode::Delegate, false).await;
    let mut socket = fixture.connect().await;
    let mut child_runs = Vec::new();
    for _ in 0..2 {
        send(&mut socket).await;
        let root = audit(&mut socket, &fixture).await;
        loop {
            match next(&mut socket).await {
                ServerMessage::ChatChunk { content, done, .. } => {
                    assert!(done && content.contains("fixture-healthy"));
                    break;
                }
                ServerMessage::Error { message, .. } => panic!("{message}"),
                _ => {}
            }
        }
        let records =
            ProtectedJournal::verify_run(&root.path, &fixture.probe.key, root.run_id).unwrap();
        let child = records
            .iter()
            .find_map(|entry| match &entry.event {
                LoopEvent::DelegationStarted {
                    audit,
                    call_id,
                    child_agent_id,
                    ..
                } => {
                    assert_eq!(call_id, "delegate-call");
                    assert_eq!(
                        *child_agent_id,
                        crate::reasoning::delegation_executor::delegated_agent_id("reviewer")
                    );
                    Some(audit)
                }
                _ => None,
            })
            .unwrap();
        let child_records =
            ProtectedJournal::verify_run(&child.path, &fixture.probe.key, child.run_id).unwrap();
        assert_ne!(child.run_id, root.run_id);
        assert!(matches!(
            child_records.last().unwrap().event,
            LoopEvent::Terminated {
                reason: TerminationReason::Completed,
                ..
            }
        ));
        let finished = records
            .iter()
            .find(|entry| matches!(entry.event, LoopEvent::DelegationFinished { .. }))
            .unwrap();
        assert!(child_records.last().unwrap().timestamp <= finished.timestamp);
        assert!(matches!(
            records.last().unwrap().event,
            LoopEvent::Terminated {
                reason: TerminationReason::Completed,
                ..
            }
        ));
        child_runs.push(child.run_id);
    }
    assert_ne!(child_runs[0], child_runs[1]);
    assert_eq!(fixture.probe.api_calls.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.probe.inference_calls.load(Ordering::SeqCst), 8);
    socket.close(None).await.unwrap();
    drop(socket);
    fixture.finish().await;
}

#[tokio::test]
async fn delegated_disconnect_finishes_child_audit_before_parent_termination() {
    let fixture = Fixture::start(Mode::DelegateStall, false).await;
    let mut socket = fixture.connect().await;
    send(&mut socket).await;
    let root = audit(&mut socket, &fixture).await;
    tokio::time::timeout(Duration::from_secs(2), fixture.probe.entered.notified())
        .await
        .unwrap();
    socket.close(None).await.unwrap();
    drop(socket);
    let records = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let records =
                ProtectedJournal::verify_run(&root.path, &fixture.probe.key, root.run_id).unwrap();
            if matches!(records.last().unwrap().event, LoopEvent::Terminated { .. }) {
                break records;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let child = records
        .iter()
        .find_map(|entry| match &entry.event {
            LoopEvent::DelegationStarted { audit, .. } => Some(audit),
            _ => None,
        })
        .unwrap();
    let child_records =
        ProtectedJournal::verify_run(&child.path, &fixture.probe.key, child.run_id).unwrap();
    assert!(matches!(
        child_records.last().unwrap().event,
        LoopEvent::Terminated {
            reason: TerminationReason::Error { .. },
            ..
        }
    ));
    assert!(matches!(
        records.last().unwrap().event,
        LoopEvent::Terminated {
            reason: TerminationReason::Error { .. },
            ..
        }
    ));
    assert!(child_records.last().unwrap().timestamp <= records.last().unwrap().timestamp);
    assert!(fixture.probe.cancelled.load(Ordering::SeqCst));
    assert_eq!(fixture.probe.inference_calls.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.probe.api_calls.load(Ordering::SeqCst), 0);
    fixture.finish().await;
}

#[tokio::test]
async fn delegated_storage_failure_prevents_child_inference_and_parent_success() {
    let fixture = Fixture::start(Mode::DelegateUnsafeStorage, false).await;
    let mut socket = fixture.connect().await;
    send(&mut socket).await;
    let root = audit(&mut socket, &fixture).await;
    loop {
        match next(&mut socket).await {
            ServerMessage::Error { code, .. } => {
                assert_eq!(code, "LOOP_ERROR");
                break;
            }
            ServerMessage::ChatChunk { .. } => panic!("audit failure cannot succeed"),
            _ => {}
        }
    }
    assert_eq!(fixture.probe.inference_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.probe.api_calls.load(Ordering::SeqCst), 0);
    let records =
        ProtectedJournal::verify_run(&root.path, &fixture.probe.key, root.run_id).unwrap();
    assert!(matches!(
        records.last().unwrap().event,
        LoopEvent::Terminated {
            reason: TerminationReason::Error { .. },
            ..
        }
    ));
    socket.close(None).await.unwrap();
    drop(socket);
    fixture.finish().await;
}

#[tokio::test]
async fn delegated_append_failure_prevents_child_action_and_parent_resume() {
    let fixture = Fixture::start(Mode::DelegateDamageJournal, false).await;
    let mut socket = fixture.connect().await;
    send(&mut socket).await;
    let root = audit(&mut socket, &fixture).await;
    loop {
        match next(&mut socket).await {
            ServerMessage::Error { code, .. } => {
                assert_eq!(code, "INVOCATION_UNRESOLVED");
                break;
            }
            ServerMessage::ChatChunk { .. } => panic!("child audit failure cannot succeed"),
            _ => {}
        }
    }
    assert_eq!(fixture.probe.inference_calls.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.probe.api_calls.load(Ordering::SeqCst), 0);
    let records =
        ProtectedJournal::verify_run(&root.path, &fixture.probe.key, root.run_id).unwrap();
    let child = records
        .iter()
        .find_map(|entry| match &entry.event {
            LoopEvent::DelegationStarted { audit, .. } => Some(audit),
            _ => None,
        })
        .unwrap();
    assert!(ProtectedJournal::verify_run(&child.path, &fixture.probe.key, child.run_id).is_err());
    assert!(matches!(
        records.last().unwrap().event,
        LoopEvent::Terminated {
            reason: TerminationReason::Error { .. },
            ..
        }
    ));
    socket.close(None).await.unwrap();
    drop(socket);
    fixture.finish().await;
}

#[tokio::test]
async fn delegated_inline_policy_prevents_the_runtime_api_effect() {
    let fixture = Fixture::start(Mode::DelegateInlineDenied, false).await;
    let mut socket = fixture.connect().await;
    send(&mut socket).await;
    let root = audit(&mut socket, &fixture).await;
    loop {
        match next(&mut socket).await {
            ServerMessage::ChatChunk { content, done, .. } => {
                assert!(
                    done && content.contains("inline policy restricted"),
                    "{content}"
                );
                break;
            }
            ServerMessage::Error { message, .. } => panic!("{message}"),
            _ => {}
        }
    }
    assert_eq!(fixture.probe.api_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.probe.inference_calls.load(Ordering::SeqCst), 4);
    let records =
        ProtectedJournal::verify_run(&root.path, &fixture.probe.key, root.run_id).unwrap();
    let child = records
        .iter()
        .find_map(|entry| match &entry.event {
            LoopEvent::DelegationStarted { audit, .. } => Some(audit),
            _ => None,
        })
        .unwrap();
    let child_records =
        ProtectedJournal::verify_run(&child.path, &fixture.probe.key, child.run_id).unwrap();
    assert!(child_records.iter().any(|entry| matches!(&entry.event, LoopEvent::PolicyEvaluated { denied_calls, .. } if denied_calls.iter().any(|call| call["reason"].as_str().is_some_and(|reason| reason.contains("inline policy restricted"))))));
    socket.close(None).await.unwrap();
    drop(socket);
    fixture.finish().await;
}

async fn identified(socket: &mut Socket, id: uuid::Uuid, content: &str, inspect: bool) {
    socket
        .send(Message::Text(
            json!({"type":if inspect {"ChatInspect"} else {"ChatSend"},"id":id,"content":content})
                .to_string(),
        ))
        .await
        .unwrap();
}

async fn completed_chat(socket: &mut Socket) -> (RunAuditReference, String, bool) {
    let ServerMessage::AuditOpened { audit, .. } = next(socket).await else {
        panic!("durable admission receipt")
    };
    loop {
        match next(socket).await {
            ServerMessage::ChatChunk {
                content,
                done: true,
                replayed,
                ..
            } => return (audit, content, replayed),
            ServerMessage::Error { code, message, .. } => panic!("{code}: {message}"),
            _ => {}
        }
    }
}

#[tokio::test]
async fn chat_ids_return_saved_results_across_connections_and_bind_the_caller_and_content() {
    let fixture = Fixture::start(Mode::Normal, false).await;
    let mut socket = fixture.connect().await;
    let id = uuid::Uuid::new_v4();
    identified(&mut socket, id, "Inspect system health", false).await;
    let (audit, output, replayed) = completed_chat(&mut socket).await;
    assert!(!replayed && output.contains("fixture-healthy"));
    let original = std::fs::read(&audit.path).unwrap();
    for inspect in [false, true] {
        identified(&mut socket, id, "Inspect system health", inspect).await;
        let (saved_audit, saved_output, replayed) = completed_chat(&mut socket).await;
        assert!(replayed);
        assert_eq!(saved_audit.run_id, audit.run_id);
        assert_eq!(saved_output, output);
    }
    identified(&mut socket, id, "Different request", false).await;
    assert!(
        matches!(next(&mut socket).await, ServerMessage::Error {code,..} if code=="INVOCATION_CONFLICT")
    );
    socket.close(None).await.unwrap();
    drop(socket);
    let mut reconnected = fixture.connect().await;
    identified(&mut reconnected, id, "Inspect system health", true).await;
    assert!(completed_chat(&mut reconnected).await.2);
    let mut other = tokio_tungstenite::connect_async(format!(
        "ws://{}/ws/chat?token=other.test-only-secret",
        fixture.address
    ))
    .await
    .unwrap()
    .0;
    identified(&mut other, id, "Inspect system health", true).await;
    assert!(
        matches!(next(&mut other).await, ServerMessage::Error {code,..} if code=="INVOCATION_CONFLICT")
    );
    assert_eq!(std::fs::read(&audit.path).unwrap(), original);
    assert_eq!(fixture.probe.inference_calls.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.probe.api_calls.load(Ordering::SeqCst), 1);
    other.close(None).await.unwrap();
    drop(other);
    reconnected.close(None).await.unwrap();
    drop(reconnected);
    fixture.finish().await;
}

#[tokio::test]
async fn chat_inspection_and_invalid_ids_never_create_claims_or_execute() {
    let fixture = Fixture::start(Mode::Normal, false).await;
    let mut socket = fixture.connect().await;
    identified(
        &mut socket,
        uuid::Uuid::new_v4(),
        "Inspect system health",
        true,
    )
    .await;
    assert!(
        matches!(next(&mut socket).await, ServerMessage::Error {code,..} if code=="INVOCATION_NOT_FOUND")
    );
    identified(
        &mut socket,
        uuid::Uuid::nil(),
        "Inspect system health",
        false,
    )
    .await;
    assert!(
        matches!(next(&mut socket).await, ServerMessage::Error {code,..} if code=="INVALID_INVOCATION_ID")
    );
    assert!(!fixture.probe.project.join(".symbiont/invocations").exists());
    assert_eq!(fixture.probe.inference_calls.load(Ordering::SeqCst), 0);
    socket.close(None).await.unwrap();
    drop(socket);
    fixture.finish().await;
}

#[tokio::test]
async fn active_and_queued_chat_claims_survive_disconnect_without_reexecution() {
    let fixture = Fixture::start(Mode::Stall, false).await;
    let mut socket = fixture.connect().await;
    let active = uuid::Uuid::new_v4();
    identified(&mut socket, active, "hold", false).await;
    let active_audit = audit(&mut socket, &fixture).await;
    tokio::time::timeout(Duration::from_secs(5), fixture.probe.entered.notified())
        .await
        .unwrap();
    let queued = uuid::Uuid::new_v4();
    identified(&mut socket, queued, "queued", false).await;
    let queued_audit = audit(&mut socket, &fixture).await;
    identified(&mut socket, queued, "queued", false).await;
    assert!(
        matches!(next(&mut socket).await, ServerMessage::Error {code,..} if code=="INVOCATION_IN_PROGRESS")
    );
    let mut other = fixture.connect().await;
    identified(&mut other, active, "hold", false).await;
    assert!(
        matches!(next(&mut other).await, ServerMessage::Error {code,..} if code=="INVOCATION_IN_PROGRESS")
    );
    socket.close(None).await.unwrap();
    drop(socket);
    for (id, content, audit) in [
        (active, "hold", active_audit),
        (queued, "queued", queued_audit.clone()),
    ] {
        tokio::time::timeout(Duration::from_secs(5),async {
            loop {
                identified(&mut other,id,content,false).await;
                match next(&mut other).await {
                    ServerMessage::AuditOpened { audit: original, .. } => {
                        assert_eq!(original.run_id,audit.run_id);
                        assert!(matches!(next(&mut other).await, ServerMessage::Error {code,..} if code=="INVOCATION_UNRESOLVED"));
                        break;
                    }
                    ServerMessage::Error {code,..} if code=="INVOCATION_IN_PROGRESS" => tokio::time::sleep(Duration::from_millis(10)).await,
                    event => panic!("unexpected retry event: {event:?}"),
                }
            }
        }).await.unwrap();
    }
    assert!(std::fs::read(&queued_audit.path).unwrap().is_empty());
    assert_eq!(fixture.probe.inference_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.probe.api_calls.load(Ordering::SeqCst), 0);
    other.close(None).await.unwrap();
    drop(other);
    fixture.finish().await;
}

#[tokio::test]
async fn sdk_chat_ids_share_durable_accounting_without_impersonating_network_credentials() {
    use super::{coordinator::CoordinatorSession, invocations::AuthenticatedCaller};
    assert_ne!(
        AuthenticatedCaller::coordinator_sdk().fingerprint(),
        AuthenticatedCaller::verified("trusted-coordinator-sdk", None).fingerprint()
    );
    let fixture = Fixture::start(Mode::Normal, false).await;
    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    let mut session = CoordinatorSession::new(fixture.state.clone(), tx);
    let id = uuid::Uuid::new_v4();
    for expected_replay in [false, true] {
        session
            .handle_chat_with_id(id, "Inspect system health".into())
            .await;
        let mut completed = false;
        while let Ok(event) = rx.try_recv() {
            match event {
                ServerMessage::ChatChunk {
                    replayed, content, ..
                } => {
                    assert_eq!(replayed, expected_replay);
                    assert!(content.contains("fixture-healthy"));
                    completed = true;
                }
                ServerMessage::Error { code, message, .. } => panic!("{code}: {message}"),
                _ => {}
            }
        }
        assert!(completed);
    }
    let mut socket = fixture.connect().await;
    identified(&mut socket, id, "Inspect system health", true).await;
    assert!(
        matches!(next(&mut socket).await,ServerMessage::Error {code,..} if code=="INVOCATION_CONFLICT")
    );
    assert_eq!(fixture.probe.api_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.probe.inference_calls.load(Ordering::SeqCst), 2);
    socket.close(None).await.unwrap();
    drop(socket);
    drop(session);
    drop(rx);
    fixture.finish().await;
}
