# Security Hardening Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix 8 security and correctness issues identified in the codebase review — no stubs, placeholders, or TODOs.

**Architecture:** Targeted fixes in the runtime crate's communication, MCP, HTTP API, and HTTP input modules. Each task is independent and addresses one review finding.

**Tech Stack:** Rust (edition 2021), Axum, tower-http, reqwest, ed25519-dalek, aes-gcm, sha2

---

### Task 1: Replace placeholder crypto in execute_agent (Issue 1)

The `execute_agent` method in `lib.rs` creates messages with `EncryptionAlgorithm::None`, empty nonces, empty signatures, and empty public keys. The `DefaultCommunicationBus` already has `generate_nonce()` and `sign_message_data()` methods — the execute path just doesn't use them.

**Files:**
- Modify: `crates/runtime/src/lib.rs:608-652`

**Step 1: Read the existing code**

Read `crates/runtime/src/lib.rs` lines 608-652 to see the `execute_agent` function, and lines 480-535 of `communication/mod.rs` to see `generate_nonce()` and `sign_message_data()`.

**Step 2: Add a `create_message` method to the CommunicationBus trait or use a helper**

The `CommunicationBus` trait (in `communication/mod.rs`) already exposes `send_message`. The `DefaultCommunicationBus` has private methods `generate_nonce()` and `sign_message_data()`. Since `AgentRuntime` only holds a `dyn CommunicationBus`, we can't call those private methods directly.

Instead, add a `create_signed_message` method to the `CommunicationBus` trait:

In `crates/runtime/src/communication/mod.rs`, add to the `CommunicationBus` trait:

```rust
/// Create a properly signed message for internal communication
fn create_internal_message(
    &self,
    sender: AgentId,
    recipient: AgentId,
    payload_data: bytes::Bytes,
    ttl: Duration,
    message_type: MessageType,
) -> Result<SecureMessage, CommunicationError>;
```

Then implement it on `DefaultCommunicationBus`:

```rust
fn create_internal_message(
    &self,
    sender: AgentId,
    recipient: AgentId,
    payload_data: bytes::Bytes,
    ttl: Duration,
    message_type: MessageType,
) -> Result<SecureMessage, CommunicationError> {
    let nonce = Self::generate_nonce();
    let payload = EncryptedPayload {
        data: payload_data,
        nonce,
        encryption_algorithm: EncryptionAlgorithm::Aes256Gcm,
    };
    let signature = self.sign_message_data(&payload.data);
    Ok(SecureMessage {
        id: MessageId::new(),
        sender,
        recipient: Some(recipient),
        topic: None,
        payload,
        signature,
        timestamp: SystemTime::now(),
        ttl,
        message_type,
    })
}
```

**Step 3: Update execute_agent to use create_internal_message**

In `crates/runtime/src/lib.rs`, replace lines 620-643 with:

```rust
let execution_id = uuid::Uuid::new_v4().to_string();
let payload_data: bytes::Bytes = serde_json::to_vec(&request)
    .map_err(|e| RuntimeError::Internal(e.to_string()))?
    .into();
let message = self
    .communication
    .create_internal_message(
        AgentId::new(), // System sender
        agent_id,
        payload_data,
        std::time::Duration::from_secs(300),
        types::MessageType::Direct(agent_id),
    )
    .map_err(RuntimeError::Communication)?;
```

**Step 4: Fix test helpers to use proper crypto too**

In `crates/runtime/src/communication/mod.rs`, update `create_test_message` (line 893) and the inline test message construction (line 1113-1131) to use random nonces and valid signatures. Create a test-only signing key:

```rust
fn create_test_message(sender: AgentId, recipient: AgentId) -> SecureMessage {
    use crate::types::RequestId;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();
    let data = b"test message";
    let sig = signing_key.sign(data);

    let nonce = {
        use aes_gcm::{aead::AeadCore, Aes256Gcm};
        Aes256Gcm::generate_nonce(&mut OsRng).to_vec()
    };

    SecureMessage {
        id: MessageId::new(),
        sender,
        recipient: Some(recipient),
        message_type: MessageType::Request(RequestId::new()),
        topic: Some("test".to_string()),
        payload: EncryptedPayload {
            data: data.to_vec().into(),
            nonce,
            encryption_algorithm: EncryptionAlgorithm::Aes256Gcm,
        },
        signature: MessageSignature {
            signature: sig.to_bytes().to_vec(),
            algorithm: SignatureAlgorithm::Ed25519,
            public_key: verifying_key.to_bytes().to_vec(),
        },
        ttl: Duration::from_secs(3600),
        timestamp: SystemTime::now(),
    }
}
```

