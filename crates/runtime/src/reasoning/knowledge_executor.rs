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
    fn prepare_action(
        &self,
        action: &ProposedAction,
        config: &LoopConfig,
    ) -> Result<super::prepared::PreparedAction, String> {
        if matches!(action, ProposedAction::ToolCall { name, .. } if KnowledgeBridge::is_knowledge_tool(name))
        {
            super::executor::prepare_registered_action(
                action,
                config,
                &self.bridge.tool_definitions(),
            )
        } else {
            self.inner.prepare_action(action, config)
        }
    }

    fn cancel_run(&self, run: &str, deadline: std::time::Instant) {
        self.inner.cancel_run(run, deadline);
    }

    async fn close_run(&self, run: &str, deadline: std::time::Instant) -> Result<(), String> {
        self.inner.close_run(run, deadline).await
    }

    async fn execute_authorized(
        &self,
        actions: Vec<super::prepared::AuthorizedAction>,
        config: &LoopConfig,
        circuit_breakers: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        let mut regular = Vec::new();
        let mut observations = Vec::new();
        for grant in actions {
            if matches!(grant.action(), ProposedAction::ToolCall { name, .. } if KnowledgeBridge::is_knowledge_tool(name))
            {
                let action = grant.action().clone();
                match grant.into_prepared() {
                    Ok(prepared) => observations.extend(
                        self.execute_actions(
                            &[prepared.action().clone()],
                            config,
                            circuit_breakers,
                        )
                        .await,
                    ),
                    Err(error) => {
                        if let ProposedAction::ToolCall { name, call_id, .. } = action {
                            observations
                                .push(Observation::tool_error(name, error).with_call_id(call_id));
                        }
                    }
                }
            } else {
                regular.push(grant);
            }
        }
        observations.extend(
            self.inner
                .execute_authorized(regular, config, circuit_breakers)
                .await,
        );
        observations
    }

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
                    observations
                        .push(Observation::tool_result(name, content).with_call_id(call_id));
                }
                Err(err) => {
                    observations.push(Observation::tool_error(name, err).with_call_id(call_id));
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

    #[tokio::test]
    async fn regular_actions_preserve_backend_failure_and_call_identity() {
        let root = tempfile::tempdir().unwrap();
        let agent_id = AgentId::new();
        let context = crate::context::manager::StandardContextManager::new(
            crate::context::manager::ContextManagerConfig {
                enable_auto_archiving: false,
                enable_persistence: false,
                secrets_config: crate::SecretsConfig::file_json(
                    root.path().join("fixture-secrets.json"),
                ),
                ..Default::default()
            },
            &agent_id.to_string(),
        )
        .await
        .unwrap();
        let bridge = Arc::new(KnowledgeBridge::new(Arc::new(context), Default::default()));
        let executor = KnowledgeAwareExecutor::new(
            Arc::new(DefaultActionExecutor::default()),
            bridge,
            agent_id,
        );
        let config = LoopConfig::default();
        let circuit_breakers = CircuitBreakerRegistry::default();

        // Regular tool calls should be delegated
        let actions = vec![ProposedAction::ToolCall {
            call_id: "c1".into(),
            name: "web_search".into(),
            arguments: r#"{"q":"test"}"#.into(),
        }];

        let obs = executor
            .execute_actions(&actions, &config, &circuit_breakers)
            .await;
        assert_eq!(obs.len(), 1);
        assert!(obs[0].is_error);
        assert_eq!(obs[0].call_id.as_deref(), Some("c1"));
        assert_eq!(obs[0].source, "web_search");
        assert!(obs[0].content.contains("no tool backend"));
    }

    #[test]
    fn test_knowledge_tool_detection() {
        assert!(KnowledgeBridge::is_knowledge_tool("recall_knowledge"));
        assert!(KnowledgeBridge::is_knowledge_tool("store_knowledge"));
        assert!(!KnowledgeBridge::is_knowledge_tool("web_search"));
    }
}
