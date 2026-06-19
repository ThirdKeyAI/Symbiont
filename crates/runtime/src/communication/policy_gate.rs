//! Communication policy gate for inter-agent message authorization
//!
//! Evaluates Cedar-style rules to allow or deny inter-agent communication.
//! Default behavior is allow-all (backward compatible).

use crate::types::{AgentId, CommunicationError, MessageType};
#[cfg(feature = "session")]
use std::sync::{Arc, Mutex};
#[cfg(feature = "session")]
use symbi_session::monitor::SessionMonitor;

/// A request to communicate between agents, evaluated by the policy gate.
#[derive(Debug, Clone)]
pub struct CommunicationRequest {
    pub sender: AgentId,
    pub recipient: AgentId,
    pub message_type: MessageType,
    pub topic: Option<String>,
    /// Optional session this message belongs to. When set together with
    /// `protocol_label`, the gate's session monitor checks message ordering.
    pub session_id: Option<String>,
    /// Optional protocol message label used to step the session FSMs.
    pub protocol_label: Option<String>,
}

/// Condition that determines when a rule applies.
#[derive(Debug, Clone)]
pub enum CommunicationCondition {
    SenderIs(AgentId),
    RecipientIs(AgentId),
    Always,
    All(Vec<CommunicationCondition>),
    Any(Vec<CommunicationCondition>),
}

/// Effect of a matched rule.
#[derive(Debug, Clone)]
pub enum CommunicationEffect {
    Allow,
    Deny { reason: String },
}

/// A single policy rule with priority.
#[derive(Debug, Clone)]
pub struct CommunicationPolicyRule {
    pub id: String,
    pub name: String,
    pub condition: CommunicationCondition,
    pub effect: CommunicationEffect,
    pub priority: u32,
}

/// Policy gate that evaluates rules for inter-agent communication.
/// Rules evaluated in priority order (highest first). First matching rule wins.
/// Default is deny (fail-closed). Use `permissive()` to opt into allow-by-default.
#[derive(Debug, Clone, Default)]
pub struct CommunicationPolicyGate {
    rules: Vec<CommunicationPolicyRule>,
    default_allow: bool,
    /// Optional multiparty session monitor. When present, requests carrying a
    /// `session_id` + `protocol_label` are also checked for legal ordering.
    #[cfg(feature = "session")]
    session_monitor: Option<Arc<SessionMonitor>>,
    /// Optional protocol transcript. When present, every session transition is
    /// recorded (allowed or denied) for offline-verifiable audit.
    #[cfg(feature = "session")]
    transcript: Option<Arc<Mutex<crate::session::SessionTranscript>>>,
}

impl CommunicationPolicyGate {
    pub fn new(mut rules: Vec<CommunicationPolicyRule>) -> Self {
        rules.sort_by_key(|r| std::cmp::Reverse(r.priority));
        Self {
            rules,
            default_allow: false,
            #[cfg(feature = "session")]
            session_monitor: None,
            #[cfg(feature = "session")]
            transcript: None,
        }
    }

    pub fn permissive() -> Self {
        Self {
            rules: Vec::new(),
            default_allow: true,
            #[cfg(feature = "session")]
            session_monitor: None,
            #[cfg(feature = "session")]
            transcript: None,
        }
    }

    /// Attach a session monitor; requests carrying session metadata will be
    /// checked for legal message ordering in addition to the policy rules.
    #[cfg(feature = "session")]
    pub fn with_session_monitor(mut self, m: Arc<SessionMonitor>) -> Self {
        self.session_monitor = Some(m);
        self
    }

    /// Attach a protocol transcript; each session transition is recorded (allowed
    /// or denied) for offline-verifiable audit.
    #[cfg(feature = "session")]
    pub fn with_transcript(mut self, t: Arc<Mutex<crate::session::SessionTranscript>>) -> Self {
        self.transcript = Some(t);
        self
    }

    pub fn deny_by_default(rules: Vec<CommunicationPolicyRule>) -> Self {
        let mut gate = Self::new(rules);
        gate.default_allow = false;
        gate
    }

    pub fn evaluate(&self, request: &CommunicationRequest) -> Result<(), CommunicationError> {
        // First, the existing rule-based authorization decision.
        self.authorize(request)?;
        // Then, if a session monitor is attached and the request carries session
        // metadata, enforce legal message ordering for the choreography.
        #[cfg(feature = "session")]
        self.check_session(request)?;
        Ok(())
    }

    /// The rule-based allow/deny decision (unchanged behavior).
    fn authorize(&self, request: &CommunicationRequest) -> Result<(), CommunicationError> {
        for rule in &self.rules {
            if self.matches_condition(&rule.condition, request) {
                return match &rule.effect {
                    CommunicationEffect::Allow => Ok(()),
                    CommunicationEffect::Deny { reason } => Err(CommunicationError::PolicyDenied {
                        reason: format!("[{}] {}", rule.name, reason).into(),
                    }),
                };
            }
        }
        if self.default_allow {
            Ok(())
        } else {
            Err(CommunicationError::PolicyDenied {
                reason: "No matching rule and default is deny".into(),
            })
        }
    }

