//! Symbiont Agent Runtime System
//! 
//! The Agent Runtime System is the core orchestration layer of the Symbiont platform,
//! responsible for managing the complete lifecycle of autonomous agents.

pub mod types;
pub mod scheduler;
pub mod lifecycle;
pub mod resource;
pub mod communication;
pub mod error_handler;
pub mod integrations;

// Re-export commonly used types
pub use types::*;
pub use scheduler::{AgentScheduler, DefaultAgentScheduler, SchedulerConfig};
pub use lifecycle::{LifecycleController, DefaultLifecycleController, LifecycleConfig};
pub use resource::{ResourceManager, DefaultResourceManager, ResourceManagerConfig};
pub use communication::{CommunicationBus, DefaultCommunicationBus, CommunicationConfig};
pub use error_handler::{ErrorHandler, DefaultErrorHandler, ErrorHandlerConfig};

use std::sync::Arc;
use tokio::sync::RwLock;

/// Main Agent Runtime System
#[derive(Clone)]
pub struct AgentRuntime {
    pub scheduler: Arc<dyn scheduler::AgentScheduler + Send + Sync>,
    pub lifecycle: Arc<dyn lifecycle::LifecycleController + Send + Sync>,
    pub resource_manager: Arc<dyn resource::ResourceManager + Send + Sync>,
    pub communication: Arc<dyn communication::CommunicationBus + Send + Sync>,
    pub error_handler: Arc<dyn error_handler::ErrorHandler + Send + Sync>,
    config: Arc<RwLock<RuntimeConfig>>,
}

impl AgentRuntime {
    /// Create a new Agent Runtime System instance
    pub async fn new(config: RuntimeConfig) -> Result<Self, RuntimeError> {
        let config = Arc::new(RwLock::new(config));
        
        // Initialize components
        let scheduler = Arc::new(scheduler::DefaultAgentScheduler::new(
            config.read().await.scheduler.clone()
        ).await?);
        
        let resource_manager = Arc::new(resource::DefaultResourceManager::new(
            config.read().await.resource_manager.clone()
        ).await?);
        
        let communication = Arc::new(communication::DefaultCommunicationBus::new(
            config.read().await.communication.clone()
        ).await?);
        
        let error_handler = Arc::new(error_handler::DefaultErrorHandler::new(
            config.read().await.error_handler.clone()
        ).await?);
        
        let lifecycle_config = lifecycle::LifecycleConfig {
            max_agents: 1000,
            initialization_timeout: std::time::Duration::from_secs(30),
            termination_timeout: std::time::Duration::from_secs(30),
            state_check_interval: std::time::Duration::from_secs(10),
            enable_auto_recovery: true,
            max_restart_attempts: 3,
        };
        let lifecycle = Arc::new(lifecycle::DefaultLifecycleController::new(lifecycle_config).await?);

        Ok(Self {
            scheduler,
            lifecycle,
            resource_manager,
            communication,
            error_handler,
            config,
        })
    }

    /// Get the current runtime configuration
    pub async fn get_config(&self) -> RuntimeConfig {
        self.config.read().await.clone()
    }

    /// Update the runtime configuration
    pub async fn update_config(&self, config: RuntimeConfig) -> Result<(), RuntimeError> {
        *self.config.write().await = config;
        Ok(())
    }

    /// Shutdown the runtime system gracefully
    pub async fn shutdown(&self) -> Result<(), RuntimeError> {
        // Shutdown components in reverse order of initialization
        self.lifecycle.shutdown().await.map_err(|e| RuntimeError::Lifecycle(e))?;
        self.communication.shutdown().await.map_err(|e| RuntimeError::Communication(e))?;
        self.resource_manager.shutdown().await.map_err(|e| RuntimeError::Resource(e))?;
        self.scheduler.shutdown().await.map_err(|e| RuntimeError::Scheduler(e))?;
        self.error_handler.shutdown().await.map_err(|e| RuntimeError::ErrorHandler(e))?;
        
        Ok(())
    }

    /// Get system status
    pub async fn get_status(&self) -> SystemStatus {
        self.scheduler.get_system_status().await
    }
}

/// Runtime configuration
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub scheduler: scheduler::SchedulerConfig,
    pub resource_manager: resource::ResourceManagerConfig,
    pub communication: communication::CommunicationConfig,
    pub security: SecurityConfig,
    pub audit: AuditConfig,
    pub error_handler: error_handler::ErrorHandlerConfig,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            scheduler: scheduler::SchedulerConfig::default(),
            resource_manager: resource::ResourceManagerConfig::default(),
            communication: communication::CommunicationConfig::default(),
            security: SecurityConfig::default(),
            audit: AuditConfig::default(),
            error_handler: error_handler::ErrorHandlerConfig::default(),
        }
    }
}