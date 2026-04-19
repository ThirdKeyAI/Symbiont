//! E2E-1: /api/v1 auth + scope matrix.
//!
//! Drives every category of route with three classes of caller:
//! 1. unauthenticated — must get 401
//! 2. admin key (unscoped) — must succeed on admin + per-agent routes
//! 3. scoped key — must succeed on its scoped agent, 403 on other agents,
//!    403 on admin-only routes.
//!
//! Uses a temp-dir-backed `api_keys_file` — the legacy
//! `SYMBIONT_API_TOKEN` path is explicitly disabled via
//! `SYMBIONT_REFUSE_LEGACY_API_TOKEN=1` so this test exercises the key
//! store and nothing else.

#![cfg(feature = "e2e")]

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;
use symbi_e2e::{spawn_runtime_server, test_client, SpawnOptions};
use symbi_runtime::api::api_keys::{ApiKeyRecord, ApiKeyStore};
use symbi_runtime::communication::CommunicationBus;
use symbi_runtime::AgentId;
use tempfile::tempdir;

#[derive(Serialize)]
struct ProvisionedKey {
    id_prefix: String,
    secret: String,
    /// The wire value: `{id}.{secret}`
    wire: String,
}

fn provision_key(id_prefix: &str) -> (ApiKeyRecord, ProvisionedKey) {
    let secret = format!("secret-{}", uuid::Uuid::new_v4());
    let hash = ApiKeyStore::hash_key(&secret).expect("hash");
    let record = ApiKeyRecord {
        key_id: id_prefix.to_string(),
        key_hash: hash,
        agent_scope: None,
        description: "e2e admin".to_string(),
        created_at: "2026-04-16T00:00:00Z".to_string(),
        revoked: false,
    };
    let wire = format!("{}.{}", id_prefix, secret);
    (
        record,
        ProvisionedKey {
            id_prefix: id_prefix.to_string(),
            secret,
            wire,
        },
    )
}

fn write_keys_file(dir: &tempfile::TempDir, records: &[ApiKeyRecord]) -> PathBuf {
    let path = dir.path().join("api_keys.json");
    let json = serde_json::to_string_pretty(records).unwrap();
    std::fs::write(&path, json).unwrap();
    path
}

fn ensure_key_store_only() {
    // Disable the legacy env-token path so we know every authenticated
    // request is going through the file-backed key store.
    std::env::remove_var("SYMBIONT_API_TOKEN");
    std::env::set_var("SYMBIONT_REFUSE_LEGACY_API_TOKEN", "1");
}

