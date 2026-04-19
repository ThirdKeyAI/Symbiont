//! HTTP API route handlers
//!
//! This module contains route handler implementations for the HTTP API.

#[cfg(feature = "http-api")]
use axum::{
    extract::{Extension, Path, State},
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
    GetAgentHistoryResponse, HeartbeatRequest, IdentityMappingEntry, MessageStatusResponse,
    NextRunsResponse, PushEventRequest, ReceiveMessagesResponse, RegisterChannelRequest,
    RegisterChannelResponse, ScheduleActionResponse, ScheduleDetail, ScheduleHistoryResponse,
    ScheduleSummary, SchedulerHealthResponse, SendMessageRequest, SendMessageResponse,
    UpdateAgentRequest, UpdateAgentResponse, UpdateChannelRequest, UpdateScheduleRequest,
    WorkflowExecutionRequest,
};

#[cfg(feature = "http-api")]
use super::api_keys::ValidatedKey;

#[cfg(feature = "http-api")]
use crate::types::AgentId;

/// Enforce that the authenticated key is permitted to act on `agent_id`.
///
/// - `Some(key)` with `agent_scope = None`: admin/unrestricted key — allowed.
/// - `Some(key)` with `agent_scope = Some(scope)`: the agent ID must appear
///   in the scope list (match on UUID string representation).
/// - `None`: request was authenticated via the legacy env-token path. That
///   path is only available to operators with access to the env var, so we
///   treat it as admin for backward compatibility.
#[cfg(feature = "http-api")]
fn check_agent_access(
    validated: Option<&ValidatedKey>,
    agent_id: &AgentId,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    match validated {
        Some(key) => match &key.agent_scope {
            None => Ok(()),
            Some(scope) => {
                let target = agent_id.0.to_string();
                if scope.iter().any(|s| s == &target) {
                    Ok(())
                } else {
                    Err((
                        StatusCode::FORBIDDEN,
                        Json(ErrorResponse {
                            error: "API key is not scoped to this agent".to_string(),
                            code: "AGENT_SCOPE_DENIED".to_string(),
                            details: None,
                        }),
                    ))
                }
            }
        },
        None => Ok(()),
    }
}

/// Reject requests that aren't made with an unscoped (admin) key.
///
/// Used on routes that manipulate shared infrastructure (schedules,
/// channels, cross-agent listings, system metrics). Scoped keys are meant
/// for per-agent data plane access (send/receive/heartbeat) and must not
/// touch the control plane.
#[cfg(feature = "http-api")]
fn require_admin(
    validated: Option<&ValidatedKey>,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    match validated {
        Some(key) if key.agent_scope.is_some() => Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: "Scoped API keys may not access this endpoint".to_string(),
                code: "ADMIN_REQUIRED".to_string(),
                details: None,
            }),
        )),
        _ => Ok(()),
    }
}

