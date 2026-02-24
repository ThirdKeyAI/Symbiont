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
use utoipa;

#[cfg(feature = "http-api")]
use super::traits::RuntimeApiProvider;

#[cfg(feature = "http-api")]
use super::types::{
    AddIdentityMappingRequest, AgentStatusResponse, ChannelActionResponse, ChannelAuditResponse,
    ChannelDetail, ChannelHealthResponse, ChannelSummary, CreateAgentRequest, CreateAgentResponse,
    CreateScheduleRequest, CreateScheduleResponse, DeleteAgentResponse, DeleteChannelResponse,
    DeleteScheduleResponse, ErrorResponse, ExecuteAgentRequest, ExecuteAgentResponse,
    GetAgentHistoryResponse, IdentityMappingEntry, NextRunsResponse, RegisterChannelRequest,
    RegisterChannelResponse, ScheduleActionResponse, ScheduleDetail, ScheduleHistoryResponse,
    ScheduleSummary, SchedulerHealthResponse, UpdateAgentRequest, UpdateAgentResponse,
    UpdateChannelRequest, UpdateScheduleRequest, WorkflowExecutionRequest,
};

#[cfg(feature = "http-api")]
use crate::types::AgentId;

// ── AGENTS.md endpoint ─────────────────────────────────────────────────

/// Serve AGENTS.md with sensitive sections stripped.
///
/// Reads `AGENTS.md` from the working directory, removes content between
/// `<!-- agents-md:sensitive-start -->` and `<!-- agents-md:sensitive-end -->`
/// markers, and returns the filtered markdown.
#[cfg(feature = "http-api")]
pub async fn serve_agents_md() -> Result<
    (
        StatusCode,
        [(axum::http::header::HeaderName, &'static str); 1],
        String,
    ),
    StatusCode,
> {
    let content = std::fs::read_to_string("AGENTS.md").map_err(|_| StatusCode::NOT_FOUND)?;
    let filtered = strip_sensitive_sections(&content);
    Ok((
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/markdown; charset=utf-8",
        )],
        filtered,
    ))
}

/// Strip sensitive sections from AGENTS.md content (inline helper).
///
/// Removes all content between `<!-- agents-md:sensitive-start -->` and
/// `<!-- agents-md:sensitive-end -->` markers, including the markers themselves.
#[cfg(feature = "http-api")]
fn strip_sensitive_sections(content: &str) -> String {
    const SENSITIVE_START: &str = "<!-- agents-md:sensitive-start -->";
    const SENSITIVE_END: &str = "<!-- agents-md:sensitive-end -->";

    let mut result = content.to_string();
    while let (Some(start), Some(end)) = (result.find(SENSITIVE_START), result.find(SENSITIVE_END))
    {
        if end <= start {
            break;
        }
        let end_pos = end + SENSITIVE_END.len();
        let end_pos = if result[end_pos..].starts_with('\n') {
            end_pos + 1
        } else {
            end_pos
        };
        let start_pos = if start > 0 && result.as_bytes()[start - 1] == b'\n' {
            start - 1
        } else {
            start
        };
        result = format!("{}{}", &result[..start_pos], &result[end_pos..]);
    }
    result
}

// ── Workflow / Agent / Schedule / Channel endpoints ────────────────────

/// Workflow execution endpoint handler
#[cfg(feature = "http-api")]
#[utoipa::path(
    post,
    path = "/api/v1/workflows/execute",
    request_body = WorkflowExecutionRequest,
    responses(
        (status = 200, description = "Workflow executed successfully", body = serde_json::Value),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "workflows"
)]
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
#[utoipa::path(
    get,
    path = "/api/v1/agents/{id}/status",
    params(
        ("id" = AgentId, Path, description = "Agent identifier")
    ),
    responses(
        (status = 200, description = "Agent status retrieved successfully", body = AgentStatusResponse),
        (status = 404, description = "Agent not found", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn get_agent_status(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(agent_id): Path<AgentId>,
) -> Result<Json<AgentStatusResponse>, (StatusCode, Json<ErrorResponse>)> {
    match provider.get_agent_status(agent_id).await {
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
#[utoipa::path(
    get,
    path = "/api/v1/agents",
    responses(
        (status = 200, description = "Agents listed successfully", body = Vec<String>),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "agents"
)]
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
#[utoipa::path(
    get,
    path = "/api/v1/metrics",
    responses(
        (status = 200, description = "Metrics retrieved successfully", body = serde_json::Value),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "system"
)]
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

/// Create agent endpoint handler
#[cfg(feature = "http-api")]
#[utoipa::path(
    post,
    path = "/api/v1/agents",
    request_body = CreateAgentRequest,
    responses(
        (status = 200, description = "Agent created successfully", body = CreateAgentResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn create_agent(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Json(request): Json<CreateAgentRequest>,
) -> Result<Json<CreateAgentResponse>, (StatusCode, Json<ErrorResponse>)> {
    match provider.create_agent(request).await {
        Ok(response) => Ok(Json(response)),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "AGENT_CREATION_FAILED".to_string(),
                details: None,
            }),
        )),
    }
}

