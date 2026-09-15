//! HTTP Input server implementation
//!
//! This module provides the HTTP input server that receives webhook/HTTP requests
//! and routes them to appropriate Symbiont agents based on configuration rules.

#[cfg(feature = "http-input")]
#[path = "invocations.rs"]
mod invocations;
#[cfg(all(feature = "http-input", unix))]
use invocations::invoke_agent;
#[cfg(feature = "http-input")]
use invocations::{AuthenticatedCaller, HttpInvocation};

#[cfg(feature = "http-input")]
use std::path::{Path, PathBuf};
#[cfg(feature = "http-input")]
use std::sync::Arc;

#[cfg(feature = "http-input")]
use axum::{
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode, Uri},
    middleware,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
#[cfg(feature = "http-input")]
use serde_json::Value;
#[cfg(feature = "http-input")]
use tokio::sync::{RwLock, Semaphore};
#[cfg(feature = "http-input")]
use tower_http::cors::CorsLayer;

#[cfg(feature = "http-input")]
use super::config::{HttpInputConfig, ResponseControlConfig, RouteMatch};
#[cfg(feature = "http-input")]
use crate::reasoning::circuit_breaker::CircuitBreakerRegistry;
#[cfg(feature = "http-input")]
use crate::reasoning::context_manager::DefaultContextManager;
#[cfg(feature = "http-input")]
use crate::reasoning::conversation::{Conversation, ConversationMessage, MessageRole};
#[cfg(feature = "http-input")]
use crate::reasoning::executor::ActionExecutor;
#[cfg(feature = "http-input")]
use crate::reasoning::inference::InferenceProvider;
#[cfg(feature = "http-input")]
use crate::reasoning::loop_types::{LoopConfig, TerminationReason};
#[cfg(feature = "http-input")]
use crate::reasoning::policy_bridge::{DefaultPolicyGate, ReasoningPolicyGate};
#[cfg(feature = "http-input")]
use crate::reasoning::reasoning_loop::ReasoningLoopRunner;
#[cfg(feature = "http-input")]
use crate::reasoning::tool_executor_builder::build_tool_executor;
#[cfg(feature = "http-input")]
use crate::secrets::{new_secret_store, SecretStore, SecretsConfig};
#[cfg(feature = "http-input")]
use crate::text_util::truncate_utf8;
#[cfg(feature = "http-input")]
use crate::types::{AgentId, RuntimeError};

/// HTTP Input Server that handles incoming webhook requests
#[cfg(feature = "http-input")]
pub struct HttpInputServer {
    config: Arc<RwLock<HttpInputConfig>>,
    project_root: PathBuf,
    runtime: Option<Arc<crate::AgentRuntime>>,
    secret_store: Option<Arc<dyn SecretStore + Send + Sync>>,
    executor: Option<Arc<dyn ActionExecutor>>,
    inference_provider: Option<Arc<dyn InferenceProvider>>,
    policy_gate: Option<Arc<dyn ReasoningPolicyGate>>,
    concurrency_limiter: Arc<Semaphore>,
    resolved_auth_header: Arc<RwLock<Option<String>>>,
}

#[cfg(feature = "http-input")]
impl HttpInputServer {
    /// Create a new HTTP Input server instance
    pub fn new(config: HttpInputConfig) -> Self {
        let concurrency_limiter = Arc::new(Semaphore::new(config.concurrency));

        Self {
            config: Arc::new(RwLock::new(config)),
            project_root: PathBuf::from("."),
            runtime: None,
            secret_store: None,
            executor: None,
            inference_provider: None,
            policy_gate: None,
            concurrency_limiter,
            resolved_auth_header: Arc::new(RwLock::new(None)),
        }
    }

    /// Set the trusted project containing tool manifests and sandbox profiles.
    /// Relative paths are resolved once when the server starts.
    pub fn with_project_root(mut self, project_root: PathBuf) -> Self {
        self.project_root = project_root;
        self
    }

    /// Set the runtime for agent invocation
    pub fn with_runtime(mut self, runtime: Arc<crate::AgentRuntime>) -> Self {
        self.runtime = Some(runtime);
        self
    }

    /// Set the tool executor used to dispatch model-proposed tool calls.
    /// If unset, `start()` defaults to `build_tool_executor(Path::new("tools"))`:
    /// `ToolCladExecutor` when `tools/*.clad.toml` manifests are present,
    /// otherwise the honest `UnavailableToolExecutor`.
    pub fn with_executor(mut self, executor: Arc<dyn ActionExecutor>) -> Self {
        self.executor = Some(executor);
        self
    }

    /// Set the inference provider driving the governed reasoning loop.
    /// If unset, `start()` resolves one from the environment (requires the
    /// `cloud-llm` feature); `None` at that point disables the LLM/tool-calling
    /// path. Every HTTP invocation requires an actual inference provider.
    pub fn with_inference_provider(mut self, provider: Arc<dyn InferenceProvider>) -> Self {
        self.inference_provider = Some(provider);
        self
    }

    /// Set the policy gate every model-proposed action must pass through
    /// before dispatch. If unset, `start()` resolves it to
    /// **fail-closed** (`DefaultPolicyGate::new()`) — never permissive. An
    /// embedder that forgets to wire a gate gets denial for every tool call,
    /// not free rein.
    pub fn with_policy_gate(mut self, gate: Arc<dyn ReasoningPolicyGate>) -> Self {
        self.policy_gate = Some(gate);
        self
    }

    /// Set the secret store for auth header resolution
    pub fn with_secret_store(mut self, secret_store: Arc<dyn SecretStore + Send + Sync>) -> Self {
        self.secret_store = Some(secret_store);
        self
    }

