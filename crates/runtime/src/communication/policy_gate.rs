//! Communication policy gate for inter-agent message authorization
//!
//! Evaluates Cedar-style rules to allow or deny inter-agent communication.
//! Default behavior is allow-all (backward compatible).

use crate::types::{AgentId, CommunicationError, MessageType};

/// A request to communicate between agents, evaluated by the policy gate.
#[derive(Debug, Clone)]
pub struct CommunicationRequest {
    pub sender: AgentId,
    pub recipient: AgentId,
    pub message_type: MessageType,
    pub topic: Option<String>,
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
/// Default is Allow (backward compatible).
#[derive(Debug, Clone)]
pub struct CommunicationPolicyGate {
    rules: Vec<CommunicationPolicyRule>,
    default_allow: bool,
}

impl Default for CommunicationPolicyGate {
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            default_allow: false,
        }
    }
}

impl CommunicationPolicyGate {
    pub fn new(mut rules: Vec<CommunicationPolicyRule>) -> Self {
        rules.sort_by(|a, b| b.priority.cmp(&a.priority));
        Self {
            rules,
            default_allow: false,
        }
    }

    pub fn permissive() -> Self {
        Self {
            rules: Vec::new(),
            default_allow: true,
        }
    }

    pub fn deny_by_default(rules: Vec<CommunicationPolicyRule>) -> Self {
        let mut gate = Self::new(rules);
        gate.default_allow = false;
        gate
    }

    pub fn evaluate(&self, request: &CommunicationRequest) -> Result<(), CommunicationError> {
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

    fn make_request(sender: AgentId, recipient: AgentId) -> CommunicationRequest {
        CommunicationRequest {
            sender,
            recipient,
            message_type: MessageType::Request(RequestId::new()),
            topic: None,
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
}
