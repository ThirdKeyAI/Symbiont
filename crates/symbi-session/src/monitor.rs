//! A runtime session monitor that drives one projected FSM per role and checks
//! observed messages against the choreography.
//!
//! A [`SessionMonitor`] holds zero or more running [`SessionState`]s keyed by
//! [`SessionId`]. Each session binds agent identities (opaque strings) to
//! [`Role`]s and keeps, per role, the FSM compiled from that role's projection of
//! the global protocol plus the role's current state. Observing a message steps
//! the sender's FSM (a `Send`) and the receiver's FSM (a `Recv`) atomically: the
//! message is accepted only if both steps are legal, and on rejection neither
//! role advances.

use crate::fsm::{Event, Fsm, IllegalTransition, StateId};
use crate::global::{Global, Role};
use crate::project::project;
use std::collections::HashMap;

/// Stable identifier for a running session.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct SessionId(pub String);

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Why a session operation failed.
#[derive(Debug)]
pub enum SessionError {
    /// No session is registered under this id.
    UnknownSession(SessionId),
    /// An agent identity participating in a message has no role binding.
    UnknownRole(String),
    /// A message was not a legal transition for `role`'s projected FSM.
    Illegal {
        role: Role,
        transition: IllegalTransition,
    },
    /// The global protocol could not be projected onto a role.
    ProjectionFailed(String),
    /// The session has been terminally aborted; no further messages are accepted.
    Aborted(SessionId),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::UnknownSession(id) => write!(f, "unknown session '{id}'"),
            SessionError::UnknownRole(agent) => {
                write!(f, "agent '{agent}' has no role binding in this session")
            }
            SessionError::Illegal { role, transition } => {
                write!(f, "role '{role}': {transition}")
            }
            SessionError::ProjectionFailed(detail) => {
                write!(f, "projection failed: {detail}")
            }
            SessionError::Aborted(id) => write!(f, "session '{id}' has been aborted"),
        }
    }
}

impl std::error::Error for SessionError {}

/// One running session: role bindings + per-role projected FSM state.
#[derive(Debug)]
struct SessionState {
    /// Agent identity (string) -> role.
    role_of: HashMap<String, Role>,
    /// Role -> the FSM compiled from that role's projection.
    fsm: HashMap<Role, Fsm>,
    /// Role -> current state within its FSM.
    state: HashMap<Role, StateId>,
    /// When true the session has been terminally aborted; no further `observe`
    /// calls are accepted.
    aborted: bool,
}

/// Monitors one or more concurrent sessions against their choreographies.
#[derive(Debug, Default)]
pub struct SessionMonitor {
    sessions: std::sync::Mutex<HashMap<SessionId, SessionState>>,
}

impl SessionMonitor {
    /// Create an empty monitor.
    pub fn new() -> Self {
        Self::default()
    }

    /// Establish a session: project `global` onto every role and bind agents to
    /// roles. `assignment` maps agent-identity-string -> [`Role`].
    pub fn establish(
        &self,
        id: SessionId,
        global: &Global,
        assignment: HashMap<String, Role>,
    ) -> Result<(), SessionError> {
        let mut fsm = HashMap::new();
        let mut state = HashMap::new();

        // Project onto each distinct role that appears in the assignment.
        let mut roles: Vec<Role> = assignment.values().cloned().collect();
        roles.sort();
        roles.dedup();

        for role in roles {
            let local = project(global, &role)
                .map_err(|e| SessionError::ProjectionFailed(e.to_string()))?;
            let machine = Fsm::from_local(&local);
            state.insert(role.clone(), machine.start());
            fsm.insert(role, machine);
        }

        let session = SessionState {
            role_of: assignment,
            fsm,
            state,
            aborted: false,
        };

        self.sessions
            .lock()
            .expect("session monitor mutex poisoned")
            .insert(id, session);
        Ok(())
    }

    /// Observe a message `from_agent` -> `to_agent` carrying `label`. Steps the
    /// sender's role FSM (`Send`) and the receiver's role FSM (`Recv`),
    /// advancing both on success. On an illegal transition for either role,
    /// returns `Err` and does NOT advance either role.
    pub fn observe(
        &self,
        id: &SessionId,
        from_agent: &str,
        to_agent: &str,
        label: &str,
    ) -> Result<(), SessionError> {
        let mut guard = self
            .sessions
            .lock()
            .expect("session monitor mutex poisoned");
        let session = guard
            .get_mut(id)
            .ok_or_else(|| SessionError::UnknownSession(id.clone()))?;

        if session.aborted {
            return Err(SessionError::Aborted(id.clone()));
        }

        let sender_role = session
            .role_of
            .get(from_agent)
            .cloned()
            .ok_or_else(|| SessionError::UnknownRole(from_agent.to_string()))?;
        let receiver_role = session
            .role_of
            .get(to_agent)
            .cloned()
            .ok_or_else(|| SessionError::UnknownRole(to_agent.to_string()))?;

        let send_ev = Event::Send {
            to: receiver_role.clone(),
            label: label.to_string(),
        };
        let recv_ev = Event::Recv {
            from: sender_role.clone(),
            label: label.to_string(),
        };

        // Try both steps against the current states first; commit only if both
        // succeed, so a failure on either leaves the session unchanged.
        let sender_state = session.state[&sender_role];
        let receiver_state = session.state[&receiver_role];

        let new_sender_state = session.fsm[&sender_role]
            .step(sender_state, &send_ev)
            .map_err(|transition| SessionError::Illegal {
                role: sender_role.clone(),
                transition,
            })?;
        let new_receiver_state = session.fsm[&receiver_role]
            .step(receiver_state, &recv_ev)
            .map_err(|transition| SessionError::Illegal {
                role: receiver_role.clone(),
                transition,
            })?;

        // Both legal: commit.
        session.state.insert(sender_role, new_sender_state);
        session.state.insert(receiver_role, new_receiver_state);
        Ok(())
    }

