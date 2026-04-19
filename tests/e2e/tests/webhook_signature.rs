//! E2E-10: webhook signature verification.
//!
//! Boots `HttpInputServer` with a GitHub-style HMAC `webhook_verify`
//! config and drives the webhook endpoint with valid / invalid / missing
//! signatures. The HMAC is computed here so the test exercises the real
//! `WebhookVerifier` pipeline inside the runtime.

#![cfg(feature = "e2e")]

use std::sync::Arc;
use std::time::Duration;

use hmac::{Hmac, Mac};
use sha2::Sha256;
use symbi_e2e::{pick_free_port, test_client};
use symbi_runtime::http_input::{HttpInputConfig, HttpInputServer, WebhookVerifyConfig};
use symbi_runtime::{AgentId, AgentRuntime, RuntimeConfig};

type HmacSha256 = Hmac<Sha256>;

fn github_sig(secret: &[u8], body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("hmac key");
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

async fn spawn_http_input(secret: &str) -> (String, tokio::task::JoinHandle<()>) {
    let runtime = Arc::new(
        AgentRuntime::new(RuntimeConfig::default())
            .await
            .expect("runtime"),
    );
    let port = pick_free_port().await;
    let config = HttpInputConfig {
        bind_address: "127.0.0.1".to_string(),
        port,
        path: "/webhook".to_string(),
        agent: AgentId::new(),
        auth_header: None,
        jwt_public_key_path: None,
        max_body_bytes: 65_536,
        concurrency: 4,
        routing_rules: None,
        response_control: None,
        forward_headers: vec![],
        cors_origins: vec![],
        audit_enabled: false,
        webhook_verify: Some(WebhookVerifyConfig {
            provider: "github".to_string(),
            secret: secret.to_string(),
        }),
    };
    let server = HttpInputServer::new(config).with_runtime(runtime);
    let handle = tokio::spawn(async move {
        let _ = server.start().await;
    });

    // Wait for port.
    let url = format!("http://127.0.0.1:{}", port);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            break;
        }
        if std::time::Instant::now() > deadline {
            panic!("http_input on port {} never came up", port);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    (url, handle)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn webhook_accepts_valid_hmac_signature() {
    let secret = "github-secret";
    let (base_url, handle) = spawn_http_input(secret).await;
    let client = test_client();

    let body = br#"{"action":"opened","issue":{"number":1}}"#;
    let sig = github_sig(secret.as_bytes(), body);

    let resp = client
        .post(format!("{}/webhook", base_url))
        .header("Content-Type", "application/json")
        .header("X-Hub-Signature-256", sig)
        .body(body.to_vec())
        .send()
        .await
        .expect("req");

    // The runtime rejects because the target AgentId isn't registered,
    // but crucially the signature gate passed — we should NOT see 401.
    assert_ne!(
        resp.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "valid signature must not be rejected as 401 — got {}, body: {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );

    handle.abort();
    let _ = handle.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn webhook_rejects_invalid_hmac_signature() {
    let secret = "github-secret";
    let (base_url, handle) = spawn_http_input(secret).await;
    let client = test_client();

    let body = br#"{"action":"opened"}"#;
    let bad = "sha256=deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

    let resp = client
        .post(format!("{}/webhook", base_url))
        .header("Content-Type", "application/json")
        .header("X-Hub-Signature-256", bad)
        .body(body.to_vec())
        .send()
        .await
        .expect("req");

    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    handle.abort();
    let _ = handle.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn webhook_rejects_missing_signature_header() {
    let secret = "github-secret";
    let (base_url, handle) = spawn_http_input(secret).await;
    let client = test_client();

    let body = br#"{"action":"opened"}"#;
    let resp = client
        .post(format!("{}/webhook", base_url))
        .header("Content-Type", "application/json")
        .body(body.to_vec())
        .send()
        .await
        .expect("req");

    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    handle.abort();
    let _ = handle.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn webhook_rejects_tampered_body() {
    // A valid signature for one body must NOT verify after the body is
    // modified in flight — the signature should fail.
    let secret = "github-secret";
    let (base_url, handle) = spawn_http_input(secret).await;
    let client = test_client();

    let original = br#"{"action":"opened"}"#;
    let tampered = br#"{"action":"closed"}"#;
    let sig = github_sig(secret.as_bytes(), original);

    let resp = client
        .post(format!("{}/webhook", base_url))
        .header("Content-Type", "application/json")
        .header("X-Hub-Signature-256", sig)
        .body(tampered.to_vec())
        .send()
        .await
        .expect("req");

    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    handle.abort();
    let _ = handle.await;
}
