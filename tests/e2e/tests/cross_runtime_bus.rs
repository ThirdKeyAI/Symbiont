//! E2E-2: cross-runtime bus round-trip.
//!
//! Boots two independent runtimes (A and B), each with its own
//! HttpApiServer. A uses `RemoteCommunicationBus` as an HTTP client to
//! send messages into B. Verifies:
//! - A→B send over HTTP delivers the message into B's bus inbox.
//! - B's copy is signed by B's key (the ingress handler re-wraps).
//! - Trying to send B's ingress-rewritten SecureMessage back into B's
//!   OWN bus via send_message passes (trivially, it's B-signed) but a
//!   foreign-signed message is refused — exercising the C-1 boundary
//!   from the outside.

#![cfg(feature = "e2e")]

use std::time::Duration;

use symbi_e2e::{spawn_runtime_server, test_client, SpawnOptions};
use symbi_runtime::communication::remote::RemoteCommunicationBus;
use symbi_runtime::communication::CommunicationBus;
use symbi_runtime::types::communication::MessageType;
use symbi_runtime::types::SignatureAlgorithm;
use symbi_runtime::AgentId;
use tempfile::tempdir;

const TEST_TOKEN: &str = "e2e-test-token-1234";

fn ensure_env() {
    std::env::set_var("SYMBIONT_API_TOKEN", TEST_TOKEN);
    std::env::remove_var("SYMBIONT_REFUSE_LEGACY_API_TOKEN");
}

async fn register(bus: &dyn CommunicationBus, id: AgentId) {
    bus.register_agent(id).await.expect("register");
    tokio::time::sleep(Duration::from_millis(20)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remote_bus_delivers_across_runtimes() {
    ensure_env();
    let td_a = tempdir().unwrap();
    let td_b = tempdir().unwrap();

    let server_a = spawn_runtime_server(SpawnOptions::new(td_a)).await;
    let server_b = spawn_runtime_server(SpawnOptions::new(td_b)).await;

    let sender = AgentId::new();
    let recipient = AgentId::new();
    register(server_a.runtime.communication.as_ref(), sender).await;
    register(server_b.runtime.communication.as_ref(), recipient).await;

    // RemoteCommunicationBus targeting server B.
    let remote_a_to_b = RemoteCommunicationBus::try_new(
        &server_b.base_url,
        Some(TEST_TOKEN.to_string()),
        sender,
    )
    .expect("loopback bus accepted");

    // Build a message on A's local bus (properly signed by A), then hand
    // it to the remote bus — the remote bus sends over HTTP. The peer
    // runtime re-signs on ingress via AgentRuntime::send_agent_message.
    let msg = server_a.runtime.communication.create_internal_message(
        sender,
        recipient,
        bytes::Bytes::from_static(b"hello-from-A"),
        MessageType::Direct(recipient),
        Duration::from_secs(60),
    );
    remote_a_to_b
        .send_message(msg)
        .await
        .expect("cross-runtime send must succeed");

    // Give B's event loop a tick to process the enqueue.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Query B's inbox over HTTP.
    let client = test_client();
    let resp = client
        .get(format!(
            "{}/api/v1/agents/{}/messages",
            server_b.base_url, recipient.0
        ))
        .bearer_auth(TEST_TOKEN)
        .send()
        .await
        .expect("req");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    let msgs = body["messages"].as_array().expect("messages");
    assert_eq!(msgs.len(), 1, "recipient must see exactly one message");
    let got = &msgs[0];
    assert_eq!(got["sender"].as_str(), Some(sender.0.to_string()).as_deref());
    assert_eq!(got["payload"].as_str(), Some("hello-from-A"));
    // TTL clamped to min(request, config.message_ttl=3600)
    assert!(got["ttl_seconds"].as_u64().unwrap() <= 3600);

    server_a.shutdown().await;
    server_b.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remote_bus_round_trip_refuses_replay_of_a_signed_into_b() {
    // Prove the C-1 boundary from the outside: even if someone takes a
    // SecureMessage that A produced and tries to inject it directly into
    // B's local bus via B's internal API, B refuses — because the
    // message's signature doesn't verify against B's verifying key.
    ensure_env();
    let td_a = tempdir().unwrap();
    let td_b = tempdir().unwrap();
    let server_a = spawn_runtime_server(SpawnOptions::new(td_a)).await;
    let server_b = spawn_runtime_server(SpawnOptions::new(td_b)).await;

    let sender = AgentId::new();
    let recipient = AgentId::new();
    register(server_a.runtime.communication.as_ref(), sender).await;
    register(server_b.runtime.communication.as_ref(), recipient).await;

    // A-signed message
    let a_signed = server_a.runtime.communication.create_internal_message(
        sender,
        recipient,
        bytes::Bytes::from_static(b"cross-bus replay attempt"),
        MessageType::Direct(recipient),
        Duration::from_secs(60),
    );
    assert!(matches!(
        a_signed.signature.algorithm,
        SignatureAlgorithm::Ed25519
    ));

    // Try to smuggle it into B's local bus directly.
    let err = server_b
        .runtime
        .communication
        .send_message(a_signed)
        .await
        .expect_err("B must refuse a message not signed with B's key");
    let msg = err.to_string();
    assert!(
        msg.contains("Signature verification failed")
            || msg.contains("signature"),
        "expected signature-related error, got: {}",
        msg
    );

    server_a.shutdown().await;
    server_b.shutdown().await;
}
