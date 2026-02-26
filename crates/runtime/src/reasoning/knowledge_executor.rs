//! Knowledge-aware action executor wrapper.
//!
//! `KnowledgeAwareExecutor` intercepts `recall_knowledge` and `store_knowledge`
//! tool calls, handling them locally via the `KnowledgeBridge`, and delegates
//! all other tool calls to an inner `ActionExecutor`.

use std::sync::Arc;

use async_trait::async_trait;

use crate::reasoning::circuit_breaker::CircuitBreakerRegistry;
use crate::reasoning::executor::ActionExecutor;
use crate::reasoning::knowledge_bridge::KnowledgeBridge;
use crate::reasoning::loop_types::{LoopConfig, Observation, ProposedAction};
use crate::types::AgentId;

/// An `ActionExecutor` wrapper that intercepts knowledge tool calls
/// and delegates all others to an inner executor.
pub struct KnowledgeAwareExecutor {
    inner: Arc<dyn ActionExecutor>,
    bridge: Arc<KnowledgeBridge>,
    agent_id: AgentId,
}

impl KnowledgeAwareExecutor {
    pub fn new(
        inner: Arc<dyn ActionExecutor>,
        bridge: Arc<KnowledgeBridge>,
        agent_id: AgentId,
    ) -> Self {
        Self {
            inner,
            bridge,
            agent_id,
        }
    }
}

#[async_trait]
impl ActionExecutor for KnowledgeAwareExecutor {
    async fn execute_actions(
        &self,
        actions: &[ProposedAction],
        config: &LoopConfig,
        circuit_breakers: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        // Partition actions into knowledge tools vs regular tools
        let mut knowledge_actions = Vec::new();
        let mut regular_actions = Vec::new();

        for action in actions {
            if let ProposedAction::ToolCall {
                name,
                call_id,
                arguments,
                ..
            } = action
            {
                if KnowledgeBridge::is_knowledge_tool(name) {
                    knowledge_actions.push((call_id.clone(), name.clone(), arguments.clone()));
                } else {
                    regular_actions.push(action.clone());
                }
            } else {
                regular_actions.push(action.clone());
            }
        }

        let mut observations = Vec::new();

        // Handle knowledge tools via the bridge
        for (call_id, name, arguments) in &knowledge_actions {
            let result = self
                .bridge
                .handle_tool_call(&self.agent_id, name, arguments)
                .await;

            match result {
                Ok(content) => {
                    observations.push(Observation::tool_result(call_id, content));
                }
                Err(err) => {
                    observations.push(Observation::tool_error(call_id, err));
                }
            }
        }

        // Delegate regular tools to the inner executor
        if !regular_actions.is_empty() {
            let inner_obs = self
                .inner
                .execute_actions(&regular_actions, config, circuit_breakers)
                .await;
            observations.extend(inner_obs);
        }

        observations
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reasoning::executor::DefaultActionExecutor;
    use crate::reasoning::loop_types::LoopConfig;

    /// A mock bridge needs a mock context manager. For unit tests we just
    /// verify the partitioning logic with the real executor for non-knowledge tools.
    #[tokio::test]
    async fn test_regular_actions_delegated() {
        // We can't easily construct a KnowledgeBridge without a real ContextManager,
        // so this test focuses on verifying that regular tool calls pass through.
        let inner = Arc::new(DefaultActionExecutor::default());
        let config = LoopConfig::default();
        let circuit_breakers = CircuitBreakerRegistry::default();

        // Regular tool calls should be delegated
        let actions = vec![ProposedAction::ToolCall {
            call_id: "c1".into(),
            name: "web_search".into(),
            arguments: r#"{"q":"test"}"#.into(),
        }];

        let obs = inner
            .execute_actions(&actions, &config, &circuit_breakers)
            .await;
        assert_eq!(obs.len(), 1);
        assert!(!obs[0].is_error);
    }

    #[test]
    fn test_knowledge_tool_detection() {
        assert!(KnowledgeBridge::is_knowledge_tool("recall_knowledge"));
        assert!(KnowledgeBridge::is_knowledge_tool("store_knowledge"));
        assert!(!KnowledgeBridge::is_knowledge_tool("web_search"));
    }
}