Apply the same pattern to the inline test at line 1113.

**Step 5: Run tests**

```bash
cd /home/jascha/Documents/ThirdKey/repos/symbiont
cargo test -p symbi-runtime --lib communication -- --nocapture
```

Expected: All communication tests pass.

**Step 6: Run clippy**

```bash
cargo clippy --workspace -- -D warnings
```

Expected: Zero warnings.

**Step 7: Commit**

```bash
git add crates/runtime/src/lib.rs crates/runtime/src/communication/mod.rs
git commit -m "Replace placeholder crypto with real nonces and signatures

Internal messages now use generate_nonce() for AES-256-GCM nonces and
sign_message_data() for Ed25519 signatures instead of zero-filled bytes.
Added create_internal_message() to CommunicationBus trait. Test helpers
also use real crypto to avoid normalizing insecure defaults."
```

---

### Task 2: Implement real provider key fetching for MCP TOFU (Issue 2)

The `fetch_and_pin_key` method creates mock keys instead of fetching real public keys from the provider's `public_key_url`.

**Files:**
- Modify: `crates/runtime/src/integrations/mcp/client.rs:162-184`

**Step 1: Read the existing code**

Read `crates/runtime/src/integrations/mcp/client.rs` (full file), `types.rs` for `ToolProvider` and `McpClientError`, and `crates/runtime/src/integrations/schemapin/key_store.rs` for `PinnedKey::new`.

**Step 2: Add reqwest client to SecureMcpClient**

Add an `http_client: reqwest::Client` field to `SecureMcpClient`:

```rust
pub struct SecureMcpClient {
    config: McpClientConfig,
    schema_pin: Arc<dyn SchemaPinClient>,
    key_store: Arc<LocalKeyStore>,
    tools: Arc<RwLock<HashMap<String, McpTool>>>,
    enforcer: Arc<dyn ToolInvocationEnforcer>,
    http_client: reqwest::Client,
}
```

Initialize it in both `new()` and `with_enforcer()`:

```rust
http_client: reqwest::Client::builder()
    .timeout(Duration::from_secs(10))
    .https_only(true)
    .build()
    .expect("Failed to create HTTP client"),
```

**Step 3: Implement real key fetching**

Replace the `fetch_and_pin_key` method body (lines 162-184):

```rust
async fn fetch_and_pin_key(&self, provider: &ToolProvider) -> Result<(), McpClientError> {
    if self.key_store.has_key(&provider.identifier)? {
        return Ok(());
    }

    // Fetch the public key from the provider's published URL
    let response = self
        .http_client
        .get(&provider.public_key_url)
        .send()
        .await
        .map_err(|e| McpClientError::KeyFetchFailed {
            provider: provider.identifier.clone(),
            reason: format!("HTTP request failed: {}", e),
        })?;

    if !response.status().is_success() {
        return Err(McpClientError::KeyFetchFailed {
            provider: provider.identifier.clone(),
            reason: format!("HTTP {}", response.status()),
        });
    }

    let key_bytes = response.bytes().await.map_err(|e| {
        McpClientError::KeyFetchFailed {
            provider: provider.identifier.clone(),
            reason: format!("Failed to read response body: {}", e),
        }
    })?;

    // Compute fingerprint (SHA-256 of raw key material)
    use sha2::{Digest, Sha256};
    let fingerprint = hex::encode(Sha256::digest(&key_bytes));

    let pinned_key = PinnedKey::new(
        provider.identifier.clone(),
        String::from_utf8_lossy(&key_bytes).to_string(),
        "Ed25519".to_string(),
        fingerprint,
    );

    tracing::info!(
        provider = %provider.identifier,
        url = %provider.public_key_url,
        fingerprint = %pinned_key.fingerprint(),
        "Pinned provider public key via TOFU"
    );

    self.key_store.pin_key(pinned_key)?;
    Ok(())
}
```

