//! Real CDP transport fixture, not evidence of a shipping browser executor.
//! All page responses are synthetic; no external network request is permitted.
use crate::sandbox::{
    command::CommandBoundary,
    streams::{Reader, StreamGuard, Writer},
};
use base64::Engine;
use serde_json::{json, Value};
use std::{collections::HashMap, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

#[path = "broker_tests.rs"]
mod broker_tests;

const PIPE: &str = include_str!("pipe.py");
const DOCUMENT: &str = r#"<!doctype html><title>Contained browser fixture</title>
<button id="mutate" onclick="document.getElementById('result').textContent='changed'">Change</button>
<p id="result">initial</p><img src="https://blocked.invalid/pixel" onerror="this.dataset.denied='true'">"#;

struct Probe {
    stdin: Writer,
    stdout: BufReader<Reader>,
    guard: StreamGuard,
    stderr: tokio::task::JoinHandle<Vec<u8>>,
    pending: HashMap<u64, &'static str>,
    replies: HashMap<u64, Value>,
    requests: Vec<String>,
    next_id: u64,
    received: usize,
    broker: Option<broker_tests::Broker>,
}

impl Probe {
    async fn send(&mut self, method: &'static str, params: Value, session: Option<&str>) -> u64 {
        assert!(self.pending.len() < 32, "too many pending CDP requests");
        self.next_id += 1;
        let id = self.next_id;
        let mut message = json!({"id": id, "method": method, "params": params});
        if let Some(session) = session {
            message["sessionId"] = json!(session);
        }
        let mut bytes = serde_json::to_vec(&message).unwrap();
        bytes.push(0);
        self.stdin.write_all(&bytes).await.unwrap();
        self.stdin.flush().await.unwrap();
        self.pending.insert(id, method);
        id
    }

    async fn next_message(&mut self) -> Value {
        let mut frame = Vec::new();
        loop {
            let bytes = self.stdout.fill_buf().await.unwrap();
            assert!(!bytes.is_empty(), "CDP ended before a complete response");
            let end = bytes.iter().position(|byte| *byte == 0);
            let used = end.map_or(bytes.len(), |end| end + 1);
            self.received += used;
            assert!(self.received <= 4 * 1024 * 1024, "CDP stream limit");
            assert!(frame.len() + used <= 1024 * 1024, "CDP frame limit");
            frame.extend_from_slice(&bytes[..end.unwrap_or(used)]);
            self.stdout.consume(used);
            if end.is_some() {
                return serde_json::from_slice(&frame).expect("valid CDP JSON");
            }
        }
    }

    async fn request(
        &mut self,
        method: &'static str,
        params: Value,
        session: Option<&str>,
    ) -> Value {
        let id = self.send(method, params, session).await;
        loop {
            if let Some(reply) = self.replies.remove(&id) {
                return reply;
            }
            let message = self.next_message().await;
            if let Some(reply_id) = message["id"].as_u64() {
                let method = self.pending.remove(&reply_id).expect("known CDP reply ID");
                assert!(message.get("error").is_none(), "{method}: {message}");
                // Interception acknowledgements are checked but are not returned
                // to a command caller. Never continue a request onto the network.
                if !matches!(method, "Fetch.fulfillRequest" | "Fetch.failRequest") {
                    assert!(self
                        .replies
                        .insert(reply_id, message["result"].clone())
                        .is_none());
                }
            } else if message["method"] == "Fetch.requestPaused" {
                assert!(self.requests.len() < 16, "browser request limit");
                let params = &message["params"];
                let url = params["request"]["url"].as_str().unwrap();
                self.requests.push(url.into());
                let request_id = params["requestId"].clone();
                let session = message["sessionId"]
                    .as_str()
                    .expect("attached page session");
                if let Some(broker) = self.broker.as_mut() {
                    match broker.handle(params).await {
                        Ok(response) => {
                            self.send("Fetch.fulfillRequest", response, Some(session))
                                .await;
                        }
                        Err(error) => {
                            println!("refused browser request {url}: {error}");
                            self.send(
                                "Fetch.failRequest",
                                json!({"requestId": request_id, "errorReason":"BlockedByClient"}),
                                Some(session),
                            )
                            .await;
                        }
                    }
                } else if url == "https://fixture.invalid/" && params["request"]["method"] == "GET"
                {
                    self.send("Fetch.fulfillRequest", json!({
                        "requestId": request_id, "responseCode": 200,
                        "responseHeaders": [{"name": "Content-Type", "value": "text/html; charset=utf-8"}],
                        "body": base64::engine::general_purpose::STANDARD.encode(DOCUMENT)
                    }), Some(session)).await;
                } else {
                    self.send(
                        "Fetch.failRequest",
                        json!({
                            "requestId": request_id, "errorReason": "BlockedByClient"
                        }),
                        Some(session),
                    )
                    .await;
                }
            }
        }
    }

    async fn close(mut self) {
        self.guard
            .finish()
            .await
            .expect("acknowledged worker removal");
        let stderr = tokio::time::timeout(Duration::from_secs(3), &mut self.stderr)
            .await
            .expect("stderr reader stopped")
            .unwrap();
        assert!(stderr.len() <= 128 * 1024, "bounded browser diagnostics");
    }
}

async fn owned_containers(label: &str) -> Vec<Value> {
    let output = tokio::process::Command::new("docker")
        .args(["ps", "-aq", "--filter", &format!("label={label}")])
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    let ids = String::from_utf8(output.stdout).unwrap();
    if ids.trim().is_empty() {
        return Vec::new();
    }
    let output = tokio::process::Command::new("docker")
        .arg("inspect")
        .args(ids.split_whitespace())
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    serde_json::from_slice(&output.stdout).unwrap()
}

async fn start(label: &str, lifetime: Duration) -> Probe {
    // Pin the locally provisioned artifact; missing prerequisites fail the test.
    let image =
        std::env::var("SYMBI_BROWSER_TEST_IMAGE").expect("set the cached browser image digest");
    assert!(image.starts_with("sha256:") && image.len() == 71);
    let mut boundary = CommandBoundary::default();
    boundary.docker.image = image.clone();
    boundary.docker.working_dir = "/tmp".into();
    boundary.docker.max_execution_time = lifetime;
    boundary.docker.extra_flags = vec![format!("--label={label}")];
    let argv = [
        "python3",
        "-I",
        "-c",
        PIPE,
        "/usr/bin/chromium",
        "--headless",
        "--no-sandbox",
        "--disable-gpu",
        "--disable-dev-shm-usage",
        "--disable-background-networking",
        "--no-first-run",
        "--no-default-browser-check",
        "--user-data-dir=/tmp/browser-profile",
        "--remote-debugging-pipe",
        "about:blank",
    ]
    .map(String::from);
    let streams = boundary
        .spawn_streams(&argv, HashMap::new(), lifetime)
        .await
        .unwrap();
    let diagnostic = tokio::spawn(async move {
        let mut bytes = Vec::new();
        streams
            .stderr
            .take(128 * 1024 + 1)
            .read_to_end(&mut bytes)
            .await
            .unwrap();
        bytes
    });
    let workers = owned_containers(label).await;
    assert_eq!(workers.len(), 1, "exactly one owned browser worker");
    let worker = &workers[0];
    assert_eq!(worker["Image"], image);
    assert_eq!(worker["HostConfig"]["NetworkMode"], "none");
    assert_eq!(worker["HostConfig"]["ReadonlyRootfs"], true);
    assert_eq!(worker["HostConfig"]["Privileged"], false);
    assert_eq!(worker["Config"]["User"], "65534:65534");
    assert!(worker["Mounts"]
        .as_array()
        .unwrap()
        .iter()
        .all(|m| m["Type"] == "tmpfs"));
    assert!(worker["HostConfig"]["CapDrop"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v.as_str().is_some_and(|s| s.eq_ignore_ascii_case("all"))));
    Probe {
        stdin: streams.stdin,
        stdout: BufReader::new(streams.stdout),
        guard: streams.guard,
        stderr: diagnostic,
        pending: HashMap::new(),
        replies: HashMap::new(),
        requests: Vec::new(),
        next_id: 0,
        received: 0,
        broker: None,
    }
}

#[tokio::test]
#[ignore = "requires Docker, supervisor, and SYMBI_BROWSER_TEST_IMAGE pinned to cached Chromium"]
async fn contained_chromium_pipe_navigates_and_interacts_without_network() {
    let label = format!("symbi.browser-fixture={}", uuid::Uuid::new_v4());
    tokio::time::timeout(Duration::from_secs(35), async {
        let mut probe = start(&label, Duration::from_secs(30)).await;
        let result = probe.request("Browser.getVersion", json!({}), None).await;
        assert!(result["product"].as_str().unwrap().contains("Chrome"));
        println!("browser artifact: {result}");
        let target = probe.request("Target.createTarget", json!({"url": "about:blank"}), None).await;
        let session = probe.request("Target.attachToTarget", json!({"targetId": target["targetId"], "flatten": true}), None).await;
        let session = session["sessionId"].as_str().unwrap();
        probe.request("Page.enable", json!({}), Some(session)).await;
        probe.request("Fetch.enable", json!({}), Some(session)).await;
        let navigation = probe.request("Page.navigate", json!({"url": "https://fixture.invalid/"}), Some(session)).await;
        assert!(navigation.get("errorText").is_none(), "{navigation}");
        let page = probe.request("Runtime.evaluate", json!({
            "expression": "new Promise(resolve => { const done = () => { document.getElementById('mutate').click(); resolve({title: document.title, value: document.getElementById('result').textContent, denied: document.querySelector('img').dataset.denied}); }; if (document.readyState === 'complete') done(); else addEventListener('load', done, {once:true}); })",
            "awaitPromise": true, "returnByValue": true
        }), Some(session)).await;
        assert!(page.get("exceptionDetails").is_none(), "{page}");
        assert_eq!(page["result"]["value"], json!({"title":"Contained browser fixture", "value":"changed", "denied":"true"}));
        assert_eq!(probe.requests, ["https://fixture.invalid/", "https://blocked.invalid/pixel"]);
        // Drain interception acknowledgements before treating the scenario as done.
        probe.request("Runtime.evaluate", json!({"expression":"1"}), Some(session)).await;
        assert!(probe.pending.is_empty());
        assert!(probe.replies.is_empty());
        probe.close().await;
        assert!(owned_containers(&label).await.is_empty());
    }).await.expect("bounded browser fixture lifetime");
}

#[tokio::test]
#[ignore = "requires Docker, supervisor, and SYMBI_BROWSER_TEST_IMAGE pinned to cached Chromium"]
async fn independent_deadline_removes_a_live_chromium_worker() {
    let label = format!("symbi.browser-fixture={}", uuid::Uuid::new_v4());
    tokio::time::timeout(Duration::from_secs(25), async {
        let mut probe = start(&label, Duration::from_secs(5)).await;
        let result = probe.request("Browser.getVersion", json!({}), None).await;
        assert!(result["product"].as_str().unwrap().contains("Chrome"));
        // Retain the host handle and leave Chrome running. Only the independent
        // lifetime owner can end this worker; no Browser.close is sent.
        loop {
            if owned_containers(&label).await.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        probe.close().await;
        assert!(owned_containers(&label).await.is_empty());
    })
    .await
    .expect("independent browser deadline and cleanup");
}
