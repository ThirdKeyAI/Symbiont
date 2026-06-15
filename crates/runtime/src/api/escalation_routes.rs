//! REST handlers for the held-action escalation queue.

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
