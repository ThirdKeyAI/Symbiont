//! Symbiont Agent Runtime System
//!
//! The Agent Runtime System is the core orchestration layer of the Symbiont platform,
//! responsible for managing the complete lifecycle of autonomous agents.

pub mod communication;
pub mod config;
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
pub use config::{ConfigManager, ConfigSource, SecurityConfig, AuditConfig};
pub use context::{ContextManager, ContextManagerConfig, StandardContextManager};
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
    pub context_manager: Arc<dyn context::ContextManager + Send + Sync>,
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

        let context_manager = Arc::new(
            context::StandardContextManager::new(
                config.read().await.context_manager.clone(),
                "runtime-system"
            ).await.map_err(|e| RuntimeError::Internal(format!("Failed to create context manager: {}", e)))?
        );

        // Initialize context manager
        context_manager.initialize().await.map_err(|e| RuntimeError::Internal(format!("Failed to initialize context manager: {}", e)))?;

        Ok(Self {
            scheduler,
            lifecycle,
            resource_manager,
            communication,
            error_handler,
            context_manager,
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
        tracing::info!("Starting Agent Runtime shutdown sequence");
        
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
        
        // Shutdown context manager last to ensure all contexts are saved
        self.context_manager
            .shutdown()
            .await
            .map_err(|e| RuntimeError::Internal(format!("Context manager shutdown failed: {}", e)))?;

        tracing::info!("Agent Runtime shutdown completed successfully");
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
    pub context_manager: context::ContextManagerConfig,
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
        request: WorkflowExecutionRequest,
    ) -> Result<serde_json::Value, RuntimeError> {
        tracing::info!("Executing workflow: {}", request.workflow_id);

        // Step 1: Parse the workflow DSL and extract metadata (before any await)
        let workflow_dsl = &request.workflow_id; // For now, treat workflow_id as DSL source
        let (metadata, agent_config) = {
            let parsed_tree = dsl::parse_dsl(workflow_dsl)
                .map_err(|e| RuntimeError::Internal(format!("DSL parsing failed: {}", e)))?;

            // Extract metadata from the parsed workflow
            let metadata = dsl::extract_metadata(&parsed_tree, workflow_dsl);
            
            // Check for parsing errors
            let root_node = parsed_tree.root_node();
            if root_node.has_error() {
                return Err(RuntimeError::Internal("DSL contains syntax errors".to_string()));
            }

            // Create agent configuration from the workflow
            let agent_id = request.agent_id.unwrap_or_default();
            let agent_config = AgentConfig {
                id: agent_id,
                name: metadata.get("name").cloned().unwrap_or_else(|| "workflow_agent".to_string()),
                dsl_source: workflow_dsl.to_string(),
                execution_mode: ExecutionMode::Ephemeral,
                security_tier: SecurityTier::Tier1,
                resource_limits: ResourceLimits::default(),
                capabilities: vec![Capability::Computation], // Basic capability for workflow execution
                policies: vec![],
                metadata: metadata.clone(),
                priority: Priority::Normal,
            };

            (metadata, agent_config)
        };

        // Step 2: Schedule the agent for execution
        let scheduled_agent_id = self.scheduler
            .schedule_agent(agent_config)
            .await
            .map_err(RuntimeError::Scheduler)?;

        // Step 3: Wait briefly and check initial status (simple implementation)
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        
        // Step 4: Collect basic execution information
        let system_status = self.scheduler.get_system_status().await;
        
        // Step 5: Prepare and return result
        let mut result = serde_json::json!({
            "status": "success",
            "workflow_id": request.workflow_id,
            "agent_id": scheduled_agent_id.to_string(),
            "execution_started": true,
            "metadata": metadata,
            "system_status": {
                "total_agents": system_status.total_agents,
                "running_agents": system_status.running_agents,
                "resource_utilization": {
                    "memory_used": system_status.resource_utilization.memory_used,
                    "cpu_utilization": system_status.resource_utilization.cpu_utilization,
                    "disk_io_rate": system_status.resource_utilization.disk_io_rate,
                    "network_io_rate": system_status.resource_utilization.network_io_rate
                }
            }
        });

        // Add parameters if provided
        if !request.parameters.is_null() {
            result["parameters"] = request.parameters;
        }

        tracing::info!("Workflow execution initiated for agent: {}", scheduled_agent_id);
        Ok(result)
    }

    async fn get_agent_status(
        &self,
        agent_id: AgentId,
    ) -> Result<AgentStatusResponse, RuntimeError> {
        // Call the scheduler to get agent status
        match self.scheduler.get_agent_status(agent_id).await {
            Ok(agent_status) => {
                // Convert SystemTime to DateTime<Utc>
                let last_activity = chrono::DateTime::<chrono::Utc>::from(agent_status.last_activity);
                
                Ok(AgentStatusResponse {
                    agent_id: agent_status.agent_id,
                    state: agent_status.state,
                    last_activity,
                    resource_usage: api::types::ResourceUsage {
                        memory_bytes: agent_status.memory_usage,
                        cpu_percent: agent_status.cpu_usage,
                        active_tasks: agent_status.active_tasks,
                    },
                })
            },
            Err(scheduler_error) => {
                tracing::warn!("Failed to get agent status for {}: {}", agent_id, scheduler_error);
                Err(RuntimeError::Internal(format!("Agent {} not found", agent_id)))
            }
        }
    }

    async fn get_system_health(&self) -> Result<serde_json::Value, RuntimeError> {
        // Check health of all components
        let scheduler_health = self.scheduler.check_health().await
            .map_err(|e| RuntimeError::Internal(format!("Scheduler health check failed: {}", e)))?;
        
        let lifecycle_health = self.lifecycle.check_health().await
            .map_err(|e| RuntimeError::Internal(format!("Lifecycle health check failed: {}", e)))?;
        
        let resource_health = self.resource_manager.check_health().await
            .map_err(|e| RuntimeError::Internal(format!("Resource manager health check failed: {}", e)))?;
        
        let communication_health = self.communication.check_health().await
            .map_err(|e| RuntimeError::Internal(format!("Communication health check failed: {}", e)))?;

        // Determine overall system status
        let component_healths = vec![
            ("scheduler", &scheduler_health),
            ("lifecycle", &lifecycle_health),
            ("resource_manager", &resource_health),
            ("communication", &communication_health),
        ];

        let overall_status = if component_healths.iter().all(|(_, h)| h.status == HealthStatus::Healthy) {
            "healthy"
        } else if component_healths.iter().any(|(_, h)| h.status == HealthStatus::Unhealthy) {
            "unhealthy"
        } else {
            "degraded"
        };

        // Build response with detailed component information
        let mut components = serde_json::Map::new();
        for (name, health) in component_healths {
            let component_info = serde_json::json!({
                "status": match health.status {
                    HealthStatus::Healthy => "healthy",
                    HealthStatus::Degraded => "degraded",
                    HealthStatus::Unhealthy => "unhealthy",
                },
                "message": health.message,
                "last_check": health.last_check
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                "uptime_seconds": health.uptime.as_secs(),
                "metrics": health.metrics
            });
            components.insert(name.to_string(), component_info);
        }

        Ok(serde_json::json!({
            "status": overall_status,
            "timestamp": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            "components": components
        }))
    }

    async fn list_agents(&self) -> Result<Vec<AgentId>, RuntimeError> {
        Ok(self.scheduler.list_agents().await)
    }

    async fn shutdown_agent(&self, agent_id: AgentId) -> Result<(), RuntimeError> {
        self.scheduler
            .shutdown_agent(agent_id)
            .await
            .map_err(RuntimeError::Scheduler)
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
        // Validate input
        if request.name.is_empty() {
            return Err(RuntimeError::Internal("Agent name cannot be empty".to_string()));
        }
        
        if request.dsl.is_empty() {
            return Err(RuntimeError::Internal("Agent DSL cannot be empty".to_string()));
        }

        // Create agent configuration
        let agent_id = AgentId::new();
        let agent_config = AgentConfig {
            id: agent_id,
            name: request.name,
            dsl_source: request.dsl,
            execution_mode: ExecutionMode::Ephemeral,
            security_tier: SecurityTier::Tier1,
            resource_limits: ResourceLimits::default(),
            capabilities: vec![Capability::Computation],
            policies: vec![],
            metadata: std::collections::HashMap::new(),
            priority: Priority::Normal,
        };

        // Schedule the agent for execution
        let scheduled_agent_id = self.scheduler
            .schedule_agent(agent_config)
            .await
            .map_err(RuntimeError::Scheduler)?;

        tracing::info!("Created and scheduled agent: {}", scheduled_agent_id);

        Ok(CreateAgentResponse {
            id: scheduled_agent_id.to_string(),
            status: "scheduled".to_string(),
        })
    }

    async fn update_agent(
        &self,
        agent_id: AgentId,
        request: UpdateAgentRequest,
    ) -> Result<UpdateAgentResponse, RuntimeError> {
        // Validate that at least one field is provided for update
        if request.name.is_none() && request.dsl.is_none() {
            return Err(RuntimeError::Internal("At least one field (name or dsl) must be provided for update".to_string()));
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

        // Call the scheduler to update the agent
        self.scheduler
            .update_agent(agent_id, request)
            .await
            .map_err(RuntimeError::Scheduler)?;

        tracing::info!("Successfully updated agent: {}", agent_id);

        Ok(UpdateAgentResponse {
            id: agent_id.to_string(),
            status: "updated".to_string(),
        })
    }

    async fn delete_agent(
        &self,
        agent_id: AgentId,
    ) -> Result<DeleteAgentResponse, RuntimeError> {
        // Placeholder implementation - validate input and return success
        
        Ok(DeleteAgentResponse {
            id: agent_id.to_string(),
            status: "deleted".to_string(),
        })
    }

    async fn execute_agent(
        &self,
        _agent_id: AgentId,
        _request: ExecuteAgentRequest,
    ) -> Result<ExecuteAgentResponse, RuntimeError> {
        // Placeholder implementation - validate input and return dummy response
        

        // Generate a dummy execution ID
        use uuid::Uuid;
        
        Ok(ExecuteAgentResponse {
            execution_id: Uuid::new_v4().to_string(),
            status: "execution_started".to_string(),
        })
    }

    async fn get_agent_history(
        &self,
        _agent_id: AgentId,
    ) -> Result<GetAgentHistoryResponse, RuntimeError> {
        // Placeholder implementation - validate input and return dummy response
        

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
