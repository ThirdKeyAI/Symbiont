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

/// Default policy gate.
///
/// In its non-permissive mode (`DefaultPolicyGate::new()`) this gate is
/// **fail-closed**: every `ToolCall` and `Delegate` action is denied with
/// an explicit reason instructing the operator to wire a real policy
/// backend (e.g. [`OpaPolicyGateBridge`]) or to opt into the dev-only
/// permissive mode via [`DefaultPolicyGate::permissive_for_dev_only`].
/// `Respond` and `Terminate` remain allowed so the reasoning loop can
/// still surface the denial back to the caller.
pub struct DefaultPolicyGate {
    allow_all: bool,
}

impl DefaultPolicyGate {
    /// Create a fail-closed gate. `ToolCall` and `Delegate` actions are
    /// denied by default; only `Respond` and `Terminate` pass.
    ///
    /// Wire [`OpaPolicyGateBridge`] (or another `ReasoningPolicyGate`
    /// implementation) for production deployments.
    pub fn new() -> Self {
        Self { allow_all: false }
    }

    /// WARNING: Allows every tool call and delegation. Only safe for local
    /// development. Production deployments MUST wire `OpaPolicyGateBridge`.
    ///
    /// Use behind an explicit operator opt-in (e.g. `--insecure-allow-all`
    /// CLI flag or `SYMBI_INSECURE_ALLOW_ALL=1` env var) and surface a
    /// loud warning at startup.
    #[doc(hidden)]
    pub fn permissive_for_dev_only() -> Self {
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
            // Loud warning on every action so insecure permissive mode is
            // visible in production logs, not just at construction time.
            tracing::warn!(
                "DefaultPolicyGate is in insecure permissive mode — action={:?}",
                action
            );
            return LoopDecision::Allow;
        }

