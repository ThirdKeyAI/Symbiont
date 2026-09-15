//! Actual private inference transport, durable records and external receivers.
#![cfg(all(unix, feature = "cli-executor"))]

use serde_json::{json, Value};
use std::{
    os::unix::fs::PermissionsExt,
    sync::Arc,
    time::{Duration, Instant},
};
use symbi_runtime::{
    cli_executor::{
        broker::McpToolBroker,
        inference_broker::{InferenceBrokerConfig, ProtectedInference},
    },
    reasoning::{
        conversation::Conversation,
        governed_session::GovernedToolSession,
        loop_types::{
            JournalEntry, JournalWriter, LoopConfig, LoopEvent, LoopState, TerminationReason,
        },
        policy_bridge::DefaultPolicyGate,
        protected_journal::ProtectedJournal,
    },
    toolclad::{Manifest, ToolCladExecutor},
    types::AgentId,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, UnixStream},
    sync::mpsc,
    task::JoinHandle,
};

const KEY: &str = "synthetic-protected-provider-credential";

struct Upstream {
    url: String,
    received: mpsc::Receiver<(String, Value, String)>,
    task: JoinHandle<()>,
}
impl Drop for Upstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn upstream(status: u16, body: String, delay: Duration) -> Upstream {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (send, received) = mpsc::channel(8);
    let task = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(stream);
            let mut header = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                header.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let length = header
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .map(str::to_owned)
                })
                .unwrap()
                .parse::<usize>()
                .unwrap();
            let mut data = vec![0; length];
            reader.read_exact(&mut data).await.unwrap();
            let raw = String::from_utf8(data).unwrap();
            send.send((header, serde_json::from_str(&raw).unwrap(), raw))
                .await
                .unwrap();
            tokio::time::sleep(delay).await;
            let response = format!("HTTP/1.1 {status} fixture\r\nContent-Length: {}\r\nContent-Type: application/json\r\nLocation: http://127.0.0.1:9/should-not-follow\r\nConnection: close\r\n\r\n{body}", body.len());
            let _ = reader.get_mut().write_all(response.as_bytes()).await;
        }
    });
    Upstream {
        url,
        received,
        task,
    }
}

