//! E2E for the AgentPin messaging gate (AP-1 end-to-end).
//!
//! Builds a runtime whose `agentpin_verifier` is a Mock, boots the HTTP
//! API, and drives `/api/v1/agents/:id/messages` with:
//! - no JWT → 401
//! - JWT whose subject matches sender → 2xx
//! - JWT whose subject mismatches → 401
//!
//! This complements the unit tests in
//! `crates/runtime/tests/agentpin_integration_tests.rs` by exercising
//! the *HTTP error mapping* (401 + AGENTPIN_VERIFICATION_FAILED code),
//! not just the helper function.

#![cfg(feature = "e2e")]

use std::sync::Arc;
use std::time::Duration;

use symbi_e2e::{pick_free_port, test_client};
use symbi_runtime::api::server::{HttpApiConfig, HttpApiServer};
use symbi_runtime::communication::CommunicationBus;
use symbi_runtime::integrations::MockAgentPinVerifier;
use symbi_runtime::{AgentId, AgentRuntime, RuntimeConfig};
use tempfile::tempdir;

const TEST_TOKEN: &str = "e2e-test-token-1234";

fn ensure_env() {
    std::env::set_var("SYMBIONT_API_TOKEN", TEST_TOKEN);
    std::env::remove_var("SYMBIONT_REFUSE_LEGACY_API_TOKEN");
}

/// Build a runtime with the provided AgentPin verifier and boot an
/// `HttpApiServer` against it. Returns `(base_url, runtime, handle, tempdir)`
/// — we can't reuse the helper in lib.rs because that one builds a
/// runtime without letting us inject a verifier.
async fn boot_with_verifier(
    verifier: Arc<dyn symbi_runtime::integrations::AgentPinVerifier>,
) -> (
    String,
    Arc<AgentRuntime>,
    tokio::task::JoinHandle<()>,
    tempfile::TempDir,
) {
    ensure_env();
    let td = tempdir().unwrap();
    let mut runtime = AgentRuntime::new(RuntimeConfig::default())
        .await
        .expect("runtime");
    runtime.agentpin_verifier = Some(verifier);
    let runtime = Arc::new(runtime);

    let port = pick_free_port().await;
    let cfg = HttpApiConfig {
        bind_address: "127.0.0.1".to_string(),
        port,
        enable_cors: false,
        enable_tracing: false,
        enable_rate_limiting: false,
        api_keys_file: None,
        serve_agents_md: false,
    };
    let mut server = HttpApiServer::new(cfg).with_runtime_provider(runtime.clone());
    let handle = tokio::spawn(async move {
        let _ = server.start().await;
    });

    // Wait for port.
    let base_url = format!("http://127.0.0.1:{}", port);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            break;
        }
        if std::time::Instant::now() > deadline {
            panic!("server never came up on port {}", port);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    (base_url, runtime, handle, td)
}

async fn register(bus: &dyn CommunicationBus, id: AgentId) {
    bus.register_agent(id).await.expect("register");
    tokio::time::sleep(Duration::from_millis(10)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_jwt_returns_401_with_agentpin_code() {
    let verifier = Arc::new(MockAgentPinVerifier::new_success());
    let (base_url, runtime, handle, _td) = boot_with_verifier(verifier).await;

    let sender = AgentId::new();
    let recipient = AgentId::new();
    register(runtime.communication.as_ref(), sender).await;
    register(runtime.communication.as_ref(), recipient).await;

    let client = test_client();
    let resp = client
        .post(format!(
            "{}/api/v1/agents/{}/messages",
            base_url, recipient.0
        ))
        .bearer_auth(TEST_TOKEN)
        .json(&serde_json::json!({
            "sender": sender,
            "payload": "hi",
            "ttl_seconds": 60,
        }))
        .send()
        .await
        .expect("req");

    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["code"], "AGENTPIN_VERIFICATION_FAILED");
    handle.abort();
    let _ = handle.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jwt_covering_sender_is_accepted() {
    let sender = AgentId::new();
    let recipient = AgentId::new();
    let verifier = Arc::new(MockAgentPinVerifier::with_identity(
        sender.0.to_string(),
        "test.example.com".to_string(),
        vec![],
    ));
    let (base_url, runtime, handle, _td) = boot_with_verifier(verifier).await;

    register(runtime.communication.as_ref(), sender).await;
    register(runtime.communication.as_ref(), recipient).await;

    let client = test_client();
    let resp = client
        .post(format!(
            "{}/api/v1/agents/{}/messages",
            base_url, recipient.0
        ))
        .bearer_auth(TEST_TOKEN)
        .json(&serde_json::json!({
            "sender": sender,
            "payload": "hi",
            "ttl_seconds": 60,
            "agentpin_jwt": "any.mock.jwt",
        }))
        .send()
        .await
        .expect("req");

    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    handle.abort();
    let _ = handle.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jwt_for_wrong_subject_is_rejected() {
    let verifier = Arc::new(MockAgentPinVerifier::with_identity(
        "someone-else".to_string(),
        "test.example.com".to_string(),
        vec![],
    ));
    let (base_url, runtime, handle, _td) = boot_with_verifier(verifier).await;

    let sender = AgentId::new();
    let recipient = AgentId::new();
    register(runtime.communication.as_ref(), sender).await;
    register(runtime.communication.as_ref(), recipient).await;

    let client = test_client();
    let resp = client
        .post(format!(
            "{}/api/v1/agents/{}/messages",
            base_url, recipient.0
        ))
        .bearer_auth(TEST_TOKEN)
        .json(&serde_json::json!({
            "sender": sender,
            "payload": "hi",
            "ttl_seconds": 60,
            "agentpin_jwt": "any.mock.jwt",
        }))
        .send()
        .await
        .expect("req");

    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["code"], "AGENTPIN_VERIFICATION_FAILED");
    handle.abort();
    let _ = handle.await;
}
