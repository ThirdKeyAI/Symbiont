//! HTTP Input server implementation
//!
//! This module provides the HTTP input server that receives webhook/HTTP requests
//! and routes them to appropriate Symbiont agents based on configuration rules.

#[cfg(feature = "http-input")]
use std::path::Path;
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
use crate::reasoning::loop_types::{BufferedJournal, JournalWriter, LoopConfig};
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
            runtime: None,
            secret_store: None,
            executor: None,
            inference_provider: None,
            policy_gate: None,
            concurrency_limiter,
            resolved_auth_header: Arc::new(RwLock::new(None)),
        }
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
    /// path (the runtime communication bus path is unaffected).
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
                // Assume raw DER-encoded Ed25519 public key
                jsonwebtoken::DecodingKey::from_ed_der(&key_bytes)
            };

            tracing::info!(path = %key_path, "Loaded JWT EdDSA public key for Bearer token validation");
            Some(Arc::new(decoding_key))
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
        let executor: Arc<dyn ActionExecutor> = self
            .executor
            .clone()
            .unwrap_or_else(|| build_tool_executor(Path::new("tools")));
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

        // Circuit breakers and the journal are shared across every request
        // this server instance handles, so a tool that keeps failing trips
        // its breaker cluster-wide instead of resetting on the next request.
        let circuit_breakers = Arc::new(CircuitBreakerRegistry::default());
        let journal: Arc<dyn JournalWriter> = Arc::new(BufferedJournal::new(1000));

        // Scan agents/ directory for DSL files
        let agent_dsl_sources = scan_agent_dsl_files();
        if !agent_dsl_sources.is_empty() {
            tracing::info!(
                "Loaded {} agent DSL file(s) for LLM context",
                agent_dsl_sources.len()
            );
        }

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
            runtime: self.runtime.clone(),
            concurrency_limiter: self.concurrency_limiter.clone(),
            resolved_auth_header: self.resolved_auth_header.clone(),
            inference_provider,
            agent_dsl_sources: Arc::new(agent_dsl_sources),
            executor,
            policy_gate,
            circuit_breakers,
            journal,
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
                .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
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
    runtime: Option<Arc<crate::AgentRuntime>>,
    concurrency_limiter: Arc<Semaphore>,
    resolved_auth_header: Arc<RwLock<Option<String>>>,
    /// Inference provider driving the governed reasoning loop. `None`
    /// disables the LLM/tool-calling path (the runtime communication bus
    /// path is unaffected).
    inference_provider: Option<Arc<dyn InferenceProvider>>,
    /// Agent DSL sources: (filename, content)
    agent_dsl_sources: Arc<Vec<(String, String)>>,
    /// Tool executor dispatching model-proposed tool calls.
    executor: Arc<dyn ActionExecutor>,
    /// Policy gate every model-proposed action must pass through before
    /// dispatch. Always present: `HttpInputServer::start()` resolves an
    /// unset gate to fail-closed (`DefaultPolicyGate::new()`), never
    /// permissive.
    policy_gate: Arc<dyn ReasoningPolicyGate>,
    /// Circuit breaker registry shared across every request this server
    /// instance handles.
    circuit_breakers: Arc<CircuitBreakerRegistry>,
    /// Journal of governed reasoning-loop events, shared across requests.
    journal: Arc<dyn JournalWriter>,
    /// Optional webhook signature verifier
    webhook_verifier: Option<Arc<dyn super::webhook_verify::SignatureVerifier>>,
    /// Optional JWT EdDSA verifying key for Bearer token validation
    jwt_decoding_key: Option<Arc<jsonwebtoken::DecodingKey>>,
}

/// JWT claims structure for EdDSA token validation
#[cfg(feature = "http-input")]
#[derive(serde::Deserialize)]
struct JwtClaims {
    /// Expiration time (validated automatically by jsonwebtoken)
    #[allow(dead_code)]
    exp: u64,
}

