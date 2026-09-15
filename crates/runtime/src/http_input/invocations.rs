//! HTTP retry identities are selected before execution and bound to verified
//! callers. Claim IDs share one project-wide HTTP domain: changing credentials,
//! routing or payload cannot create another execution under an existing ID.

use super::*;
use crate::reasoning::prepared::digest_json;
use serde_json::json;
use uuid::Uuid;

#[cfg(all(test, unix))]
#[path = "invocation_tests.rs"]
mod tests;

#[derive(Clone)]
pub(super) struct AuthenticatedCaller(String);

impl AuthenticatedCaller {
    pub(super) fn static_token(credential: &str) -> Self {
        Self(digest_json(&json!(["static", credential])).expect("finite authentication identity"))
    }

    pub(super) fn jwt(authority: &str, subject: &str, issuer: Option<&str>) -> Option<Self> {
        if subject.is_empty()
            || subject.len() > 512
            || issuer.is_some_and(|s| s.is_empty() || s.len() > 2048)
        {
            return None;
        }
        Some(Self(
            digest_json(&json!(["jwt", authority, issuer, subject])).ok()?,
        ))
    }
}

pub(super) struct HttpInvocation {
    pub(super) id: Uuid,
    caller: AuthenticatedCaller,
    route: String,
}

impl HttpInvocation {
    pub(super) fn from_headers(
        headers: &HeaderMap,
        caller: AuthenticatedCaller,
        route: String,
    ) -> Result<Self, &'static str> {
        let values: Vec<_> = headers.get_all("Idempotency-Key").iter().collect();
        let id = (values.len() == 1)
            .then(|| values[0].to_str().ok())
            .flatten()
            .filter(|s| s.len() == 36)
            .and_then(|s| Uuid::parse_str(s).ok());
        match id {
            Some(id) => Ok(Self { id, caller, route }),
            None => Err("Supply exactly one Idempotency-Key header containing a UUID; reuse it for retries."),
        }
    }

    #[cfg(test)]
    pub(super) fn test_request() -> Self {
        Self {
            id: Uuid::new_v4(),
            caller: AuthenticatedCaller::static_token("fixture"),
            route: "/webhook".into(),
        }
    }
}

#[derive(Debug)]
pub(super) struct HttpInvocationReply {
    status: StatusCode,
    body: Value,
    replayed: bool,
}

impl HttpInvocationReply {
    pub(super) fn into_response(
        mut self,
        id: Uuid,
        config: Option<&ResponseControlConfig>,
    ) -> Result<Response, StatusCode> {
        self.body["invocation_id"] = json!(id);
        self.body["replayed"] = json!(self.replayed);
        let mut response = if self.status == StatusCode::OK {
            super::format_success_response(self.body, config)?
        } else {
            (self.status, Json(self.body)).into_response()
        };
        response
            .headers_mut()
            .insert("Idempotency-Key", id.to_string().parse().unwrap());
        response.headers_mut().insert(
            "Idempotency-Replayed",
            if self.replayed { "true" } else { "false" }
                .parse()
                .unwrap(),
        );
        response
            .headers_mut()
            .insert("Cache-Control", "no-store".parse().unwrap());
        Ok(response)
    }

    #[cfg(unix)]
    fn unresolved(audit: Option<crate::reasoning::run_audit::RunAuditReference>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            body: json!({"status":"unresolved", "audit":audit, "error":"The original outcome requires reconciliation; no work repeated."}),
            replayed: false,
        }
    }

    #[cfg(unix)]
    fn existing(
        existing: crate::reasoning::invocation::ExistingInvocation,
        replayed: bool,
    ) -> Result<Self, RuntimeError> {
        use crate::reasoning::invocation::ExistingInvocation;
        match existing {
            ExistingInvocation::InProgress => Ok(Self {
                status: StatusCode::CONFLICT,
                body: json!({"status":"in_progress", "error":"The original invocation is still owned; no work repeated."}),
                replayed: false,
            }),
            ExistingInvocation::Unresolved { audit } => Ok(Self::unresolved(audit)),
            ExistingInvocation::Reconciled { audit, resolution } => Ok(Self {
                status: StatusCode::CONFLICT,
                body: json!({"status":"reconciled", "audit":audit, "resolution":resolution, "work_repeated":false}),
                replayed: false,
            }),
            ExistingInvocation::Recorded { audit, result } => {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Saved {
                    completed: bool,
                    body: Value,
                }
                let mut saved: Saved = serde_json::from_value(result)
                    .map_err(|_| RuntimeError::Internal("Invalid saved HTTP result".into()))?;
                if !saved.body.is_object() {
                    return Err(RuntimeError::Internal(
                        "Invalid saved HTTP result body".into(),
                    ));
                }
                saved.body["audit"] = json!(audit);
                Ok(Self {
                    status: if saved.completed {
                        StatusCode::OK
                    } else {
                        StatusCode::UNPROCESSABLE_ENTITY
                    },
                    body: saved.body,
                    replayed,
                })
            }
        }
    }
}