    /// Mark a session terminally aborted. Subsequent `observe` calls are rejected.
    pub fn abort(&self, id: &SessionId) -> Result<(), SessionError> {
        let mut guard = self
            .sessions
            .lock()
            .expect("session monitor mutex poisoned");
        let s = guard
            .get_mut(id)
            .ok_or_else(|| SessionError::UnknownSession(id.clone()))?;
        s.aborted = true;
        Ok(())
    }

    /// True if the session exists and has been aborted.
    pub fn is_aborted(&self, id: &SessionId) -> bool {
        self.sessions
            .lock()
            .expect("session monitor mutex poisoned")
            .get(id)
            .map(|s| s.aborted)
            .unwrap_or(false)
    }

    /// The events that would be legal next for `agent`'s role in this session.
    pub fn legal_next(
        &self,
        id: &SessionId,
        agent: &str,
    ) -> Result<Vec<crate::fsm::Event>, SessionError> {
        let guard = self
            .sessions
            .lock()
            .expect("session monitor mutex poisoned");
        let s = guard
            .get(id)
            .ok_or_else(|| SessionError::UnknownSession(id.clone()))?;
        let role = s
            .role_of
            .get(agent)
            .ok_or_else(|| SessionError::UnknownRole(agent.to_string()))?;
        let fsm = s
            .fsm
            .get(role)
            .ok_or_else(|| SessionError::UnknownRole(agent.to_string()))?;
        let st = *s
            .state
            .get(role)
            .ok_or_else(|| SessionError::UnknownRole(agent.to_string()))?;
        Ok(fsm.expected(st))
    }

    /// Legal `Send` labels from `from_agent`'s role to `to_agent`'s role in the
    /// current state. Returns an empty vec if no send to that recipient is legal
    /// right now. Returns an error when either agent is unknown or the session
    /// does not exist.
    pub fn legal_labels_to(
        &self,
        id: &SessionId,
        from_agent: &str,
        to_agent: &str,
    ) -> Result<Vec<String>, SessionError> {
        let guard = self
            .sessions
            .lock()
            .expect("session monitor mutex poisoned");
        let s = guard
            .get(id)
            .ok_or_else(|| SessionError::UnknownSession(id.clone()))?;
        let from_role = s
            .role_of
            .get(from_agent)
            .ok_or_else(|| SessionError::UnknownRole(from_agent.to_string()))?;
        let to_role = s
            .role_of
            .get(to_agent)
            .ok_or_else(|| SessionError::UnknownRole(to_agent.to_string()))?;
        let fsm = s
            .fsm
            .get(from_role)
            .ok_or_else(|| SessionError::UnknownRole(from_agent.to_string()))?;
        let st = *s
            .state
            .get(from_role)
            .ok_or_else(|| SessionError::UnknownRole(from_agent.to_string()))?;
        Ok(fsm
            .expected(st)
            .into_iter()
            .filter_map(|e| match e {
                crate::fsm::Event::Send { to, label } if &to == to_role => Some(label),
                _ => None,
            })
            .collect())
    }