        match action {
            ProposedAction::ToolCall { name, .. } => {
                tracing::debug!(
                    "Policy gate denying tool call (fail-closed default): agent={} tool={}",
                    agent_id,
                    name
                );
                LoopDecision::Deny {
                    reason: "No policy gate configured (DefaultPolicyGate::new is fail-closed; wire OpaPolicyGateBridge or pass --insecure-allow-all)".to_string(),
                }
            }
            ProposedAction::Delegate { target, .. } => {
                tracing::debug!(
                    "Policy gate denying delegation (fail-closed default): agent={} target={}",
                    agent_id,
                    target
                );
                LoopDecision::Deny {
                    reason: "No policy gate configured (DefaultPolicyGate::new is fail-closed; wire OpaPolicyGateBridge or pass --insecure-allow-all)".to_string(),
                }
            }
            ProposedAction::Respond { .. } => {
                // Responses are always allowed so the loop can surface
                // policy decisions back to the caller.
                LoopDecision::Allow
            }
            ProposedAction::Terminate { .. } => {
                // Terminations are always allowed.
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

/// Policy gate that restricts tool access by name whitelist.
///
/// Non-tool actions (Respond, Delegate, Terminate) always pass through.
/// Use `ToolFilterPolicyGate::allow(&["tool_a", "tool_b"])` to restrict
/// to specific tools, or `ToolFilterPolicyGate::allow_all()` for no restriction.
pub struct ToolFilterPolicyGate {
    allowed_tools: std::collections::HashSet<String>,
    allow_all: bool,
}

impl ToolFilterPolicyGate {
    /// Create a gate that only allows the specified tools.
    pub fn allow(tools: &[&str]) -> Self {
        Self {
            allowed_tools: tools.iter().map(|s| s.to_string()).collect(),
            allow_all: false,
        }
    }

    /// Create a gate that allows all tools (no filtering).
    pub fn allow_all() -> Self {
        Self {
            allowed_tools: std::collections::HashSet::new(),
            allow_all: true,
        }
    }
}

#[async_trait]
impl ReasoningPolicyGate for ToolFilterPolicyGate {
    async fn evaluate_action(
        &self,
        _agent_id: &AgentId,
        action: &ProposedAction,
        _state: &LoopState,
    ) -> LoopDecision {
        if self.allow_all {
            return LoopDecision::Allow;
        }
        match action {
            ProposedAction::ToolCall { name, .. } => {
                if self.allowed_tools.contains(name.as_str()) {
                    LoopDecision::Allow
                } else {
                    LoopDecision::Deny {
                        reason: format!("Tool '{}' not in allowed list", name),
                    }
                }
            }
            _ => LoopDecision::Allow,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reasoning::conversation::Conversation;
    use crate::reasoning::loop_types::LoopState;

    #[tokio::test]
    async fn test_permissive_dev_only_allows_all_actions() {
        let gate = DefaultPolicyGate::permissive_for_dev_only();
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
    async fn test_default_gate_fail_closed_tool_call() {
        let gate = DefaultPolicyGate::new();
        let agent_id = AgentId::new();
        let state = LoopState::new(agent_id, Conversation::new());

        let tool_call = ProposedAction::ToolCall {
            call_id: "c1".into(),
            name: "search".into(),
            arguments: "{}".into(),
        };
        let decision = gate.evaluate_action(&agent_id, &tool_call, &state).await;
        assert!(matches!(decision, LoopDecision::Deny { .. }));

        let delegate = ProposedAction::Delegate {
            target: "other".into(),
            message: "hi".into(),
        };
        let decision = gate.evaluate_action(&agent_id, &delegate, &state).await;
        assert!(matches!(decision, LoopDecision::Deny { .. }));

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
    async fn test_tool_filter_allows_whitelisted_tools() {
        let gate = ToolFilterPolicyGate::allow(&["search", "calculator"]);
        let agent_id = AgentId::new();
        let state = LoopState::new(agent_id, Conversation::new());

        let allowed = ProposedAction::ToolCall {
            call_id: "c1".into(),
            name: "search".into(),
            arguments: "{}".into(),
        };
        let decision = gate.evaluate_action(&agent_id, &allowed, &state).await;
        assert!(matches!(decision, LoopDecision::Allow));
    }

    #[tokio::test]
    async fn test_tool_filter_denies_non_whitelisted_tools() {
        let gate = ToolFilterPolicyGate::allow(&["search"]);
        let agent_id = AgentId::new();
        let state = LoopState::new(agent_id, Conversation::new());

        let denied = ProposedAction::ToolCall {
            call_id: "c1".into(),
            name: "delete_everything".into(),
            arguments: "{}".into(),
        };
        let decision = gate.evaluate_action(&agent_id, &denied, &state).await;
        assert!(matches!(decision, LoopDecision::Deny { .. }));
        if let LoopDecision::Deny { reason } = decision {
            assert!(reason.contains("delete_everything"));
            assert!(reason.contains("not in allowed list"));
        }
    }

    #[tokio::test]
    async fn test_tool_filter_allows_non_tool_actions() {
        let gate = ToolFilterPolicyGate::allow(&["search"]);
        let agent_id = AgentId::new();
        let state = LoopState::new(agent_id, Conversation::new());

        let respond = ProposedAction::Respond {
            content: "hello".into(),
        };
        let decision = gate.evaluate_action(&agent_id, &respond, &state).await;
        assert!(matches!(decision, LoopDecision::Allow));

        let delegate = ProposedAction::Delegate {
            target: "other".into(),
            message: "hi".into(),
        };
        let decision = gate.evaluate_action(&agent_id, &delegate, &state).await;
        assert!(matches!(decision, LoopDecision::Allow));

        let terminate = ProposedAction::Terminate {
            reason: "done".into(),
            output: "result".into(),
        };
        let decision = gate.evaluate_action(&agent_id, &terminate, &state).await;
        assert!(matches!(decision, LoopDecision::Allow));
    }

    #[tokio::test]
    async fn test_tool_filter_allow_all() {
        let gate = ToolFilterPolicyGate::allow_all();
        let agent_id = AgentId::new();
        let state = LoopState::new(agent_id, Conversation::new());

        let tool_call = ProposedAction::ToolCall {
            call_id: "c1".into(),
            name: "anything".into(),
            arguments: "{}".into(),
        };
        let decision = gate.evaluate_action(&agent_id, &tool_call, &state).await;
        assert!(matches!(decision, LoopDecision::Allow));
    }
}