/// Return the list of agent IDs the validated key is scoped to, if any.
///
/// `None` means unscoped / admin (no filtering required). `Some` lists the
/// caller-permitted agents that list endpoints should intersect their
/// results against.
#[cfg(feature = "http-api")]
fn scope_filter(validated: Option<&ValidatedKey>) -> Option<&Vec<String>> {
    validated.and_then(|k| k.agent_scope.as_ref())
}

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
    let content = tokio::fs::read_to_string("AGENTS.md")
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
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
    validated: Option<Extension<ValidatedKey>>,
    Json(request): Json<WorkflowExecutionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    // If the workflow targets a specific agent and the caller is scoped,
    // enforce scope; otherwise require admin (workflow may span agents).
    if let Some(agent_id) = request.agent_id.as_ref() {
        check_agent_access(validated.as_ref().map(|Extension(k)| k), agent_id)?;
    } else {
        require_admin(validated.as_ref().map(|Extension(k)| k))?;
    }
    match provider.execute_workflow(request).await {
        Ok(result) => match serde_json::to_value(result) {
            Ok(v) => Ok(Json(v)),
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "Failed to serialize workflow result"
                );
                Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Failed to serialize workflow result".to_string(),
                        code: "WORKFLOW_SERIALIZATION_FAILED".to_string(),
                        details: None,
                    }),
                ))
            }
        },
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
    validated: Option<Extension<ValidatedKey>>,
) -> Result<Json<AgentStatusResponse>, (StatusCode, Json<ErrorResponse>)> {
    check_agent_access(validated.as_ref().map(|Extension(k)| k), &agent_id)?;
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
        (status = 200, description = "Agents listed successfully", body = Vec<AgentSummary>),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn list_agents(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    validated: Option<Extension<ValidatedKey>>,
) -> Result<Json<Vec<super::types::AgentSummary>>, (StatusCode, Json<ErrorResponse>)> {
    let scope = scope_filter(validated.as_ref().map(|Extension(k)| k)).cloned();
    match provider.list_agents_detailed().await {
        Ok(mut agents) => {
            if let Some(scope) = scope {
                // Intersect the caller's scope with the listing — scoped keys
                // must not enumerate agents they don't control.
                agents.retain(|a| scope.iter().any(|s| s == &a.id.0.to_string()));
            }
            Ok(Json(agents))
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
    validated: Option<Extension<ValidatedKey>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    require_admin(validated.as_ref().map(|Extension(k)| k))?;
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
    validated: Option<Extension<ValidatedKey>>,
    Json(request): Json<CreateAgentRequest>,
) -> Result<Json<CreateAgentResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_admin(validated.as_ref().map(|Extension(k)| k))?;
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
    validated: Option<Extension<ValidatedKey>>,
    Json(request): Json<UpdateAgentRequest>,
) -> Result<Json<UpdateAgentResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_admin(validated.as_ref().map(|Extension(k)| k))?;
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
    validated: Option<Extension<ValidatedKey>>,
) -> Result<Json<DeleteAgentResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_admin(validated.as_ref().map(|Extension(k)| k))?;
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
    validated: Option<Extension<ValidatedKey>>,
    Json(request): Json<ExecuteAgentRequest>,
) -> Result<Json<ExecuteAgentResponse>, (StatusCode, Json<ErrorResponse>)> {
    check_agent_access(validated.as_ref().map(|Extension(k)| k), &agent_id)?;
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
    validated: Option<Extension<ValidatedKey>>,
) -> Result<Json<GetAgentHistoryResponse>, (StatusCode, Json<ErrorResponse>)> {
    check_agent_access(validated.as_ref().map(|Extension(k)| k), &agent_id)?;
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
    validated: Option<Extension<ValidatedKey>>,
) -> Result<Json<Vec<ScheduleSummary>>, (StatusCode, Json<ErrorResponse>)> {
    require_admin(validated.as_ref().map(|Extension(k)| k))?;
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
    validated: Option<Extension<ValidatedKey>>,
    Json(request): Json<CreateScheduleRequest>,
) -> Result<(StatusCode, Json<CreateScheduleResponse>), (StatusCode, Json<ErrorResponse>)> {
    require_admin(validated.as_ref().map(|Extension(k)| k))?;
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
    validated: Option<Extension<ValidatedKey>>,
) -> Result<Json<ScheduleDetail>, (StatusCode, Json<ErrorResponse>)> {
    require_admin(validated.as_ref().map(|Extension(k)| k))?;
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
    validated: Option<Extension<ValidatedKey>>,
    Json(request): Json<UpdateScheduleRequest>,
) -> Result<Json<ScheduleDetail>, (StatusCode, Json<ErrorResponse>)> {
    require_admin(validated.as_ref().map(|Extension(k)| k))?;
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
    validated: Option<Extension<ValidatedKey>>,
) -> Result<Json<DeleteScheduleResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_admin(validated.as_ref().map(|Extension(k)| k))?;
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
    validated: Option<Extension<ValidatedKey>>,
) -> Result<Json<ScheduleActionResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_admin(validated.as_ref().map(|Extension(k)| k))?;
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
    validated: Option<Extension<ValidatedKey>>,
) -> Result<Json<ScheduleActionResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_admin(validated.as_ref().map(|Extension(k)| k))?;
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
    validated: Option<Extension<ValidatedKey>>,
) -> Result<Json<ScheduleActionResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_admin(validated.as_ref().map(|Extension(k)| k))?;
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
    validated: Option<Extension<ValidatedKey>>,
) -> Result<Json<ScheduleHistoryResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_admin(validated.as_ref().map(|Extension(k)| k))?;
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
    validated: Option<Extension<ValidatedKey>>,
) -> Result<Json<NextRunsResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_admin(validated.as_ref().map(|Extension(k)| k))?;
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
    validated: Option<Extension<ValidatedKey>>,
) -> Result<Json<SchedulerHealthResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_admin(validated.as_ref().map(|Extension(k)| k))?;
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
    validated: Option<Extension<ValidatedKey>>,
) -> Result<Json<Vec<ChannelSummary>>, (StatusCode, Json<ErrorResponse>)> {
    require_admin(validated.as_ref().map(|Extension(k)| k))?;
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
    validated: Option<Extension<ValidatedKey>>,
    Json(request): Json<RegisterChannelRequest>,
) -> Result<(StatusCode, Json<RegisterChannelResponse>), (StatusCode, Json<ErrorResponse>)> {
    require_admin(validated.as_ref().map(|Extension(k)| k))?;
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
    validated: Option<Extension<ValidatedKey>>,
) -> Result<Json<ChannelDetail>, (StatusCode, Json<ErrorResponse>)> {
    require_admin(validated.as_ref().map(|Extension(k)| k))?;
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
    validated: Option<Extension<ValidatedKey>>,
    Json(request): Json<UpdateChannelRequest>,
) -> Result<Json<ChannelDetail>, (StatusCode, Json<ErrorResponse>)> {
    require_admin(validated.as_ref().map(|Extension(k)| k))?;
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
    validated: Option<Extension<ValidatedKey>>,
) -> Result<Json<DeleteChannelResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_admin(validated.as_ref().map(|Extension(k)| k))?;
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
    validated: Option<Extension<ValidatedKey>>,
) -> Result<Json<ChannelActionResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_admin(validated.as_ref().map(|Extension(k)| k))?;
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
    validated: Option<Extension<ValidatedKey>>,
) -> Result<Json<ChannelActionResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_admin(validated.as_ref().map(|Extension(k)| k))?;
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
    validated: Option<Extension<ValidatedKey>>,
) -> Result<Json<ChannelHealthResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_admin(validated.as_ref().map(|Extension(k)| k))?;
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
    validated: Option<Extension<ValidatedKey>>,
) -> Result<Json<Vec<IdentityMappingEntry>>, (StatusCode, Json<ErrorResponse>)> {
    require_admin(validated.as_ref().map(|Extension(k)| k))?;
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
    validated: Option<Extension<ValidatedKey>>,
    Json(request): Json<AddIdentityMappingRequest>,
) -> Result<(StatusCode, Json<IdentityMappingEntry>), (StatusCode, Json<ErrorResponse>)> {
    require_admin(validated.as_ref().map(|Extension(k)| k))?;
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
    validated: Option<Extension<ValidatedKey>>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    require_admin(validated.as_ref().map(|Extension(k)| k))?;
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
    validated: Option<Extension<ValidatedKey>>,
) -> Result<Json<ChannelAuditResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_admin(validated.as_ref().map(|Extension(k)| k))?;
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