    /// Start the HTTP input server
    pub async fn start(&self) -> Result<(), RuntimeError> {
        let project_root = self
            .project_root
            .canonicalize()
            .map_err(|e| RuntimeError::Internal(format!("Invalid HTTP project root: {e}")))?;
        let config = self.config.read().await;
        let addr = format!("{}:{}", config.bind_address, config.port);

        // Refuse to start when CORS is configured as a wildcard. The main
        // API server uses an explicit-allowlist pattern; the HTTP input
        // server must match that to avoid being a softer target for
        // browser-origin cross-site exploitation. See SECURITY_AUDIT.md M1.
        if config.cors_origins.iter().any(|o| o == "*") {
            return Err(RuntimeError::Configuration(
                crate::types::ConfigError::Invalid(
                    "CORS wildcard '*' is not permitted on the HTTP input server. \
                     Specify explicit origins."
                        .to_string(),
                ),
            ));
        }

        // Warn when binding to a non-loopback address without any authentication
        if config.bind_address != "127.0.0.1"
            && config.bind_address != "localhost"
            && config.auth_header.is_none()
            && config.jwt_public_key_path.is_none()
            && config.webhook_verify.is_none()
        {
            tracing::warn!(
                bind = %config.bind_address,
                "HTTP input binding to non-loopback address with no authentication configured. \
                 Set auth_header, jwt_public_key_path, or webhook_verify for production use."
            );
        }

        // Resolve auth header if it's a secret reference
        if let Some(auth_header) = &config.auth_header {
            if let Some(secret_store) = &self.secret_store {
                let resolved = resolve_secret_reference(secret_store.as_ref(), auth_header).await?;
                *self.resolved_auth_header.write().await = Some(resolved);
            } else {
                *self.resolved_auth_header.write().await = Some(auth_header.clone());
            }
        }

        // Load JWT public key if configured (fail fast on invalid key)
        let jwt_decoding_key = if let Some(ref key_path) = config.jwt_public_key_path {
            let key_bytes = tokio::fs::read(key_path).await.map_err(|e| {
                RuntimeError::Configuration(crate::types::ConfigError::Invalid(format!(
                    "Failed to read JWT public key file '{}': {}",
                    key_path, e
                )))
            })?;

            // Try PEM first, fall back to raw DER (32-byte Ed25519 public key)
            let decoding_key = if key_bytes.starts_with(b"-----") {
                jsonwebtoken::DecodingKey::from_ed_pem(&key_bytes).map_err(|e| {
                    RuntimeError::Configuration(crate::types::ConfigError::Invalid(format!(
                        "Invalid Ed25519 PEM public key in '{}': {}",
                        key_path, e
                    )))
                })?
            } else {
                if key_bytes.len() != 32 {
                    return Err(RuntimeError::Configuration(
                        crate::types::ConfigError::Invalid(
                            "Raw Ed25519 JWT public keys must contain exactly 32 bytes".into(),
                        ),
                    ));
                }
                jsonwebtoken::DecodingKey::from_ed_der(&key_bytes)
            };

            tracing::info!(path = %key_path, "Loaded JWT EdDSA public key for Bearer token validation");
            use sha2::Digest;
            Some((
                Arc::new(decoding_key),
                hex::encode(sha2::Sha256::digest(&key_bytes)),
            ))
        } else {
            None
        };

        // Resolve the inference provider driving the governed reasoning loop:
        // an explicit override (set via `with_inference_provider`, e.g. by
        // tests or embedders) wins, otherwise auto-detect from the
        // environment. Auto-detection requires the `cloud-llm` feature;
        // without it this is always `None`, same as "no API key configured"
        // today. Auto-detection covers OpenAI, Anthropic, OpenRouter, and
        // Bedrock (Bedrock detection lives inside `CloudInferenceProvider`
        // via `LlmClient::from_env`).
        let inference_provider = self
            .inference_provider
            .clone()
            .or_else(inference_provider_from_env);

        // Resolve the tool executor: an explicit override wins, otherwise
        // discover `tools/*.clad.toml` manifests (or fall back to the honest
        // `UnavailableToolExecutor` when none are present).
        let executor: Arc<dyn ActionExecutor> = self.executor.clone().unwrap_or_else(|| {
            if self.runtime.is_some() {
                // A registered agent gets its executor after identity selection.
                Arc::new(crate::reasoning::executor::UnavailableToolExecutor)
            } else {
                build_tool_executor(&project_root.join("tools"))
            }
        });
        let executor_tool_count = executor.tool_definitions().len();
        if executor_tool_count > 0 {
            tracing::info!(
                "HTTP Input: tool executor loaded with {} tool(s)",
                executor_tool_count
            );
        }

        // Resolve the policy gate every model-proposed action must pass
        // through. An explicit override wins, otherwise fail-closed — an
        // embedder that forgets to wire a gate gets denial for every tool
        // call, never free rein.
        let policy_gate: Arc<dyn ReasoningPolicyGate> = self
            .policy_gate
            .clone()
            .unwrap_or_else(|| Arc::new(DefaultPolicyGate::new()));

        // Circuit breakers are shared across requests; journals are exclusive
        // to each invocation and stored outside worker-writable mounts.
        let circuit_breakers = Arc::new(CircuitBreakerRegistry::default());

        // Resolve webhook signature verifier if configured
        let webhook_verifier: Option<Arc<dyn super::webhook_verify::SignatureVerifier>> =
            if let Some(ref verify_config) = config.webhook_verify {
                let provider = match verify_config.provider.to_lowercase().as_str() {
                    "github" => super::webhook_verify::WebhookProvider::GitHub,
                    "stripe" => super::webhook_verify::WebhookProvider::Stripe,
                    "slack" => super::webhook_verify::WebhookProvider::Slack,
                    _ => super::webhook_verify::WebhookProvider::Custom,
                };
                let secret_value = if let Some(ref store) = self.secret_store {
                    match resolve_secret_reference(store.as_ref(), &verify_config.secret).await {
                        Ok(resolved) => resolved,
                        Err(e) => {
                            tracing::warn!(
                                "Failed to resolve webhook secret reference: {}. Using literal value.",
                                e
                            );
                            verify_config.secret.clone()
                        }
                    }
                } else {
                    verify_config.secret.clone()
                };
                Some(Arc::from(provider.verifier(secret_value.as_bytes())))
            } else {
                None
            };

        // Create shared server state
        let server_state = ServerState {
            config: self.config.clone(),
            project_root,
            runtime: self.runtime.clone(),
            concurrency_limiter: self.concurrency_limiter.clone(),
            resolved_auth_header: self.resolved_auth_header.clone(),
            inference_provider,
            executor,
            executor_is_override: self.executor.is_some(),
            policy_gate,
            circuit_breakers,
            webhook_verifier,
            jwt_decoding_key,
        };

        // Build the router
        let mut app = Router::new();

        // Add the webhook endpoint
        let path = config.path.clone();
        app = app.route(&path, post(webhook_handler));

        // Add wildcard catch-all route for PathPrefix routing on subpaths
        let wildcard_path = format!("{}/*rest", path.trim_end_matches('/'));
        app = app.route(&wildcard_path, post(webhook_handler));

        // Add middleware
        app = app.layer(middleware::from_fn_with_state(
            server_state.clone(),
            auth_middleware,
        ));

        // Add body size limit
        app = app.layer(DefaultBodyLimit::max(config.max_body_bytes));

        // Add CORS if origins are configured. Wildcard origin is rejected
        // at the top of `start` (see SECURITY_AUDIT.md M1), so by the time
        // we get here the list is guaranteed to be an explicit allowlist.
        if !config.cors_origins.is_empty() {
            use axum::http::{header, HeaderValue, Method};

            let origins: Vec<HeaderValue> = config
                .cors_origins
                .iter()
                .filter_map(|o| o.parse().ok())
                .collect();
            let cors = CorsLayer::new()
                .allow_origin(origins)
                .allow_methods([Method::POST])
                .allow_headers([
                    header::AUTHORIZATION,
                    header::CONTENT_TYPE,
                    axum::http::HeaderName::from_static("idempotency-key"),
                ])
                .expose_headers([
                    axum::http::HeaderName::from_static("idempotency-key"),
                    axum::http::HeaderName::from_static("idempotency-replayed"),
                ])
                .allow_credentials(false);
            app = app.layer(cors);
        }

        tracing::info!("Starting HTTP Input server on {}", addr);

        // Start the server
        let listener = tokio::net::TcpListener::bind(&addr).await.map_err(|e| {
            RuntimeError::Internal(format!("Failed to bind to address {}: {}", addr, e))
        })?;

        // Add state and convert to make service
        let app_with_state = app.with_state(server_state);
        axum::serve(listener, app_with_state.into_make_service())
            .await
            .map_err(|e| RuntimeError::Internal(format!("Server error: {}", e)))?;

        Ok(())
    }

    /// Stop the HTTP input server gracefully
    pub async fn stop(&self) -> Result<(), RuntimeError> {
        tracing::info!("HTTP Input server stopping");
        // Axum server will be stopped when the future is dropped
        Ok(())
    }

    /// Update server configuration
    pub async fn update_config(&self, new_config: HttpInputConfig) -> Result<(), RuntimeError> {
        *self.config.write().await = new_config;
        Ok(())
    }

    /// Get current server configuration
    pub async fn get_config(&self) -> HttpInputConfig {
        self.config.read().await.clone()
    }
}

/// Auto-detect an [`InferenceProvider`] from the environment (`cloud-llm`
/// build). Wraps `CloudInferenceProvider`, which already covers OpenAI,
/// Anthropic, OpenRouter, and — when also built with `bedrock` — AWS
/// Bedrock.
#[cfg(all(feature = "http-input", feature = "cloud-llm"))]
fn inference_provider_from_env() -> Option<Arc<dyn InferenceProvider>> {
    crate::reasoning::providers::cloud::CloudInferenceProvider::from_env()
        .map(|p| Arc::new(p) as Arc<dyn InferenceProvider>)
}

/// Without `cloud-llm` there is no `InferenceProvider` impl to construct
/// from the environment, so the LLM/tool-calling path is unavailable —
/// same as "no API key configured" today. The runtime communication bus
/// path is unaffected.
#[cfg(all(feature = "http-input", not(feature = "cloud-llm")))]
fn inference_provider_from_env() -> Option<Arc<dyn InferenceProvider>> {
    None
}

/// Shared state for the HTTP server
#[cfg(feature = "http-input")]
#[derive(Clone)]
struct ServerState {
    config: Arc<RwLock<HttpInputConfig>>,
    project_root: PathBuf,
    runtime: Option<Arc<crate::AgentRuntime>>,
    concurrency_limiter: Arc<Semaphore>,
    resolved_auth_header: Arc<RwLock<Option<String>>>,
    /// Inference provider driving the governed reasoning loop. `None`
    /// disables the LLM/tool-calling path (the runtime communication bus
    /// path is unaffected).
    inference_provider: Option<Arc<dyn InferenceProvider>>,
    /// Tool executor dispatching model-proposed tool calls.
    executor: Arc<dyn ActionExecutor>,
    /// Explicit SDK backends remain owned by the trusted embedder.
    executor_is_override: bool,
    /// Policy gate every model-proposed action must pass through before
    /// dispatch. Always present: `HttpInputServer::start()` resolves an
    /// unset gate to fail-closed (`DefaultPolicyGate::new()`), never
    /// permissive.
    policy_gate: Arc<dyn ReasoningPolicyGate>,
    /// Circuit breaker registry shared across every request this server
    /// instance handles.
    circuit_breakers: Arc<CircuitBreakerRegistry>,
    /// Optional webhook signature verifier
    webhook_verifier: Option<Arc<dyn super::webhook_verify::SignatureVerifier>>,
    /// Optional JWT EdDSA verifying key for Bearer token validation
    jwt_decoding_key: Option<(Arc<jsonwebtoken::DecodingKey>, String)>,
}