async fn register(bus: &dyn CommunicationBus, id: AgentId) {
    bus.register_agent(id).await.expect("register");
    tokio::time::sleep(Duration::from_millis(10)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unauthenticated_requests_are_rejected() {
    ensure_key_store_only();
    let td = tempdir().unwrap();
    let (admin_rec, _admin) = provision_key("admin");
    let path = write_keys_file(&td, &[admin_rec]);
    let mut opts = SpawnOptions::new(td);
    opts.api_keys_file = Some(path);
    let server = spawn_runtime_server(opts).await;

    let client = test_client();
    for route in [
        "/api/v1/agents",
        "/api/v1/schedules",
        "/api/v1/channels",
        "/api/v1/metrics",
    ] {
        let r = client
            .get(format!("{}{}", server.base_url, route))
            .send()
            .await
            .expect("req");
        assert_eq!(
            r.status(),
            reqwest::StatusCode::UNAUTHORIZED,
            "unauth GET {} must be 401",
            route
        );
    }
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admin_key_reaches_admin_and_agent_routes() {
    ensure_key_store_only();
    let td = tempdir().unwrap();
    let (admin_rec, admin) = provision_key("admin");
    let path = write_keys_file(&td, &[admin_rec]);
    let mut opts = SpawnOptions::new(td);
    opts.api_keys_file = Some(path);
    let server = spawn_runtime_server(opts).await;

    let client = test_client();
    // List endpoints — admin-only via require_admin.
    for route in [
        "/api/v1/agents",
        "/api/v1/schedules",
        "/api/v1/channels",
        "/api/v1/metrics",
    ] {
        let r = client
            .get(format!("{}{}", server.base_url, route))
            .bearer_auth(&admin.wire)
            .send()
            .await
            .expect("req");
        assert!(
            r.status().is_success(),
            "admin GET {} expected 2xx, got {}",
            route,
            r.status()
        );
    }
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoped_key_sees_only_its_own_agent_on_messages() {
    ensure_key_store_only();
    let td = tempdir().unwrap();

    // Scoped key: restricted to agent_a.
    let agent_a = AgentId::new();
    let agent_b = AgentId::new();

    let secret_a = format!("secret-{}", uuid::Uuid::new_v4());
    let rec_a = ApiKeyRecord {
        key_id: "scoped-a".to_string(),
        key_hash: ApiKeyStore::hash_key(&secret_a).unwrap(),
        agent_scope: Some(vec![agent_a.0.to_string()]),
        description: "scoped to agent_a".to_string(),
        created_at: "2026-04-16T00:00:00Z".to_string(),
        revoked: false,
    };
    let wire_a = format!("scoped-a.{}", secret_a);

    let path = write_keys_file(&td, &[rec_a]);
    let mut opts = SpawnOptions::new(td);
    opts.api_keys_file = Some(path);
    let server = spawn_runtime_server(opts).await;

    register(server.runtime.communication.as_ref(), agent_a).await;
    register(server.runtime.communication.as_ref(), agent_b).await;

    let client = test_client();

    // Scoped key sending as its own agent → 2xx.
    let ok = client
        .post(format!(
            "{}/api/v1/agents/{}/messages",
            server.base_url, agent_b.0
        ))
        .bearer_auth(&wire_a)
        .json(&serde_json::json!({
            "sender": agent_a,
            "payload": "hi",
            "ttl_seconds": 60,
        }))
        .send()
        .await
        .expect("req");
    assert_eq!(ok.status(), reqwest::StatusCode::OK);

    // Scoped key spoofing another sender → 403.
    let forbid = client
        .post(format!(
            "{}/api/v1/agents/{}/messages",
            server.base_url, agent_a.0
        ))
        .bearer_auth(&wire_a)
        .json(&serde_json::json!({
            "sender": agent_b,
            "payload": "forged",
            "ttl_seconds": 60,
        }))
        .send()
        .await
        .expect("req");
    assert_eq!(forbid.status(), reqwest::StatusCode::FORBIDDEN);

    // Scoped key hitting admin-only route → 403.
    let admin_forbid = client
        .get(format!("{}/api/v1/schedules", server.base_url))
        .bearer_auth(&wire_a)
        .send()
        .await
        .expect("req");
    assert_eq!(admin_forbid.status(), reqwest::StatusCode::FORBIDDEN);

    // Scoped key reading its own agent's messages → 200.
    let own = client
        .get(format!(
            "{}/api/v1/agents/{}/messages",
            server.base_url, agent_a.0
        ))
        .bearer_auth(&wire_a)
        .send()
        .await
        .expect("req");
    assert_eq!(own.status(), reqwest::StatusCode::OK);

    // Scoped key reading another agent's messages → 403.
    let other = client
        .get(format!(
            "{}/api/v1/agents/{}/messages",
            server.base_url, agent_b.0
        ))
        .bearer_auth(&wire_a)
        .send()
        .await
        .expect("req");
    assert_eq!(other.status(), reqwest::StatusCode::FORBIDDEN);

    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoked_key_is_rejected() {
    ensure_key_store_only();
    let td = tempdir().unwrap();
    let secret = format!("secret-{}", uuid::Uuid::new_v4());
    let rec = ApiKeyRecord {
        key_id: "rev".to_string(),
        key_hash: ApiKeyStore::hash_key(&secret).unwrap(),
        agent_scope: None,
        description: "revoked".to_string(),
        created_at: "2026-04-16T00:00:00Z".to_string(),
        revoked: true,
    };
    let wire = format!("rev.{}", secret);
    let path = write_keys_file(&td, &[rec]);
    let mut opts = SpawnOptions::new(td);
    opts.api_keys_file = Some(path);
    let server = spawn_runtime_server(opts).await;

    let client = test_client();
    let r = client
        .get(format!("{}/api/v1/metrics", server.base_url))
        .bearer_auth(&wire)
        .send()
        .await
        .expect("req");
    assert_eq!(r.status(), reqwest::StatusCode::UNAUTHORIZED);
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_agents_filters_by_scope() {
    ensure_key_store_only();
    let td = tempdir().unwrap();

    let agent_a = AgentId::new();
    let agent_b = AgentId::new();

    let secret = format!("secret-{}", uuid::Uuid::new_v4());
    let rec = ApiKeyRecord {
        key_id: "lister".to_string(),
        key_hash: ApiKeyStore::hash_key(&secret).unwrap(),
        agent_scope: Some(vec![agent_a.0.to_string()]),
        description: "scoped lister".to_string(),
        created_at: "2026-04-16T00:00:00Z".to_string(),
        revoked: false,
    };
    let wire = format!("lister.{}", secret);
    let path = write_keys_file(&td, &[rec]);
    let mut opts = SpawnOptions::new(td);
    opts.api_keys_file = Some(path);
    let server = spawn_runtime_server(opts).await;

    register(server.runtime.communication.as_ref(), agent_a).await;
    register(server.runtime.communication.as_ref(), agent_b).await;

    let client = test_client();
    let r = client
        .get(format!("{}/api/v1/agents", server.base_url))
        .bearer_auth(&wire)
        .send()
        .await
        .expect("req");
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let list: serde_json::Value = r.json().await.unwrap();
    let arr = list.as_array().expect("array");
    // The runtime may or may not emit agent summaries for bare
    // register_agent calls; what we can assert is that b never appears
    // in the scoped view.
    for a in arr {
        let id = a["id"]
            .as_str()
            .expect("id field present")
            .to_string();
        assert_ne!(
            id,
            agent_b.0.to_string(),
            "scoped key must never see agent_b"
        );
    }
    server.shutdown().await;
}
