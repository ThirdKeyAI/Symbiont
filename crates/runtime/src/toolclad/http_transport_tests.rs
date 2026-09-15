//! Local TCP effects through the governed dispatcher and shared HTTP transport.
use super::*;
use crate::reasoning::{
    circuit_breaker::CircuitBreakerRegistry,
    conversation::Conversation,
    dispatch::GovernedToolDispatcher,
    executor::ActionExecutor,
    inference::ToolDefinition,
    loop_types::{
        BufferedJournal, JournalEntry, JournalError, JournalWriter, LoopConfig, LoopEvent,
        LoopState, Observation, ProposedAction,
    },
    policy_bridge::DefaultPolicyGate,
    prepared::AuthorizedAction,
    protected_journal::ProtectedJournal,
};
use serde_json::json;
use std::os::unix::fs::PermissionsExt;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

fn client() -> reqwest::Client {
    // This fixture owns an explicit loopback sink. Production callers select
    // the SSRF-safe client and validate destinations before this transport.
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap()
}

#[tokio::test]
async fn request_identity_covers_each_effect_input() {
    let client = client();
    let build = |method, url, header, body| {
        client
            .request(method, url)
            .header("x-fixture", header)
            .body(body)
            .build()
            .unwrap()
    };
    let original = build(
        reqwest::Method::POST,
        "https://fixture.invalid/first",
        "value",
        " body ",
    );
    let expected = request_identity(&original).unwrap();
    for changed in [
        build(
            reqwest::Method::GET,
            "https://fixture.invalid/first",
            "value",
            " body ",
        ),
        build(
            reqwest::Method::POST,
            "https://fixture.invalid/second",
            "value",
            " body ",
        ),
        build(
            reqwest::Method::POST,
            "https://fixture.invalid/first",
            "changed",
            " body ",
        ),
        build(
            reqwest::Method::POST,
            "https://fixture.invalid/first",
            "value",
            "body",
        ),
    ] {
        assert_ne!(request_identity(&changed).unwrap().0, expected.0);
    }
}

#[tokio::test]
async fn request_bodies_must_be_finite_and_within_the_byte_budget() {
    let client = client();
    let oversized = client
        .post("https://fixture.invalid/")
        .body(vec![b'x'; 2 * 1024 * 1024 + 1])
        .build()
        .unwrap();
    assert!(request_identity(&oversized).unwrap_err().contains("2 MiB"));
    let stream = futures::stream::iter([Ok::<_, std::io::Error>(vec![b'x'])]);
    let streamed = client
        .post("https://fixture.invalid/")
        .body(reqwest::Body::wrap_stream(stream))
        .build()
        .unwrap();
    assert!(request_identity(&streamed)
        .unwrap_err()
        .contains("streaming"));
}

async fn read_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let mut buffer = [0; 1024];
        let count = stream.read(&mut buffer).await.unwrap();
        assert!(count > 0 && bytes.len() + count < 16 * 1024);
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
            let length: usize = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .unwrap_or("0")
                .parse()
                .unwrap();
            if bytes.len() >= end + 4 + length {
                return bytes;
            }
        }
    }
}

struct EffectExecutor {
    url: String,
    retained: Arc<Mutex<Option<EffectJournal>>>,
}
impl EffectExecutor {
    fn request(&self, client: &reqwest::Client) -> reqwest::Request {
        client
            .post(&self.url)
            .header("x-fixture", "synthetic-header")
            .body("allowed-body")
            .build()
            .unwrap()
    }
}
#[async_trait::async_trait]
impl ActionExecutor for EffectExecutor {
    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        vec![ToolDefinition {
            name: "network_fixture".into(),
            description: "Governed local network fixture".into(),
            parameters: json!({"type":"object","properties":{}}),
        }]
    }
    async fn execute_actions(
        &self,
        _: &[ProposedAction],
        _: &LoopConfig,
        _: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        panic!("network fixture requires exact authorized dispatch")
    }
    async fn execute_authorized(
        &self,
        actions: Vec<AuthorizedAction>,
        _: &LoopConfig,
        _: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        let mut output = Vec::new();
        for grant in actions {
            grant.check_live().unwrap();
            let ProposedAction::ToolCall { call_id, name, .. } = grant.action() else {
                panic!("tool call")
            };
            let audit = grant
                .effect_journal()
                .expect("dispatcher attaches a call journal");
            *self.retained.lock().unwrap() = Some(audit.clone());
            let client = client();
            let response = exchange(
                &client,
                self.request(&client),
                Some(&audit),
                grant.deadline(),
                1024,
            )
            .await;
            output.push(match response {
                Ok(response) => {
                    Observation::tool_result(name, String::from_utf8(response.body).unwrap())
                        .with_call_id(call_id)
                }
                Err(error) => Observation::tool_error(name, error).with_call_id(call_id),
            });
        }
        output
    }
}

