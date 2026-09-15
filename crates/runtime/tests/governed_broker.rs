//! Broker calls through actual ToolClad preparation, Cedar, approval and effects.
#![cfg(all(unix, feature = "cedar", feature = "cli-executor"))]

use async_trait::async_trait;
use serde_json::json;
use std::{
    collections::HashMap, os::unix::fs::PermissionsExt, path::PathBuf, sync::Arc, time::Duration,
};
use symbi_runtime::{
    cli_executor::{
        broker::McpToolBroker, governed::ManagedCliActionExecutor, AiCliAdapter, CliExecutorConfig,
        CodeGenRequest, CodeGenResult, StdinStrategy,
    },
    escalation::{
        Approver, Decision, EscalationGate, EscalationGateConfig, EscalationNotifier,
        EscalationQueue, HeldAction, Surface,
    },
    reasoning::{
        conversation::Conversation,
        executor::ActionExecutor,
        governed_session::{BrokerToolCall, GovernedToolSession},
        loop_types::{
            BufferedJournal, JournalEntry, JournalError, JournalWriter, LoopConfig, LoopEvent,
            LoopState, TerminationReason,
        },
        policy_bridge::ReasoningPolicyGate,
        source_policy::SourcePolicyExecutor,
        CedarPolicy, CedarPolicyGate,
    },
    sandbox::{command::CommandBoundary, ExecutionResult},
    toolclad::{Manifest, ToolCladExecutor},
    types::AgentId,
};

struct Fixture {
    root: tempfile::TempDir,
    boundary: CommandBoundary,
    executor: Arc<ToolCladExecutor>,
    journal: Arc<BufferedJournal>,
    gate: Arc<CedarPolicyGate>,
    state: LoopState,
}
impl Fixture {
    async fn new(approval: bool, container: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let effects = root.path().join("effects");
        std::fs::create_dir(&effects).unwrap();
        std::fs::set_permissions(&effects, std::fs::Permissions::from_mode(0o777)).unwrap();
        std::fs::write(root.path().join("host-canary"), "synthetic host value").unwrap();
        let destination = if container {
            "/workspace/events".into()
        } else {
            effects.join("events").display().to_string()
        };
        let mut manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "broker_fixture"
version = "1"
description = "Append an independently authorized normalized count"
binary = "python3"
timeout_seconds = 10
[args.count]
position = 1
required = true
type = "integer"
min = 1
max = 5
clamp = true
[command]
template = "placeholder"
[output]
format = "text"
"#,
        )
        .unwrap();
        manifest.tool.human_approval = approval;
        manifest.command.template = Some(format!(
            "python3 -c 'from pathlib import Path; p = Path(\"{destination}\"); p.open(\"a\").write(\"{{count}}\\n\"); print(\"recorded {{count}}\")'"
        ));
        let boundary = if container {
            let mut profile = CommandBoundary::default();
            profile.docker.volumes = vec![format!("{}:/workspace:rw", effects.display())];
            profile.docker.max_execution_time = Duration::from_secs(20);
            profile
        } else {
            CommandBoundary::development_host()
        };
        let executor = Arc::new(
            ToolCladExecutor::new(vec![("broker_fixture".into(), manifest)])
                .with_command_boundary(boundary.clone()),
        );
        let gate = Arc::new(CedarPolicyGate::deny_by_default());
        gate.add_policy(CedarPolicy {
            name: "normalized".into(), active: true,
            source: "permit(principal, action == Action::\"tool_call::broker_fixture\", resource) when { context.tenant == \"approved\" && context.invocation.arguments.count == \"5\" };".into(),
        }).await;
        let mut state = LoopState::new(AgentId::new(), Conversation::with_system("broker fixture"));
        state
            .trusted_context
            .insert("tenant".into(), json!("approved"));
        Self {
            root,
            boundary,
            executor,
            journal: Arc::new(BufferedJournal::new(100)),
            gate,
            state,
        }
    }
    async fn session(
        &self,
        gate: Arc<dyn ReasoningPolicyGate>,
        journal: Arc<dyn JournalWriter>,
    ) -> Arc<GovernedToolSession> {
        Arc::new(
            GovernedToolSession::start(
                self.executor.clone(),
                gate,
                journal,
                self.state.clone(),
                LoopConfig {
                    timeout: Duration::from_secs(20),
                    ..Default::default()
                },
            )
            .await
            .unwrap(),
        )
    }
    fn events(&self) -> String {
        std::fs::read_to_string(self.root.path().join("effects/events")).unwrap_or_default()
    }
}
fn call(id: &str, count: &str) -> BrokerToolCall {
    BrokerToolCall {
        call_id: id.into(),
        name: "broker_fixture".into(),
        arguments: json!({"count": count}),
    }
}

