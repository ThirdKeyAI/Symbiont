//! REST handlers for the held-action escalation queue.

// clippy::result_large_err — see the note in `server.rs`: the axum
// `(StatusCode, Json<ErrorResponse>)` error pair is the framework's idiom.
#![allow(clippy::result_large_err)]
#[cfg(feature = "http-api")]
use axum::{
    extract::{Extension, Path},
    http::StatusCode,
    Json,
};
#[cfg(feature = "http-api")]
use serde::Deserialize;
#[cfg(feature = "http-api")]
use std::sync::Arc;

#[cfg(feature = "http-api")]
use super::api_keys::ValidatedKey;
#[cfg(feature = "http-api")]
use super::types::ErrorResponse;
#[cfg(feature = "http-api")]
use crate::escalation::{Approver, Decision, EscalationQueue, HeldAction, ResolveError, Surface};

#[cfg(feature = "http-api")]
#[derive(Debug, Deserialize, Default)]
pub struct ResolveBody {
    pub reason: Option<String>,
}

#[cfg(feature = "http-api")]
pub(crate) async fn list_pending_inner(q: &EscalationQueue) -> Vec<HeldAction> {
    q.list_pending_async().await
}

#[cfg(feature = "http-api")]
pub(crate) async fn resolve_inner(
    q: &EscalationQueue,
    id: &str,
    decision: Decision,
    approver: Approver,
) -> Result<(), ResolveError> {
    q.resolve_async(id, decision, approver).await
}

#[cfg(feature = "http-api")]
fn approver_from_key(validated: &Option<Extension<ValidatedKey>>) -> Approver {
    let id = validated
        .as_ref()
        .map(|v| v.key_id.clone())
        .unwrap_or_else(|| "operator".into());
    Approver {
        surface: Surface::Rest,
        id: id.clone(),
        display: id,
    }
}

/// List all pending held actions awaiting approval.
#[cfg(feature = "http-api")]
pub async fn list_approvals(
    Extension(queue): Extension<Arc<EscalationQueue>>,
    _validated: Option<Extension<ValidatedKey>>,
) -> Json<Vec<HeldAction>> {
    Json(list_pending_inner(&queue).await)
}

/// Approve a held action by ID.
#[cfg(feature = "http-api")]
pub async fn approve(
    Extension(queue): Extension<Arc<EscalationQueue>>,
    Path(id): Path<String>,
    validated: Option<Extension<ValidatedKey>>,
    body: Option<Json<ResolveBody>>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    // Resolving a held action is the human-in-the-loop control that policy falls
    // back on, so it is admin-only. Without this an agent-scoped key could
    // approve an action held on its own behalf.
    super::routes::require_admin(validated.as_deref())?;
    let reason = body.and_then(|b| b.0.reason);
    do_resolve(
        queue,
        id,
        Decision::Approve { reason },
        approver_from_key(&validated),
    )
    .await
}

/// Deny a held action by ID.
#[cfg(feature = "http-api")]
pub async fn deny(
    Extension(queue): Extension<Arc<EscalationQueue>>,
    Path(id): Path<String>,
    validated: Option<Extension<ValidatedKey>>,
    body: Option<Json<ResolveBody>>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    // Admin-only for the same reason as `approve`: a scoped key must not be able
    // to resolve a held action.
    super::routes::require_admin(validated.as_deref())?;
    let reason = body.and_then(|b| b.0.reason);
    do_resolve(
        queue,
        id,
        Decision::Deny { reason },
        approver_from_key(&validated),
    )
    .await
}

#[cfg(feature = "http-api")]
async fn do_resolve(
    queue: Arc<EscalationQueue>,
    id: String,
    decision: Decision,
    approver: Approver,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    match resolve_inner(&queue, &id, decision, approver).await {
        Ok(()) => Ok(StatusCode::OK),
        Err(ResolveError::NotFound) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Held action not found".into(),
                code: "not_found".into(),
                details: None,
            }),
        )),
        Err(ResolveError::AlreadyResolved) => Err((
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: "Held action already resolved".into(),
                code: "already_resolved".into(),
                details: None,
            }),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::api_keys::ValidatedKey;

    fn scoped_key(scope: Option<Vec<String>>) -> ValidatedKey {
        ValidatedKey {
            key_id: "k1".to_string(),
            agent_scope: scope,
        }
    }

    /// Resolving a held action is the control policy falls back on. An
    /// agent-scoped key must not be able to approve — otherwise an agent whose
    /// action was held can release it with its own credential.
    #[test]
    fn scoped_keys_cannot_resolve_held_actions() {
        let scoped = scoped_key(Some(vec!["agent-a".to_string()]));
        assert!(
            crate::api::routes::require_admin(Some(&scoped)).is_err(),
            "a scoped key must be refused"
        );

        // An unscoped (admin) key still can.
        let admin = scoped_key(None);
        assert!(crate::api::routes::require_admin(Some(&admin)).is_ok());
    }

    use crate::escalation::{EscalationQueue, EscalationRequest, HeldActionKind};
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn list_returns_pending_and_approve_resolves() {
        let q = Arc::new(EscalationQueue::new());
        let q2 = q.clone();
        let h = tokio::spawn(async move {
            q2.enqueue(
                EscalationRequest {
                    agent_id: "a".into(),
                    kind: HeldActionKind::ToolCall,
                    summary: "s".into(),
                    reason: "r".into(),
                    context_snapshot: None,
                },
                Duration::from_secs(5),
            )
            .await
        });
        let id = loop {
            let p = q.list_pending_async().await;
            if let Some(x) = p.first() {
                break x.id.clone();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        };
        let listed = list_pending_inner(&q).await;
        assert_eq!(listed.len(), 1);
        let res = resolve_inner(
            &q,
            &id,
            crate::escalation::Decision::Approve { reason: None },
            crate::escalation::Approver {
                surface: crate::escalation::Surface::Rest,
                id: "op".into(),
                display: "op".into(),
            },
        )
        .await;
        assert!(res.is_ok());
        let decision = h.await.unwrap();
        assert!(matches!(
            decision,
            crate::escalation::Decision::Approve { .. }
        ));
    }
}