/// Update agent endpoint handler
#[cfg(feature = "http-api")]
#[utoipa::path(
    put,
    path = "/api/v1/agents/{id}",
    params(
        ("id" = AgentId, Path, description = "Agent identifier")
    ),
    request_body = UpdateAgentRequest,
    responses(
        (status = 200, description = "Agent updated successfully", body = UpdateAgentResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn update_agent(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(agent_id): Path<AgentId>,
    Json(request): Json<UpdateAgentRequest>,
) -> Result<Json<UpdateAgentResponse>, (StatusCode, Json<ErrorResponse>)> {
    match provider.update_agent(agent_id, request).await {
        Ok(response) => Ok(Json(response)),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "AGENT_UPDATE_FAILED".to_string(),
                details: None,
            }),
        )),
    }
}

/// Delete agent endpoint handler
#[cfg(feature = "http-api")]
#[utoipa::path(
    delete,
    path = "/api/v1/agents/{id}",
    params(
        ("id" = AgentId, Path, description = "Agent identifier")
    ),
    responses(
        (status = 200, description = "Agent deleted successfully", body = DeleteAgentResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn delete_agent(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(agent_id): Path<AgentId>,
) -> Result<Json<DeleteAgentResponse>, (StatusCode, Json<ErrorResponse>)> {
    match provider.delete_agent(agent_id).await {
        Ok(response) => Ok(Json(response)),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "AGENT_DELETION_FAILED".to_string(),
                details: None,
            }),
        )),
    }
}

/// Execute agent endpoint handler
#[cfg(feature = "http-api")]
#[utoipa::path(
    post,
    path = "/api/v1/agents/{id}/execute",
    params(
        ("id" = AgentId, Path, description = "Agent identifier")
    ),
    request_body = ExecuteAgentRequest,
    responses(
        (status = 200, description = "Agent executed successfully", body = ExecuteAgentResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn execute_agent(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(agent_id): Path<AgentId>,
    Json(request): Json<ExecuteAgentRequest>,
) -> Result<Json<ExecuteAgentResponse>, (StatusCode, Json<ErrorResponse>)> {
    match provider.execute_agent(agent_id, request).await {
        Ok(response) => Ok(Json(response)),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "AGENT_EXECUTION_FAILED".to_string(),
                details: None,
            }),
        )),
    }
}

/// Get agent execution history endpoint handler
#[cfg(feature = "http-api")]
#[utoipa::path(
    get,
    path = "/api/v1/agents/{id}/history",
    params(
        ("id" = AgentId, Path, description = "Agent identifier")
    ),
    responses(
        (status = 200, description = "Agent history retrieved successfully", body = GetAgentHistoryResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn get_agent_history(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(agent_id): Path<AgentId>,
) -> Result<Json<GetAgentHistoryResponse>, (StatusCode, Json<ErrorResponse>)> {
    match provider.get_agent_history(agent_id).await {
        Ok(response) => Ok(Json(response)),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "AGENT_HISTORY_FAILED".to_string(),
                details: None,
            }),
        )),
    }
}

// ── Schedule / Cron endpoints ──────────────────────────────────────────

/// List all scheduled jobs
#[cfg(feature = "http-api")]
#[utoipa::path(
    get,
    path = "/api/v1/schedules",
    responses(
        (status = 200, description = "Schedules listed", body = Vec<ScheduleSummary>),
        (status = 500, description = "Internal error", body = ErrorResponse)
    ),
    tag = "schedules"
)]
pub async fn list_schedules(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
) -> Result<Json<Vec<ScheduleSummary>>, (StatusCode, Json<ErrorResponse>)> {
    provider.list_schedules().await.map(Json).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "LIST_SCHEDULES_FAILED".to_string(),
                details: None,
            }),
        )
    })
}