    /// Step the session monitor for this message, if applicable. No-op when no
    /// monitor is attached or the request carries no session metadata.
    #[cfg(feature = "session")]
    fn check_session(&self, request: &CommunicationRequest) -> Result<(), CommunicationError> {
        let (Some(monitor), Some(sid_str), Some(label)) = (
            self.session_monitor.as_ref(),
            request.session_id.as_ref(),
            request.protocol_label.as_ref(),
        ) else {
            return Ok(());
        };
        let session_id = symbi_session::monitor::SessionId(sid_str.clone());
        let sender = request.sender.to_string();
        let recipient = request.recipient.to_string();
        let result = monitor.observe(&session_id, &sender, &recipient, label);

        if let Some(transcript) = &self.transcript {
            let (decision, reason) = match &result {
                Ok(()) => (crate::session::TranscriptDecision::Allowed, None),
                Err(e) => (
                    crate::session::TranscriptDecision::Denied,
                    Some(e.to_string()),
                ),
            };
            transcript
                .lock()
                .expect("transcript mutex poisoned")
                .record(
                    &session_id.to_string(),
                    &sender,
                    &recipient,
                    label,
                    decision,
                    reason,
                );
        }

        result.map_err(|e| CommunicationError::PolicyDenied {
            reason: match e {
                symbi_session::monitor::SessionError::Illegal { transition, .. } => {
                    format!("session: illegal transition — {}", transition.diagnose())
                }
                other => format!("session: {other}"),
            }
            .into(),
        })
    }

