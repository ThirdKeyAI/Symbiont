//! Cedar policy gate for reasoning loops
//!
//! Wraps the `cedar-policy` crate to provide formally verified,
//! sub-millisecond authorization for agent actions.
//!
//! Feature-gated behind `cedar`. When enabled, `CedarPolicyGate`
//! implements `ReasoningPolicyGate` and maps agent actions to Cedar
//! authorization requests.

use cedar_policy::{
    Authorizer, Context, Decision, Entities, EntityId, EntityTypeName, EntityUid, PolicySet,
    Request,
};

use crate::reasoning::loop_types::{LoopDecision, LoopState, ProposedAction};
use crate::reasoning::policy_bridge::ReasoningPolicyGate;
use crate::types::AgentId;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;

/// A Cedar policy in source form.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CedarPolicy {
    /// Unique name for this policy.
    pub name: String,
    /// Cedar policy source text (must be valid Cedar syntax).
    ///
    /// Entity types used in policies:
    /// - Principal: `Agent::"<agent_id>"`
    /// - Action: `Action::"respond"`, `Action::"tool_call::<name>"`, etc.
    /// - Resource: `Resource::"default"`
    pub source: String,
    /// Whether this policy is currently active.
    pub active: bool,
}

/// Cedar-based policy gate for reasoning loops.
///
/// Evaluates agent actions against Cedar policies using the `cedar-policy`
/// crate's `Authorizer::is_authorized()`. When no policies are loaded,
/// defaults to deny-all for safety.
pub struct CedarPolicyGate {
    policies: Arc<RwLock<Vec<CedarPolicy>>>,
    default_decision: LoopDecision,
}

impl Default for CedarPolicyGate {
    fn default() -> Self {
        Self::deny_by_default()
    }
}

impl CedarPolicyGate {
    /// Create a gate that denies all actions by default.
    pub fn deny_by_default() -> Self {
        Self {
            policies: Arc::new(RwLock::new(Vec::new())),
            default_decision: LoopDecision::Deny {
                reason: "No Cedar policies loaded".into(),
            },
        }
    }

    /// Create a gate that allows all actions by default (for development).
    pub fn allow_by_default() -> Self {
        Self {
            policies: Arc::new(RwLock::new(Vec::new())),
            default_decision: LoopDecision::Allow,
        }
    }

    /// Add a Cedar policy.
    pub async fn add_policy(&self, policy: CedarPolicy) {
        self.policies.write().await.push(policy);
    }

    /// Remove a policy by name.
    pub async fn remove_policy(&self, name: &str) -> bool {
        let mut policies = self.policies.write().await;
        let before = policies.len();
        policies.retain(|p| p.name != name);
        policies.len() < before
    }

    /// List all loaded policies.
    pub async fn list_policies(&self) -> Vec<CedarPolicy> {
        self.policies.read().await.clone()
    }

    /// Get active policy count.
    pub async fn active_policy_count(&self) -> usize {
        self.policies
            .read()
            .await
            .iter()
            .filter(|p| p.active)
            .count()
    }