/// JWT claims structure for EdDSA token validation
#[cfg(feature = "http-input")]
#[derive(serde::Deserialize)]
struct JwtClaims {
    /// Expiration time (validated automatically by jsonwebtoken)
    #[allow(dead_code)]
    exp: u64,
    /// Stable authenticated subject; token renewal must not change request identity.
    sub: String,
    #[serde(default)]
    iss: Option<String>,
}

#[cfg(feature = "http-input")]
fn jwt_validation() -> jsonwebtoken::Validation {
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::EdDSA);
    validation.set_required_spec_claims(&["exp", "sub"]);
    validation.validate_aud = false;
    validation.leeway = 5;
    validation
}

/// Authentication middleware
///
/// Auth flow:
/// 1. Try static `auth_header` match (constant-time comparison) — if it matches, allow through
/// 2. Try JWT: extract Bearer token, verify Ed25519 signature + `exp` expiration
/// 3. If neither method validates AND at least one is configured, return 401
/// 4. If no auth is configured, refuse the request
#[cfg(feature = "http-input")]
async fn auth_middleware(
    State(state): State<ServerState>,
    headers: HeaderMap,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<Response, StatusCode> {
    let resolved_auth = state.resolved_auth_header.read().await;
    let has_static_auth = resolved_auth.is_some();
    let has_jwt_auth = state.jwt_decoding_key.is_some();

    // No auth configured: refuse rather than allow through. This endpoint drives
    // an agent that can execute ToolClad tools, so an unconfigured deployment
    // must not be an open one. `HttpInputConfig::default()` leaves both
    // `auth_header` and `jwt_public_key_path` unset, so a library embedder that
    // takes the defaults previously served this unauthenticated.
    if !has_static_auth && !has_jwt_auth {
        tracing::error!(
            "HTTP input received a request but no authentication is configured \
             (set auth_header or jwt_public_key_path); refusing the request"
        );
        return Err(StatusCode::UNAUTHORIZED);
    }

    let auth_header = headers.get("Authorization").and_then(|h| h.to_str().ok());

    // 1. Try static auth_header match first (constant-time comparison)
    if let Some(expected_auth) = resolved_auth.as_ref() {
        if let Some(provided_auth) = auth_header {
            if subtle::ConstantTimeEq::ct_eq(provided_auth.as_bytes(), expected_auth.as_bytes())
                .into()
            {
                req.extensions_mut()
                    .insert(AuthenticatedCaller::static_token(expected_auth));
                drop(resolved_auth);
                return Ok(next.run(req).await);
            }
        }
    }

    // 2. Try JWT Bearer token validation
    if let Some((ref decoding_key, ref authority)) = state.jwt_decoding_key {
        if let Some(provided_auth) = auth_header {
            if let Some(token) = provided_auth.strip_prefix("Bearer ") {
                // The configured key loader accepts Ed25519. Mixing algorithm
                // families in Validation also rejects valid EdDSA signatures.
                let header = match jsonwebtoken::decode_header(token) {
                    Ok(h) => h,
                    Err(e) => {
                        tracing::warn!(error = %e, "JWT header decode failed");
                        return Err(StatusCode::UNAUTHORIZED);
                    }
                };
                if header.alg != jsonwebtoken::Algorithm::EdDSA {
                    tracing::warn!(
                        algorithm = ?header.alg,
                        "JWT algorithm not allowed on asymmetric Bearer path \
                         (expected EdDSA)"
                    );
                    return Err(StatusCode::UNAUTHORIZED);
                }

                let validation = jwt_validation();

                match jsonwebtoken::decode::<JwtClaims>(token, decoding_key, &validation) {
                    Ok(token_data) => {
                        let caller = AuthenticatedCaller::jwt(
                            authority,
                            &token_data.claims.sub,
                            token_data.claims.iss.as_deref(),
                        )
                        .ok_or(StatusCode::UNAUTHORIZED)?;
                        req.extensions_mut().insert(caller);
                        drop(resolved_auth);
                        return Ok(next.run(req).await);
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "JWT validation failed");
                    }
                }
            }
        }
    }

    // Neither auth method succeeded
    if auth_header.is_none() {
        tracing::warn!("Authentication failed: missing Authorization header");
    } else {
        tracing::warn!("Authentication failed: no configured auth method accepted the token");
    }
    Err(StatusCode::UNAUTHORIZED)
}

/// Main webhook handler
#[cfg(feature = "http-input")]
async fn webhook_handler(
    State(state): State<ServerState>,
    axum::Extension(caller): axum::Extension<AuthenticatedCaller>,
    uri: Uri,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Response, StatusCode> {
    // Check concurrency limits
    let _permit = state.concurrency_limiter.try_acquire().map_err(|_| {
        tracing::warn!("Concurrency limit exceeded");
        StatusCode::TOO_MANY_REQUESTS
    })?;

    // Verify webhook signature if configured
    if let Some(ref verifier) = state.webhook_verifier {
        let header_pairs: Vec<(String, String)> = headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|v| (name.to_string(), v.to_string()))
            })
            .collect();

        if let Err(e) = verifier.verify(&header_pairs, &body).await {
            tracing::warn!("Webhook signature verification failed: {}", e);
            return Err(StatusCode::UNAUTHORIZED);
        }
    }

    // Parse JSON from raw body
    let payload: Value = serde_json::from_slice(&body).map_err(|e| {
        tracing::warn!("Invalid JSON body: {}", e);
        StatusCode::BAD_REQUEST
    })?;

    let config = state.config.read().await;

    // Log audit information if enabled
    if config.audit_enabled {
        tracing::info!(
            "HTTP Input: Received request with {} headers",
            headers.len()
        );
    }

    // Route to appropriate agent
    let agent_id = route_request(&config, uri.path(), &payload, &headers).await;

    let invocation = match HttpInvocation::from_headers(&headers, caller, uri.to_string()) {
        Ok(invocation) => invocation,
        Err(message) => {
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "status":"invalid_invocation_id", "error":message
                })),
            )
                .into_response())
        }
    };
    let id = invocation.id;
    #[cfg(unix)]
    let result = invoke_agent(
        state.runtime.as_deref(),
        agent_id,
        payload,
        state.inference_provider.clone(),
        &state.project_root,
        state.executor.clone(),
        state.executor_is_override,
        state.policy_gate.clone(),
        state.circuit_breakers.clone(),
        invocation,
    )
    .await;
    #[cfg(not(unix))]
    let result = Err::<invocations::HttpInvocationReply, _>(RuntimeError::Internal(
        "Protected HTTP invocation storage is unavailable on this platform".into(),
    ));
    match result {
        Ok(result) => result.into_response(id, config.response_control.as_ref()),
        Err(e) => {
            tracing::error!("Agent invocation failed: {:?}", e);
            let response_config = config.response_control.as_ref();
            format_error_response(e, response_config)
        }
    }
}

/// Route incoming request to appropriate agent
#[cfg(feature = "http-input")]
async fn route_request(
    config: &HttpInputConfig,
    request_path: &str,
    payload: &Value,
    headers: &HeaderMap,
) -> AgentId {
    // Check routing rules if configured
    if let Some(routing_rules) = &config.routing_rules {
        for rule in routing_rules {
            if matches_route_condition(&rule.condition, request_path, payload, headers).await {
                tracing::debug!("Request routed to agent {} via rule", rule.agent);
                return rule.agent;
            }
        }
    }

    // Return default agent
    tracing::debug!("Request routed to default agent {}", config.agent);
    config.agent
}

/// Check if a route condition matches the request
#[cfg(feature = "http-input")]
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

