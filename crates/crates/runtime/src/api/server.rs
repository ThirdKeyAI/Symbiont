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
use super::types::{ErrorResponse, HealthResponse, WorkflowExecutionRequest, AgentStatusResponse, CreateAgentRequest, CreateAgentResponse, UpdateAgentRequest, UpdateAgentResponse, DeleteAgentResponse, ExecuteAgentRequest, ExecuteAgentResponse, GetAgentHistoryResponse, AgentExecutionRecord, ResourceUsage};

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
            ErrorResponse
        )
    ),
    tags(
        (name = "agents", description = "Agent management endpoints"),
        (name = "workflows", description = "Workflow execution endpoints"),
        (name = "system", description = "System monitoring and health endpoints")
    ),
    info(
        title = "Symbiont Runtime API",
        description = "HTTP API for the Symbiont Agent Runtime System",
        version = "0.3.0",
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
}

#[cfg(feature = "http-api")]
impl Default for HttpApiConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1".to_string(),
            port: 8080,
            enable_cors: true,
            enable_tracing: true,
        }
    }
}

/// HTTP API Server
#[cfg(feature = "http-api")]
pub struct HttpApiServer {
    config: HttpApiConfig,
    runtime_provider: Option<Arc<dyn RuntimeApiProvider>>,
    start_time: Instant,
}

#[cfg(feature = "http-api")]
impl HttpApiServer {
    /// Create a new HTTP API server instance
    pub fn new(config: HttpApiConfig) -> Self {
        Self {
            config,
            runtime_provider: None,
            start_time: Instant::now(),
        }
    }

    /// Set the runtime provider for the API server
    pub fn with_runtime_provider(mut self, provider: Arc<dyn RuntimeApiProvider>) -> Self {
        self.runtime_provider = Some(provider);
        self
    }

    /// Start the HTTP API server
    pub async fn start(&self) -> Result<(), RuntimeError> {
        let app = self.create_router();

        let addr = format!("{}:{}", self.config.bind_address, self.config.port);
        let listener = TcpListener::bind(&addr)
            .await
            .map_err(|e| RuntimeError::Internal(format!("Failed to bind to {}: {}", addr, e)))?;

        tracing::info!("HTTP API server starting on {}", addr);

        axum::serve(listener, app)
            .await
            .map_err(|e| RuntimeError::Internal(format!("Server error: {}", e)))?;

        Ok(())
    }

    /// Create the Axum router with all routes and middleware
    fn create_router(&self) -> Router {
        use axum::routing::{get, post, put};
        
        let mut router = Router::new()
            .route("/api/v1/health", get(health_check))
            .with_state(self.start_time);

        // Add Swagger UI
        router = router.merge(
            SwaggerUi::new("/swagger-ui")
                .url("/api-docs/openapi.json", ApiDoc::openapi())
        );

        // Add stateful routes if we have a runtime provider
        if let Some(provider) = &self.runtime_provider {
            use super::routes::{create_agent, delete_agent, execute_agent, execute_workflow, get_agent_history, get_agent_status, list_agents, get_metrics, update_agent};
            use super::middleware::auth_middleware;
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
            
            // Other routes without authentication
            let other_router = Router::new()
                .route("/api/v1/workflows/execute", post(execute_workflow))
                .route("/api/v1/metrics", get(get_metrics))
                .with_state(provider.clone());
            
            router = router.merge(agent_router).merge(other_router);
        }

        // Add middleware conditionally
        if self.config.enable_tracing {
            router = router.layer(TraceLayer::new_for_http());
        }

        if self.config.enable_cors {
            router = router.layer(CorsLayer::permissive());
        }

        // Apply security headers to all responses
        router = router.layer(axum::middleware::from_fn(crate::api::middleware::security_headers_middleware));

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
