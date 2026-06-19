//! Runtime session machinery for symbi-session: role binding, session-id minting,
//! and the lifecycle registry.

mod registry;
pub use registry::{RegistryError, SessionRegistry, SessionStatus};

mod transcript;
pub use transcript::{SessionTranscript, TranscriptDecision, TranscriptEntry};

use crate::types::AgentId;
use std::collections::HashMap;
use symbi_session::monitor::SessionId;

/// A typed binding of agents to protocol roles for one session.
#[derive(Debug, Clone, Default)]
pub struct RoleBinding {
    map: HashMap<AgentId, String>,
}

impl RoleBinding {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind `agent` to `role` (builder style).
    pub fn bind(mut self, agent: AgentId, role: &str) -> Self {
        self.map.insert(agent, role.to_string());
        self
    }

    /// The `agent-id-string -> role` assignment the SessionMonitor expects.
    pub fn assignment(&self) -> HashMap<String, String> {
        self.map
            .iter()
            .map(|(a, r)| (a.to_string(), r.clone()))
            .collect()
    }
}

/// Mint a fresh, unique session id.
pub fn new_session_id() -> SessionId {
    SessionId(uuid::Uuid::new_v4().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::AgentId;

    #[test]
    fn role_binding_maps_agents_and_new_session_ids_are_unique() {
        let a = AgentId::new();
        let b = AgentId::new();
        let rb = RoleBinding::new()
            .bind(a, "Coordinator")
            .bind(b, "Validator");
        let assign = rb.assignment();
        assert_eq!(
            assign.get(&a.to_string()).map(String::as_str),
            Some("Coordinator")
        );
        assert_eq!(
            assign.get(&b.to_string()).map(String::as_str),
            Some("Validator")
        );
        assert_ne!(new_session_id().0, new_session_id().0);
    }
}