/// Create a new scheduled job
#[cfg(feature = "http-api")]
#[utoipa::path(
    post,
    path = "/api/v1/schedules",
    request_body = CreateScheduleRequest,
    responses(
        (status = 201, description = "Schedule created", body = CreateScheduleResponse),
        (status = 400, description = "Bad request", body = ErrorResponse),
        (status = 500, description = "Internal error", body = ErrorResponse)
    ),
    tag = "schedules"
)]
pub async fn create_schedule(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Json(request): Json<CreateScheduleRequest>,
) -> Result<(StatusCode, Json<CreateScheduleResponse>), (StatusCode, Json<ErrorResponse>)> {
    provider
        .create_schedule(request)
        .await
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code: "CREATE_SCHEDULE_FAILED".to_string(),
                    details: None,
                }),
            )
        })
}

/// Get details of a scheduled job
#[cfg(feature = "http-api")]
#[utoipa::path(
    get,
    path = "/api/v1/schedules/{id}",
    params(("id" = String, Path, description = "Job UUID")),
    responses(
        (status = 200, description = "Schedule details", body = ScheduleDetail),
        (status = 404, description = "Not found", body = ErrorResponse)
    ),
    tag = "schedules"
)]
pub async fn get_schedule(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(id): Path<String>,
) -> Result<Json<ScheduleDetail>, (StatusCode, Json<ErrorResponse>)> {
    provider.get_schedule(&id).await.map(Json).map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "SCHEDULE_NOT_FOUND".to_string(),
                details: None,
            }),
        )
    })
}

/// Update a scheduled job
#[cfg(feature = "http-api")]
#[utoipa::path(
    put,
    path = "/api/v1/schedules/{id}",
    params(("id" = String, Path, description = "Job UUID")),
    request_body = UpdateScheduleRequest,
    responses(
        (status = 200, description = "Schedule updated", body = ScheduleDetail),
        (status = 404, description = "Not found", body = ErrorResponse)
    ),
    tag = "schedules"
)]
pub async fn update_schedule(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(id): Path<String>,
    Json(request): Json<UpdateScheduleRequest>,
) -> Result<Json<ScheduleDetail>, (StatusCode, Json<ErrorResponse>)> {
    provider
        .update_schedule(&id, request)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code: "UPDATE_SCHEDULE_FAILED".to_string(),
                    details: None,
                }),
            )
        })
}

/// Delete a scheduled job
#[cfg(feature = "http-api")]
#[utoipa::path(
    delete,
    path = "/api/v1/schedules/{id}",
    params(("id" = String, Path, description = "Job UUID")),
    responses(
        (status = 200, description = "Schedule deleted", body = DeleteScheduleResponse),
        (status = 404, description = "Not found", body = ErrorResponse)
    ),
    tag = "schedules"
)]
pub async fn delete_schedule(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(id): Path<String>,
) -> Result<Json<DeleteScheduleResponse>, (StatusCode, Json<ErrorResponse>)> {
    provider.delete_schedule(&id).await.map(Json).map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "SCHEDULE_NOT_FOUND".to_string(),
                details: None,
            }),
        )
    })
}

/// Pause a scheduled job
#[cfg(feature = "http-api")]
#[utoipa::path(
    post,
    path = "/api/v1/schedules/{id}/pause",
    params(("id" = String, Path, description = "Job UUID")),
    responses(
        (status = 200, description = "Schedule paused", body = ScheduleActionResponse),
        (status = 404, description = "Not found", body = ErrorResponse)
    ),
    tag = "schedules"
)]
pub async fn pause_schedule(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(id): Path<String>,
) -> Result<Json<ScheduleActionResponse>, (StatusCode, Json<ErrorResponse>)> {
    provider.pause_schedule(&id).await.map(Json).map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "SCHEDULE_PAUSE_FAILED".to_string(),
                details: None,
            }),
        )
    })
}

