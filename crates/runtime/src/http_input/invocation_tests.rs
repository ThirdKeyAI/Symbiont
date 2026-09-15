use super::*;
use crate::reasoning::{executor::UnavailableToolExecutor, inference::*};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Provider {
    calls: AtomicUsize,
    wait: bool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

impl Provider {
    fn new(wait: bool) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            wait,
            entered: Default::default(),
            release: Default::default(),
        })
    }
}

#[async_trait::async_trait]
impl InferenceProvider for Provider {
    async fn complete(
        &self,
        _: &Conversation,
        _: &InferenceOptions,
    ) -> Result<InferenceResponse, InferenceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        if self.wait {
            self.release.notified().await;
        }
        Ok(InferenceResponse {
            content: "private fixture result".into(),
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
            usage: Usage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: 2,
            },
            model: "fixture".into(),
        })
    }
    fn provider_name(&self) -> &str {
        "fixture"
    }
    fn default_model(&self) -> &str {
        "fixture"
    }
    fn supports_native_tools(&self) -> bool {
        true
    }
    fn supports_structured_output(&self) -> bool {
        false
    }
}

async fn serve(
    root: &Path,
    agent: AgentId,
    credential: Option<&str>,
    key: Option<&Path>,
    provider: Arc<Provider>,
) -> (u16, tokio::task::JoinHandle<Result<(), RuntimeError>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let server = HttpInputServer::new(HttpInputConfig {
        port,
        agent,
        auth_header: credential.map(str::to_owned),
        jwt_public_key_path: key.map(|p| p.display().to_string()),
        ..HttpInputConfig::default()
    })
    .with_project_root(root.to_owned())
    .with_inference_provider(provider)
    .with_executor(Arc::new(UnavailableToolExecutor));
    let handle = tokio::spawn(async move { server.start().await });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            assert!(!handle.is_finished(), "server stopped before readiness");
            if tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_ok()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    (port, handle)
}

fn request(port: u16, credential: &str, id: Uuid, payload: Value) -> reqwest::RequestBuilder {
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("http://127.0.0.1:{port}/webhook"))
        .header("Authorization", credential)
        .header("Idempotency-Key", id.to_string())
        .json(&payload)
}

