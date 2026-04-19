use std::sync::{Arc, Mutex};
use symbi_runtime::communication::policy_gate::CommunicationPolicyGate;
use symbi_runtime::communication::{
    CommunicationBus, CommunicationConfig, DefaultCommunicationBus,
};
use symbi_runtime::context::manager::{ContextManagerConfig, StandardContextManager};
use symbi_runtime::integrations::policy_engine::engine::{
    OpaPolicyEngine, PolicyDecision, PolicyEngine,
};
use symbi_runtime::lifecycle::{DefaultLifecycleController, LifecycleConfig, LifecycleController};
use symbi_runtime::reasoning::agent_registry::AgentRegistry;
use symbi_runtime::reasoning::inference::InferenceProvider;
use symbi_runtime::types::agent::AgentConfig;
use symbi_runtime::types::security::Capability;
use symbi_runtime::types::AgentId;

/// The RuntimeBridge manages a simulated, in-process Symbiont runtime environment.
pub struct RuntimeBridge {
    lifecycle_controller: Arc<Mutex<Option<Arc<DefaultLifecycleController>>>>,
    context_manager: Arc<Mutex<Option<Arc<StandardContextManager>>>>,
    policy_engine: Arc<Mutex<OpaPolicyEngine>>,
    /// Inference provider for reasoning builtins.
    inference_provider: Arc<Mutex<Option<Arc<dyn InferenceProvider>>>>,
    /// Agent registry for multi-agent composition.
    agent_registry: Arc<AgentRegistry>,
    /// Communication bus for agent-to-agent messaging (set in initialize()).
    comm_bus: Arc<Mutex<Option<Arc<dyn CommunicationBus + Send + Sync>>>>,
    /// Communication policy gate (deny-by-default; replaced via set_comm_policy).
    comm_policy: Arc<Mutex<Arc<CommunicationPolicyGate>>>,
}

impl Default for RuntimeBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeBridge {
    /// Construct a RuntimeBridge with a **deny-by-default** communication policy.
    ///
    /// Production callers should immediately follow with [`Self::set_comm_policy`]
    /// to install a policy containing the rules required for their agent topology.
    /// Inter-agent messaging will fail with `PolicyDenied` until rules are configured.
    ///
    /// For tests or trusted single-tenant dev environments, use
    /// [`Self::new_permissive_for_dev`] to opt into allow-all semantics.
    pub fn new() -> Self {
        Self::with_policy(Arc::new(CommunicationPolicyGate::new(Vec::new())))
    }

    /// Construct a RuntimeBridge with a permissive (allow-all) communication policy.
    ///
    /// This is ONLY safe for local development, tests, or single-tenant
    /// trusted environments. Never use in multi-tenant or production deployments:
    /// permissive mode allows any agent to message any other agent with no gating.
    pub fn new_permissive_for_dev() -> Self {
        Self::with_policy(Arc::new(CommunicationPolicyGate::permissive()))
    }

    /// Construct a RuntimeBridge with an explicit communication policy gate.
    pub fn with_policy(comm_policy_gate: Arc<CommunicationPolicyGate>) -> Self {
        let lifecycle_controller = Arc::new(Mutex::new(None));
        let context_manager = Arc::new(Mutex::new(None));
        let policy_engine = Arc::new(Mutex::new(OpaPolicyEngine::new()));
        let inference_provider = Arc::new(Mutex::new(None));
        let agent_registry = Arc::new(AgentRegistry::new());
        let comm_bus = Arc::new(Mutex::new(None));
        let comm_policy = Arc::new(Mutex::new(comm_policy_gate));

        Self {
            lifecycle_controller,
            context_manager,
            policy_engine,
            inference_provider,
            agent_registry,
            comm_bus,
            comm_policy,
        }
    }

    /// Set the inference provider for reasoning builtins.
    pub fn set_inference_provider(&self, provider: Arc<dyn InferenceProvider>) {
        *self.inference_provider.lock().unwrap() = Some(provider);
    }

