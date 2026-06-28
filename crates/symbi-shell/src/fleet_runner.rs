//! Builds a governed per-agent `Orchestrator` for fleet agents addressed with
//! `@name`. Shares the inference provider and policy gate with the main
//! orchestrator; each agent gets a tool-scoped executor (manifest tools, never
//! `delegate`).

use std::collections::HashSet;
use std::sync::Arc;
use symbi_runtime::reasoning::executor::ActionExecutor;
use symbi_runtime::reasoning::inference::InferenceProvider;
use symbi_runtime::reasoning::policy_bridge::ReasoningPolicyGate;

use crate::agents::AgentCard;
use crate::orchestrator::Orchestrator;
use crate::orchestrator_executor::OrchestratorExecutor;
use crate::validation::constraints::ProjectConstraints;

pub struct FleetRunnerFactory {
    provider: Arc<dyn InferenceProvider>,
    constraints: Arc<ProjectConstraints>,
    bridge: Arc<repl_core::RuntimeBridge>,
    cards: Arc<tokio::sync::RwLock<Vec<AgentCard>>>,
    allow_shell: bool,
    gate: Arc<dyn ReasoningPolicyGate>,
}

impl FleetRunnerFactory {
    pub fn new(
        provider: Arc<dyn InferenceProvider>,
        constraints: Arc<ProjectConstraints>,
        bridge: Arc<repl_core::RuntimeBridge>,
        cards: Arc<tokio::sync::RwLock<Vec<AgentCard>>>,
        allow_shell: bool,
        gate: Arc<dyn ReasoningPolicyGate>,
    ) -> Self {
        Self {
            provider,
            constraints,
            bridge,
            cards,
            allow_shell,
            gate,
        }
    }

    /// Build a governed runner for `name`, scoped to `manifest_tools` minus
    /// `delegate` (orchestrator-only). Returns `None` if the agent is unknown.
    pub async fn build(&self, name: &str, manifest_tools: &[String]) -> Option<Orchestrator> {
        let system_prompt = self.bridge.agent_system_prompt(name).await?;
        let allowed: HashSet<String> = manifest_tools
            .iter()
            .filter(|t| t.as_str() != "delegate")
            .cloned()
            .collect();
        let engine = Arc::new(repl_core::ReplEngine::new(Arc::clone(&self.bridge)));
        let executor: Arc<dyn ActionExecutor> = Arc::new(
            OrchestratorExecutor::new(
                Arc::clone(&self.constraints),
                engine,
                Arc::clone(&self.bridge),
                Arc::clone(&self.cards),
                self.allow_shell,
            )
            .with_allowed_tools(allowed),
        );
        Some(Orchestrator::for_agent(
            Arc::clone(&self.provider),
            executor,
            Arc::clone(&self.gate),
            &system_prompt,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use symbi_runtime::reasoning::inference::{
        FinishReason, InferenceError, InferenceOptions, InferenceResponse, Usage,
    };
    use symbi_runtime::reasoning::policy_bridge::DefaultPolicyGate;

    struct Mock;
    #[async_trait]
    impl InferenceProvider for Mock {
        async fn complete(
            &self,
            _c: &symbi_runtime::reasoning::conversation::Conversation,
            _o: &InferenceOptions,
        ) -> Result<InferenceResponse, InferenceError> {
            Ok(InferenceResponse {
                content: "ok".into(),
                tool_calls: vec![],
                finish_reason: FinishReason::Stop,
                usage: Usage::default(),
                model: "mock".into(),
            })
        }
        fn provider_name(&self) -> &str {
            "mock"
        }
        fn default_model(&self) -> &str {
            "mock"
        }
        fn supports_native_tools(&self) -> bool {
            false
        }
        fn supports_structured_output(&self) -> bool {
            false
        }
    }

    #[tokio::test]
    async fn build_unknown_agent_is_none() {
        let bridge = Arc::new(repl_core::RuntimeBridge::new_permissive_for_dev());
        let cards = Arc::new(tokio::sync::RwLock::new(Vec::<AgentCard>::new()));
        let f = FleetRunnerFactory::new(
            Arc::new(Mock),
            Arc::new(ProjectConstraints::default()),
            bridge,
            cards,
            false,
            Arc::new(DefaultPolicyGate::permissive_for_dev_only()),
        );
        assert!(f.build("ghost", &[]).await.is_none());
    }

    #[tokio::test]
    async fn build_known_agent_returns_runner() {
        let bridge = Arc::new(repl_core::RuntimeBridge::new_permissive_for_dev());
        bridge
            .register_agent("reviewer", "You review.", vec!["read_file".into()])
            .await;
        let cards = Arc::new(tokio::sync::RwLock::new(Vec::<AgentCard>::new()));
        let f = FleetRunnerFactory::new(
            Arc::new(Mock),
            Arc::new(ProjectConstraints::default()),
            bridge,
            cards,
            false,
            Arc::new(DefaultPolicyGate::permissive_for_dev_only()),
        );
        let mut runner = f
            .build("reviewer", &["read_file".to_string()])
            .await
            .unwrap();
        assert_eq!(runner.send("hi").await.unwrap().content, "ok");
    }
}
