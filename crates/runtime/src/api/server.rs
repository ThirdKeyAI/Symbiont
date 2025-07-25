//! HTTP API server implementation
//!
//! This module provides the main HTTP server implementation using Axum.

#[cfg(feature = "http-api")]
use axum::{http::StatusCode, response::Json, Router};

#[cfg(feature = "http-api")]
use std::sync::Arc;

#[cfg(feature = "http-api")]
use tokio::net::TcpListener;

#[cfg(feature = "http-api")]
use tower_http::{cors::CorsLayer, trace::TraceLayer};

#[cfg(feature = "http-api")]
use super::types::{ErrorResponse, HealthResponse};

#[cfg(feature = "http-api")]
use super::traits::RuntimeApiProvider;

#[cfg(feature = "http-api")]
use crate::types::RuntimeError;

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
}

#[cfg(feature = "http-api")]
impl HttpApiServer {
    /// Create a new HTTP API server instance
    pub fn new(config: HttpApiConfig) -> Self {
        Self {
            config,
            runtime_provider: None,
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
        use axum::routing::{get, post};
        
        let mut router = Router::new().route("/api/v1/health", get(health_check));

        // Add stateful routes if we have a runtime provider
        if let Some(provider) = &self.runtime_provider {
            use super::routes::{execute_workflow, get_agent_status, list_agents, get_metrics};
            
            let stateful_router = Router::new()
                .route("/api/v1/workflows/execute", post(execute_workflow))
                .route("/api/v1/agents", get(list_agents))
                .route("/api/v1/agents/:id/status", get(get_agent_status))
                .route("/api/v1/metrics", get(get_metrics))
                .with_state(provider.clone());
            
            router = router.merge(stateful_router);
        }

        // Add middleware conditionally
        if self.config.enable_tracing {
            router = router.layer(TraceLayer::new_for_http());
        }

        if self.config.enable_cors {
            router = router.layer(CorsLayer::permissive());
        }

        router
    }
}

/// Health check endpoint handler
#[cfg(feature = "http-api")]
async fn health_check() -> Result<Json<HealthResponse>, (StatusCode, Json<ErrorResponse>)> {
    let response = HealthResponse {
        status: "healthy".to_string(),
        uptime_seconds: 0, // TODO: implement actual uptime tracking
        timestamp: chrono::Utc::now(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    };

    Ok(Json(response))
}