**Step 4: Add the KeyFetchFailed variant to McpClientError**

In `crates/runtime/src/integrations/mcp/types.rs`, add:

```rust
/// Failed to fetch provider public key
KeyFetchFailed {
    provider: String,
    reason: String,
},
```

And add the Display match arm if McpClientError implements Display manually (check the existing error variants pattern).

**Step 5: Run tests**

```bash
cargo test -p symbi-runtime --lib integrations::mcp -- --nocapture
```

**Step 6: Run clippy**

```bash
cargo clippy --workspace -- -D warnings
```

**Step 7: Commit**

```bash
git add crates/runtime/src/integrations/mcp/client.rs crates/runtime/src/integrations/mcp/types.rs
git commit -m "Implement real HTTPS key fetching for MCP TOFU pinning

Replace mock key generation with actual HTTP fetch from provider's
public_key_url. Enforces HTTPS-only transport, computes SHA-256
fingerprint of fetched key material, and logs provenance on pin."
```

---

### Task 3: Separate health endpoint from auth-required routes (Issue 3)

The scheduler health endpoint is grouped with workflows/metrics behind auth. Health checks should be unauthenticated for load balancer probes. Additionally, add a startup warning when no auth is configured.

**Files:**
- Modify: `crates/runtime/src/api/server.rs:326-338`

**Step 1: Read the existing router construction**

Read `crates/runtime/src/api/server.rs` lines 254-382.

**Step 2: Split health endpoint out of the authed router**

Replace lines 326-338:

```rust
// Workflow and metrics routes — require authentication
let protected_router = Router::new()
    .route("/api/v1/workflows/execute", post(execute_workflow))
    .route("/api/v1/metrics", get(get_metrics))
    .layer(middleware::from_fn(auth_middleware))
    .with_state(provider.clone());

// Health endpoint — no auth (load balancer probes must work unauthenticated)
let health_router = Router::new()
    .route("/api/v1/health/scheduler", get(get_scheduler_health))
    .with_state(provider.clone());

router = router
    .merge(agent_router)
    .merge(schedule_router)
    .merge(channel_router)
    .merge(protected_router)
    .merge(health_router);
```

**Step 3: Add startup auth warning**

In the `start` or `create_router` method, before building the router, add a startup check:

```rust
if std::env::var("SYMBIONT_API_TOKEN").is_err() {
    tracing::warn!(
        "SYMBIONT_API_TOKEN not set — API routes are effectively unauthenticated. \
         Set this variable in production."
    );
}
```

**Step 4: Run tests**

```bash
cargo test -p symbi-runtime --lib api -- --nocapture
```

**Step 5: Run clippy**

```bash
cargo clippy --workspace -- -D warnings
```

**Step 6: Commit**

```bash
git add crates/runtime/src/api/server.rs
git commit -m "Separate health endpoint from auth-guarded routes

Move /api/v1/health/scheduler to its own unauthenticated router so
load balancer probes work without credentials. Workflows and metrics
remain behind auth. Add startup warning when SYMBIONT_API_TOKEN is unset."
```

---

### Task 4: Replace permissive CORS with explicit allow-lists (Issue 4)

The HTTP input server uses `CorsLayer::permissive()`. Replace with configurable origin allow-lists.

**Files:**
- Modify: `crates/runtime/src/http_input/config.rs:49` (field type change)
- Modify: `crates/runtime/src/http_input/server.rs:153-156` (CORS construction)

**Step 1: Read the existing code**

Read `crates/runtime/src/http_input/config.rs` and `server.rs` lines 140-170.

**Step 2: Replace cors_enabled bool with cors_origins list**

In `config.rs`, change:

```rust
// Before:
pub cors_enabled: bool,
// After:
/// Allowed CORS origins. Empty list = CORS disabled.
/// Use ["*"] only in development.
pub cors_origins: Vec<String>,
```

Update Default impl:

```rust
// Before:
cors_enabled: false,
// After:
cors_origins: vec![],
```

**Step 3: Replace CorsLayer::permissive() with explicit config**

In `server.rs`, replace lines 153-156:

