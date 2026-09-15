//! Authenticated scheduler submission and retry response contracts.

use super::types::WorkflowExecutionRequest;
use crate::types::{
    AgentConfig, AgentId, Capability, ExecutionMode, Priority, ResourceLimits, SecurityTier,
};
use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Clone)]
pub struct AuthenticatedCaller(String);
impl AuthenticatedCaller {
    pub(super) fn fingerprint(&self) -> &str {
        &self.0
    }

    pub(super) fn coordinator_sdk() -> Self {
        Self(
            crate::reasoning::prepared::digest_json(&json!(["trusted-sdk", "coordinator"]))
                .expect("finite SDK caller identity"),
        )
    }

    pub(crate) fn verified(token: &str, key: Option<&super::api_keys::ValidatedKey>) -> Self {
        Self(
            crate::reasoning::prepared::digest_json(&json!([
                "runtime-api",
                token,
                key.map(|k| (&k.key_id, &k.agent_scope))
            ]))
            .expect("finite verified caller identity"),
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AdmissionError {
    #[error("invalid scheduled request: {0}")]
    Invalid(String),
    #[error("schedule admission forbidden: {0}")]
    Forbidden(String),
    #[error("registered agent not found")]
    NotFound,
    #[error("invocation ID conflicts with a different request")]
    Conflict,
    #[error("persistent admission unavailable: {0}")]
    Unavailable(String),
}
impl AdmissionError {
    pub(crate) fn storage(error: String) -> Self {
        if error == "invocation ID conflicts with a different request" {
            Self::Conflict
        } else {
            Self::Unavailable(error)
        }
    }
}

pub enum ExecutionTarget {
    Agent { id: AgentId, input: Value },
    Workflow(WorkflowExecutionRequest),
    Schedule { job_id: String },
}

pub(crate) fn workflow_config(
    request: &WorkflowExecutionRequest,
    default_id: AgentId,
) -> Result<AgentConfig, String> {
    let source = &request.workflow_id;
    if source.len() > 1024 * 1024 {
        return Err("workflow definition exceeds 1 MiB".into());
    }
    let tree = dsl::parse_dsl(source).map_err(|e| e.to_string())?;
    if tree.root_node().has_error() {
        return Err("DSL contains syntax errors".into());
    }
    let metadata = dsl::extract_metadata(&tree, source);
    let selector = metadata
        .get("name")
        .map(|s| serde_json::from_str::<String>(s).unwrap_or_else(|_| s.clone()))
        .or_else(|| dsl::extract_agent_name(&tree, source))
        .ok_or("workflow declares no agent")?;
    let policy = dsl::ExecutionPolicy::parse(source, &selector)?;
    Ok(AgentConfig {
        id: request.agent_id.unwrap_or(default_id),
        name: policy.settings().agent_name.clone(),
        dsl_source: source.clone(),
        execution_mode: ExecutionMode::Ephemeral,
        security_tier: SecurityTier::Tier1,
        resource_limits: ResourceLimits::default(),
        capabilities: vec![Capability::Computation],
        policies: vec![],
        metadata,
        priority: Priority::Normal,
    })
}

pub(crate) async fn submit(
    provider: &dyn super::RuntimeApiProvider,
    caller: Option<AuthenticatedCaller>,
    headers: HeaderMap,
    target: ExecutionTarget,
) -> Response {
    let Some(caller) = caller else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let values: Vec<_> = headers.get_all("Idempotency-Key").iter().collect();
    let id = (values.len() == 1)
        .then(|| values[0].to_str().ok())
        .flatten()
        .filter(|s| s.len() == 36)
        .and_then(|s| Uuid::parse_str(s).ok());
    let Some(id) = id else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"status":"invalid_invocation_id",
            "error":"Supply exactly one Idempotency-Key UUID; reuse it for retries."})),
        )
            .into_response();
    };
    #[cfg(unix)]
    let (status, body) = {
        use crate::{
            reasoning::invocation::ExistingInvocation,
            scheduler::invocations::{recorded_completion, Admission, InvocationIdentity},
        };
        let route = match &target {
            ExecutionTarget::Agent { .. } => "agent.execute",
            ExecutionTarget::Workflow(_) => "workflow.execute",
            ExecutionTarget::Schedule { .. } => "schedule.trigger",
        };
        match provider
            .admit_execution(
                target,
                InvocationIdentity {
                    id,
                    context: json!({"route":route,"caller":caller.0}),
                },
            )
            .await
        {
            Ok(Admission::Queued { handle, audit }) => (
                StatusCode::OK,
                json!({
                    "status":"queued", "execution_id":handle.run_id(), "agent_id":handle.agent_id(), "audit":audit, "replayed":false
                }),
            ),
            Ok(Admission::Existing(ExistingInvocation::InProgress)) => (
                StatusCode::CONFLICT,
                json!({"status":"in_progress", "replayed":false}),
            ),
            Ok(Admission::Existing(ExistingInvocation::Unresolved { audit })) => (
                StatusCode::CONFLICT,
                json!({"status":"unresolved", "audit":audit, "replayed":false}),
            ),
            Ok(Admission::Existing(ExistingInvocation::Reconciled { audit, resolution })) => (
                StatusCode::CONFLICT,
                json!({"status":"reconciled", "audit":audit, "resolution":resolution, "replayed":false}),
            ),
            Ok(Admission::Existing(ExistingInvocation::Recorded { audit, result })) => {
                match recorded_completion(&audit, result) {
                    Ok(completion) => (
                        if completion.status
                            == crate::scheduler::task_manager::TaskStatus::Completed
                        {
                            StatusCode::OK
                        } else {
                            StatusCode::UNPROCESSABLE_ENTITY
                        },
                        json!({"status":if completion.status == crate::scheduler::task_manager::TaskStatus::Completed {"completed"} else {"failed"},
                        "execution_id":completion.run_id,"agent_id":completion.agent_id,"audit":audit,"result":completion,"replayed":true}),
                    ),
                    Err(error) => (
                        StatusCode::CONFLICT,
                        json!({"status":"unresolved","error":error,"audit":audit,"replayed":false}),
                    ),
                }
            }
            Err(AdmissionError::Conflict) => (
                StatusCode::CONFLICT,
                json!({"status":"conflict","replayed":false}),
            ),
            Err(AdmissionError::Invalid(error)) => (
                StatusCode::BAD_REQUEST,
                json!({"status":"invalid_request","error":error,"replayed":false}),
            ),
            Err(AdmissionError::Forbidden(error)) => (
                StatusCode::FORBIDDEN,
                json!({"status":"forbidden","error":error,"replayed":false}),
            ),
            Err(AdmissionError::NotFound) => (
                StatusCode::NOT_FOUND,
                json!({"status":"not_found","replayed":false}),
            ),
            Err(AdmissionError::Unavailable(error)) => (
                StatusCode::SERVICE_UNAVAILABLE,
                json!({"status":"unavailable","error":error,"replayed":false}),
            ),
        }
    };
    #[cfg(not(unix))]
    let (status, body) = {
        let _ = (provider, caller, target);
        (
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"status":"unavailable","error":"protected scheduler admission requires Unix","replayed":false}),
        )
    };
    let mut body = body;
    body["invocation_id"] = json!(id);
    let replayed = body["replayed"].as_bool().unwrap_or(false);
    let mut response = (status, Json(body)).into_response();
    response.headers_mut().insert(
        "Idempotency-Key",
        id.to_string().parse().expect("UUID header"),
    );
    response.headers_mut().insert(
        "Idempotency-Replayed",
        if replayed { "true" } else { "false" }
            .parse()
            .expect("boolean header"),
    );
    response
        .headers_mut()
        .insert("Cache-Control", "no-store".parse().expect("static header"));
    response
}
