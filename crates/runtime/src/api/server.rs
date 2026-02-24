//! HTTP API server implementation
//!
//! This module provides the main HTTP server implementation using Axum.

#[cfg(feature = "http-api")]
use axum::{http::StatusCode, response::Json, Router};

#[cfg(feature = "http-api")]
use std::sync::Arc;

#[cfg(feature = "http-api")]
use std::time::Instant;

#[cfg(feature = "http-api")]
use tokio::net::TcpListener;

#[cfg(feature = "http-api")]
use tower_http::{cors::CorsLayer, trace::TraceLayer};

#[cfg(feature = "http-api")]
use utoipa::OpenApi;

#[cfg(feature = "http-api")]
use utoipa_swagger_ui::SwaggerUi;

#[cfg(feature = "http-api")]
use super::types::{
    AddIdentityMappingRequest, AgentExecutionRecord, AgentStatusResponse, ChannelActionResponse,
    ChannelAuditEntry, ChannelAuditResponse, ChannelDetail, ChannelHealthResponse, ChannelSummary,
    CreateAgentRequest, CreateAgentResponse, CreateScheduleRequest, CreateScheduleResponse,
    DeleteAgentResponse, DeleteChannelResponse, DeleteScheduleResponse, ErrorResponse,
    ExecuteAgentRequest, ExecuteAgentResponse, GetAgentHistoryResponse, HealthResponse,
    IdentityMappingEntry, NextRunsResponse, RegisterChannelRequest, RegisterChannelResponse,
    ResourceUsage, ScheduleActionResponse, ScheduleDetail, ScheduleHistoryResponse,
    ScheduleRunEntry, ScheduleSummary, SchedulerHealthResponse, UpdateAgentRequest,
    UpdateAgentResponse, UpdateChannelRequest, UpdateScheduleRequest, WorkflowExecutionRequest,
};

#[cfg(feature = "http-api")]
use super::traits::RuntimeApiProvider;

#[cfg(feature = "http-api")]
use crate::types::RuntimeError;

/// OpenAPI documentation structure
#[cfg(feature = "http-api")]
#[derive(OpenApi)]
#[openapi(
    paths(
        super::routes::execute_workflow,
        super::routes::get_agent_status,
        super::routes::list_agents,
        super::routes::get_metrics,
        super::routes::create_agent,
        super::routes::update_agent,
        super::routes::delete_agent,
        super::routes::execute_agent,
        super::routes::get_agent_history,
        super::routes::list_schedules,
        super::routes::create_schedule,
        super::routes::get_schedule,
        super::routes::update_schedule,
        super::routes::delete_schedule,
        super::routes::pause_schedule,
        super::routes::resume_schedule,
        super::routes::trigger_schedule,
        super::routes::get_schedule_history,
        super::routes::get_schedule_next_runs,
        super::routes::get_scheduler_health,
        super::routes::list_channels,
        super::routes::register_channel,
        super::routes::get_channel,
        super::routes::update_channel,
        super::routes::delete_channel,
        super::routes::start_channel,
        super::routes::stop_channel,
        super::routes::get_channel_health,
        super::routes::list_channel_mappings,
        super::routes::add_channel_mapping,
        super::routes::remove_channel_mapping,
        super::routes::get_channel_audit,
        health_check
    ),
    components(
        schemas(
            WorkflowExecutionRequest,
            AgentStatusResponse,
            ResourceUsage,
            HealthResponse,
            CreateAgentRequest,
            CreateAgentResponse,
            UpdateAgentRequest,
            UpdateAgentResponse,
            DeleteAgentResponse,
            ExecuteAgentRequest,
            ExecuteAgentResponse,
            GetAgentHistoryResponse,
            AgentExecutionRecord,
            ErrorResponse,
            CreateScheduleRequest,
            CreateScheduleResponse,
            UpdateScheduleRequest,
            ScheduleSummary,
            ScheduleDetail,
            NextRunsResponse,
            ScheduleRunEntry,
            ScheduleHistoryResponse,
            ScheduleActionResponse,
            DeleteScheduleResponse,
            SchedulerHealthResponse,
            RegisterChannelRequest,
            RegisterChannelResponse,
            UpdateChannelRequest,
            ChannelSummary,
            ChannelDetail,
            ChannelActionResponse,
            DeleteChannelResponse,
            ChannelHealthResponse,
            IdentityMappingEntry,
            AddIdentityMappingRequest,
            ChannelAuditEntry,
            ChannelAuditResponse
        )
    ),
    tags(
        (name = "agents", description = "Agent management endpoints"),
        (name = "workflows", description = "Workflow execution endpoints"),
        (name = "system", description = "System monitoring and health endpoints"),
        (name = "schedules", description = "Cron schedule management endpoints"),
        (name = "channels", description = "Channel adapter management endpoints")
    ),
    info(
        title = "Symbiont Runtime API",
        description = "HTTP API for the Symbiont Agent Runtime System",
        version = "1.0.0",
        contact(
            name = "ThirdKey.ai",
            url = "https://github.com/thirdkeyai/symbiont"
        ),
        license(
            name = "MIT",
            url = "https://opensource.org/licenses/MIT"
        )
    )
)]
pub struct ApiDoc;