```rust
// Add CORS if origins are configured
if !config.cors_origins.is_empty() {
    use axum::http::{header, HeaderValue, Method};

    let cors = if config.cors_origins.iter().any(|o| o == "*") {
        tracing::warn!("CORS configured with wildcard origin — not recommended for production");
        CorsLayer::permissive()
    } else {
        let origins: Vec<HeaderValue> = config
            .cors_origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        CorsLayer::new()
            .allow_origin(origins)
            .allow_methods([Method::POST])
            .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
    };
    app = app.layer(cors);
}
```

**Step 4: Fix any compilation errors from the field rename**

Search for `cors_enabled` in the codebase and update all references to `cors_origins`.

**Step 5: Run tests**

```bash
cargo test -p symbi-runtime -- --nocapture
```

**Step 6: Run clippy**

```bash
cargo clippy --workspace -- -D warnings
```

**Step 7: Commit**

```bash
git add crates/runtime/src/http_input/config.rs crates/runtime/src/http_input/server.rs
git commit -m "Replace permissive CORS with explicit origin allow-lists

CORS on HTTP input server now requires explicit origins instead of
CorsLayer::permissive(). Empty list disables CORS. Wildcard origin
logs a warning. Production callers must list specific origins."
```

---

### Task 5: Harden HTTP input defaults (Issue 5)

Default config binds to `0.0.0.0` with no auth. Change to loopback and log a warning when externally bound without auth.

**Files:**
- Modify: `crates/runtime/src/http_input/config.rs:59-78`
- Modify: `crates/runtime/src/http_input/server.rs` (add startup warning)

**Step 1: Change default bind address to loopback**

In `config.rs`, change the Default impl:

```rust
bind_address: "127.0.0.1".to_string(),
```

**Step 2: Add startup safety check in server.rs**

In the `start` method, after reading config (around line 72), add:

```rust
// Warn if binding externally without authentication
if config.bind_address != "127.0.0.1" && config.bind_address != "localhost" {
    if config.auth_header.is_none() && config.jwt_public_key_path.is_none() && config.webhook_verify.is_none() {
        tracing::warn!(
            bind = %config.bind_address,
            "HTTP input binding to non-loopback address with no authentication configured. \
             Set auth_header, jwt_public_key_path, or webhook_verify for production use."
        );
    }
}
```

**Step 3: Run tests**

```bash
cargo test -p symbi-runtime -- --nocapture
```

**Step 4: Run clippy**

```bash
cargo clippy --workspace -- -D warnings
```

**Step 5: Commit**

```bash
git add crates/runtime/src/http_input/config.rs crates/runtime/src/http_input/server.rs
git commit -m "Default HTTP input to loopback, warn on external bind without auth

Change default bind_address from 0.0.0.0 to 127.0.0.1. Emit a warning
at startup when binding to a non-loopback address without any auth
mechanism (auth_header, JWT, or webhook verification) configured."
```

---

### Task 6: Implement JWT validation in HTTP input auth (Issue 6)

`jwt_public_key_path` exists in config but is never used. Implement JWT Bearer token validation.

**Files:**
- Modify: `crates/runtime/src/http_input/server.rs:196-237` (ServerState + auth_middleware)

**Step 1: Read the existing auth middleware**

Read `server.rs` lines 193-237 and `config.rs` lines 28-31.

**Step 2: Add JWT public key to ServerState**

```rust
struct ServerState {
    config: Arc<RwLock<HttpInputConfig>>,
    runtime: Option<Arc<crate::AgentRuntime>>,
    concurrency_limiter: Arc<Semaphore>,
    resolved_auth_header: Arc<RwLock<Option<String>>>,
    jwt_verifying_key: Option<Arc<ed25519_dalek::VerifyingKey>>,
    llm_client: Option<Arc<LlmClient>>,
    agent_dsl_sources: Arc<Vec<(String, String)>>,
    webhook_verifier: Option<Arc<dyn super::webhook_verify::SignatureVerifier>>,
}
```

**Step 3: Load JWT key in server startup**

In the `start` method, after resolving the auth header, load the JWT key:

