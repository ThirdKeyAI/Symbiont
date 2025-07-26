//! Symbiont Agent Runtime System
//!
//! The Agent Runtime System is the core orchestration layer of the Symbiont platform,
//! responsible for managing the complete lifecycle of autonomous agents.

pub mod communication;
pub mod context;
pub mod crypto;
pub mod error_handler;
pub mod integrations;
pub mod lifecycle;
pub mod rag;
pub mod resource;
pub mod scheduler;
pub mod secrets;
pub mod types;

#[cfg(feature = "http-api")]
pub mod api;

// Re-export commonly used types
pub use communication::{CommunicationBus, CommunicationConfig, DefaultCommunicationBus};
pub use error_handler::{DefaultErrorHandler, ErrorHandler, ErrorHandlerConfig};
pub use lifecycle::{DefaultLifecycleController, LifecycleConfig, LifecycleController};
pub use resource::{DefaultResourceManager, ResourceManager, ResourceManagerConfig};
pub use scheduler::{AgentScheduler, DefaultAgentScheduler, SchedulerConfig};
pub use types::*;

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
        let scheduler = Arc::new(
            scheduler::DefaultAgentScheduler::new(config.read().await.scheduler.clone()).await?,
        );

        let resource_manager = Arc::new(
            resource::DefaultResourceManager::new(config.read().await.resource_manager.clone())
                .await?,
        );

        let communication = Arc::new(
            communication::DefaultCommunicationBus::new(config.read().await.communication.clone())
                .await?,
        );

        let error_handler = Arc::new(
            error_handler::DefaultErrorHandler::new(config.read().await.error_handler.clone())
                .await?,
        );

        let lifecycle_config = lifecycle::LifecycleConfig {
            max_agents: 1000,
            initialization_timeout: std::time::Duration::from_secs(30),
            termination_timeout: std::time::Duration::from_secs(30),
            state_check_interval: std::time::Duration::from_secs(10),
            enable_auto_recovery: true,
            max_restart_attempts: 3,
        };
        let lifecycle =
            Arc::new(lifecycle::DefaultLifecycleController::new(lifecycle_config).await?);

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
        self.lifecycle
            .shutdown()
            .await
            .map_err(RuntimeError::Lifecycle)?;
        self.communication
            .shutdown()
            .await
            .map_err(RuntimeError::Communication)?;
        self.resource_manager
            .shutdown()
            .await
            .map_err(RuntimeError::Resource)?;
        self.scheduler
            .shutdown()
            .await
            .map_err(RuntimeError::Scheduler)?;
        self.error_handler
            .shutdown()
            .await
            .map_err(RuntimeError::ErrorHandler)?;

        Ok(())
    }

    /// Get system status
    pub async fn get_status(&self) -> SystemStatus {
        self.scheduler.get_system_status().await
    }
}

/// Runtime configuration
#[derive(Debug, Clone, Default)]
pub struct RuntimeConfig {
    pub scheduler: scheduler::SchedulerConfig,
    pub resource_manager: resource::ResourceManagerConfig,
    pub communication: communication::CommunicationConfig,
    pub security: SecurityConfig,
    pub audit: AuditConfig,
    pub error_handler: error_handler::ErrorHandlerConfig,
}
