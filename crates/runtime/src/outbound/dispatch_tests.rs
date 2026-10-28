//! Real governed HTTP dispatch: cancellation must fence the socket, not just
//! invalidate a standalone authority object.
use crate::{
    reasoning::{
        circuit_breaker::CircuitBreakerRegistry,
        conversation::Conversation,
        dispatch::GovernedToolDispatcher,
        executor::ActionExecutor,
        loop_types::{JournalError, LoopConfig, LoopState, Observation, ProposedAction},
        policy_bridge::DefaultPolicyGate,
        prepared::execution_run_key,
        protected_journal::ProtectedJournal,
    },
    toolclad::{executor::ToolCladExecutor, manifest::load_manifests_from_dir},
};
use std::{
    os::unix::fs::PermissionsExt,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{io::AsyncReadExt, net::TcpListener};

struct Fixture {
    _root: tempfile::TempDir,
    executor: ToolCladExecutor,
    journal: ProtectedJournal,
    state: LoopState,
}

fn fixture(port: u16) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::create_dir(root.path().join("tools")).unwrap();
    std::fs::write(
        root.path().join("symbiont.toml"),
        format!(
            r#"
[outbound]
schema_version=1
tenant="fixture"
workspace="cases"
[outbound.capabilities.read_case]
tool="read_case"
origin="http://127.0.0.1:{port}"
method="GET"
path_prefix="/records/"
private_origin=true
"#
        ),
    )
    .unwrap();
    std::fs::write(
        root.path().join("tools/read_case.clad.toml"),
        format!(
            r#"
[tool]
name="read_case"
version="1"
description="Read a synthetic record"
[http]
capability="read_case"
method="GET"
url="http://127.0.0.1:{port}/records/7"
[output]
format="json"
"#
        ),
    )
    .unwrap();
    let executor =
        ToolCladExecutor::new(load_manifests_from_dir(&root.path().join("tools")).unwrap())
            .with_project_scope(root.path());
    executor.validate_configuration().unwrap();
    let mut state = LoopState::new(crate::types::AgentId::new(), Conversation::new());
    state.trusted_context = executor.execution_context();
    let journal = ProtectedJournal::create(root.path(), state.agent_id).unwrap();
    Fixture {
        _root: root,
        executor,
        journal,
        state,
    }
}

async fn dispatch(fixture: &Fixture, id: &str) -> Result<Vec<Observation>, JournalError> {
    let config = LoopConfig {
        timeout: Duration::from_secs(10),
        tool_timeout: Duration::from_secs(5),
        tool_definitions: fixture.executor.tool_definitions(),
        ..Default::default()
    };
    GovernedToolDispatcher {
        executor: &fixture.executor,
        journal: &fixture.journal,
        gate: &DefaultPolicyGate::permissive_for_dev_only(),
        circuit_breakers: &CircuitBreakerRegistry::default(),
    }
    .dispatch(
        &[ProposedAction::ToolCall {
            call_id: id.into(),
            name: "read_case".into(),
            arguments: "{}".into(),
        }],
        &fixture.state,
        &config,
    )
    .await
}

#[tokio::test]
async fn outbound_cancel_before_first_dispatch_prevents_socket_creation() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let fixture = fixture(listener.local_addr().unwrap().port());
    fixture.executor.cancel_run(
        &execution_run_key(&fixture.state),
        Instant::now() + Duration::from_secs(10),
    );
    let result = dispatch(&fixture, "first").await.unwrap();
    assert!(serde_json::to_string(&result).unwrap().contains("closed"));
    assert!(
        tokio::time::timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn outbound_cancelling_in_flight_dispatch_leaves_an_unconfirmed_effect_and_rejects_later_calls(
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let fixture = Arc::new(fixture(listener.local_addr().unwrap().port()));
    let owner = fixture.clone();
    let call = tokio::spawn(async move { dispatch(&owner, "first").await });
    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
        .await
        .unwrap()
        .unwrap();
    let mut request = [0; 2048];
    assert!(socket.read(&mut request).await.unwrap() > 0);
    fixture.executor.cancel_run(
        &execution_run_key(&fixture.state),
        Instant::now() + Duration::from_secs(10),
    );
    let result = tokio::time::timeout(Duration::from_secs(1), call)
        .await
        .unwrap()
        .unwrap();
    let result = result.unwrap();
    assert!(result.iter().any(Observation::has_unconfirmed_effect));
    let later = dispatch(&fixture, "second").await;
    assert!(later.unwrap_err().to_string().contains("reconciliation"));
    assert!(
        tokio::time::timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
    let entries =
        ProtectedJournal::verify(fixture.journal.path(), &fixture.journal.public_key()).unwrap();
    let events = serde_json::to_string(&entries).unwrap();
    assert!(events.contains("assignment_epoch") && events.contains("NetworkRequestFinished"));
}

#[tokio::test]
async fn outbound_dispatch_adopts_the_session_assignment_epoch() {
    use tokio::io::AsyncWriteExt;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut fixture = fixture(listener.local_addr().unwrap().port());
    let assignment = uuid::Uuid::new_v4();
    fixture
        .state
        .trusted_context
        .insert("outbound_assignment".into(), serde_json::json!(assignment));
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 2048];
        let _ = socket.read(&mut request).await.unwrap();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}")
            .await
            .unwrap();
    });
    let result = dispatch(&fixture, "first").await.unwrap();
    server.await.unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(text.contains(&assignment.to_string()), "{text}");
    let entries =
        ProtectedJournal::verify(fixture.journal.path(), &fixture.journal.public_key()).unwrap();
    let events = serde_json::to_value(&entries).unwrap();
    let journaled = find_key(&events, "assignment_epoch").expect("journaled assignment epoch");
    assert_eq!(journaled, serde_json::json!(assignment));
}

fn find_key(value: &serde_json::Value, key: &str) -> Option<serde_json::Value> {
    match value {
        serde_json::Value::Object(map) => map
            .get(key)
            .cloned()
            .or_else(|| map.values().find_map(|v| find_key(v, key))),
        serde_json::Value::Array(items) => items.iter().find_map(|v| find_key(v, key)),
        _ => None,
    }
}
