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
pub mod logging;
pub mod metrics;
pub mod models;
pub mod rag;
pub mod reasoning;
pub mod resource;
pub mod routing;
pub mod sandbox;
pub mod scheduler;
pub mod secrets;
pub mod skills;
pub mod types;

#[cfg(feature = "cli-executor")]
pub mod cli_executor;

#[cfg(feature = "http-api")]
pub mod api;

#[cfg(feature = "http-api")]
use api::traits::RuntimeApiProvider;
#[cfg(feature = "http-api")]
use api::types::{
    AddIdentityMappingRequest, AgentStatusResponse, ChannelActionResponse, ChannelAuditResponse,
    ChannelDetail, ChannelHealthResponse, ChannelSummary, CreateAgentRequest, CreateAgentResponse,
    CreateScheduleRequest, CreateScheduleResponse, DeleteAgentResponse, DeleteChannelResponse,
    DeleteScheduleResponse, ExecuteAgentRequest, ExecuteAgentResponse, GetAgentHistoryResponse,
    IdentityMappingEntry, NextRunsResponse, RegisterChannelRequest, RegisterChannelResponse,
    ScheduleActionResponse, ScheduleDetail, ScheduleHistoryResponse, ScheduleRunEntry,
    ScheduleSummary, UpdateAgentRequest, UpdateAgentResponse, UpdateChannelRequest,
    UpdateScheduleRequest, WorkflowExecutionRequest,
};
#[cfg(feature = "http-api")]
use async_trait::async_trait;

#[cfg(feature = "http-input")]
pub mod http_input;