/// Authentication middleware
///
/// Auth flow:
/// 1. Try static `auth_header` match (constant-time comparison) — if it matches, allow through
/// 2. Try JWT: extract Bearer token, verify Ed25519 signature + `exp` expiration
/// 3. If neither method validates AND at least one is configured, return 401
/// 4. If no auth is configured at all, allow through (startup warning handles this)
#[cfg(feature = "http-input")]
async fn auth_middleware(
    State(state): State<ServerState>,
    headers: HeaderMap,
    req: axum::extract::Request,
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
                return Ok(next.run(req).await);
            }
        }
    }

    // 2. Try JWT Bearer token validation
    if let Some(ref decoding_key) = state.jwt_decoding_key {
        if let Some(provided_auth) = auth_header {
            if let Some(token) = provided_auth.strip_prefix("Bearer ") {
                // SECURITY (C4 / RUSTSEC-2023-0071): this is the asymmetric
                // verifier path. We accept ES256 / EdDSA only and reject
                // RSA-family, HMAC, and `none` algorithms explicitly to
                // neutralise the Marvin Attack reachability through
                // `jsonwebtoken`'s transitive `rsa` dependency.
                let header = match jsonwebtoken::decode_header(token) {
                    Ok(h) => h,
                    Err(e) => {
                        tracing::warn!(error = %e, "JWT header decode failed");
                        return Err(StatusCode::UNAUTHORIZED);
                    }
                };
                if !matches!(
                    header.alg,
                    jsonwebtoken::Algorithm::ES256 | jsonwebtoken::Algorithm::EdDSA
                ) {
                    tracing::warn!(
                        algorithm = ?header.alg,
                        "JWT algorithm not allowed on asymmetric Bearer path \
                         (expected ES256 or EdDSA)"
                    );
                    return Err(StatusCode::UNAUTHORIZED);
                }

                let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::EdDSA);
                // Pin the algorithm allowlist explicitly. `Validation::new`
                // only sets a single algorithm; assigning to `algorithms`
                // overrides any later additions and prevents alg confusion.
                validation.algorithms = vec![
                    jsonwebtoken::Algorithm::ES256,
                    jsonwebtoken::Algorithm::EdDSA,
                ];
                // Require exp claim; do not enforce audience/issuer
                validation.set_required_spec_claims(&["exp"]);
                validation.validate_aud = false;
                // 5 s clock-skew tolerance. Larger windows widen the
                // replay surface for captured tokens; 30 s was excessive.
                validation.leeway = 5;

                match jsonwebtoken::decode::<JwtClaims>(token, decoding_key, &validation) {
                    Ok(_token_data) => {
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

    // Invoke agent
    match invoke_agent(
        state.runtime.as_deref(),
        agent_id,
        payload,
        state.inference_provider.clone(),
        &state.agent_dsl_sources,
        state.executor.clone(),
        state.policy_gate.clone(),
        state.circuit_breakers.clone(),
        state.journal.clone(),
    )
    .await
    {
        Ok(result) => {
            let response_config = config.response_control.as_ref();
            format_success_response(result, response_config)
        }
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

/// Invoke an agent with the provided input data, using runtime execution or a
/// governed reasoning loop.
///
/// When the runtime communication bus can't dispatch (the agent isn't
/// registered as `Running`), this drives `ReasoningLoopRunner::run` directly:
/// every model-proposed tool call passes through `policy_gate` before
/// `executor` ever sees it, tool dispatch goes through the same
/// circuit-breaker-aware `ActionExecutor` other entry points use, and the run
/// is recorded to `journal`. There is no path here that acts on model output
/// without going through the gate.
#[cfg(feature = "http-input")]
#[allow(clippy::too_many_arguments)]
async fn invoke_agent(
    runtime: Option<&crate::AgentRuntime>,
    agent_id: AgentId,
    input_data: Value,
    inference_provider: Option<Arc<dyn InferenceProvider>>,
    agent_dsl_sources: &[(String, String)],
    executor: Arc<dyn ActionExecutor>,
    policy_gate: Arc<dyn ReasoningPolicyGate>,
    circuit_breakers: Arc<CircuitBreakerRegistry>,
    journal: Arc<dyn JournalWriter>,
) -> Result<Value, RuntimeError> {
    let start = std::time::Instant::now();

    // Try runtime communication bus for agents that are actively listening.
    // The communication bus delivers messages to registered (running) agents.
    // If the agent is not registered (e.g., completed or never started as a
    // persistent listener), fall through to the governed reasoning-loop path
    // which executes the agent on-demand with DSL context and tools.
    if let Some(rt) = runtime {
        // Check if the agent is actively running before attempting communication
        // bus delivery. send_message accepts messages even for unregistered agents
        // (returning Ok), but delivery fails asynchronously — causing a false
        // "started" response. Verify the agent is Running first.
        let is_running = match rt.scheduler.get_agent_status(agent_id).await {
            Ok(status) => status.state == crate::types::AgentState::Running,
            Err(_) => false,
        };

        if is_running {
            tracing::info!(
                "Agent {} is running, dispatching via communication bus",
                agent_id
            );
            let payload_data: bytes::Bytes = serde_json::to_vec(&input_data)
                .map_err(|e| RuntimeError::Internal(e.to_string()))?
                .into();
            let message = rt.communication.create_internal_message(
                rt.system_agent_id,
                agent_id,
                payload_data,
                crate::types::MessageType::Direct(agent_id),
                std::time::Duration::from_secs(300),
            );
            match rt
                .communication
                .send_message(message)
                .await
                .map_err(RuntimeError::Communication)
            {
                Ok(message_id) => {
                    let latency = start.elapsed();
                    tracing::info!(
                        "Runtime execution dispatched for agent {}: message_id={} latency={:?}",
                        agent_id,
                        message_id,
                        latency,
                    );
                    return Ok(serde_json::json!({
                        "status": "execution_started",
                        "agent_id": agent_id.to_string(),
                        "message_id": message_id.to_string(),
                        "latency_ms": latency.as_millis(),
                        "timestamp": chrono::Utc::now().to_rfc3339()
                    }));
                }
                Err(e) => {
                    tracing::warn!(
                        "Runtime execution failed for agent {}, falling back to the reasoning loop: {}",
                        agent_id,
                        e,
                    );
                    // Fall through to the reasoning-loop path
                }
            }
        } else {
            tracing::info!(
                "Agent {} is not running, using the governed reasoning-loop path",
                agent_id,
            );
        }
    }

    // Fall back to the governed reasoning loop.
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

    if !agent_dsl_sources.is_empty() {
        system_parts.push("You are an AI agent operating within the Symbiont runtime. Your behavior is governed by the following agent definitions:".to_string());
        for (filename, content) in agent_dsl_sources {
            system_parts.push(format!("\n--- {} ---\n{}", filename, content));
        }
        system_parts.push("\nFollow the capabilities and policies defined above. When tools are available, USE THEM to execute your tasks — do not just describe what you would do. Call tools to perform actual scans, lookups, and operations.".to_string());
    } else {
        system_parts.push(
            "You are an AI agent operating within the Symbiont runtime. Provide thorough, professional analysis based on the input provided.".to_string()
        );
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
    let loop_config = LoopConfig {
        max_iterations: 15,
        tool_timeout: std::time::Duration::from_secs(120),
        ..LoopConfig::default()
    };

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

    let result = runner.run(agent_id, conversation, loop_config).await;

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

    Ok(serde_json::json!({
        "status": "completed",
        "agent_id": agent_id.to_string(),
        "response": result.output,
        "tool_runs": tool_runs,
        "termination_reason": result.termination_reason,
        "iterations": result.iterations,
        "model": provider.default_model(),
        "provider": provider.provider_name(),
        "latency_ms": latency.as_millis(),
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
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

/// Scan the agents/ directory for .dsl files and return their contents
#[cfg(feature = "http-input")]
fn scan_agent_dsl_files() -> Vec<(String, String)> {
    let agents_dir = std::path::Path::new("agents");
    let mut sources = Vec::new();

    if !agents_dir.exists() || !agents_dir.is_dir() {
        return sources;
    }

    if let Ok(entries) = std::fs::read_dir(agents_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if dsl::is_symbi_file(&path) {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let filename = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    sources.push((filename, content));
                }
            }
        }
    }

    sources
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

    // Load ToolClad manifests and create executor for tool-calling. Also
    // loads `toolclad.toml` custom argument types — `build_tool_executor`'s
    // generic default (used by `HttpInputServer::start()` when no executor
    // was set via `with_executor`) has no project-root context to find that
    // file, so it is preserved here to keep this production entrypoint at
    // full parity with custom-typed manifests.
    let tools_dir = std::path::Path::new("tools");
    if tools_dir.is_dir() {
        let manifests = crate::toolclad::manifest::load_manifests_from_dir(tools_dir);
        if !manifests.is_empty() {
            // load_custom_types expects the project root (joins "toolclad.toml" internally)
            let project_root = std::path::Path::new(".");
            let custom_types = crate::toolclad::manifest::load_custom_types(project_root);
            let executor = crate::toolclad::executor::ToolCladExecutor::with_custom_types(
                manifests.clone(),
                custom_types,
            );
            tracing::info!(
                "HTTP Input: ToolClad executor loaded with {} tool(s)",
                manifests.len()
            );
            server = server.with_executor(Arc::new(executor));
        }
    }

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
    fn test_jwt_bearer_validation_allowlist_excludes_rsa_and_hmac() {
        // Reconstruct the same Validation the auth middleware builds.
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::EdDSA);
        validation.algorithms = vec![
            jsonwebtoken::Algorithm::ES256,
            jsonwebtoken::Algorithm::EdDSA,
        ];

        assert!(validation
            .algorithms
            .contains(&jsonwebtoken::Algorithm::ES256));
        assert!(validation
            .algorithms
            .contains(&jsonwebtoken::Algorithm::EdDSA));

        for forbidden in [
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
    }

    impl ScriptedProvider {
        fn new(responses: Vec<InferenceResponse>) -> Self {
            Self {
                responses: std::sync::Mutex::new(responses.into()),
            }
        }
    }

    #[async_trait]
    impl InferenceProvider for ScriptedProvider {
        async fn complete(
            &self,
            _conversation: &Conversation,
            _options: &InferenceOptions,
        ) -> Result<InferenceResponse, InferenceError> {
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

        let executor = build_tool_executor(tools_dir.path());
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

        let executor = build_tool_executor(tools_dir.path());

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
}