```rust
let jwt_verifying_key = if let Some(ref key_path) = config.jwt_public_key_path {
    let key_pem = std::fs::read_to_string(key_path).map_err(|e| {
        RuntimeError::Internal(format!("Failed to read JWT public key at {}: {}", key_path, e))
    })?;
    // Parse PEM-encoded Ed25519 public key
    let pem = pem::parse(&key_pem).map_err(|e| {
        RuntimeError::Internal(format!("Invalid PEM in JWT public key: {}", e))
    })?;
    let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(
        pem.contents().try_into().map_err(|_| {
            RuntimeError::Internal("JWT public key must be 32 bytes (Ed25519)".to_string())
        })?,
    )
    .map_err(|e| RuntimeError::Internal(format!("Invalid Ed25519 public key: {}", e)))?;
    tracing::info!(path = %key_path, "Loaded JWT Ed25519 public key");
    Some(Arc::new(verifying_key))
} else {
    None
};
```

**Step 4: Update auth_middleware to check JWT**

Extend the existing `auth_middleware` to try JWT verification when `jwt_verifying_key` is Some and the Authorization header contains a Bearer token that looks like a JWT (contains dots):

```rust
async fn auth_middleware(
    State(state): State<ServerState>,
    headers: HeaderMap,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<Response, StatusCode> {
    // Check static auth header first
    let resolved_auth = state.resolved_auth_header.read().await;
    if let Some(expected_auth) = resolved_auth.as_ref() {
        let auth_header = headers.get("Authorization").and_then(|h| h.to_str().ok());
        match auth_header {
            Some(provided_auth) if provided_auth == expected_auth => {
                return Ok(next.run(req).await);
            }
            _ => {}
        }
    }

    // Try JWT validation if configured
    if let Some(ref verifying_key) = state.jwt_verifying_key {
        let auth_header = headers.get("Authorization").and_then(|h| h.to_str().ok());
        if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
            // Validate JWT: header.payload.signature (Ed25519)
            let parts: Vec<&str> = token_str.split('.').collect();
            if parts.len() == 3 {
                use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
                use ed25519_dalek::Verifier;

                let signing_input = format!("{}.{}", parts[0], parts[1]);
                if let Ok(sig_bytes) = URL_SAFE_NO_PAD.decode(parts[2]) {
                    if let Ok(signature) = ed25519_dalek::Signature::from_slice(&sig_bytes) {
                        if verifying_key.verify(signing_input.as_bytes(), &signature).is_ok() {
                            // Check expiration from payload
                            if let Ok(payload_json) = URL_SAFE_NO_PAD.decode(parts[1]) {
                                if let Ok(claims) = serde_json::from_slice::<serde_json::Value>(&payload_json) {
                                    if let Some(exp) = claims.get("exp").and_then(|e| e.as_i64()) {
                                        let now = chrono::Utc::now().timestamp();
                                        if now > exp {
                                            tracing::warn!("JWT token expired");
                                            return Err(StatusCode::UNAUTHORIZED);
                                        }
                                    }
                                }
                            }
                            return Ok(next.run(req).await);
                        }
                    }
                }
                tracing::warn!("JWT signature verification failed");
                return Err(StatusCode::UNAUTHORIZED);
            }
        }
    }

    // No valid auth provided
    if state.resolved_auth_header.read().await.is_none() && state.jwt_verifying_key.is_none() {
        // No auth configured at all — allow through (but startup warning was emitted)
        return Ok(next.run(req).await);
    }

    tracing::warn!("Authentication failed: no valid credentials provided");
    Err(StatusCode::UNAUTHORIZED)
}
```

**Step 5: Add pem crate dependency if not already present**

Check `Cargo.toml` for `pem` crate. If missing, add `pem = "3"`.

**Step 6: Run tests**

```bash
cargo test -p symbi-runtime -- --nocapture
```

**Step 7: Run clippy**

```bash
cargo clippy --workspace -- -D warnings
```

**Step 8: Commit**

```bash
git add crates/runtime/src/http_input/server.rs crates/runtime/Cargo.toml
git commit -m "Implement JWT Ed25519 validation for HTTP input auth

Load Ed25519 public key from jwt_public_key_path at startup. Auth
middleware now validates JWT Bearer tokens (header.payload.signature)
with signature verification and expiration checking. Falls back to
static auth_header check. Logs warning when no auth is configured."
```

---

### Task 7: Implement PathPrefix route matching (Issue 7)

`RouteMatch::PathPrefix` unconditionally returns `false`. Implement actual path prefix matching.

