//! In-memory held-action escalation queue.
//!
//! Source-agnostic (any layer enqueues) and resolver-agnostic (TUI/REST/chat
//! resolve). `enqueue` blocks the caller until a human resolves the action or a
//! timeout fires (fail-closed deny). The queue is cheap to `clone` (Arc inside).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{oneshot, Mutex};

/// Unique identifier for a held action; 16 hex chars of CSPRNG entropy (unguessable).
pub type EscalationId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeldActionKind {
    ToolCall,
    Delegate,
    Schedule,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    Tui,
    Rest,
    Chat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeldStatus {
    Pending,
    Approved,
    Denied,
    Expired,
}

#[derive(Debug, Clone)]
pub struct EscalationRequest {
    pub agent_id: String,
    pub kind: HeldActionKind,
    pub summary: String,
    pub reason: String,
    pub context_snapshot: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeldAction {
    pub id: EscalationId,
    pub agent_id: String,
    pub kind: HeldActionKind,
    pub summary: String,
    pub reason: String,
    pub context_snapshot: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub status: HeldStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Decision {
    Approve { reason: Option<String> },
    Deny { reason: Option<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Approver {
    pub surface: Surface,
    pub id: String,
    pub display: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ResolveError {
    #[error("held action not found")]
    NotFound,
    #[error("held action already resolved")]
    AlreadyResolved,
}

#[async_trait::async_trait]
pub trait EscalationNotifier: Send + Sync {
    async fn notify(&self, action: &HeldAction);
}

/// An audit record emitted after every successful escalation resolution.
#[derive(Debug, Clone)]
pub struct AuditEvent {
    pub escalation_id: EscalationId,
    pub agent_id: String,
    pub decision: Decision,
    pub approver: Approver,
    pub at: DateTime<Utc>,
}

/// Sink that receives audit events produced by the escalation queue.
///
/// Wire a concrete implementation (e.g. one that writes to the reasoning
/// journal) via [`EscalationQueue::with_audit`].
#[async_trait::async_trait]
pub trait EscalationAudit: Send + Sync {
    async fn record(&self, event: AuditEvent);
}

struct Entry {
    action: HeldAction,
    tx: Option<oneshot::Sender<Decision>>,
}

#[derive(Clone)]
pub struct EscalationQueue {
    inner: Arc<Mutex<HashMap<EscalationId, Entry>>>,
    notifiers: Arc<Mutex<Vec<Arc<dyn EscalationNotifier>>>>,
    audit: Arc<Mutex<Option<Arc<dyn EscalationAudit>>>>,
}

impl Default for EscalationQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl EscalationQueue {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            notifiers: Arc::new(Mutex::new(Vec::new())),
            audit: Arc::new(Mutex::new(None)),
        }
    }

    /// Attach an audit sink. Returns `self` for builder-style construction.
    pub fn with_audit(self, audit: Arc<dyn EscalationAudit>) -> Self {
        if let Ok(mut g) = self.audit.try_lock() {
            *g = Some(audit);
        }
        self
    }

    pub async fn subscribe(&self, notifier: Arc<dyn EscalationNotifier>) {
        self.notifiers.lock().await.push(notifier);
    }

    /// Returns a fresh, unguessable ID: 64 bits of CSPRNG entropy as 16 hex
    /// chars. Unguessable IDs matter because resolving is an authorization-bearing
    /// action — a sequential counter would let an authorized approver resolve held
    /// actions they never actually saw announced by guessing the next id.
    fn next_id(&self) -> EscalationId {
        use rand::RngCore;
        let mut bytes = [0u8; 8];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }

    /// Register a held action and block until resolved or `timeout` elapses.
    /// On timeout, fail closed with `Decision::Deny { reason: Some("timeout") }`.
    pub async fn enqueue(&self, req: EscalationRequest, timeout: Duration) -> Decision {
        let id = self.next_id();
        let now = Utc::now();
        let expires_at = now
            + chrono::Duration::from_std(timeout)
                .unwrap_or_else(|_| chrono::Duration::seconds(120));
        let action = HeldAction {
            id: id.clone(),
            agent_id: req.agent_id,
            kind: req.kind,
            summary: req.summary,
            reason: req.reason,
            context_snapshot: req.context_snapshot,
            created_at: now,
            expires_at,
            status: HeldStatus::Pending,
        };
        let (tx, rx) = oneshot::channel();
        {
            let mut map = self.inner.lock().await;
            map.insert(
                id.clone(),
                Entry {
                    action: action.clone(),
                    tx: Some(tx),
                },
            );
        }
        // Snapshot notifiers before iterating so the mutex is not held across `.await`.
        let notifiers: Vec<Arc<dyn EscalationNotifier>> =
            self.notifiers.lock().await.iter().cloned().collect();
        for n in notifiers {
            n.notify(&action).await;
        }

        let (decision, timed_out) = match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(d)) => (d, false),
            Ok(Err(_)) | Err(_) => (
                Decision::Deny {
                    reason: Some("timeout".to_string()),
                },
                true,
            ),
        };

        let mut map = self.inner.lock().await;
        if timed_out {
            if let Some(e) = map.get_mut(&id) {
                e.action.status = HeldStatus::Expired;
            }
        }
        map.remove(&id);
        decision
    }

    /// Blocking snapshot for sync callers (NOT on the async executor).
    /// Used by later gate-path tasks that call from a blocking context.
    ///
    /// # Panics
    ///
    /// Panics if called from an async context. Use [`list_pending_async`](Self::list_pending_async)
    /// instead.
    #[allow(dead_code)]
    pub fn list_pending(&self) -> Vec<HeldAction> {
        debug_assert!(
            tokio::runtime::Handle::try_current().is_err(),
            "EscalationQueue::list_pending must not be called from an async context; use list_pending_async"
        );
        let map = self.inner.blocking_lock();
        map.values().map(|e| e.action.clone()).collect()
    }

    /// Async snapshot for callers already on the executor.
    pub async fn list_pending_async(&self) -> Vec<HeldAction> {
        self.inner
            .lock()
            .await
            .values()
            .map(|e| e.action.clone())
            .collect()
    }

    /// Resolve a held action from a sync (blocking) caller.
    /// Used by later gate-path tasks that call from a blocking context.
    ///
    /// # Panics
    ///
    /// Panics if called from an async context. Use [`resolve_async`](Self::resolve_async)
    /// instead.
    #[allow(dead_code)]
    pub fn resolve(
        &self,
        id: &str,
        decision: Decision,
        approver: Approver,
    ) -> Result<(), ResolveError> {
        debug_assert!(
            tokio::runtime::Handle::try_current().is_err(),
            "EscalationQueue::resolve must not be called from an async context; use resolve_async"
        );
        let decision_clone = decision.clone();
        let approver_clone = approver.clone();
        let mut map = self.inner.blocking_lock();
        let agent_id = Self::resolve_locked(&mut map, id, decision)?;
        drop(map);
        let ev = AuditEvent {
            escalation_id: id.to_string(),
            agent_id,
            decision: decision_clone,
            approver: approver_clone,
            at: Utc::now(),
        };
        self.emit_audit_spawn(ev);
        Ok(())
    }

    /// Resolve a held action from an async caller.
    pub async fn resolve_async(
        &self,
        id: &str,
        decision: Decision,
        approver: Approver,
    ) -> Result<(), ResolveError> {
        let decision_clone = decision.clone();
        let approver_clone = approver.clone();
        let agent_id = {
            let mut map = self.inner.lock().await;
            Self::resolve_locked(&mut map, id, decision)?
        };
        let ev = AuditEvent {
            escalation_id: id.to_string(),
            agent_id,
            decision: decision_clone,
            approver: approver_clone,
            at: Utc::now(),
        };
        self.emit_audit(ev).await;
        Ok(())
    }

    fn resolve_locked(
        map: &mut HashMap<EscalationId, Entry>,
        id: &str,
        decision: Decision,
    ) -> Result<String, ResolveError> {
        let entry = map.get_mut(id).ok_or(ResolveError::NotFound)?;
        let tx = entry.tx.take().ok_or(ResolveError::AlreadyResolved)?;
        let agent_id = entry.action.agent_id.clone();
        entry.action.status = match decision {
            Decision::Approve { .. } => HeldStatus::Approved,
            Decision::Deny { .. } => HeldStatus::Denied,
        };
        tx.send(decision)
            .map_err(|_| ResolveError::AlreadyResolved)?;
        Ok(agent_id)
    }

    async fn emit_audit(&self, ev: AuditEvent) {
        let sink = self.audit.lock().await.clone();
        if let Some(a) = sink {
            a.record(ev).await;
        }
    }

    fn emit_audit_spawn(&self, ev: AuditEvent) {
        let audit = self.audit.clone();
        tokio::spawn(async move {
            let sink = audit.lock().await.clone();
            if let Some(a) = sink {
                a.record(ev).await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;

    fn req(agent: &str, summary: &str) -> EscalationRequest {
        EscalationRequest {
            agent_id: agent.to_string(),
            kind: HeldActionKind::ToolCall,
            summary: summary.to_string(),
            reason: "test".to_string(),
            context_snapshot: None,
        }
    }

    fn approver() -> Approver {
        Approver {
            surface: Surface::Tui,
            id: "op1".into(),
            display: "Operator One".into(),
        }
    }

    #[tokio::test]
    async fn enqueue_then_approve_resolves_with_approve() {
        let q = EscalationQueue::new();
        let q2 = q.clone();
        let handle = tokio::spawn(async move {
            q2.enqueue(req("a", "do thing"), Duration::from_secs(5))
                .await
        });
        let id = loop {
            let pending = q.list_pending_async().await;
            if let Some(h) = pending.first() {
                break h.id.clone();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        };
        q.resolve_async(&id, Decision::Approve { reason: None }, approver())
            .await
            .unwrap();
        let decision = handle.await.unwrap();
        assert!(matches!(decision, Decision::Approve { .. }));
        assert!(q.list_pending_async().await.is_empty());
    }

    #[tokio::test]
    async fn timeout_denies_fail_closed() {
        let q = EscalationQueue::new();
        let decision = q.enqueue(req("a", "slow"), Duration::from_millis(20)).await;
        match decision {
            Decision::Deny { reason } => assert!(reason.as_deref() == Some("timeout")),
            _ => panic!("expected fail-closed deny on timeout"),
        }
        assert!(q.list_pending_async().await.is_empty());
    }

    #[tokio::test]
    async fn double_resolve_is_already_resolved() {
        let q = EscalationQueue::new();
        let q2 = q.clone();
        let h =
            tokio::spawn(async move { q2.enqueue(req("a", "x"), Duration::from_secs(5)).await });
        let id = loop {
            if let Some(h) = q.list_pending_async().await.first() {
                break h.id.clone();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        };
        q.resolve_async(&id, Decision::Approve { reason: None }, approver())
            .await
            .unwrap();
        let second = q
            .resolve_async(&id, Decision::Deny { reason: None }, approver())
            .await;
        assert!(matches!(second, Err(ResolveError::AlreadyResolved)));
        let _ = h.await.unwrap();
    }

    #[tokio::test]
    async fn resolve_unknown_id_errors() {
        let q = EscalationQueue::new();
        let r = q
            .resolve_async("nope", Decision::Approve { reason: None }, approver())
            .await;
        assert!(matches!(r, Err(ResolveError::NotFound)));
    }

    #[derive(Default)]
    struct RecordingAudit {
        events: StdMutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl EscalationAudit for RecordingAudit {
        async fn record(&self, ev: AuditEvent) {
            self.events.lock().unwrap().push(format!(
                "{}:{:?}:{}",
                ev.escalation_id, ev.decision, ev.approver.id
            ));
        }
    }

    #[tokio::test]
    async fn resolve_writes_audit_event() {
        let audit = Arc::new(RecordingAudit::default());
        let q = EscalationQueue::new().with_audit(audit.clone());
        let q2 = q.clone();
        let h =
            tokio::spawn(async move { q2.enqueue(req("a", "x"), Duration::from_secs(5)).await });
        let id = loop {
            if let Some(h) = q.list_pending_async().await.first() {
                break h.id.clone();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        };
        q.resolve_async(&id, Decision::Approve { reason: None }, approver())
            .await
            .unwrap();
        let _ = h.await.unwrap();
        // give the spawned audit task a moment if you use spawn; if you await inline, no sleep needed
        tokio::time::sleep(Duration::from_millis(20)).await;
        let events = audit.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].contains("op1"));
    }
}