#[tokio::test]
async fn http_claim_survives_restart_and_refuses_changed_callers_payloads_and_missing_ids() {
    let root = tempfile::tempdir().unwrap();
    let agent = AgentId::new();
    let provider = Provider::new(false);
    let (port, handle) = serve(
        root.path(),
        agent,
        Some("Bearer first"),
        None,
        provider.clone(),
    )
    .await;
    let payload = json!({"prompt":"do work"});
    let id = Uuid::new_v4();
    let missing = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("http://127.0.0.1:{port}/webhook"))
        .header("Authorization", "Bearer first")
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::BAD_REQUEST);
    let ambiguous = request(port, "Bearer first", id, payload.clone())
        .header("Idempotency-Key", id.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(ambiguous.status(), StatusCode::BAD_REQUEST);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    assert!(!root.path().join(".symbiont/invocations").exists());

    let first = request(port, "Bearer first", id, payload.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(first.headers()["Idempotency-Replayed"], "false");
    let first: Value = first.json().await.unwrap();
    assert_eq!(first["status"], "completed");
    assert_eq!(first["total_usage"]["total_tokens"], 2);
    assert!(first["budget"].is_object());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    handle.abort();
    let _ = handle.await;

    // Rebuild the actual HTTP server; persisted results must survive owner loss.
    let (port, handle) = serve(
        root.path(),
        agent,
        Some("Bearer first"),
        None,
        provider.clone(),
    )
    .await;
    let repeated = request(port, "Bearer first", id, payload.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(repeated.status(), StatusCode::OK);
    assert_eq!(repeated.headers()["Idempotency-Replayed"], "true");
    let repeated: Value = repeated.json().await.unwrap();
    assert_eq!(repeated["response"], first["response"]);
    assert_eq!(repeated["audit"], first["audit"]);
    let changed = request(port, "Bearer first", id, json!({"prompt":"different"}))
        .send()
        .await
        .unwrap();
    assert_eq!(changed.status(), StatusCode::CONFLICT);
    assert_eq!(changed.json::<Value>().await.unwrap()["status"], "conflict");
    let unauthorized = request(port, "Bearer second", id, payload.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    handle.abort();
    let _ = handle.await;

    let (port, handle) = serve(
        root.path(),
        agent,
        Some("Bearer second"),
        None,
        provider.clone(),
    )
    .await;
    let other = request(port, "Bearer second", id, payload)
        .send()
        .await
        .unwrap();
    assert_eq!(other.status(), StatusCode::CONFLICT);
    let other: Value = other.json().await.unwrap();
    assert_eq!(other["status"], "conflict");
    assert!(other.get("response").is_none() && other.get("audit").is_none());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    handle.abort();
    let _ = handle.await;
}

#[tokio::test]
async fn concurrent_http_retry_cannot_acquire_another_execution() {
    let root = tempfile::tempdir().unwrap();
    let provider = Provider::new(true);
    let (port, handle) = serve(
        root.path(),
        AgentId::new(),
        Some("Bearer fixture"),
        None,
        provider.clone(),
    )
    .await;
    let id = Uuid::new_v4();
    let call = tokio::spawn(async move {
        request(port, "Bearer fixture", id, json!({"prompt":"work"}))
            .send()
            .await
            .unwrap()
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        provider.entered.notified(),
    )
    .await
    .unwrap();
    let duplicate = request(port, "Bearer fixture", id, json!({"prompt":"work"}))
        .send()
        .await
        .unwrap();
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);
    let duplicate: Value = duplicate.json().await.unwrap();
    assert_eq!(duplicate["status"], "in_progress");
    assert!(duplicate.get("audit").is_none());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    provider.release.notify_one();
    assert_eq!(call.await.unwrap().status(), StatusCode::OK);
    handle.abort();
    let _ = handle.await;
}

fn jwt(key: &ed25519_dalek::SigningKey, claims: Value) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use ed25519_dalek::Signer;
    let input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(br#"{"alg":"EdDSA","typ":"JWT"}"#),
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
    );
    format!(
        "Bearer {input}.{}",
        URL_SAFE_NO_PAD.encode(key.sign(input.as_bytes()).to_bytes())
    )
}

#[tokio::test]
async fn verified_jwt_subject_and_authority_bind_cached_results_across_token_renewal() {
    let root = tempfile::tempdir().unwrap();
    let key = ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let public = root.path().join("jwt-public.der");
    std::fs::write(&public, key.verifying_key().to_bytes()).unwrap();
    let provider = Provider::new(false);
    let agent = AgentId::new();
    let (port, handle) = serve(root.path(), agent, None, Some(&public), provider.clone()).await;
    let id = Uuid::new_v4();
    let exp = chrono::Utc::now().timestamp() + 600;
    for claims in [
        json!({"exp":exp}),
        json!({"exp":exp,"sub":""}),
        json!({"exp":1,"sub":"alice"}),
    ] {
        let response = request(port, &jwt(&key, claims), id, json!({"prompt":"work"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    let first = jwt(&key, json!({"exp":exp, "sub":"alice", "iss":"fixture"}));
    let renewed = jwt(&key, json!({"exp":exp+600, "sub":"alice", "iss":"fixture"}));
    assert_ne!(first, renewed);
    for (credential, replayed) in [(&first, false), (&renewed, true)] {
        let response = request(port, credential, id, json!({"prompt":"work"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.json::<Value>().await.unwrap()["replayed"],
            replayed
        );
    }
    for claims in [
        json!({"exp":exp,"sub":"bob","iss":"fixture"}),
        json!({"exp":exp,"sub":"alice","iss":"other"}),
    ] {
        let response = request(port, &jwt(&key, claims), id, json!({"prompt":"work"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["status"], "conflict");
        assert!(body.get("response").is_none() && body.get("audit").is_none());
    }
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    let other_key = ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let other_token = jwt(
        &other_key,
        json!({"exp":exp, "sub":"alice", "iss":"fixture"}),
    );
    let forged = request(port, &other_token, id, json!({"prompt":"work"}))
        .send()
        .await
        .unwrap();
    assert_eq!(forged.status(), StatusCode::UNAUTHORIZED);
    handle.abort();
    let _ = handle.await;
    std::fs::write(&public, other_key.verifying_key().to_bytes()).unwrap();
    let (port, handle) = serve(root.path(), agent, None, Some(&public), provider.clone()).await;
    let rotated = request(port, &other_token, id, json!({"prompt":"work"}))
        .send()
        .await
        .unwrap();
    assert_eq!(rotated.status(), StatusCode::CONFLICT);
    assert_eq!(rotated.json::<Value>().await.unwrap()["status"], "conflict");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    handle.abort();
    let _ = handle.await;
}

#[tokio::test]
async fn setup_failure_retains_the_claim_when_a_provider_becomes_available() {
    let root = tempfile::tempdir().unwrap();
    let principal = AgentId::new();
    let id = Uuid::new_v4();
    let provider = Provider::new(false);
    let mut audit = Value::Null;
    for available in [false, true] {
        let result = invoke_agent(
            None,
            principal,
            json!({"prompt":"work"}),
            available.then(|| provider.clone() as Arc<dyn InferenceProvider>),
            root.path(),
            Arc::new(UnavailableToolExecutor),
            true,
            Arc::new(DefaultPolicyGate::new()),
            Arc::new(CircuitBreakerRegistry::default()),
            HttpInvocation {
                id,
                caller: AuthenticatedCaller::static_token("fixture"),
                route: "/webhook".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(result.status, StatusCode::CONFLICT);
        assert_eq!(result.body["status"], "unresolved");
        assert!(result.body["audit"].is_object());
        if available {
            assert_eq!(result.body["audit"], audit);
        }
        audit = result.body["audit"].clone();
    }
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    let path = Path::new(audit["path"].as_str().unwrap());
    assert_eq!(std::fs::metadata(path).unwrap().len(), 0);
}