**Files:**
- Modify: `crates/runtime/src/http_input/server.rs:329-352` (matches_route_condition + route_request)
- Modify: `crates/runtime/src/http_input/server.rs:241-244` (webhook_handler to pass path)

**Step 1: Read the existing routing code**

Read `server.rs` lines 300-352 and 241-310 (webhook_handler).

**Step 2: Add request path parameter to routing functions**

The `matches_route_condition` function needs the request path. Update its signature:

```rust
async fn matches_route_condition(
    condition: &RouteMatch,
    request_path: &str,
    payload: &Value,
    headers: &HeaderMap,
) -> bool {
    match condition {
        RouteMatch::PathPrefix(prefix) => request_path.starts_with(prefix),
        RouteMatch::HeaderEquals(header_name, expected_value) => headers
            .get(header_name)
            .and_then(|h| h.to_str().ok())
            .map(|value| value == expected_value)
            .unwrap_or(false),
        RouteMatch::JsonFieldEquals(field_name, expected_value) => payload
            .get(field_name)
            .and_then(|v| v.as_str())
            .map(|value| value == expected_value)
            .unwrap_or(false),
    }
}
```

**Step 3: Update route_request to pass path**

```rust
async fn route_request(
    config: &HttpInputConfig,
    request_path: &str,
    payload: &Value,
    headers: &HeaderMap,
) -> AgentId {
    if let Some(routing_rules) = &config.routing_rules {
        for rule in routing_rules {
            if matches_route_condition(&rule.condition, request_path, payload, headers).await {
                tracing::debug!("Request routed to agent {} via rule", rule.agent);
                return rule.agent;
            }
        }
    }
    tracing::debug!("Request routed to default agent {}", config.agent);
    config.agent
}
```

**Step 4: Update webhook_handler to extract and pass path**

In `webhook_handler`, extract the URI path from the request. Currently the handler receives `headers` and `body`. Add the URI extraction. This requires adding `uri: axum::http::Uri` to the handler signature:

```rust
async fn webhook_handler(
    State(state): State<ServerState>,
    uri: axum::http::Uri,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Response, StatusCode> {
```

Then pass `uri.path()` to `route_request`:

```rust
let agent_id = route_request(&config, uri.path(), &payload, &headers).await;
```

**Step 5: Update the route registration to use a wildcard**

To support path-prefix routing, change the route registration from a fixed path to a wildcard that captures subpaths:

```rust
// In start(), change:
app = app.route(&path, post(webhook_handler));
// To:
app = app
    .route(&path, post(webhook_handler.clone()))
    .route(&format!("{}/*rest", path), post(webhook_handler));
```

Actually, Axum's handler isn't Clone by default. A simpler approach: register the handler on a catch-all path if routing rules with PathPrefix exist:

```rust
app = app.route(&path, post(webhook_handler));
// If any routing rules use PathPrefix, add wildcard route
if config.routing_rules.as_ref().map_or(false, |rules| {
    rules.iter().any(|r| matches!(r.condition, RouteMatch::PathPrefix(_)))
}) {
    app = app.route(&format!("{}/{{*rest}}", path.trim_end_matches('/')), post(webhook_handler));
}
```

Note: This requires the handler to be reusable. If that's complex, just add the wildcard unconditionally — Axum will use the most specific match first.

**Step 6: Run tests**

```bash
cargo test -p symbi-runtime -- --nocapture
```

**Step 7: Run clippy**

```bash
cargo clippy --workspace -- -D warnings
```

**Step 8: Commit**

```bash
git add crates/runtime/src/http_input/server.rs
git commit -m "Implement PathPrefix route matching for HTTP input

RouteMatch::PathPrefix now performs actual prefix comparison against the
request URI path. Updated webhook_handler to extract URI and pass it
through the routing chain. Added wildcard route registration when
PathPrefix rules are configured."
```

---

### Task 8: Replace invoke_agent stub with runtime execution (Issue 8)

The `invoke_agent` function ignores the `_runtime` parameter and returns a stub when no LLM is configured. It should use the runtime for actual agent invocation, with LLM as an enhancement.

**Files:**
- Modify: `crates/runtime/src/http_input/server.rs:354-447`

**Step 1: Read the existing invoke_agent**

Read `server.rs` lines 354-447 and understand the full flow from `webhook_handler`.