/// Resume a paused scheduled job
#[cfg(feature = "http-api")]
#[utoipa::path(
    post,
    path = "/api/v1/schedules/{id}/resume",
    params(("id" = String, Path, description = "Job UUID")),
    responses(
        (status = 200, description = "Schedule resumed", body = ScheduleActionResponse),
        (status = 404, description = "Not found", body = ErrorResponse)
    ),
    tag = "schedules"
)]
pub async fn resume_schedule(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(id): Path<String>,
) -> Result<Json<ScheduleActionResponse>, (StatusCode, Json<ErrorResponse>)> {
    provider.resume_schedule(&id).await.map(Json).map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "SCHEDULE_RESUME_FAILED".to_string(),
                details: None,
            }),
        )
    })
}

/// Force-trigger a scheduled job immediately
#[cfg(feature = "http-api")]
#[utoipa::path(
    post,
    path = "/api/v1/schedules/{id}/trigger",
    params(("id" = String, Path, description = "Job UUID")),
    responses(
        (status = 200, description = "Schedule triggered", body = ScheduleActionResponse),
        (status = 404, description = "Not found", body = ErrorResponse)
    ),
    tag = "schedules"
)]
pub async fn trigger_schedule(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(id): Path<String>,
) -> Result<Json<ScheduleActionResponse>, (StatusCode, Json<ErrorResponse>)> {
    provider.trigger_schedule(&id).await.map(Json).map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "SCHEDULE_TRIGGER_FAILED".to_string(),
                details: None,
            }),
        )
    })
}

/// Get run history for a scheduled job
#[cfg(feature = "http-api")]
#[utoipa::path(
    get,
    path = "/api/v1/schedules/{id}/history",
    params(("id" = String, Path, description = "Job UUID")),
    responses(
        (status = 200, description = "Schedule history", body = ScheduleHistoryResponse),
        (status = 404, description = "Not found", body = ErrorResponse)
    ),
    tag = "schedules"
)]
pub async fn get_schedule_history(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(id): Path<String>,
) -> Result<Json<ScheduleHistoryResponse>, (StatusCode, Json<ErrorResponse>)> {
    provider
        .get_schedule_history(&id, 50)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code: "SCHEDULE_HISTORY_FAILED".to_string(),
                    details: None,
                }),
            )
        })
}

/// Get next N run times for a scheduled job
#[cfg(feature = "http-api")]
#[utoipa::path(
    get,
    path = "/api/v1/schedules/{id}/next-runs",
    params(("id" = String, Path, description = "Job UUID")),
    responses(
        (status = 200, description = "Next runs", body = NextRunsResponse),
        (status = 404, description = "Not found", body = ErrorResponse)
    ),
    tag = "schedules"
)]
pub async fn get_schedule_next_runs(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(id): Path<String>,
) -> Result<Json<NextRunsResponse>, (StatusCode, Json<ErrorResponse>)> {
    provider
        .get_schedule_next_runs(&id, 10)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code: "SCHEDULE_NEXT_RUNS_FAILED".to_string(),
                    details: None,
                }),
            )
        })
}

/// Get scheduler health and metrics
#[cfg(feature = "http-api")]
#[utoipa::path(
    get,
    path = "/api/v1/health/scheduler",
    responses(
        (status = 200, description = "Scheduler health", body = SchedulerHealthResponse),
        (status = 500, description = "Internal error", body = ErrorResponse)
    ),
    tag = "system"
)]
pub async fn get_scheduler_health(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
) -> Result<Json<SchedulerHealthResponse>, (StatusCode, Json<ErrorResponse>)> {
    provider
        .get_scheduler_health()
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code: "SCHEDULER_HEALTH_FAILED".to_string(),
                    details: None,
                }),
            )
        })
}

// ── Channel endpoints ──────────────────────────────────────────

/// List all registered channel adapters
#[cfg(feature = "http-api")]
#[utoipa::path(
    get,
    path = "/api/v1/channels",
    responses(
        (status = 200, description = "Channels listed", body = Vec<ChannelSummary>),
        (status = 500, description = "Internal error", body = ErrorResponse)
    ),
    tag = "channels"
)]
pub async fn list_channels(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
) -> Result<Json<Vec<ChannelSummary>>, (StatusCode, Json<ErrorResponse>)> {
    provider.list_channels().await.map(Json).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "LIST_CHANNELS_FAILED".to_string(),
                details: None,
            }),
        )
    })
}