    /// True if every role in the session is at an accepting (End) state.
    pub fn is_complete(&self, id: &SessionId) -> bool {
        let guard = self
            .sessions
            .lock()
            .expect("session monitor mutex poisoned");
        let Some(session) = guard.get(id) else {
            return false;
        };
        session.state.iter().all(|(role, &st)| {
            session
                .fsm
                .get(role)
                .map(|f| f.is_accepting(st))
                .unwrap_or(false)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::examples::coordinator_pipeline;

    fn pipeline_session() -> (SessionMonitor, SessionId) {
        let (global, _roles) = coordinator_pipeline();
        let monitor = SessionMonitor::new();
        let id = SessionId("s1".to_string());
        let mut assignment = HashMap::new();
        assignment.insert("agent-coord".to_string(), "Coordinator".to_string());
        assignment.insert("agent-val".to_string(), "Validator".to_string());
        assignment.insert("agent-proc".to_string(), "Processor".to_string());
        monitor.establish(id.clone(), &global, assignment).unwrap();
        (monitor, id)
    }

    #[test]
    fn conforming_sequence_accepts_and_completes() {
        let (monitor, id) = pipeline_session();
        // Coordinator -> Validator : task
        monitor
            .observe(&id, "agent-coord", "agent-val", "task")
            .unwrap();
        // Validator -> Coordinator : ok
        monitor
            .observe(&id, "agent-val", "agent-coord", "ok")
            .unwrap();
        // Coordinator -> Processor : task
        monitor
            .observe(&id, "agent-coord", "agent-proc", "task")
            .unwrap();
        // Processor -> Coordinator : done
        monitor
            .observe(&id, "agent-proc", "agent-coord", "done")
            .unwrap();
        assert!(monitor.is_complete(&id));
    }

    #[test]
    fn out_of_order_first_message_is_rejected() {
        let (monitor, id) = pipeline_session();
        // Coordinator -> Processor : task is illegal as the first message;
        // the protocol must start with Coordinator -> Validator : task.
        let err = monitor
            .observe(&id, "agent-coord", "agent-proc", "task")
            .unwrap_err();
        match err {
            SessionError::Illegal { role, transition } => {
                assert_eq!(role, "Coordinator");
                assert_eq!(
                    transition.expected,
                    vec![Event::Send {
                        to: "Validator".to_string(),
                        label: "task".to_string(),
                    }]
                );
                assert_eq!(
                    transition.got,
                    Event::Send {
                        to: "Processor".to_string(),
                        label: "task".to_string(),
                    }
                );
            }
            other => panic!("expected Illegal, got {other:?}"),
        }
        // The session did not advance: a correct first message still works.
        assert!(monitor
            .observe(&id, "agent-coord", "agent-val", "task")
            .is_ok());
    }

    #[test]
    fn unknown_session_errors() {
        let monitor = SessionMonitor::new();
        let id = SessionId("missing".to_string());
        let err = monitor.observe(&id, "a", "b", "x").unwrap_err();
        assert!(matches!(err, SessionError::UnknownSession(_)));
        assert!(!monitor.is_complete(&id));
    }

    #[test]
    fn unknown_role_errors() {
        let (monitor, id) = pipeline_session();
        let err = monitor
            .observe(&id, "agent-coord", "stranger", "task")
            .unwrap_err();
        assert!(matches!(err, SessionError::UnknownRole(_)));
    }

    #[test]
    fn aborted_session_rejects_further_observe_and_reports_aborted() {
        let (g, _roles) = crate::examples::coordinator_pipeline();
        let m = SessionMonitor::new();
        let sid = SessionId("s1".into());
        let mut assign = std::collections::HashMap::new();
        assign.insert("A".to_string(), "Coordinator".to_string());
        assign.insert("B".to_string(), "Validator".to_string());
        assign.insert("C".to_string(), "Processor".to_string());
        m.establish(sid.clone(), &g, assign).unwrap();
        assert!(!m.is_aborted(&sid));
        m.abort(&sid).unwrap();
        assert!(m.is_aborted(&sid));
        let err = m.observe(&sid, "A", "B", "task").unwrap_err();
        assert!(matches!(err, SessionError::Aborted(_)));
        assert!(matches!(
            m.abort(&SessionId("nope".into())),
            Err(SessionError::UnknownSession(_))
        ));
    }

    #[test]
    fn legal_labels_to_returns_send_labels_for_an_edge() {
        let (g, _roles) = crate::examples::coordinator_pipeline();
        let m = SessionMonitor::new();
        let sid = SessionId("ll1".into());
        let mut assign = std::collections::HashMap::new();
        assign.insert("A".to_string(), "Coordinator".to_string());
        assign.insert("B".to_string(), "Validator".to_string());
        assign.insert("C".to_string(), "Processor".to_string());
        m.establish(sid.clone(), &g, assign).unwrap();
        assert_eq!(
            m.legal_labels_to(&sid, "A", "B").unwrap(),
            vec!["task".to_string()]
        );
        assert!(m.legal_labels_to(&sid, "A", "C").unwrap().is_empty());
        assert!(matches!(
            m.legal_labels_to(&sid, "ZZ", "B"),
            Err(SessionError::UnknownRole(_))
        ));
        assert!(matches!(
            m.legal_labels_to(&SessionId("nope".into()), "A", "B"),
            Err(SessionError::UnknownSession(_))
        ));
    }

    #[test]
    fn legal_next_lists_expected_events_for_current_state() {
        let (g, _roles) = crate::examples::coordinator_pipeline();
        let m = SessionMonitor::new();
        let sid = SessionId("s2".into());
        let mut assign = std::collections::HashMap::new();
        assign.insert("A".to_string(), "Coordinator".to_string());
        assign.insert("B".to_string(), "Validator".to_string());
        assign.insert("C".to_string(), "Processor".to_string());
        m.establish(sid.clone(), &g, assign).unwrap();
        let next = m.legal_next(&sid, "A").unwrap();
        assert!(next.iter().any(
            |e| matches!(e, crate::fsm::Event::Send{to,label} if to=="Validator" && label=="task")
        ));
        assert!(matches!(
            m.legal_next(&sid, "ZZZ"),
            Err(SessionError::UnknownRole(_))
        ));
        assert!(matches!(
            m.legal_next(&SessionId("nope".into()), "A"),
            Err(SessionError::UnknownSession(_))
        ));
    }
}