#[tokio::test]
async fn broker_normalizes_before_cedar_and_rejects_replay_and_extra_arguments() {
    let fixture = Fixture::new(false, false).await;
    let session = fixture
        .session(fixture.gate.clone(), fixture.journal.clone())
        .await;
    let result = session.call(call("approved", "999")).await.unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(fixture.events(), "5\n");
    assert!(result.metadata["authorized_arguments"].contains("\"5\""));
    assert!(session
        .call(call("approved", "999"))
        .await
        .unwrap_err()
        .contains("already used"));
    assert!(session.call(call("denied", "1")).await.unwrap().is_error);
    let mut extra = call("extra", "5");
    extra.arguments["tenant"] = json!("approved");
    assert!(session.call(extra).await.unwrap().is_error);
    let mut unknown = call("unknown", "5");
    unknown.name = "not_advertised".into();
    assert!(session.call(unknown).await.unwrap().is_error);
    assert_eq!(fixture.events(), "5\n");
    session.close().await.unwrap();
    assert!(session.call(call("closed", "5")).await.is_err());
    let entries = fixture.journal.entries().await;
    assert!(entries
        .iter()
        .all(|entry| entry.agent_id == fixture.state.agent_id));
    assert!(matches!(
        entries.last().unwrap().event,
        LoopEvent::Terminated {
            reason: TerminationReason::Completed,
            ..
        }
    ));
}

#[test]
fn client_cannot_supply_identity_context_or_approval() {
    for field in [
        "principal",
        "agent_id",
        "trusted_context",
        "approval",
        "sandbox",
    ] {
        let mut request =
            json!({"call_id":"client", "name":"broker_fixture", "arguments":{"count":"5"}});
        request[field] = json!("forged");
        assert!(
            serde_json::from_value::<BrokerToolCall>(request).is_err(),
            "{field}"
        );
    }
}

struct FailingJournal;
#[async_trait]
impl JournalWriter for FailingJournal {
    async fn append(&self, entry: JournalEntry) -> Result<(), JournalError> {
        if matches!(entry.event, LoopEvent::PolicyEvaluated { .. }) {
            Err(JournalError::WriteFailed(
                "synthetic storage failure".into(),
            ))
        } else {
            Ok(())
        }
    }
    async fn next_sequence(&self) -> u64 {
        0
    }
}

#[tokio::test]
async fn broker_journal_failure_prevents_effects_and_closes_session() {
    let fixture = Fixture::new(false, false).await;
    let session = fixture
        .session(fixture.gate.clone(), Arc::new(FailingJournal))
        .await;
    assert!(session
        .call(call("failed", "5"))
        .await
        .unwrap_err()
        .contains("storage failure"));
    assert!(session.call(call("later", "5")).await.is_err());
    assert!(session.close().await.is_err());
    assert_eq!(fixture.events(), "");
}

#[tokio::test]
async fn broker_manifest_approval_is_mandatory_despite_permitting_cedar() {
    let fixture = Fixture::new(true, false).await;
    let session = fixture
        .session(fixture.gate.clone(), fixture.journal.clone())
        .await;
    let result = session.call(call("missing", "5")).await.unwrap();
    assert!(result.is_error && result.content.contains("approval"));
    assert_eq!(fixture.events(), "");
    session.close().await.unwrap();
}