// ── External agent endpoints ───────────────────────────────────────────

/// Heartbeat endpoint for external agents
#[cfg(feature = "http-api")]
#[utoipa::path(
    post,
    path = "/api/v1/agents/{id}/heartbeat",
    params(
        ("id" = AgentId, Path, description = "Agent identifier")
    ),
    request_body = HeartbeatRequest,
    responses(
        (status = 200, description = "Heartbeat accepted"),
        (status = 404, description = "Agent not found or not external", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn agent_heartbeat(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(agent_id): Path<AgentId>,
    validated: Option<Extension<ValidatedKey>>,
    Json(request): Json<HeartbeatRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    check_agent_access(validated.as_ref().map(|Extension(k)| k), &agent_id)?;
    match provider.update_agent_heartbeat(agent_id, request).await {
        Ok(()) => Ok(StatusCode::OK),
        Err(e) => {
            use crate::types::RuntimeError;
            let (status, code) = match &e {
                RuntimeError::Authentication(_) => {
                    (StatusCode::UNAUTHORIZED, "AGENTPIN_VERIFICATION_FAILED")
                }
                _ => (StatusCode::NOT_FOUND, "HEARTBEAT_FAILED"),
            };
            Err((
                status,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code: code.to_string(),
                    details: None,
                }),
            ))
        }
    }
}