async fn dispatch(
    executor: &EffectExecutor,
    journal: &dyn JournalWriter,
    state: &LoopState,
) -> Result<Vec<Observation>, JournalError> {
    let config = LoopConfig {
        tool_definitions: executor.tool_definitions(),
        tool_timeout: Duration::from_secs(3),
        ..Default::default()
    };
    GovernedToolDispatcher {
        executor,
        journal,
        gate: &DefaultPolicyGate::permissive_for_dev_only(),
        circuit_breakers: &CircuitBreakerRegistry::default(),
    }
    .dispatch(
        &[ProposedAction::ToolCall {
            call_id: "network-call".into(),
            name: "network_fixture".into(),
            arguments: "{}".into(),
        }],
        state,
        &config,
    )
    .await
}

#[tokio::test]
async fn signed_request_record_precedes_the_socket_and_matches_the_call() {
    let sink = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let state = LoopState::new(crate::types::AgentId::new(), Conversation::new());
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let journal = Arc::new(ProtectedJournal::create(root.path(), state.agent_id).unwrap());
    let executor = EffectExecutor {
        url: format!("http://{}/effect", sink.local_addr().unwrap()),
        retained: Arc::default(),
    };
    let expected_hash = request_identity(&executor.request(&client())).unwrap();
    let observer = journal.clone();
    let server = tokio::spawn(async move {
        let (mut stream, _) = sink.accept().await.unwrap();
        let entries = ProtectedJournal::verify(observer.path(), &observer.public_key()).unwrap();
        assert_eq!(
            entries.len(),
            3,
            "policy, dispatch and request are persisted before connection"
        );
        assert!(matches!(
            entries[2].event,
            LoopEvent::ToolEffect {
                effect: ToolEffect::NetworkRequestStarted { .. },
                ..
            }
        ));
        let request = read_request(&mut stream).await;
        assert!(request.starts_with(b"POST /effect HTTP/1.1\r\n"));
        assert!(request.ends_with(b"allowed-body"));
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nallowed")
            .await
            .unwrap();
    });
    let results = dispatch(&executor, journal.as_ref(), &state).await.unwrap();
    assert_eq!(results.len(), 1);
    assert!(!results[0].is_error, "{:?}", results[0]);
    assert_eq!(results[0].content, "allowed");
    server.await.unwrap();
    let entries = ProtectedJournal::verify(journal.path(), &journal.public_key()).unwrap();
    assert_eq!(entries.len(), 6);
    assert!(matches!(
        entries[1].event,
        LoopEvent::ToolDispatchStarted { .. }
    ));
    assert!(matches!(
        entries[4].event,
        LoopEvent::ToolDispatchFinished {
            is_error: false,
            ..
        }
    ));
    let LoopEvent::PolicyEvaluated { approved_calls, .. } = &entries[0].event else {
        panic!("policy")
    };
    let fingerprint = approved_calls[0]["fingerprint"].as_str().unwrap();
    let LoopEvent::ToolEffect {
        run_key,
        call_fingerprint,
        effect:
            ToolEffect::NetworkRequestStarted {
                request_id,
                request_hash,
                request_bytes,
                ..
            },
    } = &entries[2].event
    else {
        panic!("request")
    };
    assert_eq!(call_fingerprint, fingerprint);
    assert!(!run_key.is_empty());
    assert_eq!((request_hash.clone(), *request_bytes), expected_hash);
    let LoopEvent::ToolEffect {
        call_fingerprint,
        effect:
            ToolEffect::NetworkRequestFinished {
                request_id: finished,
                status,
                response_hash,
                response_bytes,
                error,
                ..
            },
        ..
    } = &entries[3].event
    else {
        panic!("response")
    };
    assert_eq!(call_fingerprint, fingerprint);
    assert_eq!(finished, request_id);
    assert_eq!(*status, Some(200));
    assert_eq!(*response_bytes, 7);
    assert_eq!(
        response_hash.as_deref(),
        Some(hex::encode(Sha256::digest(b"allowed")).as_str())
    );
    assert!(error.is_none());
    assert!(entries.iter().all(|e| e.agent_id == state.agent_id));
    assert!(!std::fs::read_to_string(journal.path())
        .unwrap()
        .contains("synthetic-header"));
    // A worker retaining the handle cannot obtain another record after return.
    let retained = executor.retained.lock().unwrap().clone().unwrap();
    assert!(retained.check_live().is_err());
    assert!(retained
        .append(ToolEffect::NetworkRequestStarted {
            request_id: "late".into(),
            method: "POST".into(),
            url: executor.url.clone(),
            request_hash: "none".into(),
            request_bytes: 0
        })
        .await
        .is_err());
    assert_eq!(
        ProtectedJournal::verify(journal.path(), &journal.public_key())
            .unwrap()
            .len(),
        6
    );
}