struct Fixture {
    _root: tempfile::TempDir,
    broker: McpToolBroker,
    session: Arc<GovernedToolSession>,
    journal: Arc<ProtectedJournal>,
}
async fn new_fixture(url: &str, output_budget: u64, timeout: u64) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let id = AgentId::new();
    let journal = Arc::new(ProtectedJournal::create(root.path(), id).unwrap());
    let manifest: Manifest = toml::from_str(
        r#"
[tool]
name = "unused"
version = "1"
binary = "echo"
description = "Never dispatched by inference fixtures"
[command]
template = "echo unused"
[output]
format = "text"
"#,
    )
    .unwrap();
    let session = Arc::new(
        GovernedToolSession::start(
            Arc::new(ToolCladExecutor::new(vec![("unused".into(), manifest)])),
            Arc::new(DefaultPolicyGate::new()),
            journal.clone(),
            LoopState::new(id, Conversation::with_system("protected fixture")),
            LoopConfig {
                timeout: Duration::from_secs(10),
                ..Default::default()
            },
        )
        .await
        .unwrap(),
    );
    let inference = ProtectedInference::new(
        InferenceBrokerConfig {
            base_url: url.into(),
            model: "fixed-model".into(),
            api_key_env: "SYNTHETIC_PROVIDER_KEY".into(),
            max_requests: 8,
            max_output_tokens_per_request: 8,
            request_timeout_seconds: timeout,
            beta_headers: vec![],
        },
        KEY.into(),
        output_budget,
        Instant::now() + Duration::from_secs(10),
        session.clone(),
    )
    .unwrap();
    let broker = McpToolBroker::start_with_inference(session.clone(), root.path(), Some(inference))
        .await
        .unwrap();
    Fixture {
        _root: root,
        broker,
        session,
        journal,
    }
}
fn request() -> Value {
    json!({"model":"fixed-model","max_tokens":5,"messages":[{"role":"user","content":"synthetic prompt"}]})
}
async fn exchange(fixture: &Fixture, path: &str, body: &str) -> String {
    let mut stream = UnixStream::connect(
        fixture
            .broker
            .socket_path()
            .with_file_name("inference.sock"),
    )
    .await
    .unwrap();
    let message = format!("POST {path} HTTP/1.1\r\nHost: forged-host\r\nX-Api-Key: forged-key\r\nAuthorization: forged-token\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    stream.write_all(message.as_bytes()).await.unwrap();
    let mut response = String::new();
    tokio::time::timeout(Duration::from_secs(4), stream.read_to_string(&mut response))
        .await
        .unwrap()
        .unwrap();
    response
}

#[tokio::test]
async fn inference_reconstructs_headers_normalizes_json_and_reserves_output_budget() {
    let mut server = upstream(
        200,
        "{\"content\":\"approved answer\"}".into(),
        Duration::ZERO,
    )
    .await;
    let fixture = new_fixture(&server.url, 10, 2).await;
    let duplicate =
        r#"{"model":"forged-model","model":"fixed-model","max_tokens":5,"messages":[]}"#;
    let response = exchange(&fixture, "/v1/messages", duplicate).await;
    assert!(
        response.contains("200") && response.contains("approved answer"),
        "{response}"
    );
    let (headers, body, raw) = server.received.recv().await.unwrap();
    assert!(headers.contains(KEY));
    assert!(
        !headers.contains("forged") && !headers.to_ascii_lowercase().contains("authorization:")
    );
    assert_eq!(body["model"], "fixed-model");
    assert!(!raw.contains("forged-model"));
    let mut history_hint = request();
    history_hint["context_management"] =
        json!({"edits":[{"type":"clear_thinking_20251015","keep":"all"}]});
    assert!(
        exchange(&fixture, "/v1/messages", &history_hint.to_string())
            .await
            .contains("approved answer")
    );
    server.received.recv().await.unwrap();
    assert!(exchange(&fixture, "/v1/messages", &request().to_string())
        .await
        .contains("budget exhausted"));
    assert!(server.received.try_recv().is_err());
    fixture.broker.close().await.unwrap();
    // Verify the exact persisted payload independently of typed JournalEntry
    // reconstruction, which can otherwise hide f32 serialization differences.
    #[derive(serde::Deserialize)]
    struct RawRecord {
        payload: Box<serde_json::value::RawValue>,
        signature: String,
    }
    use base64::Engine;
    let key = ed25519_dalek::VerifyingKey::from_bytes(&fixture.journal.public_key()).unwrap();
    for line in std::fs::read_to_string(fixture.journal.path())
        .unwrap()
        .lines()
    {
        let record: RawRecord = serde_json::from_str(line).unwrap();
        let signature = base64::engine::general_purpose::STANDARD
            .decode(record.signature)
            .unwrap();
        key.verify_strict(
            record.payload.get().as_bytes(),
            &ed25519_dalek::Signature::from_slice(&signature).unwrap(),
        )
        .unwrap();
    }
    let entries =
        ProtectedJournal::verify(fixture.journal.path(), &fixture.journal.public_key()).unwrap();
    assert_eq!(
        entries
            .iter()
            .filter(|entry| matches!(entry.event, LoopEvent::InferenceRequested { .. }))
            .count(),
        2
    );
    assert_eq!(
        entries
            .iter()
            .filter(|entry| matches!(entry.event, LoopEvent::InferenceCompleted { .. }))
            .count(),
        2
    );
    assert!(!std::fs::read_to_string(fixture.journal.path())
        .unwrap()
        .contains(KEY));
}

#[tokio::test]
async fn inference_denies_model_routes_server_tools_and_remote_sources_before_send() {
    let mut server = upstream(200, "{}".into(), Duration::ZERO).await;
    let fixture = new_fixture(&server.url, 100, 2).await;
    let mut candidates = vec![];
    let mut body = request();
    body["model"] = json!("other-model");
    candidates.push(body);
    let mut body = request();
    body["mcp_servers"] = json!([]);
    candidates.push(body);
    let mut body = request();
    body["tools"] = json!([{"type":"web_search_20250305","name":"web_search"}]);
    candidates.push(body);
    let mut body = request();
    body["messages"][0]["content"] =
        json!([{"type":"image","source":{"type":"url","url":"http://127.0.0.1:9/private"}}]);
    candidates.push(body);
    let mut body = request();
    body["max_tokens"] = json!(9);
    candidates.push(body);
    for context in [
        json!({"edits":[{"type":"compact_20260112"}]}),
        json!({"edits":[],"future_remote_service":"https://example.test"}),
    ] {
        let mut body = request();
        body["context_management"] = context;
        candidates.push(body);
    }
    for body in candidates {
        assert!(exchange(&fixture, "/v1/messages", &body.to_string())
            .await
            .contains("400"));
    }
    assert!(exchange(&fixture, "/v1/other", &request().to_string())
        .await
        .contains("unavailable"));
    assert!(server.received.try_recv().is_err());
    fixture.broker.close().await.unwrap();
}

#[tokio::test]
async fn inference_does_not_follow_redirects_or_return_reflected_credentials() {
    let mut server = upstream(302, KEY.into(), Duration::ZERO).await;
    let fixture = new_fixture(&server.url, 20, 2).await;
    let response = exchange(&fixture, "/v1/messages", &request().to_string()).await;
    assert!(response.contains("302") && !response.contains(KEY));
    assert!(!response.contains("Location:"));
    server.received.recv().await.unwrap();
    assert!(server.received.try_recv().is_err());
    fixture.broker.close().await.unwrap();

    let server = upstream(200, json!({"echo":KEY}).to_string(), Duration::ZERO).await;
    let fixture = new_fixture(&server.url, 20, 2).await;
    let response = exchange(&fixture, "/v1/messages", &request().to_string()).await;
    assert!(!response.contains(KEY));
    assert!(fixture.broker.close().await.is_err());
    let entries =
        ProtectedJournal::verify(fixture.journal.path(), &fixture.journal.public_key()).unwrap();
    assert!(matches!(
        entries.last().unwrap().event,
        LoopEvent::Terminated {
            reason: TerminationReason::Error { .. },
            ..
        }
    ));
}

#[tokio::test]
async fn inference_timeout_cancels_the_session_and_preserves_interrupted_evidence() {
    let mut server = upstream(200, "{}".into(), Duration::from_secs(2)).await;
    let fixture = new_fixture(&server.url, 5, 1).await;
    assert!(!exchange(&fixture, "/v1/messages", &request().to_string())
        .await
        .contains("200"));
    server.received.recv().await.unwrap();
    assert!(fixture.broker.close().await.is_err());
    assert!(fixture
        .session
        .call(symbi_runtime::reasoning::governed_session::BrokerToolCall {
            call_id: "late".into(),
            name: "unused".into(),
            arguments: json!({}),
        })
        .await
        .is_err());
    let entries =
        ProtectedJournal::verify(fixture.journal.path(), &fixture.journal.public_key()).unwrap();
    assert!(entries
        .iter()
        .any(|entry| matches!(entry.event, LoopEvent::InferenceRequested { .. })));
    assert!(!entries
        .iter()
        .any(|entry| matches!(entry.event, LoopEvent::InferenceCompleted { .. })));
}

#[tokio::test]
async fn inference_journal_failure_prevents_receiver_effects() {
    let mut server = upstream(200, "{}".into(), Duration::ZERO).await;
    let fixture = new_fixture(&server.url, 20, 2).await;
    // Simulate external storage damage after the successful start checkpoint.
    std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(fixture.journal.path())
        .unwrap();
    let _ = exchange(&fixture, "/v1/messages", &request().to_string()).await;
    assert!(server.received.try_recv().is_err());
    assert!(fixture.broker.close().await.is_err());
}

#[tokio::test]
async fn signed_journal_survives_reopen_concurrent_writes_and_detects_tampering() {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let id = AgentId::new();
    let journal = Arc::new(ProtectedJournal::create(root.path(), id).unwrap());
    let mut tasks = vec![];
    for iteration in 0..12 {
        let journal = journal.clone();
        tasks.push(tokio::spawn(async move {
            let mut observation = symbi_runtime::reasoning::loop_types::Observation::tool_result(
                "probe",
                "synthetic",
            );
            observation.metadata.insert("b".into(), "two".into());
            observation.metadata.insert("a".into(), "one".into());
            journal
                .append(JournalEntry {
                    sequence: 0,
                    timestamp: std::time::SystemTime::now().into(),
                    agent_id: id,
                    iteration,
                    event: LoopEvent::ToolBatchCompleted {
                        iteration,
                        observations: vec![observation],
                        duration: Duration::ZERO,
                    },
                })
                .await
                .unwrap();
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    let path = journal.path().to_owned();
    let key = journal.public_key();
    drop(journal);
    let entries = ProtectedJournal::verify(&path, &key).unwrap();
    assert_eq!(entries.len(), 12);
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.sequence)
            .collect::<Vec<_>>(),
        (0..12).collect::<Vec<_>>()
    );
    let reopened = ProtectedJournal::create(root.path(), AgentId::new()).unwrap();
    assert_eq!(reopened.public_key(), key);
    assert!(ProtectedJournal::create(root.path(), id).is_err());
    let mut content = std::fs::read_to_string(&path).unwrap();
    content = content.replacen("synthetic", "tampered!", 1);
    std::fs::write(&path, content).unwrap();
    assert!(ProtectedJournal::verify(&path, &key).is_err());
    assert_eq!(
        std::fs::metadata(root.path().join("audit-signing.key"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn project_inference_config_rejects_credentials_and_unbounded_settings_before_use() {
    let root = tempfile::tempdir().unwrap();
    for (url, extra) in [
        ("https://user:synthetic-private-value@example.test", ""),
        ("https://example.test?key=synthetic-private-value", ""),
        ("https://example.test#synthetic-private-value", ""),
        ("file:///tmp/provider", ""),
        ("https://example.test", "max_requests = 0"),
        (
            "https://example.test",
            "max_output_tokens_per_request = 32769",
        ),
        ("https://example.test", "request_timeout_seconds = 121"),
        ("https://example.test", "beta_headers = ['invalid header']"),
    ] {
        std::fs::write(root.path().join("symbiont.toml"), format!(
            "[managed_cli.inference]\nbase_url = {url:?}\nmodel = 'fixed-model'\napi_key_env = 'SYNTHETIC_KEY'\n{extra}\n"
        )).unwrap();
        let error = InferenceBrokerConfig::from_project(root.path())
            .err()
            .unwrap();
        assert!(!error.contains("synthetic-private-value"));
    }
    std::fs::write(root.path().join("symbiont.toml"),
        "[managed_cli.inference]\nbase_url = 'http://127.0.0.1:1234'\nmodel = 'fixed-model'\napi_key_env = 'SYNTHETIC_KEY'\n"
    ).unwrap();
    assert!(InferenceBrokerConfig::from_project(root.path()).is_ok());
}