// Re-export commonly used types
pub use communication::{CommunicationBus, CommunicationConfig, DefaultCommunicationBus};
pub use config::SecurityConfig;
pub use context::{ContextManager, ContextManagerConfig, StandardContextManager};
pub use error_handler::{DefaultErrorHandler, ErrorHandler, ErrorHandlerConfig};
pub use lifecycle::{DefaultLifecycleController, LifecycleConfig, LifecycleController};
pub use logging::{LoggingConfig, ModelInteractionType, ModelLogger, RequestData, ResponseData};
pub use models::{ModelCatalog, ModelCatalogError, SlmRunner, SlmRunnerError};
pub use resource::{DefaultResourceManager, ResourceManager, ResourceManagerConfig};
pub use routing::{
    DefaultRoutingEngine, RouteDecision, RoutingConfig, RoutingContext, RoutingEngine, TaskType,
};
pub use sandbox::{E2BSandbox, ExecutionResult, SandboxRunner, SandboxTier};
#[cfg(feature = "cron")]
pub use scheduler::{
    cron_scheduler::{
        CronMetrics, CronScheduler, CronSchedulerConfig, CronSchedulerError, CronSchedulerHealth,
    },
    cron_types::{
        AuditLevel, CronJobDefinition, CronJobId, CronJobStatus, DeliveryChannel, DeliveryConfig,
        DeliveryReceipt, JobRunRecord, JobRunStatus,
    },
    delivery::{CustomDeliveryHandler, DefaultDeliveryRouter, DeliveryResult, DeliveryRouter},
    heartbeat::{
        HeartbeatAssessment, HeartbeatConfig, HeartbeatContextMode, HeartbeatSeverity,
        HeartbeatState,
    },
    job_store::{JobStore, JobStoreError, SqliteJobStore},
    policy_gate::{
        PolicyGate, ScheduleContext, SchedulePolicyCondition, SchedulePolicyDecision,
        SchedulePolicyEffect, SchedulePolicyRule,
    },
};
pub use scheduler::{AgentScheduler, DefaultAgentScheduler, SchedulerConfig};
pub use secrets::{SecretStore, SecretsConfig};
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
    pub model_logger: Option<Arc<logging::ModelLogger>>,
    pub model_catalog: Option<Arc<models::ModelCatalog>>,
    #[cfg(feature = "cron")]
    cron_scheduler: Option<Arc<scheduler::cron_scheduler::CronScheduler>>,
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
                "runtime-system",
            )
            .await
            .map_err(|e| {
                RuntimeError::Internal(format!("Failed to create context manager: {}", e))
            })?,
        );

        // Initialize context manager
        context_manager.initialize().await.map_err(|e| {
            RuntimeError::Internal(format!("Failed to initialize context manager: {}", e))
        })?;

        // Initialize model logger if enabled
        let model_logger = if config.read().await.logging.enabled {
            // For now, initialize without secret store to avoid type conversion issues
            match logging::ModelLogger::new(config.read().await.logging.clone(), None) {
                Ok(logger) => {
                    tracing::info!("Model logging initialized successfully");
                    Some(Arc::new(logger))
                }
                Err(e) => {
                    tracing::warn!("Failed to initialize model logger: {}", e);
                    None
                }
            }
        } else {
            tracing::info!("Model logging is disabled");
            None
        };

        // Initialize model catalog if SLM is enabled
        let model_catalog = if let Some(ref slm_config) = config.read().await.slm {
            if slm_config.enabled {
                match models::ModelCatalog::new(slm_config.clone()) {
                    Ok(catalog) => {
                        tracing::info!(
                            "Model catalog initialized with {} models",
                            catalog.list_models().len()
                        );
                        Some(Arc::new(catalog))
                    }
                    Err(e) => {
                        tracing::warn!("Failed to initialize model catalog: {}", e);
                        None
                    }
                }
            } else {
                tracing::info!("SLM support is disabled");
                None
            }
        } else {
            tracing::info!("No SLM configuration provided");
            None
        };

        Ok(Self {
            scheduler,
            lifecycle,
            resource_manager,
            communication,
            error_handler,
            context_manager,
            model_logger,
            model_catalog,
            #[cfg(feature = "cron")]
            cron_scheduler: None,
            config,
        })
    }

    /// Attach a CronScheduler to the runtime so schedule APIs become functional.
    #[cfg(feature = "cron")]
    pub fn with_cron_scheduler(
        mut self,
        cron: Arc<scheduler::cron_scheduler::CronScheduler>,
    ) -> Self {
        self.cron_scheduler = Some(cron);
        self
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
        self.context_manager.shutdown().await.map_err(|e| {
            RuntimeError::Internal(format!("Context manager shutdown failed: {}", e))
        })?;

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
    pub logging: logging::LoggingConfig,
    pub slm: Option<config::Slm>,
    pub routing: Option<routing::RoutingConfig>,
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
                return Err(RuntimeError::Internal(
                    "DSL contains syntax errors".to_string(),
                ));
            }

            // Create agent configuration from the workflow
            let agent_id = request.agent_id.unwrap_or_default();
            let agent_config = AgentConfig {
                id: agent_id,
                name: metadata
                    .get("name")
                    .cloned()
                    .unwrap_or_else(|| "workflow_agent".to_string()),
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
        let scheduled_agent_id = self
            .scheduler
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

        tracing::info!(
            "Workflow execution initiated for agent: {}",
            scheduled_agent_id
        );
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
                let last_activity =
                    chrono::DateTime::<chrono::Utc>::from(agent_status.last_activity);

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
            }
            Err(scheduler_error) => {
                tracing::warn!(
                    "Failed to get agent status for {}: {}",
                    agent_id,
                    scheduler_error
                );
                Err(RuntimeError::Internal(format!(
                    "Agent {} not found",
                    agent_id
                )))
            }
        }
    }

    async fn get_system_health(&self) -> Result<serde_json::Value, RuntimeError> {
        // Check health of all components
        let scheduler_health =
            self.scheduler.check_health().await.map_err(|e| {
                RuntimeError::Internal(format!("Scheduler health check failed: {}", e))
            })?;

        let lifecycle_health =
            self.lifecycle.check_health().await.map_err(|e| {
                RuntimeError::Internal(format!("Lifecycle health check failed: {}", e))
            })?;

        let resource_health = self.resource_manager.check_health().await.map_err(|e| {
            RuntimeError::Internal(format!("Resource manager health check failed: {}", e))
        })?;

        let communication_health = self.communication.check_health().await.map_err(|e| {
            RuntimeError::Internal(format!("Communication health check failed: {}", e))
        })?;

        // Determine overall system status
        let component_healths = vec![
            ("scheduler", &scheduler_health),
            ("lifecycle", &lifecycle_health),
            ("resource_manager", &resource_health),
            ("communication", &communication_health),
        ];

        let overall_status = if component_healths
            .iter()
            .all(|(_, h)| h.status == HealthStatus::Healthy)
        {
            "healthy"
        } else if component_healths
            .iter()
            .any(|(_, h)| h.status == HealthStatus::Unhealthy)
        {
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
        let status = self.get_status().await;
        Ok(serde_json::json!({
            "agents": {
                "total": status.total_agents,
                "running": status.running_agents,
                "idle": status.total_agents - status.running_agents,
                "error": 0
            },
            "system": {
                "uptime": 0,
                "memory_usage": status.resource_utilization.memory_used,
                "cpu_usage": status.resource_utilization.cpu_utilization
            }
        }))
    }

    async fn create_agent(
        &self,
        request: CreateAgentRequest,
    ) -> Result<CreateAgentResponse, RuntimeError> {
        // Validate input
        if request.name.is_empty() {
            return Err(RuntimeError::Internal(
                "Agent name cannot be empty".to_string(),
            ));
        }

        if request.dsl.is_empty() {
            return Err(RuntimeError::Internal(
                "Agent DSL cannot be empty".to_string(),
            ));
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
        let scheduled_agent_id = self
            .scheduler
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
            return Err(RuntimeError::Internal(
                "At least one field (name or dsl) must be provided for update".to_string(),
            ));
        }

        // Validate optional fields if provided
        if let Some(ref name) = request.name {
            if name.is_empty() {
                return Err(RuntimeError::Internal(
                    "Agent name cannot be empty".to_string(),
                ));
            }
        }

        if let Some(ref dsl) = request.dsl {
            if dsl.is_empty() {
                return Err(RuntimeError::Internal(
                    "Agent DSL cannot be empty".to_string(),
                ));
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

    async fn delete_agent(&self, agent_id: AgentId) -> Result<DeleteAgentResponse, RuntimeError> {
        self.scheduler
            .delete_agent(agent_id)
            .await
            .map_err(RuntimeError::Scheduler)?;

        Ok(DeleteAgentResponse {
            id: agent_id.to_string(),
            status: "deleted".to_string(),
        })
    }

    async fn execute_agent(
        &self,
        agent_id: AgentId,
        request: ExecuteAgentRequest,
    ) -> Result<ExecuteAgentResponse, RuntimeError> {
        // Ensure the agent exists in the registry
        if !self.scheduler.has_agent(agent_id) {
            return Err(RuntimeError::Internal(format!(
                "Agent {} not found",
                agent_id
            )));
        }

        // Re-schedule from stored config if the agent isn't currently active
        let status = self.get_agent_status(agent_id).await?;
        if status.state == AgentState::Completed {
            if let Some(config) = self.scheduler.get_agent_config(agent_id) {
                self.scheduler
                    .schedule_agent(config)
                    .await
                    .map_err(RuntimeError::Scheduler)?;
            }
        } else if status.state != AgentState::Running {
            self.lifecycle
                .start_agent(agent_id)
                .await
                .map_err(RuntimeError::Lifecycle)?;
        }
        let execution_id = uuid::Uuid::new_v4().to_string();
        let payload_data: bytes::Bytes = serde_json::to_vec(&request)
            .map_err(|e| RuntimeError::Internal(e.to_string()))?
            .into();
        let message = self.communication.create_internal_message(
            AgentId::new(), // System sender
            agent_id,
            payload_data,
            types::MessageType::Direct(agent_id),
            std::time::Duration::from_secs(300),
        );
        self.communication
            .send_message(message)
            .await
            .map_err(RuntimeError::Communication)?;
        Ok(ExecuteAgentResponse {
            execution_id,
            status: "execution_started".to_string(),
        })
    }

    async fn get_agent_history(
        &self,
        _agent_id: AgentId,
    ) -> Result<GetAgentHistoryResponse, RuntimeError> {
        // TODO(T-59): Requires a persistent execution-log storage layer (SQLite
        // or in-memory ring buffer). The per-schedule history
        // (/schedules/{id}/history) already works via CronScheduler.
        let history = vec![];
        Ok(GetAgentHistoryResponse { history })
    }

    // ── Schedule endpoints ──────────────────────────────────────────

    async fn list_schedules(&self) -> Result<Vec<ScheduleSummary>, RuntimeError> {
        #[cfg(feature = "cron")]
        if let Some(ref cron) = self.cron_scheduler {
            let jobs = cron
                .list_jobs()
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            return Ok(jobs
                .into_iter()
                .map(|j| ScheduleSummary {
                    job_id: j.job_id.to_string(),
                    name: j.name,
                    cron_expression: j.cron_expression,
                    timezone: j.timezone,
                    status: format!("{:?}", j.status),
                    enabled: j.enabled,
                    next_run: j.next_run.map(|t| t.to_rfc3339()),
                    run_count: j.run_count,
                })
                .collect());
        }
        Ok(vec![])
    }

    async fn create_schedule(
        &self,
        request: CreateScheduleRequest,
    ) -> Result<CreateScheduleResponse, RuntimeError> {
        #[cfg(feature = "cron")]
        if let Some(ref cron) = self.cron_scheduler {
            use scheduler::cron_types::{CronJobDefinition, CronJobId};
            let now = chrono::Utc::now();
            let tz = if request.timezone.is_empty() {
                "UTC".to_string()
            } else {
                request.timezone
            };
            let agent_config = types::AgentConfig {
                id: types::AgentId::new(),
                name: request.agent_name,
                dsl_source: String::new(),
                execution_mode: Default::default(),
                security_tier: Default::default(),
                resource_limits: Default::default(),
                capabilities: Vec::new(),
                policies: Vec::new(),
                metadata: Default::default(),
                priority: Default::default(),
            };
            let job = CronJobDefinition {
                job_id: CronJobId::new(),
                name: request.name,
                cron_expression: request.cron_expression,
                timezone: tz,
                agent_config,
                policy_ids: request.policy_ids,
                audit_level: Default::default(),
                status: scheduler::cron_types::CronJobStatus::Active,
                enabled: true,
                one_shot: request.one_shot,
                created_at: now,
                updated_at: now,
                last_run: None,
                next_run: None,
                run_count: 0,
                failure_count: 0,
                max_retries: 3,
                max_concurrent: 1,
                delivery_config: None,
                jitter_max_secs: 0,
                session_mode: Default::default(),
                agentpin_jwt: None,
            };
            let job_id = cron
                .add_job(job)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            // Retrieve the saved job to get the computed next_run.
            let saved = cron
                .get_job(job_id)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            return Ok(CreateScheduleResponse {
                job_id: job_id.to_string(),
                next_run: saved.next_run.map(|t| t.to_rfc3339()),
                status: "created".to_string(),
            });
        }
        Err(RuntimeError::Internal(
            "Schedule API requires a running CronScheduler".to_string(),
        ))
    }

    async fn get_schedule(&self, job_id: &str) -> Result<ScheduleDetail, RuntimeError> {
        #[cfg(feature = "cron")]
        if let Some(ref cron) = self.cron_scheduler {
            let id: scheduler::cron_types::CronJobId = job_id
                .parse()
                .map_err(|_| RuntimeError::Internal(format!("Invalid job ID: {}", job_id)))?;
            let j = cron
                .get_job(id)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            return Ok(ScheduleDetail {
                job_id: j.job_id.to_string(),
                name: j.name,
                cron_expression: j.cron_expression,
                timezone: j.timezone,
                status: format!("{:?}", j.status),
                enabled: j.enabled,
                one_shot: j.one_shot,
                next_run: j.next_run.map(|t| t.to_rfc3339()),
                last_run: j.last_run.map(|t| t.to_rfc3339()),
                run_count: j.run_count,
                failure_count: j.failure_count,
                created_at: j.created_at.to_rfc3339(),
                updated_at: j.updated_at.to_rfc3339(),
            });
        }
        Err(RuntimeError::Internal(
            "Schedule API requires a running CronScheduler".to_string(),
        ))
    }

    async fn update_schedule(
        &self,
        job_id: &str,
        request: UpdateScheduleRequest,
    ) -> Result<ScheduleDetail, RuntimeError> {
        #[cfg(feature = "cron")]
        if let Some(ref cron) = self.cron_scheduler {
            let id: scheduler::cron_types::CronJobId = job_id
                .parse()
                .map_err(|_| RuntimeError::Internal(format!("Invalid job ID: {}", job_id)))?;
            let mut job = cron
                .get_job(id)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            if let Some(expr) = request.cron_expression {
                job.cron_expression = expr;
            }
            if let Some(tz) = request.timezone {
                job.timezone = tz;
            }
            if let Some(pids) = request.policy_ids {
                job.policy_ids = pids;
            }
            if let Some(one_shot) = request.one_shot {
                job.one_shot = one_shot;
            }
            cron.update_job(job)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            return self.get_schedule(job_id).await;
        }
        Err(RuntimeError::Internal(
            "Schedule API requires a running CronScheduler".to_string(),
        ))
    }

    async fn delete_schedule(&self, job_id: &str) -> Result<DeleteScheduleResponse, RuntimeError> {
        #[cfg(feature = "cron")]
        if let Some(ref cron) = self.cron_scheduler {
            let id: scheduler::cron_types::CronJobId = job_id
                .parse()
                .map_err(|_| RuntimeError::Internal(format!("Invalid job ID: {}", job_id)))?;
            cron.remove_job(id)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            return Ok(DeleteScheduleResponse {
                job_id: job_id.to_string(),
                deleted: true,
            });
        }
        Err(RuntimeError::Internal(
            "Schedule API requires a running CronScheduler".to_string(),
        ))
    }

    async fn pause_schedule(&self, job_id: &str) -> Result<ScheduleActionResponse, RuntimeError> {
        #[cfg(feature = "cron")]
        if let Some(ref cron) = self.cron_scheduler {
            let id: scheduler::cron_types::CronJobId = job_id
                .parse()
                .map_err(|_| RuntimeError::Internal(format!("Invalid job ID: {}", job_id)))?;
            cron.pause_job(id)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            return Ok(ScheduleActionResponse {
                job_id: job_id.to_string(),
                action: "pause".to_string(),
                status: "paused".to_string(),
            });
        }
        Err(RuntimeError::Internal(
            "Schedule API requires a running CronScheduler".to_string(),
        ))
    }

    async fn resume_schedule(&self, job_id: &str) -> Result<ScheduleActionResponse, RuntimeError> {
        #[cfg(feature = "cron")]
        if let Some(ref cron) = self.cron_scheduler {
            let id: scheduler::cron_types::CronJobId = job_id
                .parse()
                .map_err(|_| RuntimeError::Internal(format!("Invalid job ID: {}", job_id)))?;
            cron.resume_job(id)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            return Ok(ScheduleActionResponse {
                job_id: job_id.to_string(),
                action: "resume".to_string(),
                status: "active".to_string(),
            });
        }
        Err(RuntimeError::Internal(
            "Schedule API requires a running CronScheduler".to_string(),
        ))
    }

    async fn trigger_schedule(&self, job_id: &str) -> Result<ScheduleActionResponse, RuntimeError> {
        #[cfg(feature = "cron")]
        if let Some(ref cron) = self.cron_scheduler {
            let id: scheduler::cron_types::CronJobId = job_id
                .parse()
                .map_err(|_| RuntimeError::Internal(format!("Invalid job ID: {}", job_id)))?;
            cron.trigger_now(id)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            return Ok(ScheduleActionResponse {
                job_id: job_id.to_string(),
                action: "trigger".to_string(),
                status: "triggered".to_string(),
            });
        }
        Err(RuntimeError::Internal(
            "Schedule API requires a running CronScheduler".to_string(),
        ))
    }

    async fn get_schedule_history(
        &self,
        job_id: &str,
        limit: usize,
    ) -> Result<ScheduleHistoryResponse, RuntimeError> {
        #[cfg(feature = "cron")]
        if let Some(ref cron) = self.cron_scheduler {
            let id: scheduler::cron_types::CronJobId = job_id
                .parse()
                .map_err(|_| RuntimeError::Internal(format!("Invalid job ID: {}", job_id)))?;
            let runs = cron
                .get_run_history(id, limit)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            return Ok(ScheduleHistoryResponse {
                job_id: job_id.to_string(),
                history: runs
                    .into_iter()
                    .map(|r| ScheduleRunEntry {
                        run_id: r.run_id.to_string(),
                        started_at: r.started_at.to_rfc3339(),
                        completed_at: r.completed_at.map(|t| t.to_rfc3339()),
                        status: format!("{:?}", r.status),
                        error: r.error,
                        execution_time_ms: r.execution_time_ms,
                    })
                    .collect(),
            });
        }
        Err(RuntimeError::Internal(
            "Schedule API requires a running CronScheduler".to_string(),
        ))
    }

    async fn get_schedule_next_runs(
        &self,
        job_id: &str,
        count: usize,
    ) -> Result<NextRunsResponse, RuntimeError> {
        #[cfg(feature = "cron")]
        if let Some(ref cron) = self.cron_scheduler {
            let id: scheduler::cron_types::CronJobId = job_id
                .parse()
                .map_err(|_| RuntimeError::Internal(format!("Invalid job ID: {}", job_id)))?;
            let job = cron
                .get_job(id)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            let runs = cron
                .get_next_runs(&job.cron_expression, &job.timezone, count)
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            return Ok(NextRunsResponse {
                job_id: job_id.to_string(),
                next_runs: runs.into_iter().map(|t| t.to_rfc3339()).collect(),
            });
        }
        Err(RuntimeError::Internal(
            "Schedule API requires a running CronScheduler".to_string(),
        ))
    }

    async fn get_scheduler_health(
        &self,
    ) -> Result<api::types::SchedulerHealthResponse, RuntimeError> {
        #[cfg(feature = "cron")]
        if let Some(ref cron) = self.cron_scheduler {
            let h = cron
                .check_health()
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            let m = cron.metrics();
            return Ok(api::types::SchedulerHealthResponse {
                is_running: h.is_running,
                store_accessible: h.store_accessible,
                jobs_total: h.jobs_total,
                jobs_active: h.jobs_active,
                jobs_paused: h.jobs_paused,
                jobs_dead_letter: h.jobs_dead_letter,
                global_active_runs: h.global_active_runs,
                max_concurrent: h.max_concurrent,
                runs_total: m.runs_total,
                runs_succeeded: m.runs_succeeded,
                runs_failed: m.runs_failed,
                average_execution_time_ms: m.average_execution_time_ms,
                longest_run_ms: m.longest_run_ms,
            });
        }
        // No CronScheduler — return a minimal response.
        Ok(api::types::SchedulerHealthResponse {
            is_running: false,
            store_accessible: false,
            jobs_total: 0,
            jobs_active: 0,
            jobs_paused: 0,
            jobs_dead_letter: 0,
            global_active_runs: 0,
            max_concurrent: 0,
            runs_total: 0,
            runs_succeeded: 0,
            runs_failed: 0,
            average_execution_time_ms: 0.0,
            longest_run_ms: 0,
        })
    }

    // ── Channel endpoints ──────────────────────────────────────────

    async fn list_channels(&self) -> Result<Vec<ChannelSummary>, RuntimeError> {
        // No ChannelAdapterManager — return empty list so dashboard renders gracefully
        Ok(vec![])
    }

    async fn register_channel(
        &self,
        _request: RegisterChannelRequest,
    ) -> Result<RegisterChannelResponse, RuntimeError> {
        Err(RuntimeError::Internal(
            "Channel API requires a running ChannelAdapterManager".to_string(),
        ))
    }

    async fn get_channel(&self, _id: &str) -> Result<ChannelDetail, RuntimeError> {
        Err(RuntimeError::Internal(
            "Channel API requires a running ChannelAdapterManager".to_string(),
        ))
    }

    async fn update_channel(
        &self,
        _id: &str,
        _request: UpdateChannelRequest,
    ) -> Result<ChannelDetail, RuntimeError> {
        Err(RuntimeError::Internal(
            "Channel API requires a running ChannelAdapterManager".to_string(),
        ))
    }

    async fn delete_channel(&self, _id: &str) -> Result<DeleteChannelResponse, RuntimeError> {
        Err(RuntimeError::Internal(
            "Channel API requires a running ChannelAdapterManager".to_string(),
        ))
    }

    async fn start_channel(&self, _id: &str) -> Result<ChannelActionResponse, RuntimeError> {
        Err(RuntimeError::Internal(
            "Channel API requires a running ChannelAdapterManager".to_string(),
        ))
    }

    async fn stop_channel(&self, _id: &str) -> Result<ChannelActionResponse, RuntimeError> {
        Err(RuntimeError::Internal(
            "Channel API requires a running ChannelAdapterManager".to_string(),
        ))
    }

    async fn get_channel_health(&self, _id: &str) -> Result<ChannelHealthResponse, RuntimeError> {
        Err(RuntimeError::Internal(
            "Channel API requires a running ChannelAdapterManager".to_string(),
        ))
    }

    async fn list_channel_mappings(
        &self,
        _id: &str,
    ) -> Result<Vec<IdentityMappingEntry>, RuntimeError> {
        Err(RuntimeError::Internal(
            "Channel identity mappings require enterprise edition".to_string(),
        ))
    }

    async fn add_channel_mapping(
        &self,
        _id: &str,
        _request: AddIdentityMappingRequest,
    ) -> Result<IdentityMappingEntry, RuntimeError> {
        Err(RuntimeError::Internal(
            "Channel identity mappings require enterprise edition".to_string(),
        ))
    }

    async fn remove_channel_mapping(&self, _id: &str, _user_id: &str) -> Result<(), RuntimeError> {
        Err(RuntimeError::Internal(
            "Channel identity mappings require enterprise edition".to_string(),
        ))
    }

    async fn get_channel_audit(
        &self,
        _id: &str,
        _limit: usize,
    ) -> Result<ChannelAuditResponse, RuntimeError> {
        Err(RuntimeError::Internal(
            "Channel audit log requires enterprise edition".to_string(),
        ))
    }
}
