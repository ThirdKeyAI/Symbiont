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

#[cfg(feature = "http-api")]
use api::types::{AgentExecutionRecord, AgentStatusResponse, CreateAgentRequest, CreateAgentResponse, DeleteAgentResponse, ExecuteAgentRequest, ExecuteAgentResponse, GetAgentHistoryResponse, UpdateAgentRequest, UpdateAgentResponse, WorkflowExecutionRequest};
#[cfg(feature = "http-api")]
use api::traits::RuntimeApiProvider;
#[cfg(feature = "http-api")]
use async_trait::async_trait;

#[cfg(feature = "http-input")]
pub mod http_input;

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

/// Implementation of RuntimeApiProvider for AgentRuntime
#[cfg(feature = "http-api")]
#[async_trait]
impl RuntimeApiProvider for AgentRuntime {
    async fn execute_workflow(
        &self,
        _request: WorkflowExecutionRequest,
    ) -> Result<serde_json::Value, RuntimeError> {
        // Placeholder implementation
        Ok(serde_json::json!({
            "status": "success",
            "message": "Workflow execution not yet implemented"
        }))
    }

    async fn get_agent_status(
        &self,
        _agent_id: AgentId,
    ) -> Result<AgentStatusResponse, RuntimeError> {
        // Placeholder implementation
        use crate::types::AgentState;
        Ok(AgentStatusResponse {
            agent_id: AgentId::new(),
            state: AgentState::Running,
            last_activity: chrono::Utc::now(),
            resource_usage: api::types::ResourceUsage {
                memory_bytes: 0,
                cpu_percent: 0.0,
                active_tasks: 0,
            },
        })
    }

    async fn get_system_health(&self) -> Result<serde_json::Value, RuntimeError> {
        // Placeholder implementation
        Ok(serde_json::json!({
            "status": "healthy",
            "components": {
                "scheduler": "healthy",
                "lifecycle": "healthy",
                "resource_manager": "healthy",
                "communication": "healthy"
            }
        }))
    }

    async fn list_agents(&self) -> Result<Vec<AgentId>, RuntimeError> {
        // Placeholder implementation - return empty list for now
        Ok(vec![])
    }

    async fn shutdown_agent(&self, _agent_id: AgentId) -> Result<(), RuntimeError> {
        // Placeholder implementation
        Ok(())
    }

    async fn get_metrics(&self) -> Result<serde_json::Value, RuntimeError> {
        // Placeholder implementation
        Ok(serde_json::json!({
            "agents": {
                "total": 0,
                "running": 0,
                "idle": 0,
                "error": 0
            },
            "system": {
                "uptime": 0,
                "memory_usage": 0,
                "cpu_usage": 0.0
            }
        }))
    }

    async fn create_agent(
        &self,
        request: CreateAgentRequest,
    ) -> Result<CreateAgentResponse, RuntimeError> {
        // Placeholder implementation - generate a UUID and return success
        use uuid::Uuid;
        
        // For now, just validate that we have the required fields
        if request.name.is_empty() {
            return Err(RuntimeError::Internal("Agent name cannot be empty".to_string()));
        }
        
        if request.dsl.is_empty() {
            return Err(RuntimeError::Internal("Agent DSL cannot be empty".to_string()));
        }

        Ok(CreateAgentResponse {
            id: Uuid::new_v4().to_string(),
            status: "created".to_string(),
        })
    }

    async fn update_agent(
        &self,
        agent_id: String,
        request: UpdateAgentRequest,
    ) -> Result<UpdateAgentResponse, RuntimeError> {
        // Placeholder implementation - validate input and return success
        
        // Validate that at least one field is provided for update
        if request.name.is_none() && request.dsl.is_none() {
            return Err(RuntimeError::Internal("At least one field (name or dsl) must be provided for update".to_string()));
        }
        
        // Validate agent_id is not empty
        if agent_id.is_empty() {
            return Err(RuntimeError::Internal("Agent ID cannot be empty".to_string()));
        }
        
        // Validate optional fields if provided
        if let Some(ref name) = request.name {
            if name.is_empty() {
                return Err(RuntimeError::Internal("Agent name cannot be empty".to_string()));
            }
        }
        
        if let Some(ref dsl) = request.dsl {
            if dsl.is_empty() {
                return Err(RuntimeError::Internal("Agent DSL cannot be empty".to_string()));
            }
        }

        Ok(UpdateAgentResponse {
            id: agent_id,
            status: "updated".to_string(),
        })
    }

    async fn delete_agent(
        &self,
        agent_id: String,
    ) -> Result<DeleteAgentResponse, RuntimeError> {
        // Placeholder implementation - validate input and return success
        
        // Validate agent_id is not empty
        if agent_id.is_empty() {
            return Err(RuntimeError::Internal("Agent ID cannot be empty".to_string()));
        }

        Ok(DeleteAgentResponse {
            id: agent_id,
            status: "deleted".to_string(),
        })
    }

    async fn execute_agent(
        &self,
        agent_id: String,
        _request: ExecuteAgentRequest,
    ) -> Result<ExecuteAgentResponse, RuntimeError> {
        // Placeholder implementation - validate input and return dummy response
        
        // Validate agent_id is not empty
        if agent_id.is_empty() {
            return Err(RuntimeError::Internal("Agent ID cannot be empty".to_string()));
        }

        // Generate a dummy execution ID
        use uuid::Uuid;
        
        Ok(ExecuteAgentResponse {
            execution_id: Uuid::new_v4().to_string(),
            status: "execution_started".to_string(),
        })
    }

    async fn get_agent_history(
        &self,
        agent_id: String,
    ) -> Result<GetAgentHistoryResponse, RuntimeError> {
        // Placeholder implementation - validate input and return dummy response
        
        // Validate agent_id is not empty
        if agent_id.is_empty() {
            return Err(RuntimeError::Internal("Agent ID cannot be empty".to_string()));
        }

        // Return a dummy history with one sample record
        use uuid::Uuid;
        
        let sample_record = AgentExecutionRecord {
            execution_id: Uuid::new_v4().to_string(),
            status: "completed".to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        };

        Ok(GetAgentHistoryResponse {
            history: vec![sample_record],
        })
    }
}