    /// Evaluate an action against loaded Cedar policies using the real Authorizer.
    ///
    /// Maps agent_id → Cedar principal (`Agent::"<id>"`),
    /// the action → Cedar action (`Action::"<name>"`),
    /// and uses a default resource (`Resource::"default"`).
    fn evaluate_against_policies(
        &self,
        policies: &[CedarPolicy],
        agent_id: &AgentId,
        action: &ProposedAction,
        _state: &LoopState,
    ) -> LoopDecision {
        let active_policies: Vec<_> = policies.iter().filter(|p| p.active).collect();

        if active_policies.is_empty() {
            return self.default_decision.clone();
        }

        // Map the action to a Cedar action name
        let action_name = match action {
            ProposedAction::ToolCall { name, .. } => format!("tool_call::{}", name),
            ProposedAction::Respond { .. } => "respond".to_string(),
            ProposedAction::Delegate { target, .. } => format!("delegate::{}", target),
            ProposedAction::Terminate { .. } => "terminate".to_string(),
        };

        // Concatenate all active policy sources into one policy set
        let combined_source: String = active_policies
            .iter()
            .map(|p| p.source.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        // Parse into a Cedar PolicySet
        let policy_set = match combined_source.parse::<PolicySet>() {
            Ok(ps) => ps,
            Err(e) => {
                tracing::error!("Cedar policy parse error: {}", e);
                return LoopDecision::Deny {
                    reason: format!("Cedar policy parse error: {}", e),
                };
            }
        };

        // Build Cedar PARC request
        let Ok(agent_type) = EntityTypeName::from_str("Agent") else {
            return LoopDecision::Deny {
                reason: "Cedar: invalid entity type 'Agent'".into(),
            };
        };
        let Ok(agent_eid) = EntityId::from_str(&agent_id.to_string()) else {
            return LoopDecision::Deny {
                reason: format!("Cedar: invalid agent id '{}'", agent_id),
            };
        };
        let principal = EntityUid::from_type_name_and_id(agent_type, agent_eid);

        let Ok(action_type) = EntityTypeName::from_str("Action") else {
            return LoopDecision::Deny {
                reason: "Cedar: invalid entity type 'Action'".into(),
            };
        };
        let Ok(action_eid) = EntityId::from_str(&action_name) else {
            return LoopDecision::Deny {
                reason: format!("Cedar: invalid action name '{}'", action_name),
            };
        };
        let cedar_action = EntityUid::from_type_name_and_id(action_type, action_eid);

        let Ok(resource_type) = EntityTypeName::from_str("Resource") else {
            return LoopDecision::Deny {
                reason: "Cedar: invalid entity type 'Resource'".into(),
            };
        };
        let Ok(resource_eid) = EntityId::from_str("default") else {
            return LoopDecision::Deny {
                reason: "Cedar: invalid entity id 'default'".into(),
            };
        };
        let resource = EntityUid::from_type_name_and_id(resource_type, resource_eid);

        let request = match Request::new(principal, cedar_action, resource, Context::empty(), None)
        {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("Cedar request construction error: {}", e);
                return LoopDecision::Deny {
                    reason: format!("Cedar request error: {}", e),
                };
            }
        };

        // Run the Cedar Authorizer
        let authorizer = Authorizer::new();
        let response = authorizer.is_authorized(&request, &policy_set, &Entities::empty());

        match response.decision() {
            Decision::Allow => LoopDecision::Allow,
            Decision::Deny => {
                let errors: Vec<String> = response
                    .diagnostics()
                    .errors()
                    .map(|e| e.to_string())
                    .collect();
                let reason = if errors.is_empty() {
                    format!(
                        "Cedar denied action '{}' for agent {}",
                        action_name, agent_id
                    )
                } else {
                    format!(
                        "Cedar denied action '{}' for agent {}: {}",
                        action_name,
                        agent_id,
                        errors.join("; ")
                    )
                };
                LoopDecision::Deny { reason }
            }
        }
    }
}

#[async_trait::async_trait]
impl ReasoningPolicyGate for CedarPolicyGate {
    async fn evaluate_action(
        &self,
        agent_id: &AgentId,
        action: &ProposedAction,
        state: &LoopState,
    ) -> LoopDecision {
        let policies = self.policies.read().await;
        self.evaluate_against_policies(&policies, agent_id, action, state)
    }
}

/// Errors from the Cedar gate.
#[derive(Debug, thiserror::Error)]
pub enum CedarGateError {
    #[error("Cedar policy parse error: {0}")]
    ParseError(String),