/// HTTP API Server configuration
#[cfg(feature = "http-api")]
#[derive(Debug, Clone)]
pub struct HttpApiConfig {
    /// Server bind address
    pub bind_address: String,
    /// Server port
    pub port: u16,
    /// Enable CORS
    pub enable_cors: bool,
    /// Enable request tracing
    pub enable_tracing: bool,
    /// Enable per-IP rate limiting (100 req/min)
    pub enable_rate_limiting: bool,
    /// Optional path to API keys JSON file for per-agent authentication
    pub api_keys_file: Option<std::path::PathBuf>,
    /// Serve AGENTS.md at /agents.md and /.well-known/agents.md (auth-gated)
    pub serve_agents_md: bool,
}

#[cfg(feature = "http-api")]
impl Default for HttpApiConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1".to_string(),
            port: 8080,
            enable_cors: true,
            enable_tracing: true,
            enable_rate_limiting: true,
            api_keys_file: None,
            serve_agents_md: false,
        }
    }
}

/// HTTP API Server
#[cfg(feature = "http-api")]
pub struct HttpApiServer {
    config: HttpApiConfig,
    runtime_provider: Option<Arc<dyn RuntimeApiProvider>>,
    start_time: Instant,
    api_key_store: Option<Arc<super::api_keys::ApiKeyStore>>,
}

#[cfg(feature = "http-api")]
impl HttpApiServer {
    /// Create a new HTTP API server instance
    pub fn new(config: HttpApiConfig) -> Self {
        Self {
            config,
            runtime_provider: None,
            start_time: Instant::now(),
            api_key_store: None,
        }
    }

    /// Set the runtime provider for the API server
    pub fn with_runtime_provider(mut self, provider: Arc<dyn RuntimeApiProvider>) -> Self {
        self.runtime_provider = Some(provider);
        self
    }

    /// Start the HTTP API server
    pub async fn start(&mut self) -> Result<(), RuntimeError> {
        // Initialize trusted proxy configuration from SYMBIONT_TRUSTED_PROXIES
        super::middleware::init_trusted_proxies();

        // Load API key store if configured
        if let Some(ref keys_path) = self.config.api_keys_file {
            match super::api_keys::ApiKeyStore::load_from_file(keys_path) {
                Ok(store) => {
                    tracing::info!("Loaded API key store from {}", keys_path.display());
                    self.api_key_store = Some(Arc::new(store));
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to load API key store from {}: {} — falling back to legacy auth",
                        keys_path.display(),
                        e
                    );
                }
            }
        }

        let app = self.create_router();

        let addr = format!("{}:{}", self.config.bind_address, self.config.port);
        let listener = TcpListener::bind(&addr)
            .await
            .map_err(|e| RuntimeError::Internal(format!("Failed to bind to {}: {}", addr, e)))?;

        // Warn about authentication configuration
        let has_key_store = self.api_key_store.as_ref().is_some_and(|s| s.has_records());
        let has_env_token = std::env::var("SYMBIONT_API_TOKEN").is_ok();

        if has_key_store && has_env_token {
            tracing::warn!(
                "API key store is configured — SYMBIONT_API_TOKEN will be ignored. \
                 Remove the env var to avoid confusion."
            );
        } else if !has_key_store && has_env_token {
            tracing::warn!(
                "Using legacy SYMBIONT_API_TOKEN for authentication. \
                 Migrate to an API key store (--api-keys-file) for production."
            );
        } else if !has_key_store && !has_env_token {
            tracing::error!(
                "No API key store and no SYMBIONT_API_TOKEN — \
                 all authenticated endpoints will reject requests."
            );
        }

        tracing::info!("HTTP API server starting on {}", addr);

        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .map_err(|e| RuntimeError::Internal(format!("Server error: {}", e)))?;