async fn pending(queue: &EscalationQueue) -> HeldAction {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(action) = queue.list_pending_async().await.into_iter().next() {
                return action;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("actual approval request was not queued")
}
fn approver() -> Approver {
    Approver {
        surface: Surface::Rest,
        id: "fixture-operator".into(),
        display: "Fixture operator".into(),
    }
}

#[tokio::test]
async fn broker_real_approval_queue_binds_normalized_call_and_cannot_be_replayed() {
    let fixture = Fixture::new(true, false).await;
    let queue = Arc::new(EscalationQueue::new());
    let gate = Arc::new(EscalationGate::new(
        fixture.gate.clone(),
        queue.clone(),
        EscalationGateConfig {
            require_approval_tools: vec![],
            timeout: Duration::from_secs(5),
        },
    ));
    let session = fixture.session(gate, fixture.journal.clone()).await;
    let active = session.clone();
    let task = tokio::spawn(async move { active.call(call("approved", "999")).await });
    let held = pending(&queue).await;
    assert_eq!(held.agent_id, fixture.state.agent_id.to_string());
    assert_eq!(
        held.context_snapshot.as_ref().unwrap()["invocation"]["arguments"]["count"],
        "5"
    );
    assert_eq!(fixture.events(), "");
    queue
        .resolve_async(&held.id, Decision::Approve { reason: None }, approver())
        .await
        .unwrap();
    assert!(!task.await.unwrap().unwrap().is_error);
    assert!(queue
        .resolve_async(&held.id, Decision::Approve { reason: None }, approver())
        .await
        .is_err());
    assert!(session.call(call("approved", "5")).await.is_err());
    assert_eq!(fixture.events(), "5\n");
    session.close().await.unwrap();
    assert_approval_resolution(&fixture.journal.entries().await, &held);
}

fn assert_approval_resolution(entries: &[JournalEntry], held: &HeldAction) {
    let call = entries
        .iter()
        .find_map(|entry| match &entry.event {
            LoopEvent::PolicyEvaluated { approved_calls, .. } => approved_calls.first(),
            _ => None,
        })
        .expect("approved invocation must be recorded before dispatch");
    let resolution = &call["approval"]["resolution"];
    assert_eq!(resolution["escalation_id"], held.id);
    assert_eq!(resolution["agent_id"], held.agent_id);
    assert_eq!(resolution["approver"]["id"], "fixture-operator");
    assert_eq!(resolution["approver"]["surface"], "rest");
    assert_eq!(resolution["decision"]["decision"], "approve");
    let at = chrono::DateTime::parse_from_rfc3339(resolution["at"].as_str().unwrap()).unwrap();
    assert!(at >= held.created_at && at <= held.expires_at);
    assert_eq!(
        call["approval"]["id"],
        held.context_snapshot.as_ref().unwrap()["approval_id"]
    );
}

struct StalledApprovalAudit;

#[async_trait]
impl symbi_runtime::escalation::EscalationAudit for StalledApprovalAudit {
    async fn record(&self, _: symbi_runtime::escalation::AuditEvent) {
        std::future::pending::<()>().await;
    }
}

#[tokio::test]
async fn approval_identity_reaches_required_audit_even_if_optional_callback_stalls() {
    let fixture = Fixture::new(true, false).await;
    let queue = Arc::new(EscalationQueue::new().with_audit(Arc::new(StalledApprovalAudit)));
    let gate = Arc::new(EscalationGate::new(
        fixture.gate.clone(),
        queue.clone(),
        EscalationGateConfig {
            require_approval_tools: vec![],
            timeout: Duration::from_secs(5),
        },
    ));
    let session = fixture.session(gate, fixture.journal.clone()).await;
    let active = session.clone();
    let call = tokio::spawn(async move { active.call(call("approved", "999")).await });
    let held = pending(&queue).await;
    assert_eq!(fixture.events(), "");
    let resolver = queue.clone();
    let id = held.id.clone();
    let resolution = tokio::spawn(async move {
        resolver
            .resolve_async(&id, Decision::Approve { reason: None }, approver())
            .await
    });
    assert!(
        !tokio::time::timeout(Duration::from_secs(3), call)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .is_error
    );
    assert!(!resolution.is_finished());
    assert_eq!(fixture.events(), "5\n");
    session.close().await.unwrap();
    assert_approval_resolution(&fixture.journal.entries().await, &held);
    assert!(queue.list_pending_async().await.is_empty());
    resolution.abort();
    assert!(resolution.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn broker_caller_abort_cancels_approval_and_records_interruption() {
    let fixture = Fixture::new(true, false).await;
    let queue = Arc::new(EscalationQueue::new());
    let gate = Arc::new(EscalationGate::new(
        fixture.gate.clone(),
        queue.clone(),
        EscalationGateConfig {
            require_approval_tools: vec![],
            timeout: Duration::from_secs(10),
        },
    ));
    let session = fixture.session(gate, fixture.journal.clone()).await;
    let active = session.clone();
    let task = tokio::spawn(async move { active.call(call("cancelled", "5")).await });
    let held = pending(&queue).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(session.call(call("new", "5")).await.is_err());
    assert!(session.close().await.is_err());
    assert!(queue.list_pending_async().await.is_empty());
    assert!(queue
        .resolve_async(&held.id, Decision::Approve { reason: None }, approver())
        .await
        .is_err());
    assert_eq!(fixture.events(), "");
    assert!(matches!(
        fixture.journal.entries().await.last().unwrap().event,
        LoopEvent::Terminated {
            reason: TerminationReason::Error { .. },
            ..
        }
    ));
}

#[tokio::test]
async fn broker_budget_counts_denials_and_expires_without_successful_completion() {
    let fixture = Fixture::new(false, false).await;
    let session = GovernedToolSession::start(
        fixture.executor.clone(),
        fixture.gate.clone(),
        fixture.journal.clone(),
        fixture.state.clone(),
        LoopConfig {
            max_iterations: 1,
            timeout: Duration::from_secs(20),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(session.call(call("denied", "1")).await.unwrap().is_error);
    assert!(session
        .call(call("over-budget", "5"))
        .await
        .unwrap_err()
        .contains("budget"));
    assert!(session.close().await.is_err());
    assert_eq!(fixture.events(), "");

    let fixture = Fixture::new(false, false).await;
    let session = GovernedToolSession::start(
        fixture.executor.clone(),
        fixture.gate.clone(),
        fixture.journal.clone(),
        fixture.state.clone(),
        LoopConfig {
            timeout: Duration::from_secs(1),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(session.call(call("expired", "5")).await.is_err());
    assert!(session.close().await.is_err());
    assert_eq!(fixture.events(), "");
    assert!(matches!(
        fixture.journal.entries().await.last().unwrap().event,
        LoopEvent::Terminated {
            reason: TerminationReason::Error { .. },
            ..
        }
    ));
}

#[tokio::test]
async fn broker_owner_close_cancels_pending_wire_approval_and_removes_channel() {
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
        net::UnixStream,
    };
    let fixture = Fixture::new(true, false).await;
    let queue = Arc::new(EscalationQueue::new());
    let gate = Arc::new(EscalationGate::new(
        fixture.gate.clone(),
        queue.clone(),
        EscalationGateConfig {
            require_approval_tools: vec![],
            timeout: Duration::from_secs(10),
        },
    ));
    let session = fixture.session(gate, fixture.journal.clone()).await;
    let broker = McpToolBroker::start(session.clone(), fixture.root.path())
        .await
        .unwrap();
    let socket = broker.socket_path();
    let mut connection = BufReader::new(UnixStream::connect(&socket).await.unwrap());
    connection.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-06-18\",\"capabilities\":{},\"clientInfo\":{}}}\n").await.unwrap();
    let mut response = String::new();
    connection.read_line(&mut response).await.unwrap();
    assert!(serde_json::from_str::<serde_json::Value>(&response).unwrap()["result"].is_object());
    connection.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"broker_fixture\",\"arguments\":{\"count\":\"5\"}}}\n").await.unwrap();
    let held = pending(&queue).await;
    assert!(broker.close().await.is_err());
    assert!(queue.list_pending_async().await.is_empty());
    assert!(queue
        .resolve_async(&held.id, Decision::Approve { reason: None }, approver())
        .await
        .is_err());
    assert!(!socket.exists());
    assert!(session.call(call("after-close", "5")).await.is_err());
    assert_eq!(fixture.events(), "");
    assert!(matches!(
        fixture.journal.entries().await.last().unwrap().event,
        LoopEvent::Terminated {
            reason: TerminationReason::Error { .. },
            ..
        }
    ));
}

struct PendingNotification(tokio_util::sync::CancellationToken);

#[async_trait]
impl EscalationNotifier for PendingNotification {
    async fn notify(&self, _: &HeldAction) {
        let _guard = self.0.clone().drop_guard();
        std::future::pending::<()>().await;
    }
}

/// Real ToolClad/Cedar/receipt/dispatch and signed audit, with a notifier that
/// never finishes. Only the exact approval may produce the container file effect.
async fn container_approval_lifecycle(outcome: &str) {
    let chat = outcome.starts_with("chat-");
    let outcome = outcome.strip_prefix("chat-").unwrap_or(outcome);
    use symbi_runtime::reasoning::{
        protected_journal::ProtectedJournal, run_audit::open_run_journal,
    };
    let fixture = Fixture::new(true, true).await;
    let queue = Arc::new(EscalationQueue::new());
    let notification_dropped = tokio_util::sync::CancellationToken::new();
    queue
        .subscribe(Arc::new(PendingNotification(notification_dropped.clone())))
        .await;
    let gate = Arc::new(EscalationGate::new(
        fixture.gate.clone(),
        queue.clone(),
        EscalationGateConfig {
            require_approval_tools: vec![],
            timeout: if outcome == "expired" {
                Duration::from_millis(500)
            } else {
                Duration::from_secs(10)
            },
        },
    ));
    let (journal, audit) = open_run_journal(fixture.root.path(), fixture.state.agent_id)
        .await
        .unwrap();
    let session = fixture.session(gate, journal).await;
    let active = session.clone();
    let task = tokio::spawn(async move { active.call(call("held", "999")).await });
    let held = pending(&queue).await;
    assert_eq!(fixture.events(), "");
    assert_eq!(
        held.context_snapshot.as_ref().unwrap()["invocation"]["arguments"]["count"],
        "5"
    );
    if chat {
        use symbi_channel_adapter::{
            traits::InboundCommandInterceptor,
            types::{ChatPlatform, InboundMessage},
        };
        use symbi_runtime::escalation::{ChannelApprovers, EscalationCommandInterceptor};
        let approvers = ChannelApprovers::from([(
            (ChatPlatform::Slack, "approvers".into()),
            std::collections::HashSet::from(["fixture-operator".into()]),
        )]);
        let interceptor = EscalationCommandInterceptor::new(queue.clone(), approvers);
        let mut message = InboundMessage {
            id: "chat-fixture".into(),
            platform: ChatPlatform::Slack,
            workspace_id: "workspace".into(),
            channel_id: "approvers".into(),
            thread_id: None,
            sender_id: "fixture-operator".into(),
            sender_name: "Fixture operator".into(),
            content: format!("/symbi gate show {}", held.id),
            command: None,
            timestamp: chrono::Utc::now(),
            raw_payload: None,
        };
        let review = interceptor.try_handle(&message).await.unwrap();
        let complete_json = review
            .split_once("```json\n")
            .expect(&review)
            .1
            .split_once("\n```")
            .unwrap()
            .0;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(complete_json).unwrap(),
            serde_json::to_value(&held).unwrap()
        );
        let approve = review
            .lines()
            .find_map(|line| line.strip_prefix("Approve: "))
            .unwrap()
            .to_owned();
        for invalid in [
            format!("/symbi gate approve {}", held.id),
            format!("/symbi gate approve {} wrong-digest", held.id),
        ] {
            message.content = invalid;
            assert!(!interceptor
                .try_handle(&message)
                .await
                .unwrap()
                .contains("approved."));
            assert_eq!(queue.list_pending_async().await.len(), 1);
            assert_eq!(fixture.events(), "");
        }
        message.content = if outcome == "approved" {
            approve
        } else {
            format!("/symbi gate deny {}", held.id)
        };
        message.sender_id = "untrusted".into();
        assert!(interceptor
            .try_handle(&message)
            .await
            .unwrap()
            .contains("not authorized"));
        message.sender_id = "fixture-operator".into();
        let result = interceptor.try_handle(&message).await.unwrap();
        assert!(result.ends_with(&format!("{outcome}.")), "{result}");
    } else {
        match outcome {
            "approved" => queue
                .resolve_async(&held.id, Decision::Approve { reason: None }, approver())
                .await
                .unwrap(),
            "denied" => queue
                .resolve_async(&held.id, Decision::Deny { reason: None }, approver())
                .await
                .unwrap(),
            "cancelled" => task.abort(),
            "expired" => {}
            _ => panic!("unknown fixture outcome"),
        }
    }
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("approval must not wait for the stalled notifier");
    if outcome == "cancelled" {
        assert!(result.unwrap_err().is_cancelled());
        assert!(session.close().await.is_err());
    } else {
        let result = result.unwrap().unwrap();
        assert_eq!(result.is_error, outcome != "approved", "{}", result.content);
        if outcome == "expired" {
            assert!(result.content.contains("timeout"), "{}", result.content);
        }
        session.close().await.unwrap();
    }
    assert!(notification_dropped.is_cancelled());
    assert!(queue.list_pending_async().await.is_empty());
    assert!(queue
        .resolve_async(&held.id, Decision::Approve { reason: None }, approver())
        .await
        .is_err());
    assert_eq!(
        fixture.events(),
        if outcome == "approved" { "5\n" } else { "" }
    );
    assert_eq!(
        std::fs::read_to_string(fixture.root.path().join("host-canary")).unwrap(),
        "synthetic host value"
    );
    let key: [u8; 32] = hex::decode(&audit.public_key).unwrap().try_into().unwrap();
    let entries = ProtectedJournal::verify_run(&audit.path, &key, audit.run_id).unwrap();
    assert!(entries
        .iter()
        .all(|entry| entry.agent_id == fixture.state.agent_id));
    let LoopEvent::Terminated { reason, .. } = &entries.last().unwrap().event else {
        panic!("signed journal must have a terminal record");
    };
    if outcome == "cancelled" {
        assert!(matches!(reason, TerminationReason::Error { .. }));
    } else {
        assert!(matches!(reason, TerminationReason::Completed));
    }
    let text = serde_json::to_string(&entries).unwrap();
    if outcome == "approved" {
        if chat {
            let call = entries
                .iter()
                .find_map(|entry| match &entry.event {
                    LoopEvent::PolicyEvaluated { approved_calls, .. } => approved_calls.first(),
                    _ => None,
                })
                .unwrap();
            let resolution = &call["approval"]["resolution"];
            assert_eq!(resolution["escalation_id"], held.id);
            assert_eq!(resolution["agent_id"], held.agent_id);
            assert_eq!(resolution["approver"]["surface"], "chat");
            assert_eq!(
                resolution["approver"]["id"],
                "slack:workspace:approvers:fixture-operator"
            );
            assert!(resolution["decision"]["reason"]
                .as_str()
                .unwrap()
                .starts_with("chat review "));
            assert_eq!(
                call["approval"]["id"],
                held.context_snapshot.as_ref().unwrap()["approval_id"]
            );
        } else {
            assert_approval_resolution(&entries, &held);
        }
        assert!(text.contains("authorized_arguments"));
        assert!(text.contains("recorded 5"));
    } else {
        assert!(!text.contains("recorded 5"));
    }
}

#[tokio::test]
#[ignore = "requires Docker and a matching sandbox supervisor"]
async fn container_approval_succeeds_while_notification_is_stalled() {
    container_approval_lifecycle("approved").await;
}

#[tokio::test]
#[ignore = "requires Docker and a matching sandbox supervisor"]
async fn container_approval_denial_has_no_effects_or_pending_state() {
    container_approval_lifecycle("denied").await;
}

#[tokio::test]
#[ignore = "requires Docker and a matching sandbox supervisor"]
async fn container_approval_expiry_has_no_effects_or_pending_state() {
    container_approval_lifecycle("expired").await;
}

#[tokio::test]
#[ignore = "requires Docker and a matching sandbox supervisor"]
async fn container_approval_cancellation_has_no_effects_or_pending_state() {
    container_approval_lifecycle("cancelled").await;
}

struct ClientAdapter {
    code: String,
}
#[async_trait]
impl AiCliAdapter for ClientAdapter {
    fn name(&self) -> &str {
        "broker-client-fixture"
    }
    fn executable(&self) -> &str {
        "python3"
    }
    fn build_args(&self, _: &CodeGenRequest) -> Vec<String> {
        vec!["-u".into(), "-c".into(), self.code.clone()]
    }
    fn non_interactive_env(&self) -> HashMap<String, String> {
        HashMap::new()
    }
    fn stdin_strategy(&self) -> StdinStrategy {
        StdinStrategy::CloseImmediately
    }
    fn parse_output(&self, _: &CodeGenRequest, execution: ExecutionResult) -> CodeGenResult {
        CodeGenResult {
            success: execution.success,
            execution,
            parsed_output: None,
            files_modified: vec![],
            adapter_name: self.name().into(),
        }
    }
    async fn health_check(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

#[tokio::test]
#[ignore = "requires Docker and a matching sandbox supervisor"]
async fn contained_cli_uses_mcp_broker_without_source_access() {
    contained_cli_broker(false).await;
}

#[tokio::test]
#[ignore = "requires Docker and a matching sandbox supervisor"]
async fn contained_cli_cancellation_waits_for_worker_cleanup() {
    contained_cli_broker(true).await;
}

async fn contained_cli_broker(cancel_after_effect: bool) {
    let mut fixture = Fixture::new(false, true).await;
    let label = format!("symbi.host-admission={}", uuid::Uuid::new_v4());
    fixture
        .boundary
        .docker
        .extra_flags
        .push(format!("--label={label}"));
    let source_policy = dsl::ExecutionPolicy::parse(
        r#"
policy scope { allow: ["claude_code", "broker_fixture"] }
agent fixture(input: String) -> String {
    policy normalized { deny: "broker_fixture" if invocation.arguments.count != "5" }
    with { return input; }
}
"#,
        "fixture",
    )
    .unwrap();
    fixture.gate.add_policy(CedarPolicy { name: "launch".into(), active: true,
        source: format!("permit(principal == Agent::\"{}\", action == Action::\"tool_call::claude_code\", resource) when {{ context.source_policy.agent_name == \"fixture\" && context.invocation.source_policy.source_hash == context.source_policy.source_hash }};", fixture.state.agent_id),
    }).await;
    let queue = Arc::new(EscalationQueue::new());
    let gate = Arc::new(EscalationGate::new(
        fixture.gate.clone(),
        queue.clone(),
        EscalationGateConfig {
            require_approval_tools: vec![],
            timeout: Duration::from_secs(10),
        },
    ));
    let backend = Arc::new(SourcePolicyExecutor::new(
        fixture.executor.clone(),
        source_policy.clone(),
    ));
    let session = Arc::new(
        GovernedToolSession::start(
            backend,
            gate,
            fixture.journal.clone(),
            fixture.state.clone(),
            LoopConfig {
                timeout: Duration::from_secs(20),
                ..Default::default()
            },
        )
        .await
        .unwrap(),
    );
    let broker = McpToolBroker::start(session.clone(), fixture.root.path())
        .await
        .unwrap();
    let child = broker.child_boundary(&fixture.boundary).unwrap();
    assert_eq!(child.docker.volumes.len(), 1);
    assert_eq!(child.docker.network_mode, "none");
    let canary = serde_json::to_string(&fixture.root.path().join("host-canary")).unwrap();
    let pause = if cancel_after_effect {
        "import time; time.sleep(60)"
    } else {
        ""
    };
    let adapter = ClientAdapter {
        code: format!(
            r#"
import json, os, pathlib, subprocess
assert os.getuid() == 65534
assert not pathlib.Path({canary}).exists()
assert not pathlib.Path('/workspace/events').exists()
bridge = subprocess.Popen(['python3', '/opt/symbi-broker/bridge.py', '/opt/symbi-broker/tools.sock'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
def send(message):
    bridge.stdin.write(json.dumps(message) + '\n'); bridge.stdin.flush()
    if 'id' in message:
        response = json.loads(bridge.stdout.readline())
        assert response['id'] == message['id'], response
        return response
send(dict(jsonrpc='2.0', id=1, method='initialize', params=dict(protocolVersion='2025-06-18', capabilities={{}}, clientInfo=dict(name='fixture', version='1'))))
send(dict(jsonrpc='2.0', method='notifications/initialized'))
listed = send(dict(jsonrpc='2.0', id=2, method='tools/list'))
assert [tool['name'] for tool in listed['result']['tools']] == ['broker_fixture']
message = dict(jsonrpc='2.0', id=3, method='tools/call', params=dict(name='broker_fixture', arguments=dict(count='999')))
allowed = send(message); assert not allowed['result']['isError'], allowed
replay = send(message); assert 'error' in replay, replay
denied = send(dict(jsonrpc='2.0', id=4, method='tools/call', params=dict(name='broker_fixture', arguments=dict(count='1'))))
assert denied['result']['isError'], denied
forged = send(dict(jsonrpc='2.0', id=5, method='tools/call', params=dict(name='broker_fixture', arguments=dict(count='5'), principal='forged')))
assert 'error' in forged, forged
assert not pathlib.Path('/workspace/events').exists()
bridge.stdin.close(); assert bridge.wait(timeout=5) == 0
print('brokered work completed without direct source access')
{pause}
"#
        ),
    };
    let request = CodeGenRequest {
        prompt: "exercise broker".into(),
        working_dir: PathBuf::from("/workspace"),
        target_files: vec![],
        system_context: None,
        model: None,
        options: HashMap::new(),
    };
    let launch = Arc::new(
        ManagedCliActionExecutor::new(
            Arc::new(adapter),
            request,
            child,
            CliExecutorConfig {
                max_runtime: Duration::from_secs(15),
                ..Default::default()
            },
            json!({"tool_sandbox":fixture.boundary.descriptor().unwrap()}),
            true,
        )
        .unwrap(),
    );
    let action = launch.proposal("worker-admission");
    let governed: Arc<dyn ActionExecutor> =
        Arc::new(SourcePolicyExecutor::new(launch.clone(), source_policy));
    let active = session.clone();
    let executor = governed.clone();
    let task = tokio::spawn(async move { active.dispatch_host_action(executor, action).await });
    let held = pending(&queue).await;
    assert_eq!(held.agent_id, fixture.state.agent_id.to_string());
    let invocation = &held.context_snapshot.as_ref().unwrap()["invocation"];
    assert_eq!(invocation["source_policy"]["agent_name"], "fixture");
    assert_eq!(invocation["arguments"]["argv"][0], "python3");
    assert_eq!(
        invocation["arguments"]["request"]["prompt"],
        "exercise broker"
    );
    assert_eq!(fixture.events(), "");
    assert!(session
        .dispatch_host_action(governed, launch.proposal("duplicate-admission"))
        .await
        .is_err());
    queue
        .resolve_async(&held.id, Decision::Approve { reason: None }, approver())
        .await
        .unwrap();
    if cancel_after_effect {
        tokio::time::timeout(Duration::from_secs(5), async {
            while fixture.events() != "5\n" {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("child did not reach its broker effect");
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(broker.close().await.is_err());
        let output = tokio::process::Command::new("docker")
            .args(["ps", "-aq", "--filter", &format!("label={label}")])
            .output()
            .await
            .unwrap();
        assert!(output.status.success());
        assert!(output.stdout.is_empty(), "worker survived terminal audit");
        assert!(matches!(
            fixture.journal.entries().await.last().unwrap().event,
            LoopEvent::Terminated {
                reason: TerminationReason::Error { .. },
                ..
            }
        ));
        assert_eq!(fixture.events(), "5\n");
        return;
    }
    let observations = task.await.unwrap().unwrap();
    assert_eq!(observations.len(), 1);
    assert!(!observations[0].is_error, "{}", observations[0].content);
    let result = launch.take_result().unwrap();
    assert!(result.success, "{:?}", result.execution);
    assert!(result.execution.stdout.contains("brokered work completed"));
    assert_eq!(fixture.events(), "5\n");
    broker.close().await.unwrap();
    assert_approval_resolution(&fixture.journal.entries().await, &held);
}

#[tokio::test]
#[ignore = "requires Docker and a matching sandbox supervisor"]
async fn container_chat_review_approval_binds_actual_effect_and_signed_identity() {
    container_approval_lifecycle("chat-approved").await;
}

#[tokio::test]
#[ignore = "requires Docker and a matching sandbox supervisor"]
async fn container_chat_denial_and_wrong_reviews_never_execute() {
    container_approval_lifecycle("chat-denied").await;
}