/// Register a new channel adapter
#[cfg(feature = "http-api")]
#[utoipa::path(
    post,
    path = "/api/v1/channels",
    request_body = RegisterChannelRequest,
    responses(
        (status = 201, description = "Channel registered", body = RegisterChannelResponse),
        (status = 400, description = "Bad request", body = ErrorResponse),
        (status = 500, description = "Internal error", body = ErrorResponse)
    ),
    tag = "channels"
)]
pub async fn register_channel(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Json(request): Json<RegisterChannelRequest>,
) -> Result<(StatusCode, Json<RegisterChannelResponse>), (StatusCode, Json<ErrorResponse>)> {
    provider
        .register_channel(request)
        .await
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code: "REGISTER_CHANNEL_FAILED".to_string(),
                    details: None,
                }),
            )
        })
}

/// Get details of a channel adapter
#[cfg(feature = "http-api")]
#[utoipa::path(
    get,
    path = "/api/v1/channels/{id}",
    params(("id" = String, Path, description = "Channel ID")),
    responses(
        (status = 200, description = "Channel details", body = ChannelDetail),
        (status = 404, description = "Not found", body = ErrorResponse)
    ),
    tag = "channels"
)]
pub async fn get_channel(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(id): Path<String>,
) -> Result<Json<ChannelDetail>, (StatusCode, Json<ErrorResponse>)> {
    provider.get_channel(&id).await.map(Json).map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "CHANNEL_NOT_FOUND".to_string(),
                details: None,
            }),
        )
    })
}

/// Update a channel adapter configuration
#[cfg(feature = "http-api")]
#[utoipa::path(
    put,
    path = "/api/v1/channels/{id}",
    params(("id" = String, Path, description = "Channel ID")),
    request_body = UpdateChannelRequest,
    responses(
        (status = 200, description = "Channel updated", body = ChannelDetail),
        (status = 404, description = "Not found", body = ErrorResponse)
    ),
    tag = "channels"
)]
pub async fn update_channel(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(id): Path<String>,
    Json(request): Json<UpdateChannelRequest>,
) -> Result<Json<ChannelDetail>, (StatusCode, Json<ErrorResponse>)> {
    provider
        .update_channel(&id, request)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code: "UPDATE_CHANNEL_FAILED".to_string(),
                    details: None,
                }),
            )
        })
}

/// Delete a channel adapter
#[cfg(feature = "http-api")]
#[utoipa::path(
    delete,
    path = "/api/v1/channels/{id}",
    params(("id" = String, Path, description = "Channel ID")),
    responses(
        (status = 200, description = "Channel deleted", body = DeleteChannelResponse),
        (status = 404, description = "Not found", body = ErrorResponse)
    ),
    tag = "channels"
)]
pub async fn delete_channel(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(id): Path<String>,
) -> Result<Json<DeleteChannelResponse>, (StatusCode, Json<ErrorResponse>)> {
    provider.delete_channel(&id).await.map(Json).map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "CHANNEL_NOT_FOUND".to_string(),
                details: None,
            }),
        )
    })
}

/// Start a channel adapter
#[cfg(feature = "http-api")]
#[utoipa::path(
    post,
    path = "/api/v1/channels/{id}/start",
    params(("id" = String, Path, description = "Channel ID")),
    responses(
        (status = 200, description = "Channel started", body = ChannelActionResponse),
        (status = 404, description = "Not found", body = ErrorResponse)
    ),
    tag = "channels"
)]
pub async fn start_channel(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(id): Path<String>,
) -> Result<Json<ChannelActionResponse>, (StatusCode, Json<ErrorResponse>)> {
    provider.start_channel(&id).await.map(Json).map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "CHANNEL_START_FAILED".to_string(),
                details: None,
            }),
        )
    })
}

/// Stop a channel adapter
#[cfg(feature = "http-api")]
#[utoipa::path(
    post,
    path = "/api/v1/channels/{id}/stop",
    params(("id" = String, Path, description = "Channel ID")),
    responses(
        (status = 200, description = "Channel stopped", body = ChannelActionResponse),
        (status = 404, description = "Not found", body = ErrorResponse)
    ),
    tag = "channels"
)]
pub async fn stop_channel(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(id): Path<String>,
) -> Result<Json<ChannelActionResponse>, (StatusCode, Json<ErrorResponse>)> {
    provider.stop_channel(&id).await.map(Json).map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "CHANNEL_STOP_FAILED".to_string(),
                details: None,
            }),
        )
    })
}

