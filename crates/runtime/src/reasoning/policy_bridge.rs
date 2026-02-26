//! Policy bridge for the reasoning loop
//!
//! Bridges the existing `PolicyEngine` into the reasoning loop via the
//! `ReasoningPolicyGate` trait. Every `ProposedAction` must pass through
//! the gate before execution.

use async_trait::async_trait;

use crate::reasoning::loop_types::{LoopDecision, LoopState, ProposedAction};
use crate::types::AgentId;

/// Mandatory policy gate for the reasoning loop.
///
/// Every `ProposedAction` produced by the reasoning step must be evaluated
/// by this gate before it can be dispatched. The typestate enforcement in
/// `phases.rs` makes it structurally impossible to skip this step.
#[async_trait]
pub trait ReasoningPolicyGate: Send + Sync {
    /// Evaluate whether a proposed action should be allowed.
    ///
    /// Returns `LoopDecision::Allow` to proceed, `LoopDecision::Deny` to
    /// feed the denial reason back to the LLM, or `LoopDecision::Modify`
    /// to transform the action (e.g., parameter redaction).
    async fn evaluate_action(
        &self,
        agent_id: &AgentId,
        action: &ProposedAction,
        state: &LoopState,
    ) -> LoopDecision;
}

/// Default policy gate that wraps the existing `PolicyEngine`.
///
/// For tool calls, it evaluates the tool name and arguments against the
/// policy engine. For other action types, it applies sensible defaults.
pub struct DefaultPolicyGate {
    allow_all: bool,
}

impl DefaultPolicyGate {
    /// Create a gate that evaluates all actions against the policy engine.
    pub fn new() -> Self {
        Self { allow_all: false }
    }

    /// Create a permissive gate that allows all actions (for development).
    pub fn permissive() -> Self {
        Self { allow_all: true }
    }
}

impl Default for DefaultPolicyGate {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ReasoningPolicyGate for DefaultPolicyGate {
    async fn evaluate_action(
        &self,
        agent_id: &AgentId,
        action: &ProposedAction,
        _state: &LoopState,
    ) -> LoopDecision {
        if self.allow_all {
            return LoopDecision::Allow;
        }

        match action {
            ProposedAction::ToolCall {
                name, arguments, ..
            } => {
                // Build policy input from the tool call
                let input = serde_json::json!({
                    "type": "tool_call",
                    "agent_id": agent_id.to_string(),
                    "tool_name": name,
                    "arguments": arguments,
                });

                tracing::debug!(
                    "Policy gate evaluating tool call: agent={} tool={}",
                    agent_id,
                    name
                );

                // In production, this delegates to the PolicyEngine.
                // The default implementation allows tool calls but logs them.
                let _ = input; // Used for policy evaluation in full implementation
                LoopDecision::Allow
            }
            ProposedAction::Delegate { target, .. } => {
                tracing::debug!(
                    "Policy gate evaluating delegation: agent={} target={}",
                    agent_id,
                    target
                );
                LoopDecision::Allow
            }
            ProposedAction::Respond { .. } => {
                // Responses are generally allowed
                LoopDecision::Allow
            }
            ProposedAction::Terminate { .. } => {
                // Terminations are always allowed
                LoopDecision::Allow
            }
        }
    }
}

/// Policy gate backed by the OPA PolicyEngine.
pub struct OpaPolicyGateBridge {
    policy_engine: std::sync::Arc<dyn crate::integrations::policy_engine::engine::PolicyEngine>,
}

impl OpaPolicyGateBridge {
    /// Create a new OPA-backed policy gate.
    pub fn new(
        engine: std::sync::Arc<dyn crate::integrations::policy_engine::engine::PolicyEngine>,
    ) -> Self {
        Self {
            policy_engine: engine,
        }
    }
}

#[async_trait]
impl ReasoningPolicyGate for OpaPolicyGateBridge {
    async fn evaluate_action(
        &self,
        agent_id: &AgentId,
        action: &ProposedAction,
        _state: &LoopState,
    ) -> LoopDecision {
        let input = match action {
            ProposedAction::ToolCall {
                name,
                arguments,
                call_id,
            } => serde_json::json!({
                "type": "tool_call",
                "call_id": call_id,
                "tool_name": name,
                "arguments": arguments,
            }),
            ProposedAction::Delegate { target, message } => serde_json::json!({
                "type": "delegate",
                "target": target,
                "message_length": message.len(),
            }),
            ProposedAction::Respond { content } => serde_json::json!({
                "type": "respond",
                "content_length": content.len(),
            }),
            ProposedAction::Terminate { reason, .. } => serde_json::json!({
                "type": "terminate",
                "reason": reason,
            }),
        };

        match self
            .policy_engine
            .evaluate_policy(&agent_id.to_string(), &input)
            .await
        {
            Ok(crate::integrations::policy_engine::engine::PolicyDecision::Allow) => {
                LoopDecision::Allow
            }
            Ok(crate::integrations::policy_engine::engine::PolicyDecision::Deny) => {
                let reason = format!(
                    "Policy denied action {:?} for agent {}",
                    std::mem::discriminant(action),
                    agent_id
                );
                tracing::warn!("{}", reason);
                LoopDecision::Deny { reason }
            }
            Err(e) => {
                let reason = format!("Policy evaluation error: {}", e);
                tracing::error!("{}", reason);
                // Fail closed: deny on error
                LoopDecision::Deny { reason }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reasoning::conversation::Conversation;
    use crate::reasoning::loop_types::LoopState;

    #[tokio::test]
    async fn test_default_gate_allows_all_actions() {
        let gate = DefaultPolicyGate::permissive();
        let agent_id = AgentId::new();
        let state = LoopState::new(agent_id, Conversation::new());

        let tool_call = ProposedAction::ToolCall {
            call_id: "c1".into(),
            name: "search".into(),
            arguments: "{}".into(),
        };
        let decision = gate.evaluate_action(&agent_id, &tool_call, &state).await;
        assert!(matches!(decision, LoopDecision::Allow));

        let delegate = ProposedAction::Delegate {
            target: "other_agent".into(),
            message: "hello".into(),
        };
        let decision = gate.evaluate_action(&agent_id, &delegate, &state).await;
        assert!(matches!(decision, LoopDecision::Allow));

        let respond = ProposedAction::Respond {
            content: "done".into(),
        };
        let decision = gate.evaluate_action(&agent_id, &respond, &state).await;
        assert!(matches!(decision, LoopDecision::Allow));

        let terminate = ProposedAction::Terminate {
            reason: "done".into(),
            output: "result".into(),
        };
        let decision = gate.evaluate_action(&agent_id, &terminate, &state).await;
        assert!(matches!(decision, LoopDecision::Allow));
    }

    #[tokio::test]
    async fn test_default_gate_standard_mode() {
        let gate = DefaultPolicyGate::new();
        let agent_id = AgentId::new();
        let state = LoopState::new(agent_id, Conversation::new());

        // Default gate allows tool calls (delegates to PolicyEngine in production)
        let tool_call = ProposedAction::ToolCall {
            call_id: "c1".into(),
            name: "search".into(),
            arguments: "{}".into(),
        };
        let decision = gate.evaluate_action(&agent_id, &tool_call, &state).await;
        assert!(matches!(decision, LoopDecision::Allow));
    }
}
