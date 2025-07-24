//! HTTP API route handlers
//!
//! This module contains route handler implementations for the HTTP API.

#[cfg(feature = "http-api")]
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};

#[cfg(feature = "http-api")]
use std::sync::Arc;

#[cfg(feature = "http-api")]
use super::traits::RuntimeApiProvider;

#[cfg(feature = "http-api")]
use super::types::{AgentStatusResponse, ErrorResponse, WorkflowExecutionRequest};

#[cfg(feature = "http-api")]
use crate::types::AgentId;

/// Workflow execution endpoint handler
#[cfg(feature = "http-api")]
pub async fn execute_workflow(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Json(request): Json<WorkflowExecutionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    match provider.execute_workflow(request).await {
        Ok(result) => Ok(Json(serde_json::to_value(result).unwrap_or_default())),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "WORKFLOW_EXECUTION_FAILED".to_string(),
                details: None,
            }),
        )),
    }
}

/// Agent status endpoint handler
#[cfg(feature = "http-api")]
pub async fn get_agent_status(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(_agent_id): Path<String>,
) -> Result<Json<AgentStatusResponse>, (StatusCode, Json<ErrorResponse>)> {
    let _agent_id = AgentId::new(); // TODO: Parse agent_id from string parameter

    match provider.get_agent_status(_agent_id).await {
        Ok(status) => Ok(Json(status)),
        Err(e) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "AGENT_NOT_FOUND".to_string(),
                details: None,
            }),
        )),
    }
}

/// List agents endpoint handler
#[cfg(feature = "http-api")]
pub async fn list_agents(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
) -> Result<Json<Vec<String>>, (StatusCode, Json<ErrorResponse>)> {
    match provider.list_agents().await {
        Ok(agents) => {
            let agent_ids: Vec<String> = agents.into_iter().map(|id| id.to_string()).collect();
            Ok(Json(agent_ids))
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "LIST_AGENTS_FAILED".to_string(),
                details: None,
            }),
        )),
    }
}

/// System metrics endpoint handler
#[cfg(feature = "http-api")]
pub async fn get_metrics(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    match provider.get_metrics().await {
        Ok(metrics) => Ok(Json(metrics)),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "METRICS_FAILED".to_string(),
                details: None,
            }),
        )),
    }
}
