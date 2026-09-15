//! In-memory held-action escalation queue.
//!
//! Source-agnostic (any layer enqueues) and resolver-agnostic (TUI/REST/chat
//! resolve). `enqueue` blocks the caller until a human resolves the action or a
//! timeout fires (fail-closed deny). The queue is cheap to `clone` (Arc inside).

use chrono::{DateTime, Utc};
use parking_lot::Mutex as StateMutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{oneshot, Mutex};
use tokio::time::Instant;

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
    Terminal,
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
    #[error("held action expired")]
    Expired,
}

/// Notification delivery must use nonblocking, cancellation-safe I/O. The queue
/// polls notifiers concurrently and drops their futures on resolution, expiry,
/// or caller cancellation; implementations must not leave detached input readers.
#[async_trait::async_trait]
pub trait EscalationNotifier: Send + Sync {
    async fn notify(&self, action: &HeldAction);
}

/// An audit record emitted after every successful escalation resolution.
#[derive(Debug, Clone, Serialize)]
pub struct AuditEvent {
    pub escalation_id: EscalationId,
    pub agent_id: String,
    pub decision: Decision,
    pub approver: Approver,
    pub at: DateTime<Utc>,
}

/// Resolution evidence travels with the decision, rather than depending on an
/// optional notification callback completing before the waiting gate resumes.
pub(crate) struct ResolvedDecision {
    pub(crate) decision: Decision,
    pub(crate) resolution: Option<AuditEvent>,
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
    deadline: Instant,
    tx: Option<oneshot::Sender<ResolvedDecision>>,
}

/// The waiting future owns its entry even while a notifier is pending. Cleanup
/// is synchronous so cancellation cannot depend on another task being scheduled.
struct PendingEntry {
    inner: Arc<StateMutex<HashMap<EscalationId, Entry>>>,
    id: EscalationId,
}

impl Drop for PendingEntry {
    fn drop(&mut self) {
        self.inner.lock().remove(&self.id);
    }
}

impl Entry {
    fn expired(&self) -> bool {
        Instant::now() >= self.deadline || Utc::now() >= self.action.expires_at
    }
}

#[derive(Clone)]
pub struct EscalationQueue {
    // No state lock is held across an await or a notifier/audit callback.
    inner: Arc<StateMutex<HashMap<EscalationId, Entry>>>,
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
            inner: Arc::new(StateMutex::new(HashMap::new())),
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

    /// Register a held action and wait until resolved or `timeout` elapses,
    /// including notification delivery. Dropping the future removes the action.
    /// On timeout, fail closed with `Decision::Deny { reason: Some("timeout") }`.
    pub async fn enqueue(&self, req: EscalationRequest, timeout: Duration) -> Decision {
        self.enqueue_resolved(req, timeout).await.decision
    }

    pub(crate) async fn enqueue_resolved(
        &self,
        req: EscalationRequest,
        timeout: Duration,
    ) -> ResolvedDecision {
        let deny_timeout = || ResolvedDecision {
            decision: Decision::Deny {
                reason: Some("timeout".to_string()),
            },
            resolution: None,
        };
        let now = Utc::now();
        let bounds = Instant::now().checked_add(timeout).zip(
            chrono::Duration::from_std(timeout)
                .ok()
                .and_then(|duration| now.checked_add_signed(duration)),
        );
        let Some((deadline, expires_at)) = bounds.filter(|_| !timeout.is_zero()) else {
            return deny_timeout();
        };
        let id = self.next_id();
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
        let (tx, mut rx) = oneshot::channel();
        let _pending = PendingEntry {
            inner: self.inner.clone(),
            id: id.clone(),
        };
        {
            let mut map = self.inner.lock();
            map.insert(
                id.clone(),
                Entry {
                    action: action.clone(),
                    deadline,
                    tx: Some(tx),
                },
            );
        }
        let notifications = async {
            let notifiers = self.notifiers.lock().await.clone();
            futures::future::join_all(notifiers.iter().map(|n| n.notify(&action))).await;
        };
        let decision = async {
            tokio::select! {
                biased;
                decision = &mut rx => decision,
                () = notifications => rx.await,
            }
        };
        tokio::select! {
            // An elapsed deadline wins even if a decision is also ready.
            biased;
            () = tokio::time::sleep_until(deadline) => deny_timeout(),
            decision = decision => {
                if Instant::now() >= deadline || Utc::now() >= expires_at {
                    deny_timeout()
                } else {
                    decision.unwrap_or_else(|_| deny_timeout())
                }
            },
        }
    }

