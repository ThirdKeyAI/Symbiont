//! HTTP Input server implementation
//!
//! This module provides the HTTP input server that receives webhook/HTTP requests
//! and routes them to appropriate Symbiont agents based on configuration rules.

#[cfg(feature = "http-input")]
use std::sync::Arc;

#[cfg(feature = "http-input")]
use axum::{
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
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
use super::llm_client::LlmClient;
#[cfg(feature = "http-input")]
use crate::secrets::{new_secret_store, SecretStore, SecretsConfig};
#[cfg(feature = "http-input")]
use crate::types::{AgentId, RuntimeError};

/// HTTP Input Server that handles incoming webhook requests
#[cfg(feature = "http-input")]
pub struct HttpInputServer {
    config: Arc<RwLock<HttpInputConfig>>,
    runtime: Option<Arc<crate::AgentRuntime>>,
    secret_store: Option<Arc<dyn SecretStore + Send + Sync>>,
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
            concurrency_limiter,
            resolved_auth_header: Arc::new(RwLock::new(None)),
        }
    }

    /// Set the runtime for agent invocation
    pub fn with_runtime(mut self, runtime: Arc<crate::AgentRuntime>) -> Self {
        self.runtime = Some(runtime);
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

        // Resolve auth header if it's a secret reference
        if let Some(auth_header) = &config.auth_header {
            if let Some(secret_store) = &self.secret_store {
                let resolved = resolve_secret_reference(secret_store.as_ref(), auth_header).await?;
                *self.resolved_auth_header.write().await = Some(resolved);
            } else {
                *self.resolved_auth_header.write().await = Some(auth_header.clone());
            }
        }

        // Initialize LLM client from environment
        let llm_client = LlmClient::from_env().map(Arc::new);

        // Scan agents/ directory for DSL files
        let agent_dsl_sources = scan_agent_dsl_files();
        if !agent_dsl_sources.is_empty() {
            tracing::info!(
                "Loaded {} agent DSL file(s) for LLM context",
                agent_dsl_sources.len()
            );
        }

        // Create shared server state
        let server_state = ServerState {
            config: self.config.clone(),
            runtime: self.runtime.clone(),
            concurrency_limiter: self.concurrency_limiter.clone(),
            resolved_auth_header: self.resolved_auth_header.clone(),
            llm_client,
            agent_dsl_sources: Arc::new(agent_dsl_sources),
        };

        // Build the router
        let mut app = Router::new();

        // Add the webhook endpoint
        let path = config.path.clone();
        app = app.route(&path, post(webhook_handler));

        // Add middleware
        app = app.layer(middleware::from_fn_with_state(
            server_state.clone(),
            auth_middleware,
        ));

        // Add body size limit
        app = app.layer(DefaultBodyLimit::max(config.max_body_bytes));

        // Add CORS if enabled
        if config.cors_enabled {
            app = app.layer(CorsLayer::permissive());
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

/// Shared state for the HTTP server
#[cfg(feature = "http-input")]
#[derive(Clone)]
struct ServerState {
    config: Arc<RwLock<HttpInputConfig>>,
    runtime: Option<Arc<crate::AgentRuntime>>,
    concurrency_limiter: Arc<Semaphore>,
    resolved_auth_header: Arc<RwLock<Option<String>>>,
    llm_client: Option<Arc<LlmClient>>,
    /// Agent DSL sources: (filename, content)
    agent_dsl_sources: Arc<Vec<(String, String)>>,
}

/// Authentication middleware
#[cfg(feature = "http-input")]
async fn auth_middleware(
    State(state): State<ServerState>,
    headers: HeaderMap,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<Response, StatusCode> {
    // Check if authentication is required
    let resolved_auth = state.resolved_auth_header.read().await;
    if let Some(expected_auth) = resolved_auth.as_ref() {
        // Extract Authorization header
        let auth_header = headers.get("Authorization").and_then(|h| h.to_str().ok());

        match auth_header {
            Some(provided_auth) => {
                if provided_auth != expected_auth {
                    tracing::warn!("Authentication failed: invalid authorization header");
                    return Err(StatusCode::UNAUTHORIZED);
                }
            }
            None => {
                tracing::warn!("Authentication failed: missing authorization header");
                return Err(StatusCode::UNAUTHORIZED);
            }
        }
    }

    Ok(next.run(req).await)
}

/// Main webhook handler
#[cfg(feature = "http-input")]
async fn webhook_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Result<Response, StatusCode> {
    // Check concurrency limits
    let _permit = state.concurrency_limiter.try_acquire().map_err(|_| {
        tracing::warn!("Concurrency limit exceeded");
        StatusCode::TOO_MANY_REQUESTS
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
    let agent_id = route_request(&config, &payload, &headers).await;

    // Invoke agent
    match invoke_agent(
        state.runtime.as_deref(),
        agent_id,
        payload,
        state.llm_client.as_deref(),
        &state.agent_dsl_sources,
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
async fn route_request(config: &HttpInputConfig, payload: &Value, headers: &HeaderMap) -> AgentId {
    // Check routing rules if configured
    if let Some(routing_rules) = &config.routing_rules {
        for rule in routing_rules {
            if matches_route_condition(&rule.condition, payload, headers).await {
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
    payload: &Value,
    headers: &HeaderMap,
) -> bool {
    match condition {
        RouteMatch::PathPrefix(_path) => {
            // Path matching would be handled at router level
            false
        }
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

/// Invoke an agent with the provided input data, using LLM if available
#[cfg(feature = "http-input")]
async fn invoke_agent(
    _runtime: Option<&crate::AgentRuntime>,
    agent_id: AgentId,
    input_data: Value,
    llm_client: Option<&LlmClient>,
    agent_dsl_sources: &[(String, String)],
) -> Result<Value, RuntimeError> {
    let start = std::time::Instant::now();

    // If no LLM client is available, fall back to stub behavior
    let llm = match llm_client {
        Some(client) => client,
        None => {
            tracing::info!(
                "No LLM client available, returning stub response for agent {}",
                agent_id
            );
            return Ok(serde_json::json!({
                "status": "invoked",
                "agent_id": agent_id.to_string(),
                "message": "No LLM provider configured. Set OPENROUTER_API_KEY, OPENAI_API_KEY, or ANTHROPIC_API_KEY.",
                "timestamp": chrono::Utc::now().to_rfc3339()
            }));
        }
    };

    // Build system prompt from DSL sources
    let mut system_parts: Vec<String> = Vec::new();

    // Include agent DSL context if available
    if !agent_dsl_sources.is_empty() {
        system_parts.push("You are an AI agent operating within the Symbiont runtime. Your behavior is governed by the following agent definitions:".to_string());
        for (filename, content) in agent_dsl_sources {
            system_parts.push(format!("\n--- {} ---\n{}", filename, content));
        }
        system_parts.push("\nFollow the capabilities and policies defined above. Provide thorough, professional analysis.".to_string());
    } else {
        system_parts.push(
            "You are an AI agent operating within the Symbiont runtime. Provide thorough, professional analysis based on the input provided.".to_string()
        );
    }

    // Allow caller to prepend a custom system prompt
    if let Some(custom_system) = input_data.get("system_prompt").and_then(|v| v.as_str()) {
        system_parts.push(format!("\n{}", custom_system));
    }

    let system_prompt = system_parts.join("\n");

    // Build user message from payload
    let user_message = if let Some(prompt) = input_data.get("prompt").and_then(|v| v.as_str()) {
        prompt.to_string()
    } else {
        // Serialize entire payload as context
        let payload_str =
            serde_json::to_string_pretty(&input_data).unwrap_or_else(|_| input_data.to_string());
        format!(
            "Analyze the following data and provide a comprehensive report:\n\n{}",
            payload_str
        )
    };

    tracing::info!(
        "Invoking LLM for agent {}: provider={} model={} system_len={} user_len={}",
        agent_id,
        llm.provider(),
        llm.model(),
        system_prompt.len(),
        user_message.len(),
    );

    // Call LLM
    let response_text = llm.chat_completion(&system_prompt, &user_message).await?;
    let latency = start.elapsed();

    tracing::info!(
        "LLM invocation completed for agent {}: latency={:?} response_len={}",
        agent_id,
        latency,
        response_text.len(),
    );

    Ok(serde_json::json!({
        "status": "completed",
        "agent_id": agent_id.to_string(),
        "response": response_text,
        "model": llm.model(),
        "provider": format!("{}", llm.provider()),
        "latency_ms": latency.as_millis(),
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
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
            if path.extension().is_some_and(|ext| ext == "dsl") {
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

    let error_body = serde_json::json!({
        "error": error.to_string(),
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

        Ok(secret.value)
    } else {
        // Not a secret reference, return as-is
        Ok(reference.to_string())
    }
}

/// Create a function to start the HTTP input server
#[cfg(feature = "http-input")]
pub async fn start_http_input(
    config: HttpInputConfig,
    runtime: Option<Arc<crate::AgentRuntime>>,
    secrets_config: Option<SecretsConfig>,
) -> Result<(), RuntimeError> {
    let mut server = HttpInputServer::new(config);

    // Add runtime if provided
    if let Some(runtime) = runtime {
        server = server.with_runtime(runtime);
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
