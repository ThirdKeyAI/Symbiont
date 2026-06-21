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
    /// Active session id shared across all clones of the reasoning context.
    #[cfg(feature = "session")]
    active_session: Arc<Mutex<Option<symbi_session::monitor::SessionId>>>,
    /// Registry that owns the SessionMonitor; minted once per bridge.
    #[cfg(feature = "session")]
    session_registry: Arc<symbi_runtime::session::SessionRegistry>,
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
            #[cfg(feature = "session")]
            active_session: Arc::new(Mutex::new(None)),
            #[cfg(feature = "session")]
            session_registry: Arc::new(symbi_runtime::session::SessionRegistry::new()),
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

    /// Register an agent (name + system prompt + tool names) into the shared
    /// registry so it can be delegated to. Returns the minted agent id.
    pub async fn register_agent(
        &self,
        name: &str,
        system_prompt: &str,
        tools: Vec<String>,
    ) -> AgentId {
        self.agent_registry
            .spawn_agent(name, system_prompt, tools, None)
            .await
    }

    /// Governed single-turn delegation to a registered agent by name. Runs the
    /// communication policy gate (and session conformance when a session is
    /// open) before invoking the agent. Returns the agent's reply text.
    pub async fn delegate(&self, target: &str, message: &str) -> crate::error::Result<String> {
        let ctx = self.reasoning_context();
        crate::dsl::agent_composition::governed_ask(&ctx, target, message, None).await
    }

    /// Governed multi-turn delegation: like `delegate`, but the caller supplies a
    /// full conversation (agent system prompt + history + new user turn). Runs the
    /// comm-policy gate before completing against the provider.
    pub async fn delegate_threaded(
        &self,
        target: &str,
        conversation: &symbi_runtime::reasoning::conversation::Conversation,
    ) -> crate::error::Result<String> {
        let ctx = self.reasoning_context();
        crate::dsl::agent_composition::governed_ask_conversation(&ctx, target, conversation).await
    }

    /// The system prompt of a registered agent, if present (used to seed a
    /// per-agent conversation thread).
    pub async fn agent_system_prompt(&self, name: &str) -> Option<String> {
        self.agent_registry
            .get_agent(name)
            .await
            .map(|a| a.system_prompt)
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
            #[cfg(feature = "session")]
            active_session: self.active_session.clone(),
            #[cfg(feature = "session")]
            session_monitor: Some(self.session_registry.monitor()),
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

    /// Open a session: establish the protocol on the registry, attach the monitor to
    /// the shared communication gate, and set the active session so the DSL builtins
    /// tag + enforce subsequent inter-agent messages.
    ///
    /// NOTE (v1a): the gate is rebuilt as permissive + monitor; merging pre-existing
    /// per-message rules with the session monitor is a documented v1b refinement.
    /// v1a also assumes a single active session per bridge — calling this again
    /// overwrites the active-session cell (the prior session's FSMs remain in the
    /// registry but are no longer the active one). Multi-session support is v1b.
    #[cfg(feature = "session")]
    pub fn open_session(
        &self,
        global: &symbi_session::Global,
        binding: symbi_runtime::session::RoleBinding,
        ttl: std::time::Duration,
    ) -> Result<symbi_session::monitor::SessionId, symbi_runtime::session::RegistryError> {
        let sid = self.session_registry.open(global, binding, ttl)?;
        let gate = symbi_runtime::communication::policy_gate::CommunicationPolicyGate::permissive()
            .with_session_monitor(self.session_registry.monitor())
            .with_transcript(self.session_registry.transcript());
        self.set_comm_policy(std::sync::Arc::new(gate));
        *self.active_session.lock().unwrap() = Some(sid.clone());
        Ok(sid)
    }

    /// The protocol transcript for this bridge's session(s) — offline-verifiable.
    #[cfg(feature = "session")]
    pub fn session_transcript(
        &self,
    ) -> std::sync::Arc<std::sync::Mutex<symbi_runtime::session::SessionTranscript>> {
        self.session_registry.transcript()
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

    #[cfg(feature = "session")]
    #[test]
    fn reasoning_context_has_no_session_by_default() {
        let bridge = RuntimeBridge::new_permissive_for_dev();
        let ctx = bridge.reasoning_context();
        assert!(ctx.active_session.lock().unwrap().is_none());
        assert!(ctx.session_monitor.is_some()); // monitor always available; no session open yet
    }

    #[tokio::test]
    async fn delegate_to_unknown_agent_errors_with_name() {
        let bridge = RuntimeBridge::new_permissive_for_dev();
        let err = bridge.delegate("nope", "hi").await.unwrap_err();
        assert!(
            format!("{err}").contains("nope"),
            "error should name the missing agent"
        );
    }

    #[tokio::test]
    async fn delegate_threaded_unknown_agent_errors() {
        let bridge = RuntimeBridge::new_permissive_for_dev();
        let conv = symbi_runtime::reasoning::conversation::Conversation::with_system("x");
        let err = bridge.delegate_threaded("nope", &conv).await.unwrap_err();
        assert!(format!("{err}").contains("nope"));
    }

    #[tokio::test]
    async fn agent_system_prompt_roundtrips() {
        let bridge = RuntimeBridge::new_permissive_for_dev();
        bridge.register_agent("w", "You are w.", vec![]).await;
        assert_eq!(
            bridge.agent_system_prompt("w").await.as_deref(),
            Some("You are w.")
        );
        assert!(bridge.agent_system_prompt("missing").await.is_none());
    }

    #[tokio::test]
    async fn register_then_registry_has_agent() {
        let bridge = RuntimeBridge::new_permissive_for_dev();
        bridge
            .register_agent("helper", "You are helper.", vec![])
            .await;
        let ctx = bridge.reasoning_context();
        let reg = ctx.agent_registry.as_ref().unwrap();
        assert!(reg.has_agent("helper").await);
    }

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
            session_id: None,
            protocol_label: None,
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
            session_id: None,
            protocol_label: None,
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

    #[cfg(feature = "session")]
    #[test]
    fn open_session_records_conforming_messages_to_transcript() {
        use crate::dsl::agent_composition::check_comm_policy;
        use std::time::Duration;
        use symbi_runtime::session::RoleBinding;
        use symbi_runtime::types::communication::MessageType;
        use symbi_runtime::types::AgentId;
        use symbi_session::examples::coordinator_pipeline;

        let bridge = RuntimeBridge::new_permissive_for_dev();
        let (g, _r) = coordinator_pipeline();
        let (c, v, p) = (AgentId::new(), AgentId::new(), AgentId::new());
        let rb = RoleBinding::new()
            .bind(c, "Coordinator")
            .bind(v, "Validator")
            .bind(p, "Processor");
        let _sid = bridge
            .open_session(&g, rb, Duration::from_secs(60))
            .unwrap();
        let ctx = bridge.reasoning_context();

        check_comm_policy(&ctx, c, v, MessageType::Direct(v), None).unwrap();
        check_comm_policy(&ctx, v, c, MessageType::Direct(c), None).unwrap();

        let t = bridge.session_transcript();
        let guard = t.lock().unwrap();
        assert!(
            guard.len() >= 2,
            "transcript should have the conforming transitions"
        );
        assert!(guard.verify());
    }

    #[cfg(feature = "session")]
    #[test]
    fn open_session_attaches_monitor_and_sets_active_session() {
        use std::time::Duration;
        use symbi_runtime::session::RoleBinding;
        use symbi_runtime::types::AgentId;
        use symbi_session::examples::coordinator_pipeline;

        let bridge = RuntimeBridge::new_permissive_for_dev();
        let (g, _r) = coordinator_pipeline();
        let (c, v, p) = (AgentId::new(), AgentId::new(), AgentId::new());
        let rb = RoleBinding::new()
            .bind(c, "Coordinator")
            .bind(v, "Validator")
            .bind(p, "Processor");
        let sid = bridge
            .open_session(&g, rb, Duration::from_secs(60))
            .unwrap();
        let ctx = bridge.reasoning_context();
        assert_eq!(ctx.active_session.lock().unwrap().as_ref(), Some(&sid));
        assert!(ctx.session_monitor.is_some());
    }
}