/// Execute one HTTP invocation under the selected agent's governed loop.
/// Policy checks precede tool effects, and each request owns its protected
/// journal and cancellation/cleanup task, independently of other invocations.
#[cfg(all(feature = "http-input", unix))]
#[allow(clippy::too_many_arguments)]
async fn execute_http_run(
    registered: Option<crate::types::AgentConfig>,
    agent_id: AgentId,
    input_data: Value,
    inference_provider: Option<Arc<dyn InferenceProvider>>,
    project_root: &Path,
    executor: Arc<dyn ActionExecutor>,
    executor_is_override: bool,
    policy_gate: Arc<dyn ReasoningPolicyGate>,
    circuit_breakers: Arc<CircuitBreakerRegistry>,
    invocation: Box<crate::reasoning::invocation::Invocation>,
    cancellation: tokio_util::sync::CancellationToken,
) -> Result<crate::reasoning::invocation::ExistingInvocation, RuntimeError> {
    let start = std::time::Instant::now();
    let resource_timeout = registered
        .as_ref()
        .map(|agent| agent.resource_limits.execution_timeout);
    let (executor, selected_source, selected_settings) = if let Some(agent) = registered {
        let project = project_root.to_path_buf();
        let (selected_executor, settings) = tokio::task::spawn_blocking(move || {
            registered_agent_executor(&project, &agent, executor_is_override.then_some(executor))
        })
        .await
        .map_err(|e| RuntimeError::Internal(e.to_string()))?
        .map_err(|e| RuntimeError::Internal(format!("Agent sandbox selection failed: {e}")))?;
        (
            selected_executor,
            Some((settings.agent_name.clone(), settings.agent_source.clone())),
            Some(settings),
        )
    } else {
        // A standalone SDK server has its explicitly configured generic agent.
        // Scanning unrelated files cannot establish an AgentId/source binding.
        (executor, None, None)
    };

    // A running invocation is not a message-bus listener. Each HTTP request
    // must execute its own governed loop and return that invocation's outcome.

    // Run the governed reasoning loop.
    let provider = match inference_provider {
        Some(p) => p,
        None => {
            return Err(RuntimeError::Internal(format!(
                "No runtime or inference provider available for agent {}. \
                 Configure an LLM provider or ensure the runtime is running.",
                agent_id
            )));
        }
    };

    // Build system prompt from DSL sources
    let mut system_parts: Vec<String> = Vec::new();

    if let Some((name, source)) = selected_source {
        system_parts.push(format!("You are agent {name:?} operating within the Symbiont runtime.\n--- Agent DSL ---\n{source}\n--- End DSL ---"));
        system_parts.push("Use the available governed tools to perform the requested task.".into());
    } else {
        system_parts.push("You are an agent operating within the Symbiont runtime. Use the available governed tools to perform the requested task.".into());
    }

    // Allow caller-supplied system_prompt but cap length and log its use.
    // This is a prompt-injection surface when the endpoint faces untrusted
    // callers — authentication should be enforced at the transport layer.
    const MAX_SYSTEM_PROMPT_LEN: usize = 4096;
    if let Some(custom_system) = input_data.get("system_prompt").and_then(|v| v.as_str()) {
        let truncated = truncate_utf8(custom_system, MAX_SYSTEM_PROMPT_LEN);
        if truncated.len() < custom_system.len() {
            tracing::warn!(
                "Caller-supplied system_prompt truncated from {} to {} bytes",
                custom_system.len(),
                truncated.len(),
            );
        }
        tracing::info!(
            "Caller-supplied system_prompt appended ({} bytes) for agent {}",
            truncated.len(),
            agent_id,
        );
        system_parts.push(format!("\n{}", truncated));
    }

    let system_prompt = system_parts.join("\n");

    let user_message = if let Some(prompt) = input_data.get("prompt").and_then(|v| v.as_str()) {
        prompt.to_string()
    } else if let Some(msg) = input_data.get("message").and_then(|v| v.as_str()) {
        msg.to_string()
    } else {
        let payload_str =
            serde_json::to_string_pretty(&input_data).unwrap_or_else(|_| input_data.to_string());
        format!(
            "Execute the following task using your available tools:\n\n{}",
            payload_str
        )
    };

    tracing::info!(
        "Invoking governed reasoning loop for agent {}: provider={} model={} tools={} system_len={} user_len={}",
        agent_id,
        provider.provider_name(),
        provider.default_model(),
        executor.tool_definitions().len(),
        system_prompt.len(),
        user_message.len(),
    );

    let mut conversation = Conversation::with_system(system_prompt);
    conversation.push(ConversationMessage::user(user_message));

    // Preserve the previous hand-rolled loop's bounds that this handler
    // still owns: up to 15 tool-calling round trips per request, and a 120s
    // per-tool timeout (ToolClad tools like nmap_scan can run for minutes).
    // Everything else — token budget, overall wall-clock timeout, concurrent
    // tool cap — is the runner's own `LoopConfig` default: the runner, not
    // this handler, now owns those bounds.
    let mut loop_config = LoopConfig {
        max_iterations: 15,
        tool_timeout: std::time::Duration::from_secs(120),
        timeout: selected_settings
            .and_then(|s| s.timeout_seconds)
            .map(std::time::Duration::from_secs)
            .map_or(LoopConfig::default().timeout, |bound| {
                bound.min(LoopConfig::default().timeout)
            }),
        ..LoopConfig::default()
    };

    if let Some(bound) = resource_timeout {
        loop_config.timeout = loop_config.timeout.min(bound);
    }

    let journal = invocation.journal();
    let audit = invocation.audit().clone();
    tracing::info!(%agent_id, run_id = %audit.run_id, path = %audit.path.display(), public_key = %audit.public_key, "Required run audit opened");

    let runner = ReasoningLoopRunner {
        provider: provider.clone(),
        policy_gate,
        executor,
        context_manager: Arc::new(DefaultContextManager::default()),
        circuit_breakers,
        journal,
        knowledge_bridge: None,
        delegation: None,
    };

    let result = runner
        .run_cancellable(agent_id, conversation, loop_config, cancellation)
        .await;

    let tool_runs = reconstruct_tool_runs(&result.conversation);

    let latency = start.elapsed();
    tracing::info!(
        "Reasoning loop completed for agent {}: latency={:?} iterations={} tool_runs={} response_len={} termination={:?}",
        agent_id,
        latency,
        result.iterations,
        tool_runs.len(),
        result.output.len(),
        result.termination_reason,
    );

    let completed = matches!(result.termination_reason, TerminationReason::Completed);
    let body = serde_json::json!({
        "status": if completed { "completed" } else { "failed" },
        "audit": audit,
        "agent_id": agent_id.to_string(),
        "response": result.output,
        "tool_runs": tool_runs,
        "termination_reason": result.termination_reason,
        "iterations": result.iterations,
        "total_usage": result.total_usage,
        "budget": result.budget,
        "model": provider.default_model(),
        "provider": provider.provider_name(),
        "latency_ms": latency.as_millis(),
        "timestamp": chrono::Utc::now().to_rfc3339()
    });
    invocation
        .finish(serde_json::json!({"completed": completed, "body": body}))
        .await
        .map_err(|error| {
            RuntimeError::Internal(format!("Invocation result could not be persisted: {error}"))
        })
}

/// Rebuild the `tool_runs` response field from the governed run's
/// conversation. Each `Tool`-role message corresponds to one tool call the
/// model proposed — executed, denied, or schema-rejected — correlated back
/// to the assistant's original call by `tool_call_id` to recover its input.
///
/// Field-fidelity note (see CHANGELOG): `tool` is the conversation's
/// `tool_name`, i.e. the executor's `Observation::source`. For calls
/// dispatched through `ToolCladExecutor` that carries a `toolclad:` prefix
/// (its own naming convention: `format!("toolclad:{}", name)`), not the bare
/// model-supplied name the previous hand-rolled loop reported. Denied /
/// schema-rejected calls never reach the executor and keep the bare name.
/// Rather than guess at stripping a prefix that may not even be ToolClad's
/// (agent delegation uses `delegate:<target>`), this reports the label the
/// governed loop actually recorded instead of fabricating the old shape.
#[cfg(feature = "http-input")]
fn reconstruct_tool_runs(conversation: &Conversation) -> Vec<serde_json::Value> {
    use std::collections::HashMap;

    let mut call_args: HashMap<&str, &str> = HashMap::new();
    for msg in conversation.messages() {
        if msg.role == MessageRole::Assistant {
            for tc in &msg.tool_calls {
                call_args.insert(tc.id.as_str(), tc.arguments.as_str());
            }
        }
    }

    conversation
        .messages()
        .iter()
        .filter(|m| m.role == MessageRole::Tool)
        .map(|m| {
            let tool_name = m.tool_name.clone().unwrap_or_default();
            let input = m
                .tool_call_id
                .as_deref()
                .and_then(|id| call_args.get(id))
                .and_then(|args| serde_json::from_str::<Value>(args).ok())
                .unwrap_or(Value::Null);
            let preview = truncate_utf8(&m.content, 500);
            serde_json::json!({
                "tool": tool_name,
                "input": input,
                "output_preview": preview,
            })
        })
        .collect()
}