    fn matches_condition(
        &self,
        condition: &CommunicationCondition,
        request: &CommunicationRequest,
    ) -> bool {
        match condition {
            CommunicationCondition::SenderIs(id) => request.sender == *id,
            CommunicationCondition::RecipientIs(id) => request.recipient == *id,
            CommunicationCondition::Always => true,
            CommunicationCondition::All(conditions) => conditions
                .iter()
                .all(|c| self.matches_condition(c, request)),
            CommunicationCondition::Any(conditions) => conditions
                .iter()
                .any(|c| self.matches_condition(c, request)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::RequestId;

    #[cfg(feature = "session")]
    #[test]
    fn session_label_mismatch_message_names_the_label() {
        use crate::types::AgentId;
        use crate::types::MessageType;
        use std::collections::HashMap;
        use std::sync::Arc;
        use symbi_session::examples::coordinator_pipeline;
        use symbi_session::monitor::{SessionId, SessionMonitor};

        let (g, _r) = coordinator_pipeline();
        let monitor = Arc::new(SessionMonitor::new());
        let (coord, validator, processor) = (AgentId::new(), AgentId::new(), AgentId::new());
        let sid = SessionId("gt1".into());
        let mut assign = HashMap::new();
        assign.insert(coord.to_string(), "Coordinator".to_string());
        assign.insert(validator.to_string(), "Validator".to_string());
        assign.insert(processor.to_string(), "Processor".to_string());
        monitor.establish(sid.clone(), &g, assign).unwrap();

        // permissive() sets default_allow = true, so only the session monitor governs
        let gate = CommunicationPolicyGate::permissive().with_session_monitor(monitor);

        let req = CommunicationRequest {
            sender: coord,
            recipient: validator,
            message_type: MessageType::Direct(validator),
            topic: None,
            session_id: Some(sid.to_string()),
            protocol_label: Some("validate".to_string()), // right target, wrong label
        };
        let err = gate.evaluate(&req).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("label 'task'"), "got: {msg}");
        assert!(msg.contains("validate"), "got: {msg}");
    }

    fn make_request(sender: AgentId, recipient: AgentId) -> CommunicationRequest {
        CommunicationRequest {
            sender,
            recipient,
            message_type: MessageType::Request(RequestId::new()),
            topic: None,
            session_id: None,
            protocol_label: None,
        }
    }

    #[test]
    fn test_permissive_gate_allows_all() {
        let gate = CommunicationPolicyGate::permissive();
        let req = make_request(AgentId::new(), AgentId::new());
        assert!(gate.evaluate(&req).is_ok());
    }

    #[test]
    fn test_deny_by_default_denies_without_rules() {
        let gate = CommunicationPolicyGate::deny_by_default(vec![]);
        let req = make_request(AgentId::new(), AgentId::new());
        let err = gate.evaluate(&req).unwrap_err();
        assert!(matches!(err, CommunicationError::PolicyDenied { .. }));
    }

    #[test]
    fn test_deny_rule_blocks_sender() {
        let blocked = AgentId::new();
        let allowed = AgentId::new();
        let recipient = AgentId::new();

        // Use permissive gate so unmatched requests are allowed
        let mut gate = CommunicationPolicyGate::permissive();
        gate.rules = vec![CommunicationPolicyRule {
            id: "r1".into(),
            name: "block-sender".into(),
            condition: CommunicationCondition::SenderIs(blocked),
            effect: CommunicationEffect::Deny {
                reason: "blocked".into(),
            },
            priority: 10,
        }];

        let blocked_req = make_request(blocked, recipient);
        assert!(gate.evaluate(&blocked_req).is_err());

        let allowed_req = make_request(allowed, recipient);
        assert!(gate.evaluate(&allowed_req).is_ok());
    }

    #[test]
    fn test_priority_ordering() {
        let agent = AgentId::new();
        let recipient = AgentId::new();

        let gate = CommunicationPolicyGate::new(vec![
            CommunicationPolicyRule {
                id: "allow".into(),
                name: "low-allow".into(),
                condition: CommunicationCondition::SenderIs(agent),
                effect: CommunicationEffect::Allow,
                priority: 1,
            },
            CommunicationPolicyRule {
                id: "deny".into(),
                name: "high-deny".into(),
                condition: CommunicationCondition::SenderIs(agent),
                effect: CommunicationEffect::Deny {
                    reason: "denied".into(),
                },
                priority: 100,
            },
        ]);

        let req = make_request(agent, recipient);
        assert!(gate.evaluate(&req).is_err());
    }

    #[test]
    fn test_all_condition() {
        let sender = AgentId::new();
        let recipient = AgentId::new();
        let other_recipient = AgentId::new();

        // Use permissive gate so unmatched requests fall through to allow
        let mut gate = CommunicationPolicyGate::permissive();
        gate.rules = vec![CommunicationPolicyRule {
            id: "r1".into(),
            name: "all-match".into(),
            condition: CommunicationCondition::All(vec![
                CommunicationCondition::SenderIs(sender),
                CommunicationCondition::RecipientIs(recipient),
            ]),
            effect: CommunicationEffect::Deny {
                reason: "both match".into(),
            },
            priority: 10,
        }];

        // Both conditions match -> deny
        let req = make_request(sender, recipient);
        assert!(gate.evaluate(&req).is_err());

        // Only sender matches -> falls through to default allow
        let partial_req = make_request(sender, other_recipient);
        assert!(gate.evaluate(&partial_req).is_ok());
    }

    #[test]
    fn test_any_condition() {
        let agent_a = AgentId::new();
        let agent_b = AgentId::new();
        let agent_c = AgentId::new();
        let recipient = AgentId::new();

        // Use permissive gate so unmatched requests fall through to allow
        let mut gate = CommunicationPolicyGate::permissive();
        gate.rules = vec![CommunicationPolicyRule {
            id: "r1".into(),
            name: "any-match".into(),
            condition: CommunicationCondition::Any(vec![
                CommunicationCondition::SenderIs(agent_a),
                CommunicationCondition::SenderIs(agent_b),
            ]),
            effect: CommunicationEffect::Deny {
                reason: "either match".into(),
            },
            priority: 10,
        }];

        // agent_a matches
        assert!(gate.evaluate(&make_request(agent_a, recipient)).is_err());
        // agent_b matches
        assert!(gate.evaluate(&make_request(agent_b, recipient)).is_err());
        // agent_c doesn't match -> default allow (permissive gate)
        assert!(gate.evaluate(&make_request(agent_c, recipient)).is_ok());
    }

    #[cfg(feature = "session")]
    #[test]
    fn gate_records_allowed_and_denied_transitions() {
        use crate::session::{SessionTranscript, TranscriptDecision};
        use crate::types::{communication::MessageType, AgentId};
        use std::collections::HashMap;
        use std::sync::{Arc, Mutex};
        use symbi_session::examples::coordinator_pipeline;
        use symbi_session::monitor::{SessionId, SessionMonitor};

        let (g, _r) = coordinator_pipeline();
        let monitor = Arc::new(SessionMonitor::new());
        let (c, v, p) = (AgentId::new(), AgentId::new(), AgentId::new());
        let sid = SessionId("tr1".into());
        let mut assign = HashMap::new();
        assign.insert(c.to_string(), "Coordinator".to_string());
        assign.insert(v.to_string(), "Validator".to_string());
        assign.insert(p.to_string(), "Processor".to_string());
        monitor.establish(sid.clone(), &g, assign).unwrap();
        let transcript = Arc::new(Mutex::new(SessionTranscript::new_ephemeral()));

        let gate = CommunicationPolicyGate::permissive()
            .with_session_monitor(monitor)
            .with_transcript(transcript.clone());

        let ok = CommunicationRequest {
            sender: c,
            recipient: v,
            message_type: MessageType::Direct(v),
            topic: None,
            session_id: Some(sid.to_string()),
            protocol_label: Some("task".into()),
        };
        assert!(gate.evaluate(&ok).is_ok());
        let bad = CommunicationRequest {
            sender: c,
            recipient: p,
            message_type: MessageType::Direct(p),
            topic: None,
            session_id: Some(sid.to_string()),
            protocol_label: Some("task".into()),
        };
        assert!(gate.evaluate(&bad).is_err());

        let t = transcript.lock().unwrap();
        assert_eq!(t.len(), 2);
        assert!(t.verify());
        assert_eq!(t.entries()[0].decision, TranscriptDecision::Allowed);
        assert_eq!(t.entries()[1].decision, TranscriptDecision::Denied);
    }
}
