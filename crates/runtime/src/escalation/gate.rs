//! Decorator that turns approval-required actions into held escalations.
use std::sync::Arc;
use std::time::Duration;

use crate::escalation::{Decision, EscalationQueue, EscalationRequest, HeldActionKind};
use crate::reasoning::loop_types::{LoopDecision, LoopState, ProposedAction};
use crate::reasoning::policy_bridge::ReasoningPolicyGate;
use crate::types::AgentId;

/// Configuration for the escalation gate.
#[derive(Debug, Clone)]
pub struct EscalationGateConfig {
    /// Tool names that require human approval before proceeding.
    pub require_approval_tools: Vec<String>,
    /// How long to wait for a human decision before timing out (→ Deny).
    pub timeout: Duration,
}

/// A `ReasoningPolicyGate` decorator that routes approval-required tool calls
/// through the human escalation queue before delegating to the inner gate.
pub struct EscalationGate {
    inner: Arc<dyn ReasoningPolicyGate>,
    queue: Arc<EscalationQueue>,
    config: EscalationGateConfig,
}

impl EscalationGate {
    /// Wrap an existing gate with escalation behaviour.
    pub fn new(
        inner: Arc<dyn ReasoningPolicyGate>,
        queue: Arc<EscalationQueue>,
        config: EscalationGateConfig,
    ) -> Self {
        Self {
            inner,
            queue,
            config,
        }
    }

    /// Extract the tool name from a `ProposedAction`, if it is a `ToolCall`.
    fn tool_name(action: &ProposedAction) -> Option<String> {
        match action {
            ProposedAction::ToolCall { name, .. } => Some(name.clone()),
            _ => None,
        }
    }
}

#[async_trait::async_trait]
impl ReasoningPolicyGate for EscalationGate {
    async fn evaluate_action(
        &self,
        agent_id: &AgentId,
        action: &ProposedAction,
        state: &LoopState,
    ) -> LoopDecision {
        let tool = Self::tool_name(action);
        let needs_approval = tool
            .as_deref()
            .map(|t| self.config.require_approval_tools.iter().any(|r| r == t))
            .unwrap_or(false);

        if !needs_approval {
            return self.inner.evaluate_action(agent_id, action, state).await;
        }

        let summary = tool.unwrap_or_else(|| "action".into());
        let req = EscalationRequest {
            agent_id: agent_id.to_string(),
            kind: HeldActionKind::ToolCall,
            summary: format!("tool_call {summary}"),
            reason: "policy requires human approval".to_string(),
            context_snapshot: serde_json::to_value(action).ok(),
        };

        match self.queue.enqueue(req, self.config.timeout).await {
            Decision::Approve { .. } => self.inner.evaluate_action(agent_id, action, state).await,
            Decision::Deny { reason } => LoopDecision::Deny {
                reason: reason.unwrap_or_else(|| "denied by operator".into()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::escalation::{Approver, Surface};
    use crate::reasoning::conversation::Conversation;
    use crate::reasoning::loop_types::{LoopState, ProposedAction};
    use std::sync::Arc;
    use std::time::Duration;

    struct AlwaysAllow;

    #[async_trait::async_trait]
    impl ReasoningPolicyGate for AlwaysAllow {
        async fn evaluate_action(
            &self,
            _a: &AgentId,
            _act: &ProposedAction,
            _s: &LoopState,
        ) -> LoopDecision {
            LoopDecision::Allow
        }
    }

    fn approver() -> Approver {
        Approver {
            surface: Surface::Tui,
            id: "op1".into(),
            display: "op1".into(),
        }
    }

    fn action(tool: &str) -> ProposedAction {
        ProposedAction::ToolCall {
            call_id: "cid-1".into(),
            name: tool.into(),
            arguments: "{}".into(),
        }
    }

    fn state(agent: &AgentId) -> LoopState {
        LoopState::new(*agent, Conversation::new())
    }

    fn config() -> EscalationGateConfig {
        EscalationGateConfig {
            require_approval_tools: vec!["http_post".into()],
            timeout: Duration::from_secs(5),
        }
    }

    #[tokio::test]
    async fn approval_required_tool_blocks_then_allows_on_approve() {
        let q = Arc::new(EscalationQueue::new());
        let gate = Arc::new(EscalationGate::new(
            Arc::new(AlwaysAllow),
            q.clone(),
            config(),
        ));
        let agent = AgentId::new();
        let act = action("http_post");
        let st = state(&agent);

        let gate2 = gate.clone();
        let agent2 = agent;
        let fut = tokio::spawn(async move { gate2.evaluate_action(&agent2, &act, &st).await });

        let id = loop {
            if let Some(h) = q.list_pending_async().await.first() {
                break h.id.clone();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        };

        q.resolve_async(&id, Decision::Approve { reason: None }, approver())
            .await
            .unwrap();
        assert!(matches!(fut.await.unwrap(), LoopDecision::Allow));
    }

    #[tokio::test]
    async fn non_listed_tool_delegates_to_inner() {
        let q = Arc::new(EscalationQueue::new());
        let gate = EscalationGate::new(Arc::new(AlwaysAllow), q.clone(), config());
        let agent = AgentId::new();
        let st = state(&agent);

        let d = gate
            .evaluate_action(&agent, &action("read_file"), &st)
            .await;
        assert!(matches!(d, LoopDecision::Allow));
        assert!(q.list_pending_async().await.is_empty());
    }

    #[tokio::test]
    async fn denied_on_deny() {
        let q = Arc::new(EscalationQueue::new());
        let gate = Arc::new(EscalationGate::new(
            Arc::new(AlwaysAllow),
            q.clone(),
            config(),
        ));
        let agent = AgentId::new();
        let act = action("http_post");
        let st = state(&agent);

        let gate2 = gate.clone();
        let agent2 = agent;
        let fut = tokio::spawn(async move { gate2.evaluate_action(&agent2, &act, &st).await });

        let id = loop {
            if let Some(h) = q.list_pending_async().await.first() {
                break h.id.clone();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        };

        q.resolve_async(
            &id,
            Decision::Deny {
                reason: Some("no".into()),
            },
            approver(),
        )
        .await
        .unwrap();
        assert!(matches!(fut.await.unwrap(), LoopDecision::Deny { .. }));
    }
}