    /// Get the agent registry.
    pub fn agent_registry(&self) -> Arc<AgentRegistry> {
        Arc::clone(&self.agent_registry)
    }

    /// Get the communication bus (if initialized).
    pub fn comm_bus(&self) -> Option<Arc<dyn CommunicationBus + Send + Sync>> {
        self.comm_bus.lock().unwrap().clone()
    }

    /// Replace the communication policy gate.
    pub fn set_comm_policy(&self, policy: Arc<CommunicationPolicyGate>) {
        *self.comm_policy.lock().unwrap() = policy;
    }

    /// Get the reasoning context for async builtins.
    ///
    /// Includes the communication bus and policy gate if they've been initialized
    /// via [`initialize`]. The bus is used by `ask`, `send_to`, `parallel`, and
    /// `race` builtins for policy-gated, audit-logged agent-to-agent messaging.
    pub fn reasoning_context(&self) -> crate::dsl::reasoning_builtins::ReasoningBuiltinContext {
        let provider = self.inference_provider.lock().unwrap().clone();
        let comm_bus = self.comm_bus.lock().unwrap().clone();
        let comm_policy = Some(self.comm_policy.lock().unwrap().clone());
        crate::dsl::reasoning_builtins::ReasoningBuiltinContext {
            provider,
            agent_registry: Some(Arc::clone(&self.agent_registry)),
            sender_agent_id: None,
            comm_bus,
            comm_policy,
            // RuntimeBridge today does not own a ReasoningPolicyGate; the
            // reasoning builtin will fall back to DefaultPolicyGate::new()
            // (production-default). Callers embedding the runtime should
            // install their concrete gate directly via the SDK.
            reasoning_policy_gate: None,
        }
    }

    /// Initialize the runtime bridge components asynchronously.
    ///
    /// Sets up the lifecycle controller, context manager, and communication bus.
    /// After this returns, `reasoning_context()` produces a context with a live
    /// bus and policy gate, so DSL builtins like `ask` and `send_to` will route
    /// messages through the audited communication path.
    pub async fn initialize(&self) -> Result<(), String> {
        // Initialize lifecycle controller
        let lifecycle_config = LifecycleConfig::default();
        let lifecycle_controller = Arc::new(
            DefaultLifecycleController::new(lifecycle_config)
                .await
                .map_err(|e| format!("Failed to create lifecycle controller: {}", e))?,
        );

        // Initialize context manager
        let context_config = ContextManagerConfig::default();
        let context_manager = Arc::new(
            StandardContextManager::new(context_config, "runtime_bridge")
                .await
                .map_err(|e| format!("Failed to create context manager: {}", e))?,
        );

        // Initialize the context manager
        context_manager
            .initialize()
            .await
            .map_err(|e| format!("Failed to initialize context manager: {}", e))?;

        // Initialize the communication bus
        let bus_config = CommunicationConfig::default();
        let bus = Arc::new(
            DefaultCommunicationBus::new(bus_config)
                .await
                .map_err(|e| format!("Failed to create communication bus: {}", e))?,
        ) as Arc<dyn CommunicationBus + Send + Sync>;

        // Store the initialized components
        *self.lifecycle_controller.lock().unwrap() = Some(lifecycle_controller);
        *self.context_manager.lock().unwrap() = Some(context_manager);
        *self.comm_bus.lock().unwrap() = Some(bus);

        Ok(())
    }

    pub async fn initialize_agent(&self, config: AgentConfig) -> Result<AgentId, String> {
        let controller = {
            let controller_guard = self.lifecycle_controller.lock().unwrap();
            controller_guard.clone()
        };

        if let Some(controller) = controller {
            controller
                .initialize_agent(config)
                .await
                .map_err(|e| e.to_string())
        } else {
            Err("Lifecycle controller not initialized".to_string())
        }
    }