/// Push event endpoint for external agents
#[cfg(feature = "http-api")]
#[utoipa::path(
    post,
    path = "/api/v1/agents/{id}/events",
    params(
        ("id" = AgentId, Path, description = "Agent identifier")
    ),
    request_body = PushEventRequest,
    responses(
        (status = 200, description = "Event accepted"),
        (status = 404, description = "Agent not found or not external", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn agent_push_event(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(agent_id): Path<AgentId>,
    validated: Option<Extension<ValidatedKey>>,
    Json(request): Json<PushEventRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    check_agent_access(validated.as_ref().map(|Extension(k)| k), &agent_id)?;
    match provider.push_agent_event(agent_id, request).await {
        Ok(()) => Ok(StatusCode::OK),
        Err(e) => {
            use crate::types::RuntimeError;
            let (status, code) = match &e {
                RuntimeError::Authentication(_) => {
                    (StatusCode::UNAUTHORIZED, "AGENTPIN_VERIFICATION_FAILED")
                }
                _ => (StatusCode::NOT_FOUND, "PUSH_EVENT_FAILED"),
            };
            Err((
                status,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code: code.to_string(),
                    details: None,
                }),
            ))
        }
    }
}

// ══════════════════════════════════════════════════════════════════
// Inter-agent messaging endpoints
// ══════════════════════════════════════════════════════════════════

/// Send a message to an agent through the communication bus.
#[cfg(feature = "http-api")]
#[utoipa::path(
    post,
    path = "/api/v1/agents/{id}/messages",
    params(
        ("id" = AgentId, Path, description = "Recipient agent identifier")
    ),
    request_body = SendMessageRequest,
    responses(
        (status = 200, description = "Message queued", body = SendMessageResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 500, description = "Internal error", body = ErrorResponse)
    ),
    tag = "messages"
)]
pub async fn send_agent_message(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(agent_id): Path<AgentId>,
    validated: Option<Extension<ValidatedKey>>,
    Json(request): Json<SendMessageRequest>,
) -> Result<Json<SendMessageResponse>, (StatusCode, Json<ErrorResponse>)> {
    let key = validated.as_ref().map(|Extension(k)| k);
    // The sender field is attacker-controllable JSON, so gate on the caller's
    // authenticated identity rather than on the claimed sender.
    check_agent_access(key, &request.sender)?;
    match provider.send_agent_message(agent_id, request).await {
        Ok(response) => Ok(Json(response)),
        Err(e) => {
            use crate::types::RuntimeError;
            let (status, code) = match &e {
                RuntimeError::Authentication(_) => {
                    (StatusCode::UNAUTHORIZED, "AGENTPIN_VERIFICATION_FAILED")
                }
                _ => (StatusCode::INTERNAL_SERVER_ERROR, "SEND_MESSAGE_FAILED"),
            };
            Err((
                status,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code: code.to_string(),
                    details: None,
                }),
            ))
        }
    }
}

/// Receive (and consume) pending messages for an agent.
#[cfg(feature = "http-api")]
#[utoipa::path(
    get,
    path = "/api/v1/agents/{id}/messages",
    params(
        ("id" = AgentId, Path, description = "Agent identifier")
    ),
    responses(
        (status = 200, description = "Pending messages", body = ReceiveMessagesResponse),
        (status = 500, description = "Internal error", body = ErrorResponse)
    ),
    tag = "messages"
)]
pub async fn receive_agent_messages(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(agent_id): Path<AgentId>,
    validated: Option<Extension<ValidatedKey>>,
) -> Result<Json<ReceiveMessagesResponse>, (StatusCode, Json<ErrorResponse>)> {
    let key = validated.as_ref().map(|Extension(k)| k);
    // Inbox drain: only the owning agent (or an admin key) may pull messages.
    check_agent_access(key, &agent_id)?;
    match provider.receive_agent_messages(agent_id).await {
        Ok(response) => Ok(Json(response)),
        Err(e) => {
            use crate::types::{CommunicationError, RuntimeError};
            let (status, code) = match &e {
                RuntimeError::Communication(CommunicationError::AgentNotRegistered { .. }) => {
                    (StatusCode::NOT_FOUND, "AGENT_NOT_FOUND")
                }
                _ => (StatusCode::INTERNAL_SERVER_ERROR, "RECEIVE_MESSAGES_FAILED"),
            };
            Err((
                status,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code: code.to_string(),
                    details: None,
                }),
            ))
        }
    }
}