    #[error("Cedar evaluation error: {0}")]
    EvaluationError(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reasoning::conversation::Conversation;

    fn test_state() -> LoopState {
        LoopState::new(AgentId::new(), Conversation::new())
    }

    #[tokio::test]
    async fn test_deny_by_default_no_policies() {
        let gate = CedarPolicyGate::deny_by_default();
        let agent = AgentId::new();
        let action = ProposedAction::Respond {
            content: "hello".into(),
        };

        let decision = gate.evaluate_action(&agent, &action, &test_state()).await;
        assert!(matches!(decision, LoopDecision::Deny { .. }));
    }

    #[tokio::test]
    async fn test_allow_by_default_no_policies() {
        let gate = CedarPolicyGate::allow_by_default();
        let agent = AgentId::new();
        let action = ProposedAction::Respond {
            content: "hello".into(),
        };

        let decision = gate.evaluate_action(&agent, &action, &test_state()).await;
        assert!(matches!(decision, LoopDecision::Allow));
    }

    #[tokio::test]
    async fn test_permit_policy_allows() {
        let gate = CedarPolicyGate::deny_by_default();

        gate.add_policy(CedarPolicy {
            name: "allow_respond".into(),
            source: r#"permit(principal, action == Action::"respond", resource);"#.into(),
            active: true,
        })
        .await;

        let agent = AgentId::new();
        let action = ProposedAction::Respond {
            content: "hello".into(),
        };

        let decision = gate.evaluate_action(&agent, &action, &test_state()).await;
        assert!(matches!(decision, LoopDecision::Allow));
    }

    #[tokio::test]
    async fn test_forbid_policy_denies() {
        let gate = CedarPolicyGate::allow_by_default();

        gate.add_policy(CedarPolicy {
            name: "deny_search".into(),
            source: r#"forbid(principal, action == Action::"tool_call::search", resource);"#.into(),
            active: true,
        })
        .await;

        let agent = AgentId::new();
        let action = ProposedAction::ToolCall {
            call_id: "c1".into(),
            name: "search".into(),
            arguments: "{}".into(),
        };

        let decision = gate.evaluate_action(&agent, &action, &test_state()).await;
        assert!(matches!(decision, LoopDecision::Deny { .. }));
    }

    #[tokio::test]
    async fn test_inactive_policy_ignored() {
        let gate = CedarPolicyGate::deny_by_default();

        gate.add_policy(CedarPolicy {
            name: "allow_all".into(),
            source: r#"permit(principal, action == Action::"respond", resource);"#.into(),
            active: false, // Inactive
        })
        .await;

        let agent = AgentId::new();
        let action = ProposedAction::Respond {
            content: "hello".into(),
        };

        // Should still deny because the policy is inactive
        let decision = gate.evaluate_action(&agent, &action, &test_state()).await;
        assert!(matches!(decision, LoopDecision::Deny { .. }));
    }

    #[tokio::test]
    async fn test_add_and_remove_policy() {
        let gate = CedarPolicyGate::deny_by_default();

        gate.add_policy(CedarPolicy {
            name: "test".into(),
            source: r#"permit(principal, action, resource);"#.into(),
            active: true,
        })
        .await;

        assert_eq!(gate.list_policies().await.len(), 1);
        assert!(gate.remove_policy("test").await);
        assert!(gate.list_policies().await.is_empty());
        assert!(!gate.remove_policy("test").await);
    }

    #[tokio::test]
    async fn test_active_policy_count() {
        let gate = CedarPolicyGate::deny_by_default();

        gate.add_policy(CedarPolicy {
            name: "a".into(),
            source: r#"permit(principal, action, resource);"#.into(),
            active: true,
        })
        .await;
        gate.add_policy(CedarPolicy {
            name: "b".into(),
            source: r#"forbid(principal, action, resource);"#.into(),
            active: false,
        })
        .await;
        gate.add_policy(CedarPolicy {
            name: "c".into(),
            source: r#"permit(principal, action, resource);"#.into(),
            active: true,
        })
        .await;

        assert_eq!(gate.active_policy_count().await, 2);
    }

    #[tokio::test]
    async fn test_forbid_takes_precedence() {
        let gate = CedarPolicyGate::allow_by_default();

        // Both permit and forbid for the same action — forbid wins (Cedar semantics)
        gate.add_policy(CedarPolicy {
            name: "allow_respond".into(),
            source: r#"permit(principal, action == Action::"respond", resource);"#.into(),
            active: true,
        })
        .await;
        gate.add_policy(CedarPolicy {
            name: "deny_respond".into(),
            source: r#"forbid(principal, action == Action::"respond", resource);"#.into(),
            active: true,
        })
        .await;

        let agent = AgentId::new();
        let action = ProposedAction::Respond {
            content: "hello".into(),
        };

        let decision = gate.evaluate_action(&agent, &action, &test_state()).await;
        assert!(matches!(decision, LoopDecision::Deny { .. }));
    }

    #[tokio::test]
    async fn test_delegate_action_mapping() {
        let gate = CedarPolicyGate::deny_by_default();

        gate.add_policy(CedarPolicy {
            name: "allow_delegate".into(),
            source: r#"permit(principal, action == Action::"delegate::reviewer", resource);"#
                .into(),
            active: true,
        })
        .await;

        let agent = AgentId::new();
        let action = ProposedAction::Delegate {
            target: "reviewer".into(),
            message: "review this".into(),
        };

        let decision = gate.evaluate_action(&agent, &action, &test_state()).await;
        assert!(matches!(decision, LoopDecision::Allow));
    }

    #[test]
    fn test_cedar_policy_serialization() {
        let policy = CedarPolicy {
            name: "test".into(),
            source: r#"permit(principal, action, resource);"#.into(),
            active: true,
        };

        let json = serde_json::to_string(&policy).unwrap();
        let restored: CedarPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.name, "test");
        assert!(restored.active);
    }

    #[tokio::test]
    async fn test_invalid_policy_source_returns_deny() {
        let gate = CedarPolicyGate::allow_by_default();

        gate.add_policy(CedarPolicy {
            name: "broken".into(),
            source: "this is not valid cedar policy syntax at all!!!".into(),
            active: true,
        })
        .await;

        let agent = AgentId::new();
        let action = ProposedAction::Respond {
            content: "hello".into(),
        };

        let decision = gate.evaluate_action(&agent, &action, &test_state()).await;
        assert!(
            matches!(decision, LoopDecision::Deny { reason } if reason.contains("parse error"))
        );
    }

    #[tokio::test]
    async fn test_permit_all_wildcard() {
        let gate = CedarPolicyGate::deny_by_default();

        // Cedar wildcard permit: allows any principal/action/resource
        gate.add_policy(CedarPolicy {
            name: "permit_all".into(),
            source: r#"permit(principal, action, resource);"#.into(),
            active: true,
        })
        .await;

        let agent = AgentId::new();

        // Should permit any action
        let respond = ProposedAction::Respond {
            content: "hi".into(),
        };
        assert!(matches!(
            gate.evaluate_action(&agent, &respond, &test_state()).await,
            LoopDecision::Allow
        ));

        let tool = ProposedAction::ToolCall {
            call_id: "c1".into(),
            name: "search".into(),
            arguments: "{}".into(),
        };
        assert!(matches!(
            gate.evaluate_action(&agent, &tool, &test_state()).await,
            LoopDecision::Allow
        ));
    }
}