    /// Checks if a given capability is allowed for an agent.
    pub async fn check_capability(
        &self,
        agent_id: &str,
        capability: &Capability,
    ) -> Result<PolicyDecision, String> {
        // Clone the engine to avoid holding the lock across the await
        let engine = {
            let engine_guard = self.policy_engine.lock().unwrap();
            engine_guard.clone()
        };
        engine
            .check_capability(agent_id, capability)
            .await
            .map_err(|e| e.to_string())
    }

    /// Register an event handler for an agent (stub implementation)
    pub async fn register_event_handler(
        &self,
        agent_id: &str,
        event_name: &str,
        _event_type: &str,
    ) -> Result<(), String> {
        tracing::info!(
            "Registered event handler '{}' for agent {}",
            event_name,
            agent_id
        );
        Ok(())
    }

    /// Emit an event from an agent (stub implementation)
    pub async fn emit_event(
        &self,
        agent_id: &str,
        event_name: &str,
        _data: &serde_json::Value,
    ) -> Result<(), String> {
        tracing::info!("Agent {} emitted event: {}", agent_id, event_name);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_reasoning_context_before_init_has_no_bus() {
        let bridge = RuntimeBridge::new();
        let ctx = bridge.reasoning_context();
        // Before initialize, bus is None but the policy gate is always Some
        // (deny-by-default by construction).
        assert!(ctx.comm_bus.is_none());
        assert!(ctx.comm_policy.is_some());
    }

    #[tokio::test]
    async fn test_new_default_policy_denies() {
        use symbi_runtime::types::MessageType;
        let bridge = RuntimeBridge::new();
        let ctx = bridge.reasoning_context();
        let policy = ctx.comm_policy.expect("policy present");
        let recipient = AgentId::new();
        let request = symbi_runtime::communication::policy_gate::CommunicationRequest {
            sender: AgentId::new(),
            recipient,
            message_type: MessageType::Direct(recipient),
            topic: None,
        };
        assert!(
            policy.evaluate(&request).is_err(),
            "default policy must be deny-by-default"
        );
    }

    #[tokio::test]
    async fn test_permissive_for_dev_allows() {
        use symbi_runtime::types::MessageType;
        let bridge = RuntimeBridge::new_permissive_for_dev();
        let ctx = bridge.reasoning_context();
        let policy = ctx.comm_policy.expect("policy present");
        let recipient = AgentId::new();
        let request = symbi_runtime::communication::policy_gate::CommunicationRequest {
            sender: AgentId::new(),
            recipient,
            message_type: MessageType::Direct(recipient),
            topic: None,
        };
        assert!(policy.evaluate(&request).is_ok());
    }

    #[tokio::test]
    async fn test_reasoning_context_after_init_has_bus() {
        let bridge = RuntimeBridge::new_permissive_for_dev();
        bridge
            .initialize()
            .await
            .expect("initialize should succeed");
        let ctx = bridge.reasoning_context();
        assert!(
            ctx.comm_bus.is_some(),
            "Communication bus should be populated after initialize()"
        );
        assert!(ctx.comm_policy.is_some(), "Policy gate is always present");
    }

    #[tokio::test]
    async fn test_comm_bus_accessor() {
        let bridge = RuntimeBridge::new_permissive_for_dev();
        assert!(bridge.comm_bus().is_none());
        bridge
            .initialize()
            .await
            .expect("initialize should succeed");
        assert!(bridge.comm_bus().is_some());
    }

    #[tokio::test]
    async fn test_set_comm_policy_replaces_default() {
        let bridge = RuntimeBridge::new();
        let new_policy = Arc::new(CommunicationPolicyGate::permissive());
        bridge.set_comm_policy(Arc::clone(&new_policy));
        let ctx = bridge.reasoning_context();
        let retrieved = ctx.comm_policy.expect("policy should be set");
        assert!(Arc::ptr_eq(&retrieved, &new_policy));
    }
}