/// Get channel adapter health and connectivity info
#[cfg(feature = "http-api")]
#[utoipa::path(
    get,
    path = "/api/v1/channels/{id}/health",
    params(("id" = String, Path, description = "Channel ID")),
    responses(
        (status = 200, description = "Channel health", body = ChannelHealthResponse),
        (status = 404, description = "Not found", body = ErrorResponse)
    ),
    tag = "channels"
)]
pub async fn get_channel_health(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(id): Path<String>,
) -> Result<Json<ChannelHealthResponse>, (StatusCode, Json<ErrorResponse>)> {
    provider
        .get_channel_health(&id)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code: "CHANNEL_HEALTH_FAILED".to_string(),
                    details: None,
                }),
            )
        })
}

/// List identity mappings for a channel
#[cfg(feature = "http-api")]
#[utoipa::path(
    get,
    path = "/api/v1/channels/{id}/mappings",
    params(("id" = String, Path, description = "Channel ID")),
    responses(
        (status = 200, description = "Identity mappings listed", body = Vec<IdentityMappingEntry>),
        (status = 404, description = "Not found", body = ErrorResponse),
        (status = 501, description = "Not implemented (community edition)", body = ErrorResponse)
    ),
    tag = "channels"
)]
pub async fn list_channel_mappings(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<IdentityMappingEntry>>, (StatusCode, Json<ErrorResponse>)> {
    provider
        .list_channel_mappings(&id)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code: "CHANNEL_MAPPINGS_FAILED".to_string(),
                    details: None,
                }),
            )
        })
}

/// Add an identity mapping to a channel
#[cfg(feature = "http-api")]
#[utoipa::path(
    post,
    path = "/api/v1/channels/{id}/mappings",
    params(("id" = String, Path, description = "Channel ID")),
    request_body = AddIdentityMappingRequest,
    responses(
        (status = 201, description = "Mapping added", body = IdentityMappingEntry),
        (status = 404, description = "Not found", body = ErrorResponse),
        (status = 501, description = "Not implemented (community edition)", body = ErrorResponse)
    ),
    tag = "channels"
)]
pub async fn add_channel_mapping(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(id): Path<String>,
    Json(request): Json<AddIdentityMappingRequest>,
) -> Result<(StatusCode, Json<IdentityMappingEntry>), (StatusCode, Json<ErrorResponse>)> {
    provider
        .add_channel_mapping(&id, request)
        .await
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code: "ADD_CHANNEL_MAPPING_FAILED".to_string(),
                    details: None,
                }),
            )
        })
}

/// Remove an identity mapping from a channel
#[cfg(feature = "http-api")]
#[utoipa::path(
    delete,
    path = "/api/v1/channels/{id}/mappings/{user_id}",
    params(
        ("id" = String, Path, description = "Channel ID"),
        ("user_id" = String, Path, description = "Platform user ID to remove")
    ),
    responses(
        (status = 204, description = "Mapping removed"),
        (status = 404, description = "Not found", body = ErrorResponse),
        (status = 501, description = "Not implemented (community edition)", body = ErrorResponse)
    ),
    tag = "channels"
)]
pub async fn remove_channel_mapping(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path((id, user_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    provider
        .remove_channel_mapping(&id, &user_id)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code: "REMOVE_CHANNEL_MAPPING_FAILED".to_string(),
                    details: None,
                }),
            )
        })
}

/// Get audit log entries for a channel
#[cfg(feature = "http-api")]
#[utoipa::path(
    get,
    path = "/api/v1/channels/{id}/audit",
    params(("id" = String, Path, description = "Channel ID")),
    responses(
        (status = 200, description = "Audit log", body = ChannelAuditResponse),
        (status = 404, description = "Not found", body = ErrorResponse),
        (status = 501, description = "Not implemented (community edition)", body = ErrorResponse)
    ),
    tag = "channels"
)]
pub async fn get_channel_audit(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(id): Path<String>,
) -> Result<Json<ChannelAuditResponse>, (StatusCode, Json<ErrorResponse>)> {
    provider
        .get_channel_audit(&id, 50)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code: "CHANNEL_AUDIT_FAILED".to_string(),
                    details: None,
                }),
            )
        })
}
