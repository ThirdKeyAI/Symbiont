//! Durable caller identities are claimed before queue admission. One retained
//! owner covers queued work, execution, cleanup and verified result persistence.

use super::{
    task_manager::{TaskAudit, TaskCompletion, TaskHandle, TaskStatus},
    DefaultAgentScheduler,
};
use crate::{
    reasoning::{
        invocation::{open_invocation, ExistingInvocation, OpenInvocation},
        loop_types::JournalWriter,
        run_audit::RunAuditReference,
    },
    types::AgentConfig,
};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

/// Supplied by a trusted caller adapter after authentication. The context binds
/// the caller and route; it must never come from model-selected arguments.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationIdentity {
    pub id: Uuid,
    pub context: Value,
}

/// Trusted entry-point checks run while the fresh invocation claim is owned,
/// before the queue can dispatch inference or tool effects.
#[async_trait::async_trait]
pub trait InvocationAdmissionGate: Send + Sync {
    async fn check(&self, audit: &RunAuditReference) -> Result<(), String>;
}

#[derive(Debug)]
pub enum Admission {
    Queued {
        handle: TaskHandle,
        audit: RunAuditReference,
    },
    Existing(ExistingInvocation),
}

#[derive(Clone)]
pub(crate) struct ClaimedJournal {
    pub writer: Arc<dyn JournalWriter>,
    pub audit: RunAuditReference,
}
impl std::fmt::Debug for ClaimedJournal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaimedJournal")
            .field("audit", &self.audit)
            .finish_non_exhaustive()
    }
}

impl DefaultAgentScheduler {
    pub(super) async fn admit_identified(
        &self,
        config: AgentConfig,
        input: Value,
        identity: InvocationIdentity,
        gate: Option<Arc<dyn InvocationAdmissionGate>>,
    ) -> Result<Admission, String> {
        // Bind the immutable requested configuration, not remaining queue time
        // or a newly allocated execution ID. Agent IDs remain part of authority.
        let request =
            json!({"version": 1, "context": identity.context, "target": config, "input": input});
        let claim = match open_invocation(
            self.task_manager.invocation_project()?,
            "scheduler:v1",
            identity.id,
            &request,
            config.id,
        )
        .await?
        {
            OpenInvocation::Existing(existing) => return Ok(Admission::Existing(existing)),
            OpenInvocation::Fresh(claim) => claim,
        };
        let audit = claim.audit().clone();
        if let Some(gate) = gate {
            gate.check(&audit).await?;
        }
        let public = TaskHandle::with_run_id(config.id, audit.run_id);
        let task = self
            .enqueue_invocation(
                config,
                input,
                Some(ClaimedJournal {
                    writer: claim.journal(),
                    audit: audit.clone(),
                }),
            )
            .await
            .map_err(|e| e.to_string())?;
        let returned = public.clone();
        let saved_audit = audit.clone();
        let latest_runs = self.latest_runs.clone();
        // There is no suspension between enqueue returning and installing the
        // owner. Losing the HTTP response cannot orphan or duplicate queued work.
        tokio::spawn(async move {
            struct CancelOnDrop(TaskHandle);
            impl Drop for CancelOnDrop {
                fn drop(&mut self) {
                    self.0.cancel();
                }
            }
            let guard = CancelOnDrop(task);
            let mut completion = tokio::select! {
                result = guard.0.wait() => result,
                _ = public.cancelled() => { guard.0.cancel(); guard.0.wait().await }
            };
            completion.audit = Some(TaskAudit {
                path: saved_audit.path.clone(),
                public_key: saved_audit.public_key.clone(),
            });
            let outcome = match serde_json::to_value(&completion) {
                Ok(value) => claim.finish(value).await,
                Err(error) => Err(error.to_string()),
            };
            match outcome {
                Ok(ExistingInvocation::Recorded { .. }) => {}
                result => {
                    completion.status = TaskStatus::Unresolved;
                    completion.output = None;
                    let detail = match result {
                        Err(error) => error,
                        _ => "the original journal requires reconciliation".into(),
                    };
                    completion.error = Some(format!(
                        "{}; no invocation retry permitted: {detail}",
                        completion
                            .error
                            .unwrap_or_else(|| "scheduled outcome is unconfirmed".into())
                    ));
                }
            }
            public.finish(completion);
            if let Some(mut latest) = latest_runs.get_mut(&public.agent_id()) {
                if latest.run_id() == public.run_id() {
                    *latest = public;
                }
            }
        });
        Ok(Admission::Queued {
            handle: returned,
            audit,
        })
    }
}

/// Validate cached payload identity before exposing a saved completion.
pub fn recorded_completion(
    audit: &RunAuditReference,
    result: Value,
) -> Result<TaskCompletion, String> {
    let mut completion: TaskCompletion =
        serde_json::from_value(result).map_err(|e| e.to_string())?;
    if completion.run_id != audit.run_id
        || matches!(
            completion.status,
            TaskStatus::Pending | TaskStatus::Running | TaskStatus::Unresolved
        )
    {
        return Err("invalid recorded scheduled completion".into());
    }
    completion.audit = Some(TaskAudit {
        path: audit.path.clone(),
        public_key: audit.public_key.clone(),
    });
    Ok(completion)
}

#[cfg(test)]
#[path = "invocation_tests.rs"]
mod tests;
