use symbi_runtime::context::manager::{StandardContextManager, ContextManagerConfig};
use symbi_runtime::integrations::policy_engine::engine::{OpaPolicyEngine, PolicyEngine, PolicyDecision};
use symbi_runtime::lifecycle::{DefaultLifecycleController, LifecycleConfig, LifecycleController};
use symbi_runtime::types::agent::AgentConfig;
use symbi_runtime::types::security::Capability;
use symbi_runtime::types::AgentId;
use std::sync::{Arc, Mutex};

/// The RuntimeBridge manages a simulated, in-process Symbiont runtime environment.
pub struct RuntimeBridge {
    lifecycle_controller: Arc<Mutex<Option<Arc<DefaultLifecycleController>>>>,
    context_manager: Arc<Mutex<Option<Arc<StandardContextManager>>>>,
    policy_engine: Arc<Mutex<OpaPolicyEngine>>,
    // We will store agent instances here for multi-agent simulation later
}

impl Default for RuntimeBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeBridge {
    pub fn new() -> Self {
        // Initialize with None - will be set up asynchronously
        let lifecycle_controller = Arc::new(Mutex::new(None));
        let context_manager = Arc::new(Mutex::new(None));
        let policy_engine = Arc::new(Mutex::new(OpaPolicyEngine::new()));
        
        Self {
            lifecycle_controller,
            context_manager,
            policy_engine,
        }
    }

    /// Initialize the runtime bridge components asynchronously
    pub async fn initialize(&self) -> Result<(), String> {
        // Initialize lifecycle controller
        let lifecycle_config = LifecycleConfig::default();
        let lifecycle_controller = Arc::new(DefaultLifecycleController::new(lifecycle_config)
            .await
            .map_err(|e| format!("Failed to create lifecycle controller: {}", e))?);
        
        // Initialize context manager
        let context_config = ContextManagerConfig::default();
        let context_manager = Arc::new(StandardContextManager::new(context_config, "runtime_bridge")
            .await
            .map_err(|e| format!("Failed to create context manager: {}", e))?);
        
        // Initialize the context manager
        context_manager.initialize()
            .await
            .map_err(|e| format!("Failed to initialize context manager: {}", e))?;
        
        // Store the initialized components
        *self.lifecycle_controller.lock().unwrap() = Some(lifecycle_controller);
        *self.context_manager.lock().unwrap() = Some(context_manager);
        
        Ok(())
    }

    pub async fn initialize_agent(&self, config: AgentConfig) -> Result<AgentId, String> {
        let controller = {
            let controller_guard = self.lifecycle_controller.lock().unwrap();
            controller_guard.clone()
        };
        
        if let Some(controller) = controller {
            controller.initialize_agent(config).await.map_err(|e| e.to_string())
        } else {
            Err("Lifecycle controller not initialized".to_string())
        }
    }

    /// Checks if a given capability is allowed for an agent.
    pub async fn check_capability(&self, agent_id: &str, capability: &Capability) -> Result<PolicyDecision, String> {
        // Clone the engine to avoid holding the lock across the await
        let engine = {
            let engine_guard = self.policy_engine.lock().unwrap();
            engine_guard.clone()
        };
        engine.check_capability(agent_id, capability).await.map_err(|e| e.to_string())
    }
}