        Ok(())
    }

    /// Create the Axum router with all routes and middleware
    fn create_router(&self) -> Router {
        use axum::routing::{delete, get, post, put};

        let mut router = Router::new()
            .route("/api/v1/health", get(health_check))
            .with_state(self.start_time);

        // Add Swagger UI
        router = router
            .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()));

        // Add stateful routes if we have a runtime provider
        if let Some(provider) = &self.runtime_provider {
            use super::middleware::auth_middleware;
            use super::routes::{
                add_channel_mapping, create_agent, create_schedule, delete_agent, delete_channel,
                delete_schedule, execute_agent, execute_workflow, get_agent_history,
                get_agent_status, get_channel, get_channel_audit, get_channel_health, get_metrics,
                get_schedule, get_schedule_history, get_schedule_next_runs, get_scheduler_health,
                list_agents, list_channel_mappings, list_channels, list_schedules, pause_schedule,
                register_channel, remove_channel_mapping, resume_schedule, start_channel,
                stop_channel, trigger_schedule, update_agent, update_channel, update_schedule,
            };
            use axum::middleware;

            // Agent routes that require authentication
            let agent_router = Router::new()
                .route("/api/v1/agents", get(list_agents).post(create_agent))
                .route("/api/v1/agents/:id/status", get(get_agent_status))
                .route("/api/v1/agents/:id", put(update_agent).delete(delete_agent))
                .route("/api/v1/agents/:id/execute", post(execute_agent))
                .route("/api/v1/agents/:id/history", get(get_agent_history))
                .layer(middleware::from_fn(auth_middleware))
                .with_state(provider.clone());

            // Schedule routes
            let schedule_router = Router::new()
                .route(
                    "/api/v1/schedules",
                    get(list_schedules).post(create_schedule),
                )
                .route(
                    "/api/v1/schedules/:id",
                    get(get_schedule)
                        .put(update_schedule)
                        .delete(delete_schedule),
                )
                .route("/api/v1/schedules/:id/pause", post(pause_schedule))
                .route("/api/v1/schedules/:id/resume", post(resume_schedule))
                .route("/api/v1/schedules/:id/trigger", post(trigger_schedule))
                .route("/api/v1/schedules/:id/history", get(get_schedule_history))
                .route(
                    "/api/v1/schedules/:id/next-runs",
                    get(get_schedule_next_runs),
                )
                .layer(middleware::from_fn(auth_middleware))
                .with_state(provider.clone());

            // Channel routes
            let channel_router = Router::new()
                .route(
                    "/api/v1/channels",
                    get(list_channels).post(register_channel),
                )
                .route(
                    "/api/v1/channels/:id",
                    get(get_channel).put(update_channel).delete(delete_channel),
                )
                .route("/api/v1/channels/:id/start", post(start_channel))
                .route("/api/v1/channels/:id/stop", post(stop_channel))
                .route("/api/v1/channels/:id/health", get(get_channel_health))
                .route(
                    "/api/v1/channels/:id/mappings",
                    get(list_channel_mappings).post(add_channel_mapping),
                )
                .route(
                    "/api/v1/channels/:id/mappings/:user_id",
                    delete(remove_channel_mapping),
                )
                .route("/api/v1/channels/:id/audit", get(get_channel_audit))
                .layer(middleware::from_fn(auth_middleware))
                .with_state(provider.clone());

            // Protected routes (workflows + metrics) with authentication
            let protected_router = Router::new()
                .route("/api/v1/workflows/execute", post(execute_workflow))
                .route("/api/v1/metrics", get(get_metrics))
                .layer(middleware::from_fn(auth_middleware))
                .with_state(provider.clone());

            // Health routes — no auth so load-balancer probes work without credentials
            let health_router = Router::new()
                .route("/api/v1/health/scheduler", get(get_scheduler_health))
                .with_state(provider.clone());

            router = router
                .merge(agent_router)
                .merge(schedule_router)
                .merge(channel_router)
                .merge(protected_router)
                .merge(health_router);
        }

        // Conditionally serve AGENTS.md at well-known paths (auth-gated, no provider state needed)
        if self.config.serve_agents_md {
            use super::middleware::auth_middleware;
            use axum::middleware;

            let agents_md_router = Router::new()
                .route("/agents.md", get(super::routes::serve_agents_md))
                .route(
                    "/.well-known/agents.md",
                    get(super::routes::serve_agents_md),
                )
                .layer(middleware::from_fn(auth_middleware));
            router = router.merge(agents_md_router);
        }

        // Add API key store as extension if available
        if let Some(ref store) = self.api_key_store {
            router = router.layer(axum::Extension(store.clone()));
        }

        // Add middleware conditionally
        if self.config.enable_tracing {
            router = router.layer(TraceLayer::new_for_http());
        }

        if self.config.enable_cors {
            use axum::http::{header, HeaderValue, Method};

            let allowed_origins: Vec<HeaderValue> = std::env::var("SYMBIONT_CORS_ORIGINS")
                .unwrap_or_else(|_| "http://localhost:3001,http://localhost:3000".to_string())
                .split(',')
                .filter_map(|origin| origin.trim().parse().ok())
                .collect();

            let cors = CorsLayer::new()
                .allow_origin(allowed_origins)
                .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
                .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
                .allow_credentials(false);

            router = router.layer(cors);
        }

        // Apply per-IP rate limiting
        if self.config.enable_rate_limiting {
            router = router.layer(axum::middleware::from_fn(
                crate::api::middleware::rate_limit_middleware,
            ));
        }

        // Apply security headers to all responses
        router = router.layer(axum::middleware::from_fn(
            crate::api::middleware::security_headers_middleware,
        ));

        router
    }
}

/// Health check endpoint handler
#[cfg(feature = "http-api")]
#[utoipa::path(
    get,
    path = "/api/v1/health",
    responses(
        (status = 200, description = "Health check successful", body = HealthResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "system"
)]
async fn health_check(
    axum::extract::State(start_time): axum::extract::State<Instant>,
) -> Result<Json<HealthResponse>, (StatusCode, Json<ErrorResponse>)> {
    let uptime_seconds = start_time.elapsed().as_secs();

    let response = HealthResponse {
        status: "healthy".to_string(),
        uptime_seconds,
        timestamp: chrono::Utc::now(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    };

    Ok(Json(response))
}