/// Get the delivery status of a specific message.
#[cfg(feature = "http-api")]
#[utoipa::path(
    get,
    path = "/api/v1/messages/{id}/status",
    params(
        ("id" = String, Path, description = "Message identifier (UUID)")
    ),
    responses(
        (status = 200, description = "Delivery status", body = MessageStatusResponse),
        (status = 400, description = "Invalid message ID", body = ErrorResponse),
        (status = 404, description = "Message not found", body = ErrorResponse)
    ),
    tag = "messages"
)]
pub async fn get_message_status(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    Path(message_id): Path<String>,
    validated: Option<Extension<ValidatedKey>>,
) -> Result<Json<MessageStatusResponse>, (StatusCode, Json<ErrorResponse>)> {
    // The current status backend is keyed by message_id only and has no
    // recipient/sender index, so we cannot cheaply scope-check the lookup.
    // Scoped keys could otherwise enumerate delivery state for messages they
    // have no claim to — require an unscoped (admin) key here.
    if let Some(Extension(key)) = validated.as_ref() {
        if key.agent_scope.is_some() {
            return Err((
                StatusCode::FORBIDDEN,
                Json(ErrorResponse {
                    error: "Scoped API keys may not query message status".to_string(),
                    code: "AGENT_SCOPE_DENIED".to_string(),
                    details: None,
                }),
            ));
        }
    }
    match provider.get_message_status(&message_id).await {
        Ok(response) => Ok(Json(response)),
        Err(e) => {
            use crate::types::{CommunicationError, RuntimeError};
            let (status, code) = match &e {
                RuntimeError::Communication(CommunicationError::InvalidFormat(_)) => {
                    (StatusCode::BAD_REQUEST, "INVALID_MESSAGE_ID")
                }
                _ => (StatusCode::NOT_FOUND, "MESSAGE_STATUS_FAILED"),
            };
            Err((
                status,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code: code.to_string(),
                    details: None,
                }),
            ))
        }
    }
}

#[cfg(all(test, feature = "http-api"))]
mod scope_tests {
    use super::*;

    fn key_with_scope(scope: Option<Vec<String>>) -> ValidatedKey {
        ValidatedKey {
            key_id: "test".to_string(),
            agent_scope: scope,
        }
    }

    #[test]
    fn require_admin_rejects_scoped_keys() {
        let k = key_with_scope(Some(vec!["a".into()]));
        assert!(require_admin(Some(&k)).is_err());
    }

    #[test]
    fn require_admin_allows_unscoped_key() {
        let k = key_with_scope(None);
        assert!(require_admin(Some(&k)).is_ok());
    }

    #[test]
    fn require_admin_allows_legacy_env_token() {
        assert!(require_admin(None).is_ok());
    }

    #[test]
    fn check_agent_access_blocks_out_of_scope() {
        let target = AgentId::new();
        let k = key_with_scope(Some(vec![uuid::Uuid::new_v4().to_string()]));
        assert!(check_agent_access(Some(&k), &target).is_err());
    }

    #[test]
    fn check_agent_access_allows_in_scope() {
        let target = AgentId::new();
        let k = key_with_scope(Some(vec![target.0.to_string()]));
        assert!(check_agent_access(Some(&k), &target).is_ok());
    }

    #[test]
    fn scope_filter_returns_none_for_admin() {
        let k = key_with_scope(None);
        assert!(scope_filter(Some(&k)).is_none());
    }

    #[test]
    fn scope_filter_returns_scope_list() {
        let k = key_with_scope(Some(vec!["a".into(), "b".into()]));
        assert_eq!(scope_filter(Some(&k)).unwrap().len(), 2);
    }
}
