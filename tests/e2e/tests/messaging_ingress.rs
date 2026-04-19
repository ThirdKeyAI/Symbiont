//! E2E-3: body limit + TTL clamp on /api/v1/agents/:id/messages.
//!
//! Boots a full HttpApiServer, registers two agents on the underlying
//! bus, and drives the messaging endpoints via reqwest.

#![cfg(feature = "e2e")]

use std::time::Duration;

use symbi_e2e::{spawn_runtime_server, test_client, SpawnOptions};
use symbi_runtime::communication::CommunicationBus;
use symbi_runtime::AgentId;
use tempfile::tempdir;

// The test process inherits env vars from cargo. We set the legacy token
// once per test so the auth middleware is happy without an API key file.
const TEST_TOKEN: &str = "e2e-test-token-1234";

fn ensure_env() {
    // Belt-and-braces: clear API-key-store related vars and set the token.
    std::env::set_var("SYMBIONT_API_TOKEN", TEST_TOKEN);
    std::env::remove_var("SYMBIONT_REFUSE_LEGACY_API_TOKEN");
}

async fn register(bus: &dyn CommunicationBus, id: AgentId) {
    bus.register_agent(id).await.expect("register");
    // Give the event loop a tick to process the registration.
    tokio::time::sleep(Duration::from_millis(20)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn messaging_rejects_oversize_body() {
    ensure_env();
    let td = tempdir().unwrap();
    let server = spawn_runtime_server(SpawnOptions::new(td)).await;

    let recipient = AgentId::new();
    register(server.runtime.communication.as_ref(), recipient).await;

    let client = test_client();
    // 512 KiB payload — well over the 256 KiB messaging body cap.
    let big_payload: String = "A".repeat(512 * 1024);
    let resp = client
        .post(format!(
            "{}/api/v1/agents/{}/messages",
            server.base_url, recipient.0
        ))
        .bearer_auth(TEST_TOKEN)
        .json(&serde_json::json!({
            "sender": AgentId::new(),
            "payload": big_payload,
            "ttl_seconds": 60,
        }))
        .send()
        .await
        .expect("request must reach the server");
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::PAYLOAD_TOO_LARGE,
        "oversize body must be rejected at the axum layer",
    );
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn messaging_clamps_enormous_ttl() {
    ensure_env();
    let td = tempdir().unwrap();
    let server = spawn_runtime_server(SpawnOptions::new(td)).await;

    let sender = AgentId::new();
    let recipient = AgentId::new();
    register(server.runtime.communication.as_ref(), sender).await;
    register(server.runtime.communication.as_ref(), recipient).await;

    let client = test_client();
    // Request a ridiculous TTL; server must clamp to
    // min(request, config.message_ttl, 24h). Default message_ttl is 1 hour,
    // so the clamp target is 3600.
    let send_resp = client
        .post(format!(
            "{}/api/v1/agents/{}/messages",
            server.base_url, recipient.0
        ))
        .bearer_auth(TEST_TOKEN)
        .json(&serde_json::json!({
            "sender": sender,
            "payload": "hi",
            "ttl_seconds": 999_999,
        }))
        .send()
        .await
        .expect("request");
    assert_eq!(send_resp.status(), reqwest::StatusCode::OK);

    // Give the send event a moment to land in the recipient's queue.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let recv_resp = client
        .get(format!(
            "{}/api/v1/agents/{}/messages",
            server.base_url, recipient.0
        ))
        .bearer_auth(TEST_TOKEN)
        .send()
        .await
        .expect("request");
    assert_eq!(recv_resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = recv_resp.json().await.expect("json");
    let msgs = body["messages"].as_array().expect("messages array");
    assert_eq!(msgs.len(), 1);
    let ttl = msgs[0]["ttl_seconds"].as_u64().expect("ttl_seconds u64");
    assert!(ttl <= 3600, "TTL must be clamped to <=3600, got {}", ttl);
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn messaging_rejects_bad_uuid_on_status_endpoint() {
    ensure_env();
    let td = tempdir().unwrap();
    let server = spawn_runtime_server(SpawnOptions::new(td)).await;

    let client = test_client();
    let resp = client
        .get(format!(
            "{}/api/v1/messages/not-a-uuid/status",
            server.base_url
        ))
        .bearer_auth(TEST_TOKEN)
        .send()
        .await
        .expect("request");
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::BAD_REQUEST,
        "invalid UUID must be 400, not 404"
    );
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn messaging_401_without_auth() {
    ensure_env();
    let td = tempdir().unwrap();
    let server = spawn_runtime_server(SpawnOptions::new(td)).await;

    let client = test_client();
    let resp = client
        .post(format!(
            "{}/api/v1/agents/{}/messages",
            server.base_url,
            AgentId::new().0
        ))
        .json(&serde_json::json!({
            "sender": AgentId::new(),
            "payload": "hi",
        }))
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    server.shutdown().await;
}
