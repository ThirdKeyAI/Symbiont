use super::*;
use crate::secrets::{new_secret_store, SecretsBackend, SecretsConfig};
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

fn config(url: &str, auth: serde_json::Value) -> VaultConfig {
    serde_json::from_value(json!({"url":url,"auth":auth})).unwrap()
}

#[tokio::test]
#[serial_test::serial(vault_environment)]
async fn vault_settings_use_only_explicit_authentication_and_tls() {
    struct Restore(Vec<(&'static str, Option<std::ffi::OsString>)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            for (key, value) in &self.0 {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
    let settings = [
        ("VAULT_TOKEN", "synthetic-ambient"),
        ("VAULT_SKIP_VERIFY", "false"),
        ("VAULT_CACERT", "/nonexistent/ambient-ca.pem"),
    ];
    let _restore = Restore(
        settings
            .iter()
            .map(|(key, _)| (*key, std::env::var_os(key)))
            .collect(),
    );
    for (key, value) in settings {
        std::env::set_var(key, value);
    }
    let configured = config(
        "http://127.0.0.1:1",
        json!({"method":"token","token":"synthetic-explicit"}),
    );
    let client = VaultSecretStore::create_vault_client(&configured)
        .await
        .unwrap();
    assert_eq!(client.settings.token, "synthetic-explicit");
    assert!(client.settings.verify);
    assert!(client.settings.ca_certs.is_empty());
    assert!(client.settings.identity.is_none());
}

#[tokio::test]
#[serial_test::serial(vault_environment)]
async fn vault_factory_authenticates_before_fetching_configured_secret() {
    for approle in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for step in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut buf = [0; 2048];
                    let n = stream.read(&mut buf).await.unwrap();
                    assert!(n > 0 && request.len() < 16384);
                    request.extend_from_slice(&buf[..n]);
                    if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                        let length: usize = headers
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length: "))
                            .unwrap_or("0")
                            .parse()
                            .unwrap();
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let request = String::from_utf8(request).unwrap();
                let route = request
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap()
                    .trim_end_matches('?');
                let mut reply = json!({"request_id":"fixture","lease_id":"","lease_duration":0,"renewable":false});
                if step == 0 && approle {
                    assert_eq!(route, "/v1/auth/approle/login");
                    assert!(request.starts_with("POST "));
                    assert!(
                        request.contains("synthetic-secret-id") && request.contains("fixture-role")
                    );
                    assert!(!request.contains("synthetic-ambient"));
                    reply["auth"] = json!({"client_token":"synthetic-explicit","accessor":"fixture","policies":[],"token_policies":[],"metadata":null,"lease_duration":60,"renewable":false,"entity_id":"fixture","token_type":"service","orphan":true});
                } else if step == 0 {
                    assert_eq!(route, "/v1/auth/token/lookup-self");
                    assert!(request.contains("x-vault-token: synthetic-explicit"));
                    reply["data"] = json!({"accessor":"fixture","creation_time":0,"creation_ttl":60,"display_name":"fixture","entity_id":"fixture","explicit_max_ttl":60,"id":"synthetic-explicit","issue_time":"","num_uses":0,"orphan":true,"path":"auth/token","policies":[],"renewable":false,"ttl":60});
                } else {
                    assert_eq!(route, "/v1/secret/data/agents/fixture/secrets/cases");
                    assert!(request.contains("x-vault-token: synthetic-explicit"));
                    reply["data"] = json!({"data":{"value":"synthetic-service-value"},"metadata":{"created_time":"2026-10-27T00:00:00Z","deletion_time":"","destroyed":false,"version":3}});
                }
                let body = serde_json::to_vec(&reply).unwrap();
                let head = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                stream.write_all(head.as_bytes()).await.unwrap();
                stream.write_all(&body).await.unwrap();
            }
        });
        let auth = if approle {
            json!({"method":"approle","role_id":"fixture-role","secret_id":"synthetic-secret-id"})
        } else {
            json!({"method":"token","token":"synthetic-explicit"})
        };
        let settings = SecretsConfig {
            backend: SecretsBackend::Vault(config(&url, auth)),
            common: Default::default(),
        };
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            let store = new_secret_store(&settings, "fixture").await.unwrap();
            let secret = store.get_secret("cases").await.unwrap();
            assert_eq!(secret.value(), "synthetic-service-value");
            assert_eq!(secret.version.as_deref(), Some("3"));
        })
        .await;
        if result.is_err() {
            server.abort();
        }
        result.unwrap();
        server.await.unwrap();
    }
}

#[tokio::test]
async fn vault_unsupported_authentication_fails_before_reading_secrets() {
    let settings = SecretsConfig {
        backend: SecretsBackend::Vault(config(
            "http://127.0.0.1:1",
            json!({"method":"aws","region":"fixture","role":"fixture"}),
        )),
        common: Default::default(),
    };
    assert!(new_secret_store(&settings, "fixture").await.is_err());
}