/// Bind an HTTP request to its registered source and a single sandbox snapshot.
#[cfg(feature = "http-input")]
fn registered_agent_executor(
    project: &Path,
    agent: &crate::types::AgentConfig,
    override_executor: Option<Arc<dyn ActionExecutor>>,
) -> Result<(Arc<dyn ActionExecutor>, dsl::AgentExecutionSettings), String> {
    use crate::sandbox::command::{CommandBoundary, CommandTier};
    use crate::types::SecurityTier;
    if matches!(
        agent.execution_mode,
        crate::types::ExecutionMode::External { .. }
    ) {
        return Err("external agents require their own execution transport".into());
    }
    let tree = dsl::parse_dsl(&agent.dsl_source).map_err(|error| error.to_string())?;
    if dsl::extract_metadata(&tree, &agent.dsl_source)
        .get("executor")
        .is_some_and(|kind| kind.trim_matches('"') != "orga")
    {
        return Err("HTTP reasoning execution requires an ORGA agent; the selected executor is unavailable on this route".into());
    }
    let settings = dsl::resolve_execution_settings(&agent.dsl_source, &agent.name)?;
    let source_policy = dsl::ExecutionPolicy::parse(&agent.dsl_source, &agent.name)?;
    let mut boundary = CommandBoundary::load_for_agent(project, &settings)?;
    let selected_tier = match boundary.tier {
        CommandTier::DevelopmentHost => SecurityTier::None,
        CommandTier::Docker => SecurityTier::Tier1,
        CommandTier::GVisor => SecurityTier::Tier2,
        CommandTier::Firecracker => SecurityTier::Tier3,
        CommandTier::E2B => SecurityTier::Hosted,
        // No registered SecurityTier names the landlock tier, so a registered
        // agent cannot declare it. Refuse rather than mapping it onto a
        // neighbouring tier, which would misreport the isolation in use.
        CommandTier::Landlock => {
            return Err(
                "the landlock tier cannot be selected for a registered agent; \
                        declare it in [sandbox] for direct runs"
                    .into(),
            )
        }
    };
    if selected_tier != agent.security_tier {
        return Err("registered security tier conflicts with the selected agent sandbox".into());
    }
    boundary.tighten_resources(&agent.resource_limits)?;
    let executor = override_executor.unwrap_or_else(|| {
        crate::reasoning::tool_executor_builder::build_tool_executor_with_boundary(
            &project.join("tools"),
            boundary,
        )
    });
    Ok((
        Arc::new(crate::reasoning::source_policy::SourcePolicyExecutor::new(
            executor,
            source_policy,
        )),
        settings,
    ))
}

/// Format a successful response
#[cfg(feature = "http-input")]
fn format_success_response(
    result: Value,
    response_config: Option<&ResponseControlConfig>,
) -> Result<Response, StatusCode> {
    let default_config = ResponseControlConfig::default();
    let config = response_config.unwrap_or(&default_config);

    let status = StatusCode::from_u16(config.default_status).unwrap_or(StatusCode::OK);

    if config.agent_output_to_json {
        Ok((status, Json(result)).into_response())
    } else {
        Ok((status, "OK").into_response())
    }
}