    /// Async snapshot for callers already on the executor.
    pub async fn list_pending_async(&self) -> Vec<HeldAction> {
        self.inner
            .lock()
            .values()
            .filter(|e| {
                e.action.status == HeldStatus::Pending
                    && !e.expired()
                    && e.tx.as_ref().is_some_and(|tx| !tx.is_closed())
            })
            .map(|e| e.action.clone())
            .collect()
    }

    /// Resolve a held action from an async caller.
    pub async fn resolve_async(
        &self,
        id: &str,
        decision: Decision,
        approver: Approver,
    ) -> Result<(), ResolveError> {
        let ev = {
            let mut map = self.inner.lock();
            Self::resolve_locked(&mut map, id, decision, approver)?
        };
        self.emit_audit(ev).await;
        Ok(())
    }

    fn resolve_locked(
        map: &mut HashMap<EscalationId, Entry>,
        id: &str,
        decision: Decision,
        approver: Approver,
    ) -> Result<AuditEvent, ResolveError> {
        let entry = map.get_mut(id).ok_or(ResolveError::NotFound)?;
        if entry.expired() {
            entry.action.status = HeldStatus::Expired;
            entry.tx.take();
            return Err(ResolveError::Expired);
        }
        let tx = entry.tx.take().ok_or(ResolveError::AlreadyResolved)?;
        if tx.is_closed() {
            return Err(ResolveError::AlreadyResolved);
        }
        entry.action.status = match decision {
            Decision::Approve { .. } => HeldStatus::Approved,
            Decision::Deny { .. } => HeldStatus::Denied,
        };
        let event = AuditEvent {
            escalation_id: id.to_string(),
            agent_id: entry.action.agent_id.clone(),
            decision: decision.clone(),
            approver,
            at: Utc::now(),
        };
        tx.send(ResolvedDecision {
            decision,
            resolution: Some(event.clone()),
        })
        .map_err(|_| ResolveError::AlreadyResolved)?;
        Ok(event)
    }

