//! Lifecycle registry owning the SessionMonitor: opens sessions, tracks status
//! and a deadline, and aborts expired ones.

use super::RoleBinding;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use symbi_session::monitor::{SessionId, SessionMonitor};
use symbi_session::Global;

/// Status of a session tracked by the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    Running,
    Complete,
    Aborted,
}

/// Errors produced by the registry.
#[derive(Debug)]
pub enum RegistryError {
    Establish(String),
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::Establish(s) => write!(f, "session establish failed: {s}"),
        }
    }
}

impl std::error::Error for RegistryError {}

struct Meta {
    deadline: Instant,
    status: SessionStatus,
}

/// Registry that owns the `SessionMonitor`, opens sessions (minting an id and
/// establishing projected FSMs), tracks lifecycle status and a deadline, and
/// aborts sessions that have passed their deadline.
pub struct SessionRegistry {
    monitor: Arc<SessionMonitor>,
    meta: Mutex<HashMap<SessionId, Meta>>,
    transcript: Arc<Mutex<crate::session::SessionTranscript>>,
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionRegistry {
    /// Create an empty registry with a fresh monitor.
    pub fn new() -> Self {
        Self {
            monitor: Arc::new(SessionMonitor::new()),
            meta: Mutex::new(HashMap::new()),
            transcript: Arc::new(Mutex::new(
                crate::session::SessionTranscript::new_ephemeral(),
            )),
        }
    }

    /// Return a clone of the shared monitor. Hand this to any component that
    /// needs to call `observe` (e.g. `CommunicationPolicyGate`).
    pub fn monitor(&self) -> Arc<SessionMonitor> {
        self.monitor.clone()
    }

    /// The shared protocol transcript — hand its clone to
    /// `CommunicationPolicyGate::with_transcript`.
    pub fn transcript(&self) -> Arc<Mutex<crate::session::SessionTranscript>> {
        self.transcript.clone()
    }

    /// Open a session: mint an id, establish the projected FSMs for each role,
    /// and record a deadline of `now + ttl`.
    pub fn open(
        &self,
        global: &Global,
        binding: RoleBinding,
        ttl: Duration,
    ) -> Result<SessionId, RegistryError> {
        let id = super::new_session_id();
        self.monitor
            .establish(id.clone(), global, binding.assignment())
            .map_err(|e| RegistryError::Establish(e.to_string()))?;
        self.meta.lock().expect("registry mutex poisoned").insert(
            id.clone(),
            Meta {
                deadline: Instant::now() + ttl,
                status: SessionStatus::Running,
            },
        );
        Ok(id)
    }

    /// Return the current status of a session, or `None` if unknown.
    pub fn status(&self, id: &SessionId) -> Option<SessionStatus> {
        self.meta
            .lock()
            .expect("registry mutex poisoned")
            .get(id)
            .map(|m| m.status)
    }

    /// Recompute status from the monitor for a single session.
    ///
    /// - `Running` → `Complete` when the monitor reports all roles at an
    ///   accepting state.
    /// - `Running` → `Aborted` when the monitor reports the session aborted.
    pub fn refresh(&self, id: &SessionId) {
        let mut guard = self.meta.lock().expect("registry mutex poisoned");
        if let Some(m) = guard.get_mut(id) {
            // Only a still-Running session changes status; a terminal
            // (Complete/Aborted) session is never demoted or re-transitioned.
            if m.status == SessionStatus::Running {
                if self.monitor.is_aborted(id) {
                    m.status = SessionStatus::Aborted;
                } else if self.monitor.is_complete(id) {
                    m.status = SessionStatus::Complete;
                }
            }
        }
    }

    /// Abort all `Running` sessions whose deadline has passed. Returns the ids
    /// of every session that was aborted by this call.
    pub fn abort_expired(&self) -> Vec<SessionId> {
        let now = Instant::now();
        let mut aborted = Vec::new();
        let mut guard = self.meta.lock().expect("registry mutex poisoned");
        for (id, m) in guard.iter_mut() {
            if m.status == SessionStatus::Running && now >= m.deadline {
                let _ = self.monitor.abort(id);
                m.status = SessionStatus::Aborted;
                aborted.push(id.clone());
            }
        }
        aborted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::RoleBinding;
    use crate::types::AgentId;
    use symbi_session::examples::coordinator_pipeline;

    fn binding() -> (RoleBinding, AgentId, AgentId, AgentId) {
        let (c, v, p) = (AgentId::new(), AgentId::new(), AgentId::new());
        (
            RoleBinding::new()
                .bind(c, "Coordinator")
                .bind(v, "Validator")
                .bind(p, "Processor"),
            c,
            v,
            p,
        )
    }

    #[test]
    fn open_then_observe_to_completion() {
        let reg = SessionRegistry::new();
        let (g, _r) = coordinator_pipeline();
        let (rb, c, v, p) = binding();
        let sid = reg.open(&g, rb, Duration::from_secs(60)).unwrap();
        assert_eq!(reg.status(&sid), Some(SessionStatus::Running));
        let m = reg.monitor();
        m.observe(&sid, &c.to_string(), &v.to_string(), "task")
            .unwrap();
        m.observe(&sid, &v.to_string(), &c.to_string(), "ok")
            .unwrap();
        m.observe(&sid, &c.to_string(), &p.to_string(), "task")
            .unwrap();
        m.observe(&sid, &p.to_string(), &c.to_string(), "done")
            .unwrap();
        reg.refresh(&sid);
        assert_eq!(reg.status(&sid), Some(SessionStatus::Complete));
    }

    #[test]
    fn expired_session_is_aborted() {
        let reg = SessionRegistry::new();
        let (g, _r) = coordinator_pipeline();
        let (rb, _c, _v, _p) = binding();
        let sid = reg.open(&g, rb, Duration::from_millis(0)).unwrap();
        let aborted = reg.abort_expired();
        assert!(aborted.contains(&sid));
        assert_eq!(reg.status(&sid), Some(SessionStatus::Aborted));
        assert!(reg.monitor().is_aborted(&sid));
    }

    #[test]
    fn registry_exposes_a_transcript() {
        let reg = SessionRegistry::new();
        let t = reg.transcript();
        assert!(t.lock().unwrap().is_empty());
        assert!(std::sync::Arc::ptr_eq(&reg.transcript(), &t));
    }
}
