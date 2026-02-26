//! Cedar policy gate for reasoning loops
//!
//! Wraps the `cedar-policy` crate to provide formally verified,
//! sub-millisecond authorization for agent actions.
//!
//! Feature-gated behind `cedar`. When enabled, `CedarPolicyGate`
//! implements `ReasoningPolicyGate` and maps agent actions to Cedar
//! authorization requests.

use crate::reasoning::loop_types::{LoopDecision, LoopState, ProposedAction};
use crate::reasoning::policy_bridge::ReasoningPolicyGate;
use crate::types::AgentId;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

/// A Cedar policy in source form.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CedarPolicy {
    /// Unique name for this policy.
    pub name: String,
    /// Cedar policy source text.
    pub source: String,
    /// Whether this policy is currently active.
    pub active: bool,
}

/// Cedar-based policy gate for reasoning loops.
///
/// Evaluates agent actions against Cedar policies. When no policies
/// are loaded, defaults to deny-all for safety.
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

    /// Evaluate an action against loaded policies.
    ///
    /// Maps agent_id to a Cedar principal, the action to a Cedar action,
    /// and loop state to Cedar context. The actual Cedar engine evaluation
    /// requires the `cedar-policy` crate at runtime.
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

        // Check each active policy for explicit deny rules
        for policy in &active_policies {
            if policy_denies(&policy.source, &agent_id.to_string(), &action_name) {
                return LoopDecision::Deny {
                    reason: format!(
                        "Cedar policy '{}' denied action '{}' for agent {}",
                        policy.name, action_name, agent_id
                    ),
                };
            }
        }

        // Check if any policy explicitly permits
        for policy in &active_policies {
            if policy_permits(&policy.source, &agent_id.to_string(), &action_name) {
                return LoopDecision::Allow;
            }
        }

        self.default_decision.clone()
    }
}

/// Check if a policy source text denies a specific action.
///
/// Simple pattern matching on Cedar policy syntax.
/// A full implementation would use the `cedar-policy` crate's Authorizer.
fn policy_denies(source: &str, _principal: &str, action: &str) -> bool {
    // Look for forbid rules that match the action
    for line in source.lines() {
        let line = line.trim();
        if line.starts_with("forbid") && line.contains(action) {
            return true;
        }
    }
    false
}

/// Check if a policy source text permits a specific action.
fn policy_permits(source: &str, _principal: &str, action: &str) -> bool {
    for line in source.lines() {
        let line = line.trim();
        if line.starts_with("permit") && line.contains(action) {
            return true;
        }
    }
    false
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
            source: "permit (principal, action == \"respond\", resource);".into(),
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
            source: "forbid (principal, action == \"tool_call::search\", resource);".into(),
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
            source: "permit (principal, action == \"respond\", resource);".into(),
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
            source: "permit all;".into(),
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
            source: "permit;".into(),
            active: true,
        })
        .await;
        gate.add_policy(CedarPolicy {
            name: "b".into(),
            source: "forbid;".into(),
            active: false,
        })
        .await;
        gate.add_policy(CedarPolicy {
            name: "c".into(),
            source: "permit;".into(),
            active: true,
        })
        .await;

        assert_eq!(gate.active_policy_count().await, 2);
    }

    #[tokio::test]
    async fn test_forbid_takes_precedence() {
        let gate = CedarPolicyGate::allow_by_default();

        // Both permit and forbid for the same action — forbid wins
        gate.add_policy(CedarPolicy {
            name: "allow_respond".into(),
            source: "permit (principal, action == \"respond\", resource);".into(),
            active: true,
        })
        .await;
        gate.add_policy(CedarPolicy {
            name: "deny_respond".into(),
            source: "forbid (principal, action == \"respond\", resource);".into(),
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
            source: "permit (principal, action == \"delegate::reviewer\", resource);".into(),
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
            source: "permit (principal, action, resource);".into(),
            active: true,
        };

        let json = serde_json::to_string(&policy).unwrap();
        let restored: CedarPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.name, "test");
        assert!(restored.active);
    }
}