**Step 2: Remove underscore prefix and use runtime**

Replace the function:

```rust
async fn invoke_agent(
    runtime: Option<&crate::AgentRuntime>,
    agent_id: AgentId,
    input_data: Value,
    llm_client: Option<&LlmClient>,
    agent_dsl_sources: &[(String, String)],
) -> Result<Value, RuntimeError> {
    let start = std::time::Instant::now();

    // Prefer runtime execution when available
    if let Some(rt) = runtime {
        use crate::api::types::ExecuteAgentRequest;

        let request = ExecuteAgentRequest {
            input: Some(input_data.clone()),
            parameters: None,
        };

        match rt.execute_agent_via_runtime(agent_id, request).await {
            Ok(response) => {
                let latency = start.elapsed();
                tracing::info!(
                    agent = %agent_id,
                    latency_ms = latency.as_millis(),
                    "Agent execution completed via runtime"
                );
                return Ok(serde_json::json!({
                    "status": "completed",
                    "agent_id": agent_id.to_string(),
                    "response": response.result,
                    "execution_id": response.execution_id,
                    "via": "runtime",
                    "latency_ms": latency.as_millis(),
                    "timestamp": chrono::Utc::now().to_rfc3339()
                }));
            }
            Err(e) => {
                tracing::warn!(
                    agent = %agent_id,
                    error = %e,
                    "Runtime execution failed, falling back to LLM"
                );
                // Fall through to LLM path
            }
        }
    }

    // Fall back to LLM invocation
    let llm = match llm_client {
        Some(client) => client,
        None => {
            return Err(RuntimeError::Internal(format!(
                "No runtime or LLM client available for agent {}. \
                 Configure an LLM provider or ensure the runtime is running.",
                agent_id
            )));
        }
    };

    // ... (keep existing LLM invocation code from lines 382-446 as-is)
```

Note: The `execute_agent_via_runtime` method may not exist on `AgentRuntime` yet. Check if `AgentRuntime` implements `RuntimeApiProvider` or has a direct execution method. If `AgentRuntime` only has the `execute_agent` trait method via `RuntimeApiProvider`, use that. The key change is:

1. Remove the `_` prefix from `runtime`
2. Actually try to use the runtime when available
3. Return a real error instead of a fake success when neither runtime nor LLM is available

If `AgentRuntime` doesn't expose a direct `execute_agent` method suitable for this context, use the communication bus to send a message and return the execution acknowledgment (which is what `execute_agent` in lib.rs does). The important thing is that calling with no LLM no longer returns a misleading "invoked" success.

**Step 3: Run tests**

```bash
cargo test -p symbi-runtime -- --nocapture
```

**Step 4: Run clippy**

```bash
cargo clippy --workspace -- -D warnings
```

**Step 5: Commit**

```bash
git add crates/runtime/src/http_input/server.rs
git commit -m "Use runtime for agent invocation, error on no-op path

invoke_agent now uses AgentRuntime when available instead of ignoring
it. Returns a proper error when neither runtime nor LLM client is
configured, instead of a misleading synthetic success response."
```

---

## Files Summary

| File | Tasks | Action |
|------|-------|--------|
| `crates/runtime/src/lib.rs` | 1 | Edit execute_agent to use create_internal_message |
| `crates/runtime/src/communication/mod.rs` | 1 | Add create_internal_message to trait + fix test helpers |
| `crates/runtime/src/integrations/mcp/client.rs` | 2 | Implement real HTTPS key fetch |
| `crates/runtime/src/integrations/mcp/types.rs` | 2 | Add KeyFetchFailed error variant |
| `crates/runtime/src/api/server.rs` | 3 | Split health route, add auth startup warning |
| `crates/runtime/src/http_input/config.rs` | 4, 5 | cors_origins field, loopback default |
| `crates/runtime/src/http_input/server.rs` | 4, 5, 6, 7, 8 | CORS, startup warning, JWT auth, PathPrefix, invoke_agent |
| `crates/runtime/Cargo.toml` | 6 | Add pem crate if needed |

---

## Verification

After all tasks:

```bash
cd /home/jascha/Documents/ThirdKey/repos/symbiont
cargo fmt --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

All must pass with zero warnings and zero failures.