#[cfg(unix)]
#[allow(clippy::too_many_arguments)]
pub(super) async fn invoke_agent(
    runtime: Option<&crate::AgentRuntime>,
    agent_id: AgentId,
    input_data: Value,
    inference_provider: Option<Arc<dyn InferenceProvider>>,
    project_root: &Path,
    executor: Arc<dyn ActionExecutor>,
    executor_is_override: bool,
    policy_gate: Arc<dyn ReasoningPolicyGate>,
    circuit_breakers: Arc<CircuitBreakerRegistry>,
    request: HttpInvocation,
) -> Result<HttpInvocationReply, RuntimeError> {
    use crate::reasoning::invocation::{open_invocation, OpenInvocation};
    // Read the trusted registry before looking up an old result. Runtime IDs
    // change on restart; registered source identity remains stable.
    let registered =
        match runtime {
            Some(runtime) => Some(runtime.scheduler.get_agent_config(agent_id).ok_or_else(
                || RuntimeError::Internal("Selected HTTP agent is unavailable".into()),
            )?),
            None => None,
        };
    let target = match &registered {
        Some(agent) => {
            json!({"name":agent.name, "source":agent.dsl_source, "mode":agent.execution_mode,
            "tier":agent.security_tier, "resources":agent.resource_limits})
        }
        None => json!({"sdk_agent_id":agent_id}),
    };
    let identity = json!({"version":1, "caller":request.caller.0, "route":request.route, "target":target, "input":input_data});
    let invocation = match open_invocation(
        project_root,
        "http:input:v1",
        request.id,
        &identity,
        agent_id,
    )
    .await
    {
        Ok(OpenInvocation::Existing(existing)) => {
            return HttpInvocationReply::existing(existing, true)
        }
        Ok(OpenInvocation::Fresh(invocation)) => invocation,
        Err(error) if error == "invocation ID conflicts with a different request" => {
            return Ok(HttpInvocationReply {
                status: StatusCode::CONFLICT,
                body: json!({"status":"conflict", "error":"Invocation ID is already bound to another request or caller; no work repeated."}),
                replayed: false,
            })
        }
        Err(error) => {
            tracing::error!(%error, "Required HTTP invocation storage unavailable");
            return Ok(HttpInvocationReply {
                status: StatusCode::SERVICE_UNAVAILABLE,
                body: json!({"status":"unavailable", "error":"Invocation storage could not authorize execution; preserve the ID and inspect runtime storage."}),
                replayed: false,
            });
        }
    };
    let audit = invocation.audit().clone();
    let project = project_root.to_owned();
    let cancellation = tokio_util::sync::CancellationToken::new();
    let _cancel_on_drop = cancellation.clone().drop_guard();
    // The retained owner holds the durable claim through setup, execution,
    // cleanup and result persistence, including when the HTTP caller disconnects.
    tokio::spawn(async move {
        match super::execute_http_run(registered, agent_id, input_data, inference_provider, &project,
            executor, executor_is_override, policy_gate, circuit_breakers, invocation, cancellation).await {
            Ok(existing) => HttpInvocationReply::existing(existing, false),
            Err(error) => {
                tracing::error!(%error, run_id = %audit.run_id, "HTTP invocation remains unresolved");
                Ok(HttpInvocationReply::unresolved(Some(audit)))
            }
        }
    }).await.map_err(|error| RuntimeError::Internal(format!("Governed execution owner failed: {error}")))?
}