/// Format an error response
#[cfg(feature = "http-input")]
fn format_error_response(
    error: RuntimeError,
    response_config: Option<&ResponseControlConfig>,
) -> Result<Response, StatusCode> {
    let default_config = ResponseControlConfig::default();
    let config = response_config.unwrap_or(&default_config);

    let status =
        StatusCode::from_u16(config.error_status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

    // Map internal errors to generic messages to avoid leaking details
    let public_message = match &error {
        RuntimeError::Security(_) => "Authentication error",
        RuntimeError::Configuration(_) => "Configuration error",
        _ => "Internal server error",
    };
    // Log only the error variant (a stable enum-tag, no internal paths
    // or arbitrary user input), not the full Display string. See
    // SECURITY_AUDIT.md L1.
    let kind_name = match &error {
        RuntimeError::Configuration(_) => "Configuration",
        RuntimeError::Resource(_) => "Resource",
        RuntimeError::Security(_) => "Security",
        RuntimeError::Communication(_) => "Communication",
        RuntimeError::Policy(_) => "Policy",
        RuntimeError::Sandbox(_) => "Sandbox",
        RuntimeError::Scheduler(_) => "Scheduler",
        RuntimeError::Lifecycle(_) => "Lifecycle",
        RuntimeError::Audit(_) => "Audit",
        RuntimeError::ErrorHandler(_) => "ErrorHandler",
        RuntimeError::Internal(_) => "Internal",
        RuntimeError::Authentication(_) => "Authentication",
    };
    tracing::info!(
        "HTTP error response (kind={}): public={}",
        kind_name,
        public_message
    );
    let error_body = serde_json::json!({
        "error": public_message,
        "timestamp": chrono::Utc::now().to_rfc3339()
    });

    Ok((status, Json(error_body)).into_response())
}

/// Resolve a secret reference (vault://, file://, etc.) to its actual value
#[cfg(feature = "http-input")]
async fn resolve_secret_reference(
    secret_store: &dyn SecretStore,
    reference: &str,
) -> Result<String, RuntimeError> {
    if reference.starts_with("vault://") || reference.starts_with("file://") {
        // Extract the key from the reference
        let key = reference.split("://").nth(1).ok_or_else(|| {
            RuntimeError::Configuration(crate::types::ConfigError::Invalid(
                "Invalid secret reference format".to_string(),
            ))
        })?;

        // Resolve the secret
        let secret = secret_store
            .get_secret(key)
            .await
            .map_err(|e| RuntimeError::Internal(format!("Secret resolution failed: {}", e)))?;

        // `Secret` has a zeroising `Drop`, so we can't move `value` out.
        // Clone the String; the original buffer is zeroised on drop.
        Ok(secret.value.clone())
    } else {
        // Not a secret reference, return as-is
        Ok(reference.to_string())
    }
}

/// Create a function to start the HTTP input server
///
/// `policy_gate` is the gate every model-proposed action must pass through.
/// Pass `None` only when there is truly no gate to wire — `HttpInputServer`
/// resolves that to **fail-closed** (`DefaultPolicyGate::new()`), never
/// permissive, so an embedder that forgets to pass one gets denial for every
/// tool call rather than free rein.
#[cfg(feature = "http-input")]
pub async fn start_http_input(
    config: HttpInputConfig,
    runtime: Option<Arc<crate::AgentRuntime>>,
    secrets_config: Option<SecretsConfig>,
    policy_gate: Option<Arc<dyn ReasoningPolicyGate>>,
) -> Result<(), RuntimeError> {
    let mut server = HttpInputServer::new(config);

    // Add runtime if provided
    if let Some(runtime) = runtime {
        server = server.with_runtime(runtime);
    }

    if let Some(gate) = policy_gate {
        server = server.with_policy_gate(gate);
    }

    // The standard executor factory loads custom types and binds the registered
    // agent's profile. Do not install a project-default executor as an override.

    // Add secret store if secrets config is provided
    if let Some(secrets_config) = secrets_config {
        let secret_store = new_secret_store(&secrets_config, "http_input")
            .await
            .map_err(|e| {
                RuntimeError::Internal(format!("Failed to initialize secret store: {}", e))
            })?;
        server = server.with_secret_store(Arc::from(secret_store));
    }

    server.start().await
}

#[cfg(all(test, feature = "http-input"))]
mod tests {
    use super::*;

    // `truncate_utf8` itself now lives in `crate::text_util` (shared with
    // `reasoning::knowledge_bridge`) and is covered by its own unit tests
    // there; this module keeps only server-specific behavior.

    /// C4 (RUSTSEC-2023-0071): the asymmetric Bearer-token verifier must
    /// pin its algorithm allowlist to ES256 + EdDSA. Any RSA / PS / HS /
    /// none variant in `Validation::algorithms` would re-open the Marvin
    /// Attack reachability through `jsonwebtoken`'s transitive `rsa` dep.
    #[test]
    fn test_jwt_bearer_validation_matches_the_configured_key_family() {
        let validation = jwt_validation();
        assert_eq!(validation.algorithms, vec![jsonwebtoken::Algorithm::EdDSA]);

        for forbidden in [
            jsonwebtoken::Algorithm::ES256,
            jsonwebtoken::Algorithm::ES384,
            jsonwebtoken::Algorithm::RS256,
            jsonwebtoken::Algorithm::RS384,
            jsonwebtoken::Algorithm::RS512,
            jsonwebtoken::Algorithm::PS256,
            jsonwebtoken::Algorithm::PS384,
            jsonwebtoken::Algorithm::PS512,
            jsonwebtoken::Algorithm::HS256,
            jsonwebtoken::Algorithm::HS384,
            jsonwebtoken::Algorithm::HS512,
        ] {
            assert!(
                !validation.algorithms.contains(&forbidden),
                "{:?} must not be in the asymmetric Bearer JWT allowlist",
                forbidden
            );
        }
    }

    // ---- Governed reasoning-loop tests ----
    //
    // These drive the real HTTP server (`HttpInputServer::start()`) end to
    // end with a scripted `InferenceProvider` (no network access) and a
    // real `ToolCladExecutor` backed by a throwaway manifest whose tool has
    // an observable side effect (creating a file via `touch`). That side
    // effect — not a response string — is what proves a denied call didn't
    // run and an allowed one did.

    use crate::reasoning::inference::{
        FinishReason, InferenceError, InferenceOptions, InferenceResponse, ToolCallRequest, Usage,
    };
    use crate::reasoning::policy_bridge::ToolFilterPolicyGate;
    use async_trait::async_trait;

    /// A scripted [`InferenceProvider`]: returns queued responses in order,
    /// making no network calls. Mirrors the `MockProvider` pattern used in
    /// `reasoning_loop.rs`'s own tests.
    struct ScriptedProvider {
        responses: std::sync::Mutex<std::collections::VecDeque<InferenceResponse>>,
        prompts: std::sync::Mutex<Vec<String>>,
        replace_configuration: Option<PathBuf>,
    }

    impl ScriptedProvider {
        fn new(responses: Vec<InferenceResponse>) -> Self {
            Self {
                responses: std::sync::Mutex::new(responses.into()),
                prompts: std::sync::Mutex::new(Vec::new()),
                replace_configuration: None,
            }
        }
    }

    #[async_trait]
    impl InferenceProvider for ScriptedProvider {
        async fn complete(
            &self,
            conversation: &Conversation,
            _options: &InferenceOptions,
        ) -> Result<InferenceResponse, InferenceError> {
            self.prompts.lock().unwrap().push(
                conversation
                    .messages()
                    .iter()
                    .map(|message| message.content.clone())
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            if let Some(path) = &self.replace_configuration {
                std::fs::write(path, "[sandbox]\ntier='firecracker'").unwrap();
            }
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| InferenceError::Provider("ScriptedProvider exhausted".into()))
        }

        fn provider_name(&self) -> &str {
            "scripted-test"
        }
        fn default_model(&self) -> &str {
            "scripted-test-model"
        }
        fn supports_native_tools(&self) -> bool {
            true
        }
        fn supports_structured_output(&self) -> bool {
            false
        }
    }

    fn tool_call_response(id: &str, name: &str, args: &serde_json::Value) -> InferenceResponse {
        InferenceResponse {
            content: String::new(),
            tool_calls: vec![ToolCallRequest {
                id: id.to_string(),
                name: name.to_string(),
                arguments: args.to_string(),
            }],
            finish_reason: FinishReason::ToolCalls,
            usage: Usage::default(),
            model: "scripted-test-model".into(),
        }
    }

    fn final_text_response(text: &str) -> InferenceResponse {
        InferenceResponse {
            content: text.to_string(),
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
            usage: Usage::default(),
            model: "scripted-test-model".into(),
        }
    }

    /// Write a minimal ToolClad manifest for a `write_marker` tool: running
    /// it creates a file at the given path via `touch`.
    fn write_marker_manifest(tools_dir: &std::path::Path) {
        std::fs::write(
            tools_dir.join("write_marker.clad.toml"),
            r#"
[tool]
name = "write_marker"
version = "1.0.0"
binary = "touch"
description = "create a marker file (test fixture)"

[args.path]
position = 1
required = true
type = "string"
description = "file path to touch"

[command]
template = "touch {path}"

[output]
format = "text"

[output.schema]
type = "object"
"#,
        )
        .unwrap();
    }

    async fn find_available_port() -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        port
    }

    async fn wait_for_port(port: u16) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_ok()
            {
                return;
            }
            if std::time::Instant::now() > deadline {
                panic!("server on port {} never came up", port);
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }

    fn test_config(port: u16) -> HttpInputConfig {
        HttpInputConfig {
            bind_address: "127.0.0.1".to_string(),
            port,
            path: "/webhook".to_string(),
            agent: AgentId::new(),
            auth_header: Some("Bearer test-token".to_string()),
            jwt_public_key_path: None,
            max_body_bytes: 65_536,
            concurrency: 5,
            routing_rules: None,
            response_control: None,
            forward_headers: vec![],
            cors_origins: vec![],
            audit_enabled: false,
            webhook_verify: None,
        }
    }

    /// A fail-closed gate must stop a proposed tool call from executing —
    /// proven by a real side effect (a file) not existing, not merely by an
    /// error string in the response. The denial must still be visible to
    /// the caller in `tool_runs`.
    #[tokio::test]
    async fn denied_tool_call_does_not_execute_and_denial_is_visible() {
        let tools_dir = tempfile::tempdir().unwrap();
        write_marker_manifest(tools_dir.path());
        let marker = tools_dir.path().join("marker.txt");

        let executor: Arc<dyn crate::reasoning::executor::ActionExecutor> = Arc::new(
            crate::toolclad::ToolCladExecutor::new(
                crate::toolclad::manifest::load_manifests_from_dir(tools_dir.path()),
            )
            .with_development_host_execution(),
        );
        assert!(
            executor
                .tool_definitions()
                .iter()
                .any(|d| d.name == "write_marker"),
            "fixture executor must advertise write_marker"
        );

        let provider = Arc::new(ScriptedProvider::new(vec![
            tool_call_response(
                "call_1",
                "write_marker",
                &serde_json::json!({ "path": marker.display().to_string() }),
            ),
            final_text_response("I was not able to run that tool."),
        ]));

        let port = find_available_port().await;
        let server = HttpInputServer::new(test_config(port))
            .with_project_root(tools_dir.path().to_path_buf())
            .with_executor(executor)
            .with_inference_provider(provider)
            // Fail-closed: no policies wired, no insecure-allow-all opt-in.
            .with_policy_gate(Arc::new(DefaultPolicyGate::new()));

        let handle = tokio::spawn(async move {
            let _ = server.start().await;
        });
        wait_for_port(port).await;

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("http://127.0.0.1:{}/webhook", port))
            .header("Authorization", "Bearer test-token")
            .header("Idempotency-Key", uuid::Uuid::new_v4().to_string())
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({ "prompt": "please run write_marker" }))
            .send()
            .await
            .expect("request");

        assert!(resp.status().is_success(), "status: {}", resp.status());
        let body: serde_json::Value = resp.json().await.expect("json body");

        // The whole point: the tool's real side effect must NOT have
        // happened, not merely "some error string appeared".
        assert!(
            !marker.exists(),
            "denied tool call must not execute — marker file should not exist"
        );

        let tool_runs = body["tool_runs"].as_array().expect("tool_runs array");
        assert_eq!(tool_runs.len(), 1, "tool_runs: {:?}", tool_runs);
        assert_eq!(tool_runs[0]["tool"], "write_marker");
        let preview = tool_runs[0]["output_preview"].as_str().unwrap();
        assert!(
            preview.contains("Policy denied"),
            "expected a policy-denial marker in tool_runs, got: {}",
            preview
        );

        handle.abort();
        let _ = handle.await;
    }

    /// An allowed tool call must actually execute (real side effect), and
    /// the response must still carry per-tool results.
    #[tokio::test]
    async fn allowed_tool_call_executes_and_response_carries_tool_results() {
        let tools_dir = tempfile::tempdir().unwrap();
        write_marker_manifest(tools_dir.path());
        let marker = tools_dir.path().join("marker.txt");

        let executor: Arc<dyn crate::reasoning::executor::ActionExecutor> = Arc::new(
            crate::toolclad::ToolCladExecutor::new(
                crate::toolclad::manifest::load_manifests_from_dir(tools_dir.path()),
            )
            .with_development_host_execution(),
        );

        let provider = Arc::new(ScriptedProvider::new(vec![
            tool_call_response(
                "call_1",
                "write_marker",
                &serde_json::json!({ "path": marker.display().to_string() }),
            ),
            final_text_response("Done."),
        ]));

        let port = find_available_port().await;
        let server = HttpInputServer::new(test_config(port))
            .with_project_root(tools_dir.path().to_path_buf())
            .with_executor(executor)
            .with_inference_provider(provider)
            .with_policy_gate(Arc::new(ToolFilterPolicyGate::allow_all()));

        let handle = tokio::spawn(async move {
            let _ = server.start().await;
        });
        wait_for_port(port).await;

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("http://127.0.0.1:{}/webhook", port))
            .header("Authorization", "Bearer test-token")
            .header("Idempotency-Key", uuid::Uuid::new_v4().to_string())
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({ "prompt": "please run write_marker" }))
            .send()
            .await
            .expect("request");

        assert!(resp.status().is_success(), "status: {}", resp.status());
        let body: serde_json::Value = resp.json().await.expect("json body");

        // The whole point: the tool's real side effect DID happen.
        assert!(
            marker.exists(),
            "allowed tool call must execute — marker file should exist"
        );

        let tool_runs = body["tool_runs"].as_array().expect("tool_runs array");
        assert_eq!(tool_runs.len(), 1, "tool_runs: {:?}", tool_runs);
        // ToolCladExecutor's own Observation::source naming convention
        // (`toolclad:<name>`) — see the field-fidelity note on
        // `reconstruct_tool_runs`.
        assert_eq!(tool_runs[0]["tool"], "toolclad:write_marker");
        let preview = tool_runs[0]["output_preview"].as_str().unwrap();
        assert!(
            !preview.contains("ToolClad error"),
            "expected a successful tool result, got: {}",
            preview
        );
        assert_eq!(body["response"], "Done.");

        handle.abort();
        let _ = handle.await;
    }
    fn registered_fixture(source: &str) -> crate::types::AgentConfig {
        crate::types::AgentConfig {
            id: AgentId::new(),
            name: "fixture".into(),
            dsl_source: source.into(),
            security_tier: crate::types::SecurityTier::Tier1,
            execution_mode: crate::types::ExecutionMode::Ephemeral,
            resource_limits: Default::default(),
            capabilities: vec![],
            policies: vec![],
            metadata: Default::default(),
            priority: Default::default(),
        }
    }

    #[test]
    fn registered_settings_reject_invalid_or_inconsistent_identity_configuration() {
        let root = tempfile::tempdir().unwrap();
        let mut agent = registered_fixture(r#"agent fixture() { with sandbox = "docker" {} }"#);
        assert!(registered_agent_executor(root.path(), &agent, None).is_ok());
        agent.security_tier = crate::types::SecurityTier::Tier2;
        assert!(registered_agent_executor(root.path(), &agent, None)
            .err()
            .unwrap()
            .contains("conflicts"));
        agent.dsl_source = "agent fixture() { with sandbox = }".into();
        assert!(registered_agent_executor(
            root.path(),
            &agent,
            Some(Arc::new(
                crate::reasoning::executor::UnavailableToolExecutor,
            ))
        )
        .is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    #[ignore = "requires Docker and the provisioned python:3.12-slim image"]
    async fn registered_http_agent_executes_with_its_frozen_docker_profile() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let effects = root.path().join("effects");
        std::fs::create_dir(&effects).unwrap();
        std::fs::set_permissions(&effects, std::fs::Permissions::from_mode(0o777)).unwrap();
        let tools = root.path().join("tools");
        std::fs::create_dir(&tools).unwrap();
        write_marker_manifest(&tools);
        let manifest_path = tools.join("write_marker.clad.toml");
        let manifest = std::fs::read_to_string(&manifest_path).unwrap();
        let command = r#"python3 -c 'import json,sys; from pathlib import Path; value={"memory":int(Path("/sys/fs/cgroup/memory.max").read_text()),"cpu":Path("/sys/fs/cgroup/cpu.max").read_text().strip()}; Path(sys.argv[1]).write_text(json.dumps(value)); print(json.dumps(value))' {path}"#;
        std::fs::write(
            &manifest_path,
            manifest
                .replace("binary = \"touch\"", "binary = \"python3\"")
                .replace(
                    "template = \"touch {path}\"",
                    &format!("template = {}", serde_json::to_string(command).unwrap()),
                ),
        )
        .unwrap();
        let config_path = root.path().join("symbiont.toml");
        std::fs::write(&config_path, format!(
            "[sandbox]\ntier='firecracker'\n[sandbox.docker]\nimage='python:3.12-slim'\nvolumes=['{}:/workspace:rw']", effects.display(),
        )).unwrap();
        let mut runtime_config = crate::RuntimeConfig::default();
        // Keep a separate invocation busy while HTTP performs actual work.
        runtime_config.scheduler.max_concurrent_agents = 0;
        runtime_config.context_manager.enable_persistence = false;
        runtime_config
            .context_manager
            .persistence_config
            .root_data_dir = root.path().join("state");
        struct BusyExecutor;
        #[async_trait]
        impl crate::scheduler::execution::ScheduledAgentExecutor for BusyExecutor {
            async fn execute(
                &self,
                task: &crate::scheduler::ScheduledTask,
                _: std::time::Duration,
                cancellation: tokio_util::sync::CancellationToken,
            ) -> crate::scheduler::task_manager::TaskCompletion {
                cancellation.cancelled().await;
                crate::scheduler::task_manager::TaskCompletion::new(
                    task,
                    crate::scheduler::task_manager::TaskStatus::Terminated,
                    None,
                )
            }
        }
        let mut runtime = crate::AgentRuntime::new(runtime_config).await.unwrap();
        runtime.scheduler = Arc::new(
            crate::scheduler::DefaultAgentScheduler::new_with_executor(
                crate::scheduler::SchedulerConfig {
                    max_concurrent_agents: 1,
                    ..Default::default()
                },
                None,
                Arc::new(BusyExecutor),
            )
            .await
            .unwrap(),
        );
        let runtime = Arc::new(runtime);
        let mut agent = registered_fixture(
            r#"agent sibling() { with sandbox = "firecracker" {} }
agent fixture() { with sandbox = "docker", timeout = 20.seconds {} }"#,
        );
        agent.resource_limits.memory_mb = 128;
        agent.resource_limits.cpu_cores = 0.25;
        agent.resource_limits.execution_timeout = std::time::Duration::from_secs(10);
        let agent_id = agent.id;
        let busy = runtime
            .scheduler
            .schedule_invocation(agent, serde_json::Value::Null)
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while runtime
                .scheduler
                .get_agent_status(agent_id)
                .await
                .unwrap()
                .state
                != crate::types::AgentState::Running
            {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let mut scripted = ScriptedProvider::new(vec![
            tool_call_response(
                "call_1",
                "write_marker",
                &serde_json::json!({"path":"/workspace/allowed"}),
            ),
            final_text_response("Done."),
        ]);
        scripted.replace_configuration = Some(config_path);
        let provider = Arc::new(scripted);
        let port = find_available_port().await;
        let mut config = test_config(port);
        config.agent = agent_id;
        let server = HttpInputServer::new(config)
            .with_project_root(root.path().to_path_buf())
            .with_runtime(runtime.clone())
            .with_inference_provider(provider.clone())
            .with_policy_gate(Arc::new(ToolFilterPolicyGate::allow_all()));
        let handle = tokio::spawn(async move { server.start().await });
        wait_for_port(port).await;
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(format!("http://127.0.0.1:{port}/webhook"))
            .header("Authorization", "Bearer test-token")
            .header("Idempotency-Key", uuid::Uuid::new_v4().to_string())
            .json(&serde_json::json!({"prompt":"create the allowed marker", "agent":"sibling"}))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        let body: Value = response.json().await.unwrap();
        handle.abort();
        let _ = handle.await;
        runtime.scheduler.shutdown().await.unwrap();
        assert!(effects.join("allowed").exists(), "{body}");
        let observed: Value =
            serde_json::from_str(&std::fs::read_to_string(effects.join("allowed")).unwrap())
                .unwrap();
        assert_eq!(observed["memory"], 128 * 1024 * 1024);
        let cpu: Vec<u64> = observed["cpu"]
            .as_str()
            .unwrap()
            .split_whitespace()
            .map(|part| part.parse().unwrap())
            .collect();
        assert_eq!(cpu.len(), 2);
        assert_eq!(cpu[0] * 4, cpu[1]);
        assert_eq!(body["response"], "Done.");
        assert_eq!(
            busy.wait().await.status,
            crate::scheduler::task_manager::TaskStatus::Terminated
        );
        let audit: crate::reasoning::run_audit::RunAuditReference =
            serde_json::from_value(body["audit"].clone()).unwrap();
        let key: [u8; 32] = hex::decode(&audit.public_key).unwrap().try_into().unwrap();
        let entries = crate::reasoning::protected_journal::ProtectedJournal::verify_run(
            &audit.path,
            &key,
            audit.run_id,
        )
        .unwrap();
        assert!(entries.iter().all(|entry| entry.agent_id == agent_id));
        assert!(matches!(
            entries.last().unwrap().event,
            crate::reasoning::loop_types::LoopEvent::Terminated {
                reason: TerminationReason::Completed,
                ..
            }
        ));
        let prompts = provider.prompts.lock().unwrap();
        assert_eq!(prompts.len(), 2);
        assert!(prompts.iter().all(
            |prompt| prompt.contains("agent fixture()") && !prompt.contains("agent sibling()")
        ));
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn http_audit_initialization_and_inference_failures_never_report_completion() {
        use std::os::unix::fs::PermissionsExt;
        for invalid_store in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let audit = root.path().join(".symbiont/governed");
            if invalid_store {
                std::fs::create_dir_all(&audit).unwrap();
                std::fs::set_permissions(&audit, std::fs::Permissions::from_mode(0o777)).unwrap();
            }
            let provider = Arc::new(ScriptedProvider::new(vec![]));
            let port = find_available_port().await;
            let server = HttpInputServer::new(test_config(port))
                .with_project_root(root.path().to_path_buf())
                .with_inference_provider(provider.clone())
                .with_executor(Arc::new(
                    crate::reasoning::executor::UnavailableToolExecutor,
                ));
            let handle = tokio::spawn(async move { server.start().await });
            wait_for_port(port).await;
            let response = reqwest::Client::builder()
                .no_proxy()
                .build()
                .unwrap()
                .post(format!("http://127.0.0.1:{port}/webhook"))
                .header("Authorization", "Bearer test-token")
                .header("Idempotency-Key", uuid::Uuid::new_v4().to_string())
                .json(&serde_json::json!({"prompt":"fixture"}))
                .send()
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                if invalid_store {
                    reqwest::StatusCode::SERVICE_UNAVAILABLE
                } else {
                    reqwest::StatusCode::UNPROCESSABLE_ENTITY
                }
            );
            let body: Value = response.json().await.unwrap();
            assert_eq!(
                body["status"],
                if invalid_store {
                    "unavailable"
                } else {
                    "failed"
                }
            );
            assert_eq!(
                provider.prompts.lock().unwrap().len(),
                usize::from(!invalid_store)
            );
            let journals: Vec<_> = std::fs::read_dir(&audit)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| {
                    path.extension()
                        .is_some_and(|extension| extension == "jsonl")
                })
                .collect();
            assert_eq!(journals.len(), usize::from(!invalid_store));
            if let Some(path) = journals.first() {
                let data = std::fs::read_to_string(path).unwrap();
                let last: Value = serde_json::from_str(data.lines().last().unwrap()).unwrap();
                assert!(
                    last["payload"]["entry"]["event"]["Terminated"]["reason"]["Error"].is_object()
                );
            }
            handle.abort();
            let _ = handle.await;
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dropped_http_invocation_retains_cleanup_and_a_signed_terminal_record() {
        struct WaitingProvider(Arc<tokio::sync::Notify>);
        #[async_trait]
        impl InferenceProvider for WaitingProvider {
            async fn complete(
                &self,
                _: &Conversation,
                _: &InferenceOptions,
            ) -> Result<InferenceResponse, InferenceError> {
                self.0.notify_one();
                std::future::pending().await
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
        let root = tempfile::tempdir().unwrap();
        let project = root.path().to_path_buf();
        let started = Arc::new(tokio::sync::Notify::new());
        let provider = Arc::new(WaitingProvider(started.clone()));
        let task = tokio::spawn(async move {
            invoke_agent(
                None,
                AgentId::new(),
                serde_json::json!({"prompt":"fixture"}),
                Some(provider),
                &project,
                Arc::new(crate::reasoning::executor::UnavailableToolExecutor),
                true,
                Arc::new(DefaultPolicyGate::new()),
                Arc::new(CircuitBreakerRegistry::default()),
                HttpInvocation::test_request(),
            )
            .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), started.notified())
            .await
            .unwrap();
        task.abort();
        let _ = task.await;
        let (journal_path, terminal) =
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let file = std::fs::read_dir(root.path().join(".symbiont/governed"))
                        .unwrap()
                        .map(|entry| entry.unwrap().path())
                        .find(|path| {
                            path.extension()
                                .is_some_and(|extension| extension == "jsonl")
                        })
                        .unwrap();
                    let data = std::fs::read_to_string(&file).unwrap();
                    let last = data
                        .lines()
                        .last()
                        .and_then(|line| serde_json::from_str::<Value>(line).ok());
                    if let Some(last) = last {
                        if last["payload"]["entry"]["event"]
                            .get("Terminated")
                            .is_some()
                        {
                            break (file, last);
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap();
        let key: [u8; 32] = std::fs::read(root.path().join(".symbiont/governed/audit-signing.key"))
            .unwrap()
            .try_into()
            .unwrap();
        let public = ed25519_dalek::SigningKey::from_bytes(&key)
            .verifying_key()
            .to_bytes();
        let run_id = serde_json::from_value(terminal["payload"]["run_id"].clone()).unwrap();
        crate::reasoning::protected_journal::ProtectedJournal::verify_run(
            &journal_path,
            &public,
            run_id,
        )
        .unwrap();
        assert_eq!(
            terminal["payload"]["entry"]["event"]["Terminated"]["reason"]["Error"]["message"],
            "agent execution cancelled"
        );
    }
    #[test]
    fn registered_http_does_not_reinterpret_other_execution_modes() {
        let root = tempfile::tempdir().unwrap();
        let mut agent = registered_fixture("agent fixture() { with sandbox = \"docker\" {} }");
        agent.execution_mode = crate::types::ExecutionMode::External {
            endpoint: None,
            agentpin_domain: None,
            heartbeat_interval_secs: 60,
        };
        assert!(registered_agent_executor(root.path(), &agent, None)
            .err()
            .unwrap()
            .contains("external agents"));
        agent.execution_mode = crate::types::ExecutionMode::Ephemeral;
        agent.dsl_source = "metadata { executor = \"claude_code\" } agent fixture() { with sandbox = \"docker\" {} }".into();
        assert!(registered_agent_executor(root.path(), &agent, None)
            .err()
            .unwrap()
            .contains("selected executor is unavailable"));
    }
    #[test]
    fn registered_http_rejects_invalid_resource_limits() {
        let root = tempfile::tempdir().unwrap();
        for kind in 0..3 {
            let mut agent = registered_fixture(r#"agent fixture() { with sandbox = "docker" {} }"#);
            match kind {
                0 => agent.resource_limits.memory_mb = 0,
                1 => agent.resource_limits.cpu_cores = f32::NAN,
                _ => agent.resource_limits.execution_timeout = std::time::Duration::ZERO,
            }
            assert!(registered_agent_executor(root.path(), &agent, None)
                .err()
                .unwrap()
                .contains("positive and finite"));
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn registered_http_deadline_bounds_inference_and_records_timeout() {
        struct SlowProvider;
        #[async_trait]
        impl InferenceProvider for SlowProvider {
            async fn complete(
                &self,
                _: &Conversation,
                _: &InferenceOptions,
            ) -> Result<InferenceResponse, InferenceError> {
                std::future::pending().await
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
        let root = tempfile::tempdir().unwrap();
        let mut config = crate::RuntimeConfig::default();
        config.context_manager.enable_persistence = false;
        config.context_manager.persistence_config.root_data_dir = root.path().join("state");
        let runtime = crate::AgentRuntime::new(config).await.unwrap();
        let mut agent = registered_fixture(
            r#"agent fixture() { with sandbox = "docker", timeout = 20.seconds {} }"#,
        );
        agent.resource_limits.execution_timeout = std::time::Duration::from_millis(200);
        let id = agent.id;
        runtime.scheduler.register_agent(agent).await.unwrap();
        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            invoke_agent(
                Some(&runtime),
                id,
                serde_json::json!({"prompt":"fixture"}),
                Some(Arc::new(SlowProvider)),
                root.path(),
                Arc::new(crate::reasoning::executor::UnavailableToolExecutor),
                true,
                Arc::new(DefaultPolicyGate::new()),
                Arc::new(CircuitBreakerRegistry::default()),
                HttpInvocation::test_request(),
            ),
        )
        .await
        .expect("registered deadline must end the inference call");
        assert_eq!(
            outcome
                .unwrap()
                .into_response(uuid::Uuid::new_v4(), None)
                .unwrap()
                .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        let journal = std::fs::read_dir(root.path().join(".symbiont/governed"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
            .unwrap();
        let key: [u8; 32] = std::fs::read(root.path().join(".symbiont/governed/audit-signing.key"))
            .unwrap()
            .try_into()
            .unwrap();
        let public = ed25519_dalek::SigningKey::from_bytes(&key)
            .verifying_key()
            .to_bytes();
        let entries =
            crate::reasoning::protected_journal::ProtectedJournal::verify(&journal, &public)
                .unwrap();
        assert!(matches!(
            entries.last().unwrap().event,
            crate::reasoning::loop_types::LoopEvent::Terminated {
                reason: TerminationReason::Timeout,
                ..
            }
        ));
        runtime.scheduler.shutdown().await.unwrap();
    }
}