struct InterruptedJournal {
    writes: AtomicUsize,
    hold: bool,
    entered: tokio::sync::Notify,
}
#[async_trait::async_trait]
impl JournalWriter for InterruptedJournal {
    async fn append(&self, entry: JournalEntry) -> Result<(), JournalError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        if matches!(
            entry.event,
            LoopEvent::ToolEffect {
                effect: ToolEffect::NetworkRequestStarted { .. },
                ..
            }
        ) {
            self.entered.notify_one();
            if self.hold {
                std::future::pending::<()>().await;
            }
            return Err(JournalError::WriteFailed(
                "synthetic effect checkpoint failure".into(),
            ));
        }
        Ok(())
    }
    async fn next_sequence(&self) -> u64 {
        self.writes.load(Ordering::SeqCst) as u64
    }
}

#[tokio::test]
async fn failed_or_cancelled_effect_checkpoint_prevents_the_connection() {
    for hold in [false, true] {
        let sink = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let executor = Arc::new(EffectExecutor {
            url: format!("http://{}/effect", sink.local_addr().unwrap()),
            retained: Arc::default(),
        });
        let journal = Arc::new(InterruptedJournal {
            writes: AtomicUsize::new(0),
            hold,
            entered: tokio::sync::Notify::new(),
        });
        let actor = executor.clone();
        let writer = journal.clone();
        let task = tokio::spawn(async move {
            dispatch(
                actor.as_ref(),
                writer.as_ref(),
                &LoopState::new(crate::types::AgentId::new(), Conversation::new()),
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(2), journal.entered.notified())
            .await
            .unwrap();
        if hold {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            assert!(task
                .await
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("effect checkpoint failure"));
        }
        assert!(executor
            .retained
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .check_live()
            .is_err());
        assert!(
            tokio::time::timeout(Duration::from_millis(100), sink.accept())
                .await
                .is_err()
        );
        assert_eq!(journal.writes.load(Ordering::SeqCst), 3);
    }
}

#[tokio::test]
async fn response_limits_and_truncation_are_enforced_on_real_streams() {
    for (wire,expected) in [
        ("HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nallowed", "allowed"),
        ("HTTP/1.1 200 OK\r\nContent-Length: 10000\r\nConnection: close\r\n\r\n", "output limit"),
        ("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n8\r\nexcess!!\r\n0\r\n\r\n", "output limit"),
        ("HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nshort", "read failed"),
    ] {
        let sink = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/",sink.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream,_) = sink.accept().await.unwrap(); read_request(&mut stream).await;
            let _ = stream.write_all(wire.as_bytes()).await;
        });
        let client = client();
        let result = exchange(&client,client.get(url).build().unwrap(),None,Instant::now()+Duration::from_secs(2),7).await;
        match result {
            Ok(response) => assert_eq!(String::from_utf8(response.body).unwrap(),expected),
            Err(error) => assert!(error.contains(expected),"{error}"),
        }
        server.await.unwrap();
    }
}

#[tokio::test]
async fn incomplete_response_records_the_received_status_and_bytes() {
    let sink = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let executor = EffectExecutor {
        url: format!("http://{}/effect", sink.local_addr().unwrap()),
        retained: Arc::default(),
    };
    let server = tokio::spawn(async move {
        let (mut stream, _) = sink.accept().await.unwrap();
        read_request(&mut stream).await;
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nshort")
            .await
            .unwrap();
    });
    let journal = BufferedJournal::new(10);
    let result = dispatch(
        &executor,
        &journal,
        &LoopState::new(crate::types::AgentId::new(), Conversation::new()),
    )
    .await
    .unwrap();
    assert!(result[0].is_error && result[0].content.contains("read failed"));
    assert!(result[0].has_unconfirmed_effect());
    server.await.unwrap();
    let entries = journal.entries().await;
    let LoopEvent::ToolEffect {
        effect:
            ToolEffect::NetworkRequestFinished {
                status,
                response_bytes,
                response_hash,
                response_headers_hash,
                error,
                ..
            },
        ..
    } = &entries[3].event
    else {
        panic!("network outcome")
    };
    assert_eq!(*status, Some(200));
    assert_eq!(*response_bytes, 5);
    assert!(response_hash.is_none());
    assert!(response_headers_hash.is_some());
    assert!(error.is_some());
}

#[tokio::test]
async fn deadline_closes_a_stalled_response_connection() {
    let sink = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", sink.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = sink.accept().await.unwrap();
        read_request(&mut stream).await;
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\nx")
            .await
            .unwrap();
        let mut byte = [0];
        let closed = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut byte))
            .await
            .unwrap();
        assert!(
            matches!(closed, Ok(0))
                || matches!(closed,Err(e) if e.kind()==std::io::ErrorKind::ConnectionReset)
        );
    });
    let client = client();
    let result = exchange(
        &client,
        client.get(url).build().unwrap(),
        None,
        Instant::now() + Duration::from_millis(150),
        7,
    )
    .await;
    assert!(result.err().unwrap().contains("deadline expired"));
    server.await.unwrap();
}