    async fn emit_audit(&self, ev: AuditEvent) {
        let sink = self.audit.lock().await.clone();
        if let Some(a) = sink {
            a.record(ev).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
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

    struct NotifyDrop(Arc<AtomicUsize>);

    impl Drop for NotifyDrop {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct StalledNotifier(Arc<AtomicUsize>);

    #[async_trait::async_trait]
    impl EscalationNotifier for StalledNotifier {
        async fn notify(&self, _: &HeldAction) {
            let _guard = NotifyDrop(self.0.clone());
            std::future::pending::<()>().await;
        }
    }

    struct CountingNotifier(Arc<AtomicUsize>);

    #[async_trait::async_trait]
    impl EscalationNotifier for CountingNotifier {
        async fn notify(&self, _: &HeldAction) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn notifier_stall_is_bounded_and_late_resolution_is_rejected() {
        let audit = Arc::new(RecordingAudit::default());
        let q = EscalationQueue::new().with_audit(audit.clone());
        let dropped = Arc::new(AtomicUsize::new(0));
        q.subscribe(Arc::new(StalledNotifier(dropped.clone())))
            .await;
        let mut wait = Box::pin(q.enqueue(req("a", "held"), Duration::from_secs(5)));
        assert!(futures::poll!(&mut wait).is_pending());
        let id = q.list_pending_async().await[0].id.clone();
        tokio::time::advance(Duration::from_secs(5)).await;
        // The waiter has not been polled again: resolution must check its own
        // monotonic deadline, regardless of when timeout cleanup gets CPU time.
        assert!(q.list_pending_async().await.is_empty());
        assert_eq!(
            q.resolve_async(&id, Decision::Approve { reason: None }, approver())
                .await,
            Err(ResolveError::Expired)
        );
        assert!(
            matches!(wait.await, Decision::Deny { reason } if reason.as_deref() == Some("timeout"))
        );
        assert!(q.inner.lock().is_empty());
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        assert!(audit.events.lock().unwrap().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn cancelled_wait_removes_state_and_drops_notifications_synchronously() {
        for stalled in [false, true] {
            let audit = Arc::new(RecordingAudit::default());
            let q = EscalationQueue::new().with_audit(audit.clone());
            let dropped = Arc::new(AtomicUsize::new(0));
            if stalled {
                q.subscribe(Arc::new(StalledNotifier(dropped.clone())))
                    .await;
            }
            let mut wait = Box::pin(q.enqueue(req("a", "held"), Duration::from_secs(5)));
            assert!(futures::poll!(&mut wait).is_pending());
            let id = q.list_pending_async().await[0].id.clone();
            drop(wait);
            assert!(q.inner.lock().is_empty());
            assert!(q.list_pending_async().await.is_empty());
            assert_eq!(
                q.resolve_async(&id, Decision::Approve { reason: None }, approver())
                    .await,
                Err(ResolveError::NotFound)
            );
            assert_eq!(dropped.load(Ordering::SeqCst), usize::from(stalled));
            assert!(audit.events.lock().unwrap().is_empty());
        }
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_delivery_does_not_block_other_notifiers_or_operator_decisions() {
        for approve in [true, false] {
            let q = EscalationQueue::new();
            let dropped = Arc::new(AtomicUsize::new(0));
            let delivered = Arc::new(AtomicUsize::new(0));
            q.subscribe(Arc::new(StalledNotifier(dropped.clone())))
                .await;
            q.subscribe(Arc::new(CountingNotifier(delivered.clone())))
                .await;
            let mut wait = Box::pin(q.enqueue(req("a", "held"), Duration::from_secs(5)));
            assert!(futures::poll!(&mut wait).is_pending());
            assert_eq!(delivered.load(Ordering::SeqCst), 1);
            let id = q.list_pending_async().await[0].id.clone();
            let decision = if approve {
                Decision::Approve { reason: None }
            } else {
                Decision::Deny { reason: None }
            };
            q.resolve_async(&id, decision, approver()).await.unwrap();
            assert!(q.list_pending_async().await.is_empty());
            assert!(
                matches!(futures::poll!(&mut wait), std::task::Poll::Ready(d) if matches!(d, Decision::Approve { .. }) == approve)
            );
            assert_eq!(dropped.load(Ordering::SeqCst), 1);
            assert!(q.inner.lock().is_empty());
        }
    }

    #[tokio::test(start_paused = true)]
    async fn deadline_wins_over_a_ready_but_unconsumed_approval() {
        let q = EscalationQueue::new();
        let mut wait = Box::pin(q.enqueue(req("a", "held"), Duration::from_secs(5)));
        assert!(futures::poll!(&mut wait).is_pending());
        let id = q.list_pending_async().await[0].id.clone();
        q.resolve_async(&id, Decision::Approve { reason: None }, approver())
            .await
            .unwrap();
        tokio::time::advance(Duration::from_secs(5)).await;
        assert!(matches!(wait.await, Decision::Deny { .. }));
        assert!(q.inner.lock().is_empty());
    }

    #[tokio::test]
    async fn displayed_wall_clock_expiry_also_rejects_resolution() {
        let q = EscalationQueue::new();
        let mut wait = Box::pin(q.enqueue(req("a", "held"), Duration::from_secs(5)));
        assert!(futures::poll!(&mut wait).is_pending());
        let id = q.list_pending_async().await[0].id.clone();
        q.inner.lock().get_mut(&id).unwrap().action.expires_at = Utc::now();
        assert_eq!(
            q.resolve_async(&id, Decision::Approve { reason: None }, approver())
                .await,
            Err(ResolveError::Expired)
        );
        assert!(matches!(wait.await, Decision::Deny { .. }));
    }

    #[tokio::test]
    async fn invalid_timeouts_fail_closed_without_notifications_or_state() {
        let q = EscalationQueue::new();
        let delivered = Arc::new(AtomicUsize::new(0));
        q.subscribe(Arc::new(CountingNotifier(delivered.clone())))
            .await;
        for timeout in [Duration::ZERO, Duration::MAX] {
            assert!(matches!(
                q.enqueue(req("a", "held"), timeout).await,
                Decision::Deny { .. }
            ));
        }
        assert_eq!(delivered.load(Ordering::SeqCst), 0);
        assert!(q.inner.lock().is_empty());
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
