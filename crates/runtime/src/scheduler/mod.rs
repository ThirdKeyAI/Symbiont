//! Agent Runtime Scheduler
//!
//! The central orchestrator responsible for managing agent execution across the system.

use async_trait::async_trait;
use dashmap::DashMap;
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::Notify;
use tokio::time::interval;

use crate::metrics::{
    LoadBalancerMetrics, MetricsConfig, MetricsExporter, MetricsSnapshot, SchedulerMetrics,
    SystemResourceMetrics, TaskManagerMetrics,
};
use crate::routing::{RouteDecision, RoutingContext, RoutingEngine, SecurityLevel, TaskType};
use crate::types::*;

pub mod execution;
#[cfg(unix)]
pub mod invocations;
pub mod load_balancer;
pub mod priority_queue;
pub mod task_manager;

#[cfg(feature = "cron")]
pub mod cron_scheduler;
#[cfg(feature = "cron")]
pub mod cron_types;
#[cfg(feature = "cron")]
pub mod delivery;
#[cfg(feature = "cron")]
pub mod heartbeat;
#[cfg(feature = "cron")]
pub mod job_store;
#[cfg(all(unix, feature = "cron"))]
pub mod occurrences;
#[cfg(feature = "cron")]
pub mod policy_gate;

use load_balancer::LoadBalancer;
pub use load_balancer::LoadBalancingStats;
use priority_queue::PriorityQueue;
use task_manager::TaskManager;

/// Agent status information returned by the scheduler
#[derive(Debug, Clone)]
pub struct AgentStatus {
    pub agent_id: AgentId,
    pub state: AgentState,
    pub last_activity: SystemTime,
    pub memory_usage: u64,
    pub cpu_usage: f64,
    pub active_tasks: u32,
    pub scheduled_at: SystemTime,
}

/// Agent scheduler trait
#[async_trait]
pub trait AgentScheduler {
    /// Trusted project owning execution configuration and protected retry claims.
    #[cfg(unix)]
    fn invocation_project(&self) -> Result<&std::path::Path, String> {
        Err("execution project is unavailable".into())
    }

    /// Schedule a new agent for execution
    async fn schedule_agent(&self, config: AgentConfig) -> Result<AgentId, SchedulerError>;

    /// Register configuration without starting inference or tool execution.
    async fn register_agent(&self, config: AgentConfig) -> Result<AgentId, SchedulerError>;

    /// Admit one invocation and return its exact cancellation/result handle.
    async fn schedule_invocation(
        &self,
        config: AgentConfig,
        input: serde_json::Value,
    ) -> Result<task_manager::TaskHandle, SchedulerError>;

    /// Claim a stable caller identity before enqueueing. Implementations must
    /// refuse unsupported persistence rather than generate a replacement ID.
    #[cfg(unix)]
    async fn schedule_identified_invocation(
        &self,
        config: AgentConfig,
        input: serde_json::Value,
        identity: invocations::InvocationIdentity,
    ) -> Result<invocations::Admission, String> {
        let _ = (config, input, identity);
        Err("persistent scheduler admission is unavailable".into())
    }

    /// Additional trusted checks while the fresh claim is held, before enqueue.
    #[cfg(unix)]
    async fn schedule_identified_with_gate(
        &self,
        config: AgentConfig,
        input: serde_json::Value,
        identity: invocations::InvocationIdentity,
        gate: Arc<dyn invocations::InvocationAdmissionGate>,
    ) -> Result<invocations::Admission, String> {
        let _ = (config, input, identity, gate);
        Err("protected scheduler admission gates are unavailable".into())
    }

    /// Read a durable identity without granting a new execution.
    #[cfg(unix)]
    async fn lookup_identified_invocation(
        &self,
        config: &AgentConfig,
        input: &serde_json::Value,
        identity: &invocations::InvocationIdentity,
    ) -> Result<Option<crate::reasoning::invocation::ExistingInvocation>, String> {
        let _ = (config, input, identity);
        Err("persistent scheduler lookup is unavailable".into())
    }

    /// Await actual execution and cleanup. Queue admission alone is not success.
    async fn execute_agent(
        &self,
        config: AgentConfig,
        input: serde_json::Value,
    ) -> Result<task_manager::TaskCompletion, SchedulerError> {
        Ok(self.schedule_invocation(config, input).await?.wait().await)
    }

    /// Reschedule an existing agent with new priority
    async fn reschedule_agent(
        &self,
        agent_id: AgentId,
        priority: Priority,
    ) -> Result<(), SchedulerError>;

    /// Terminate an agent
    async fn terminate_agent(&self, agent_id: AgentId) -> Result<(), SchedulerError>;

    /// Shutdown an agent gracefully
    async fn shutdown_agent(&self, agent_id: AgentId) -> Result<(), SchedulerError>;

    /// Get current system status
    async fn get_system_status(&self) -> SystemStatus;

    /// Get status of a specific agent
    async fn get_agent_status(&self, agent_id: AgentId) -> Result<AgentStatus, SchedulerError>;

    /// Shutdown the scheduler
    async fn shutdown(&self) -> Result<(), SchedulerError>;

    /// Check the health of the scheduler
    async fn check_health(&self) -> Result<ComponentHealth, SchedulerError>;

    /// List all agents known to the scheduler (both running and queued)
    async fn list_agents(&self) -> Vec<AgentId>;

    /// Update an existing agent's configuration
    #[cfg(feature = "http-api")]
    async fn update_agent(
        &self,
        agent_id: AgentId,
        request: crate::api::types::UpdateAgentRequest,
    ) -> Result<(), SchedulerError>;

    /// Check whether an agent is registered (regardless of run state)
    fn has_agent(&self, agent_id: AgentId) -> bool;

    /// Retrieve the stored config for a registered agent
    fn get_agent_config(&self, agent_id: AgentId) -> Option<AgentConfig>;

    /// Remove an agent from the registry entirely
    async fn delete_agent(&self, agent_id: AgentId) -> Result<(), SchedulerError>;

    /// Get a reference to the external agents map (for heartbeat/event handlers).
    #[cfg(feature = "http-api")]
    fn external_agents(&self) -> &Arc<DashMap<AgentId, crate::api::types::ExternalAgentState>>;

    /// Get the external agent state for a specific agent.
    #[cfg(feature = "http-api")]
    fn get_external_agent_state(
        &self,
        agent_id: AgentId,
    ) -> Option<crate::api::types::ExternalAgentState>;

    /// Check all external agents and mark those that have not sent a heartbeat
    /// within 3x their expected interval as Unreachable.
    #[cfg(feature = "http-api")]
    fn check_unreachable_agents(&self);
}

/// Scheduler configuration
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    pub max_concurrent_agents: usize,
    pub priority_levels: u8,
    pub resource_limits: ResourceLimits,
    pub scheduling_algorithm: SchedulingAlgorithm,
    pub load_balancing_strategy: LoadBalancingStrategy,
    pub task_timeout: Duration,
    pub health_check_interval: Duration,
    /// Metrics export configuration. When `Some` and `enabled`, the scheduler
    /// periodically collects and exports telemetry to the configured backends.
    pub metrics: Option<MetricsConfig>,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            max_concurrent_agents: 1000,
            priority_levels: 4,
            resource_limits: ResourceLimits::default(),
            scheduling_algorithm: SchedulingAlgorithm::PriorityBased,
            load_balancing_strategy: LoadBalancingStrategy::RoundRobin,
            task_timeout: Duration::from_secs(3600), // 1 hour
            health_check_interval: Duration::from_secs(30),
            metrics: None,
        }
    }
}

/// Scheduled task information
#[derive(Debug, Clone)]
pub struct ScheduledTask {
    #[cfg(unix)]
    pub(crate) claimed_journal: Option<invocations::ClaimedJournal>,
    pub input: serde_json::Value,
    pub handle: task_manager::TaskHandle,
    pub agent_id: AgentId,
    pub config: AgentConfig,
    pub priority: Priority,
    pub scheduled_at: SystemTime,
    pub deadline: Option<SystemTime>,
    pub retry_count: u32,
    pub resource_requirements: ResourceRequirements,
    pub route_decision: Option<RouteDecision>,
}

impl ScheduledTask {
    pub fn new(config: AgentConfig) -> Self {
        let now = SystemTime::now();
        Self {
            #[cfg(unix)]
            claimed_journal: None,
            input: serde_json::Value::Null,
            handle: task_manager::TaskHandle::new(config.id),
            agent_id: config.id,
            priority: config.priority,
            resource_requirements: config
                .metadata
                .get("resource_requirements")
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_default(),
            config,
            scheduled_at: now,
            deadline: None,
            retry_count: 0,
            route_decision: None,
        }
    }

    /// Build a `RoutingContext` from this scheduled task for routing policy evaluation.
    pub fn to_routing_context(&self) -> RoutingContext {
        let security_level = match self.config.security_tier {
            SecurityTier::None => SecurityLevel::Low,
            // Hosted execution carries no on-host isolation guarantees; from
            // the routing engine's perspective it lives in the same risk
            // bucket as native execution.
            SecurityTier::Hosted => SecurityLevel::Low,
            SecurityTier::Tier1 => SecurityLevel::Medium,
            SecurityTier::Tier2 => SecurityLevel::High,
            SecurityTier::Tier3 => SecurityLevel::Critical,
        };

        let capabilities: Vec<String> = self
            .config
            .capabilities
            .iter()
            .map(|cap| match cap {
                Capability::FileSystem => "FileSystem".to_string(),
                Capability::Network => "Network".to_string(),
                Capability::Database => "Database".to_string(),
                Capability::Computation => "Computation".to_string(),
                Capability::Communication => "Communication".to_string(),
                Capability::Custom(s) => s.clone(),
            })
            .collect();

        let task_type = self
            .config
            .metadata
            .get("task_type")
            .map(|tt| match tt.as_str() {
                "intent" => TaskType::Intent,
                "extract" => TaskType::Extract,
                "template" => TaskType::Template,
                "boilerplate_code" => TaskType::BoilerplateCode,
                "code_generation" => TaskType::CodeGeneration,
                "reasoning" => TaskType::Reasoning,
                "analysis" => TaskType::Analysis,
                "summarization" => TaskType::Summarization,
                "translation" => TaskType::Translation,
                "qa" => TaskType::QA,
                other => TaskType::Custom(other.to_string()),
            })
            .unwrap_or_else(|| TaskType::Custom("general".to_string()));

        let max_execution_time = self
            .deadline
            .and_then(|deadline| deadline.duration_since(SystemTime::now()).ok());

        let mut ctx = RoutingContext::new(self.agent_id, task_type, self.config.dsl_source.clone());
        ctx.agent_security_level = security_level;
        ctx.agent_capabilities = capabilities;
        ctx.max_execution_time = max_execution_time;
        ctx
    }
}

impl PartialEq for ScheduledTask {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}

impl Eq for ScheduledTask {}

impl PartialOrd for ScheduledTask {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScheduledTask {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Higher priority tasks come first (BinaryHeap is a max-heap)
        self.priority
            .cmp(&other.priority)
            .then_with(|| other.scheduled_at.cmp(&self.scheduled_at))
            .then_with(|| self.handle.run_id().cmp(&other.handle.run_id()))
    }
}

/// Information about suspended agents
#[derive(Debug, Clone)]
pub struct AgentSuspensionInfo {
    pub agent_id: AgentId,
    pub suspended_at: SystemTime,
    pub suspension_reason: String,
    pub original_task: ScheduledTask,
    pub can_resume: bool,
}

/// Default implementation of the Agent Scheduler
pub struct DefaultAgentScheduler {
    config: SchedulerConfig,
    priority_queue: Arc<RwLock<PriorityQueue<ScheduledTask>>>,
    load_balancer: Arc<LoadBalancer>,
    task_manager: Arc<TaskManager>,
    running_agents: Arc<DashMap<uuid::Uuid, ScheduledTask>>,
    suspended_agents: Arc<DashMap<AgentId, AgentSuspensionInfo>>,
    latest_runs: Arc<DashMap<AgentId, task_manager::TaskHandle>>,
    allocations: Arc<DashMap<uuid::Uuid, ResourceAllocation>>,
    dispatch_lock: Arc<tokio::sync::Mutex<()>>,
    /// Persistent registry of all agents that have been scheduled. Agents
    /// remain here after being dequeued so that status/execute/list continue
    /// to work even after completion.
    registered_agents: Arc<DashMap<AgentId, AgentConfig>>,
    /// State for externally-managed agents (heartbeat, events).
    #[cfg(feature = "http-api")]
    external_agents: Arc<DashMap<AgentId, crate::api::types::ExternalAgentState>>,
    system_metrics: Arc<RwLock<SystemMetrics>>,
    shutdown_notify: Arc<Notify>,
    is_running: Arc<RwLock<bool>>,
    routing_engine: Option<Arc<dyn RoutingEngine>>,
    metrics_exporter: Option<Arc<dyn MetricsExporter>>,
}

impl DefaultAgentScheduler {
    async fn enqueue_invocation(
        &self,
        config: AgentConfig,
        input: serde_json::Value,
        #[cfg(unix)] claimed_journal: Option<invocations::ClaimedJournal>,
    ) -> Result<task_manager::TaskHandle, SchedulerError> {
        let _dispatch = self.dispatch_lock.lock().await;
        if !*self.is_running.read() {
            return Err(SchedulerError::ShuttingDown);
        }
        if matches!(config.execution_mode, ExecutionMode::External { .. }) {
            return Err(SchedulerError::SchedulingFailed {
                agent_id: config.id,
                reason: "external agents require their own execution transport".into(),
            });
        }
        if input.to_string().len() > 1024 * 1024 || self.priority_queue.read().len() >= 2048 {
            return Err(SchedulerError::SchedulingFailed {
                agent_id: config.id,
                reason: "scheduled input or queue capacity exceeded".into(),
            });
        }
        let mut task = ScheduledTask::new(config.clone());
        task.input = input;
        #[cfg(unix)]
        if let Some(journal) = claimed_journal {
            task.handle = task_manager::TaskHandle::with_run_id(config.id, journal.audit.run_id);
            task.claimed_journal = Some(journal);
        }
        let handle = task.handle.clone();
        self.registered_agents.insert(config.id, config);
        self.latest_runs.insert(task.agent_id, handle.clone());
        self.priority_queue.write().push(task);
        Ok(handle)
    }

    /// Create a new scheduler instance
    pub async fn new(config: SchedulerConfig) -> Result<Self, SchedulerError> {
        Self::new_with_routing(config, None).await
    }

    /// Create a new scheduler instance with optional routing engine
    pub async fn new_with_routing(
        config: SchedulerConfig,
        routing_engine: Option<Arc<dyn RoutingEngine>>,
    ) -> Result<Self, SchedulerError> {
        Self::new_with_executor(
            config,
            routing_engine,
            Arc::new(execution::GovernedAgentExecutor::default()),
        )
        .await
    }

    /// Embed a trusted execution service while retaining queue and lifecycle controls.
    pub async fn new_with_executor(
        config: SchedulerConfig,
        routing_engine: Option<Arc<dyn RoutingEngine>>,
        executor: Arc<dyn execution::ScheduledAgentExecutor>,
    ) -> Result<Self, SchedulerError> {
        let priority_queue = Arc::new(RwLock::new(PriorityQueue::new()));
        let load_balancer = Arc::new(LoadBalancer::new(config.load_balancing_strategy.clone()));
        let task_manager = Arc::new(TaskManager::with_executor(config.task_timeout, executor));
        let running_agents = Arc::new(DashMap::new());
        let suspended_agents = Arc::new(DashMap::new());
        let registered_agents = Arc::new(DashMap::new());
        #[cfg(feature = "http-api")]
        let external_agents = Arc::new(DashMap::new());
        let system_metrics = Arc::new(RwLock::new(SystemMetrics::new()));
        let shutdown_notify = Arc::new(Notify::new());
        let is_running = Arc::new(RwLock::new(true));

        // Create metrics exporter if configured and enabled.
        let metrics_exporter = match config.metrics {
            Some(ref metrics_config) if metrics_config.enabled => {
                match crate::metrics::create_exporter(metrics_config) {
                    Ok(exporter) => {
                        tracing::info!("Metrics exporter initialized");
                        Some(exporter)
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to create metrics exporter, continuing without metrics: {}",
                            e
                        );
                        None
                    }
                }
            }
            _ => None,
        };

        let scheduler = Self {
            config,
            priority_queue,
            load_balancer,
            task_manager,
            running_agents,
            suspended_agents,
            latest_runs: Arc::new(DashMap::new()),
            allocations: Arc::new(DashMap::new()),
            dispatch_lock: Arc::new(tokio::sync::Mutex::new(())),
            registered_agents,
            #[cfg(feature = "http-api")]
            external_agents,
            system_metrics,
            shutdown_notify,
            is_running,
            routing_engine,
            metrics_exporter,
        };

        // Start background tasks
        scheduler.start_scheduler_loop().await;
        scheduler.start_metrics_export_loop().await;

        Ok(scheduler)
    }

    /// Start bounded dispatch. Cancellation and deletion serialize with admission.
    async fn start_scheduler_loop(&self) {
        let priority_queue = self.priority_queue.clone();
        let load_balancer = self.load_balancer.clone();
        let task_manager = self.task_manager.clone();
        let running_agents = self.running_agents.clone();
        let allocations = self.allocations.clone();
        let system_metrics = self.system_metrics.clone();
        let shutdown_notify = self.shutdown_notify.clone();
        let is_running = self.is_running.clone();
        let routing_engine = self.routing_engine.clone();
        let dispatch_lock = self.dispatch_lock.clone();
        let max_concurrent = self.config.max_concurrent_agents;
        let task_timeout = self.config.task_timeout;
        tokio::spawn(async move {
            let mut tick = interval(Duration::from_millis(50));
            loop {
                tokio::select! {
                    _ = shutdown_notify.notified() => break,
                    _ = tick.tick() => {}
                }
                let _dispatch = dispatch_lock.lock().await;
                if !*is_running.read() {
                    break;
                }
                // Expire queued runs even when all execution slots are occupied.
                {
                    let mut queue = priority_queue.write();
                    let pending = queue.to_vec();
                    queue.clear();
                    for task in pending {
                        let status = if task.handle.is_cancelled() {
                            Some(task_manager::TaskStatus::Terminated)
                        } else if task.handle.elapsed()
                            >= task_timeout.min(task.config.resource_limits.execution_timeout)
                        {
                            Some(task_manager::TaskStatus::TimedOut)
                        } else {
                            None
                        };
                        if let Some(status) = status {
                            task.handle.finish(task_manager::TaskCompletion::new(
                                &task,
                                status,
                                Some("invocation cancelled or expired before dispatch".into()),
                            ));
                        } else if task.handle.completion().is_none() {
                            queue.push(task);
                        }
                    }
                }
                if running_agents.len() < max_concurrent {
                    let task = priority_queue.write().pop();
                    if let Some(mut task) = task {
                        if let Some(engine) = &routing_engine {
                            let budget = task_timeout
                                .min(task.config.resource_limits.execution_timeout)
                                .saturating_sub(task.handle.elapsed())
                                .min(Duration::from_secs(10));
                            match tokio::time::timeout(
                                budget,
                                engine.route_request(&task.to_routing_context()),
                            )
                            .await
                            {
                                Ok(Ok(RouteDecision::Deny { reason, .. })) => {
                                    task.handle.finish(task_manager::TaskCompletion::new(
                                        &task,
                                        task_manager::TaskStatus::Failed,
                                        Some(format!("routing denied: {reason}")),
                                    ));
                                    continue;
                                }
                                Ok(Ok(decision)) => task.route_decision = Some(decision),
                                result => {
                                    task.handle.finish(task_manager::TaskCompletion::new(
                                        &task,
                                        task_manager::TaskStatus::Failed,
                                        Some(format!(
                                            "routing did not authorize execution: {result:?}"
                                        )),
                                    ));
                                    continue;
                                }
                            }
                        }
                        match load_balancer
                            .allocate_resources(&task.resource_requirements)
                            .await
                        {
                            Ok(allocation) => {
                                let run_id = task.handle.run_id();
                                allocations.insert(run_id, allocation);
                                running_agents.insert(run_id, task.clone());
                                if let Err(error) = task_manager.start_task(task.clone()).await {
                                    task.handle.finish(task_manager::TaskCompletion::new(
                                        &task,
                                        task_manager::TaskStatus::Failed,
                                        Some(error.to_string()),
                                    ));
                                }
                                let allocations = allocations.clone();
                                let running = running_agents.clone();
                                let balancer = load_balancer.clone();
                                tokio::spawn(async move {
                                    task.handle.wait().await;
                                    if let Some((_, allocation)) = allocations.remove(&run_id) {
                                        balancer.deallocate_resources(allocation).await;
                                    }
                                    running.remove(&run_id);
                                });
                            }
                            Err(_) => priority_queue.write().push(task),
                        }
                    }
                }
                system_metrics
                    .write()
                    .update(running_agents.len(), priority_queue.read().len());
            }
        });
    }

    /// Cancel all queued and active invocations for one principal.
    async fn cancel_agent_runs(&self, agent_id: AgentId) -> Result<(), SchedulerError> {
        let _dispatch = self.dispatch_lock.lock().await;
        self.cancel_agent_runs_locked(agent_id).await
    }

    async fn cancel_agent_runs_locked(&self, agent_id: AgentId) -> Result<(), SchedulerError> {
        if !self.registered_agents.contains_key(&agent_id) {
            return Err(SchedulerError::AgentNotFound { agent_id });
        }
        {
            let mut queue = self.priority_queue.write();
            while let Some(task) = queue.remove(&agent_id) {
                task.handle.cancel();
                task.handle.finish(task_manager::TaskCompletion::new(
                    &task,
                    task_manager::TaskStatus::Terminated,
                    Some("queued invocation cancelled".into()),
                ));
            }
        }
        self.task_manager.terminate_task(agent_id).await?;
        let runs: Vec<_> = self
            .running_agents
            .iter()
            .filter(|t| t.agent_id == agent_id)
            .map(|t| *t.key())
            .collect();
        for id in runs {
            if let Some((_, allocation)) = self.allocations.remove(&id) {
                self.load_balancer.deallocate_resources(allocation).await;
            }
            self.running_agents.remove(&id);
        }
        Ok(())
    }
}

#[async_trait]
impl AgentScheduler for DefaultAgentScheduler {
    #[cfg(unix)]
    fn invocation_project(&self) -> Result<&std::path::Path, String> {
        self.task_manager.invocation_project()
    }

    async fn register_agent(&self, config: AgentConfig) -> Result<AgentId, SchedulerError> {
        let _dispatch = self.dispatch_lock.lock().await;
        if !*self.is_running.read() {
            return Err(SchedulerError::ShuttingDown);
        }
        let agent_id = config.id;
        #[cfg(feature = "http-api")]
        if matches!(config.execution_mode, ExecutionMode::External { .. }) {
            self.external_agents
                .entry(agent_id)
                .or_insert_with(crate::api::types::ExternalAgentState::new);
        }
        self.registered_agents.insert(agent_id, config);
        Ok(agent_id)
    }

    async fn schedule_agent(&self, config: AgentConfig) -> Result<AgentId, SchedulerError> {
        if matches!(config.execution_mode, ExecutionMode::External { .. }) {
            return self.register_agent(config).await;
        }
        let agent_id = config.id;
        self.schedule_invocation(config, serde_json::Value::Null)
            .await?;
        Ok(agent_id)
    }

    async fn schedule_invocation(
        &self,
        config: AgentConfig,
        input: serde_json::Value,
    ) -> Result<task_manager::TaskHandle, SchedulerError> {
        self.enqueue_invocation(
            config,
            input,
            #[cfg(unix)]
            None,
        )
        .await
    }

    #[cfg(unix)]
    async fn schedule_identified_invocation(
        &self,
        config: AgentConfig,
        input: serde_json::Value,
        identity: invocations::InvocationIdentity,
    ) -> Result<invocations::Admission, String> {
        self.admit_identified(config, input, identity, None).await
    }

    #[cfg(unix)]
    async fn schedule_identified_with_gate(
        &self,
        config: AgentConfig,
        input: serde_json::Value,
        identity: invocations::InvocationIdentity,
        gate: Arc<dyn invocations::InvocationAdmissionGate>,
    ) -> Result<invocations::Admission, String> {
        self.admit_identified(config, input, identity, Some(gate))
            .await
    }

    #[cfg(unix)]
    async fn lookup_identified_invocation(
        &self,
        config: &AgentConfig,
        input: &serde_json::Value,
        identity: &invocations::InvocationIdentity,
    ) -> Result<Option<crate::reasoning::invocation::ExistingInvocation>, String> {
        let request = serde_json::json!({"version":1,"context":identity.context,"target":config,"input":input});
        crate::reasoning::invocation::lookup_invocation(
            self.task_manager.invocation_project()?,
            "scheduler:v1",
            identity.id,
            &request,
        )
        .await
    }

    async fn reschedule_agent(
        &self,
        agent_id: AgentId,
        priority: Priority,
    ) -> Result<(), SchedulerError> {
        if !*self.is_running.read() {
            return Err(SchedulerError::ShuttingDown);
        }

        let _dispatch = self.dispatch_lock.lock().await;
        let mut queue = self.priority_queue.write();
        let mut tasks = Vec::new();
        while let Some(mut task) = queue.remove(&agent_id) {
            task.priority = priority;
            tasks.push(task);
        }
        for task in tasks {
            queue.push(task);
        }
        if let Some(mut config) = self.registered_agents.get_mut(&agent_id) {
            config.priority = priority;
            Ok(())
        } else {
            Err(SchedulerError::AgentNotFound { agent_id })
        }
    }

    async fn terminate_agent(&self, agent_id: AgentId) -> Result<(), SchedulerError> {
        self.cancel_agent_runs(agent_id).await
    }

    async fn shutdown_agent(&self, agent_id: AgentId) -> Result<(), SchedulerError> {
        self.cancel_agent_runs(agent_id).await
    }

    async fn get_system_status(&self) -> SystemStatus {
        let (total_scheduled, uptime) = {
            let metrics = self.system_metrics.read();
            let now = SystemTime::now();
            (metrics.total_scheduled, metrics.uptime_since(now))
        };
        let resource_utilization = self.load_balancer.get_resource_utilization().await;

        SystemStatus {
            total_agents: total_scheduled,
            running_agents: self.running_agents.len(),
            suspended_agents: self.suspended_agents.len(),
            resource_utilization,
            uptime,
            last_updated: SystemTime::now(),
        }
    }

    async fn get_agent_status(&self, agent_id: AgentId) -> Result<AgentStatus, SchedulerError> {
        // Check external agents first
        #[cfg(feature = "http-api")]
        if let Some(ext) = self.external_agents.get(&agent_id) {
            let last_activity = ext
                .last_heartbeat
                .map(|dt| {
                    let secs = dt.timestamp() as u64;
                    std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs)
                })
                .unwrap_or(SystemTime::now());
            return Ok(AgentStatus {
                agent_id,
                state: ext.reported_state.clone(),
                last_activity,
                memory_usage: 0,
                cpu_usage: 0.0,
                active_tasks: 0,
                scheduled_at: SystemTime::now(),
            });
        }

        if !self.registered_agents.contains_key(&agent_id) {
            return Err(SchedulerError::AgentNotFound { agent_id });
        }
        let active_tasks = self
            .running_agents
            .iter()
            .filter(|t| t.agent_id == agent_id && t.handle.completion().is_none())
            .count() as u32;
        let handle = self.latest_runs.get(&agent_id).map(|h| h.clone());
        let health = handle.as_ref().map(task_manager::TaskHandle::get_health);
        let state = if active_tasks > 0 {
            AgentState::Running
        } else if self.priority_queue.read().contains(&agent_id) {
            AgentState::Waiting
        } else if let Some(health) = &health {
            match health.status {
                task_manager::TaskStatus::Pending => AgentState::Waiting,
                task_manager::TaskStatus::Running => AgentState::Running,
                task_manager::TaskStatus::Completed => AgentState::Completed,
                task_manager::TaskStatus::Terminated => AgentState::Terminated,
                task_manager::TaskStatus::Failed
                | task_manager::TaskStatus::TimedOut
                | task_manager::TaskStatus::Unresolved => AgentState::Failed,
            }
        } else {
            AgentState::Ready
        };
        Ok(AgentStatus {
            agent_id,
            state,
            active_tasks,
            last_activity: health
                .as_ref()
                .map_or_else(SystemTime::now, |h| h.last_activity),
            scheduled_at: SystemTime::now(),
            memory_usage: 0,
            cpu_usage: 0.0,
        })
    }

    async fn shutdown(&self) -> Result<(), SchedulerError> {
        {
            let _dispatch = self.dispatch_lock.lock().await;
            *self.is_running.write() = false;
            self.shutdown_notify.notify_waiters();
            let mut queue = self.priority_queue.write();
            while let Some(task) = queue.pop() {
                task.handle.cancel();
                task.handle.finish(task_manager::TaskCompletion::new(
                    &task,
                    task_manager::TaskStatus::Terminated,
                    Some("scheduler shut down before execution".into()),
                ));
            }
            for task in self.running_agents.iter() {
                task.handle.cancel();
            }
        }
        let agents: std::collections::HashSet<_> =
            self.running_agents.iter().map(|t| t.agent_id).collect();
        let mut failure = None;
        for agent_id in agents {
            if let Err(error) = self.cancel_agent_runs(agent_id).await {
                failure = Some(error);
            }
        }
        self.flush_metrics().await?;
        if let Some(error) = failure {
            return Err(error);
        }
        Ok(())
    }

    async fn check_health(&self) -> Result<ComponentHealth, SchedulerError> {
        let is_running = *self.is_running.read();
        if !is_running {
            return Ok(ComponentHealth::unhealthy(
                "Scheduler is shut down".to_string(),
            ));
        }

        let (total_scheduled, uptime) = {
            let metrics = self.system_metrics.read();
            let now = SystemTime::now();
            (metrics.total_scheduled, metrics.uptime_since(now))
        };

        let running_count = self.running_agents.len();
        let queue_len = self.priority_queue.read().len();
        let load_factor = running_count as f64 / self.config.max_concurrent_agents.max(1) as f64;

        let status = if load_factor > 0.9 {
            ComponentHealth::degraded(format!(
                "High load: {:.1}% capacity used ({}/{})",
                load_factor * 100.0,
                running_count,
                self.config.max_concurrent_agents
            ))
        } else if queue_len > 1000 {
            ComponentHealth::degraded(format!("Large queue: {} agents waiting", queue_len))
        } else {
            ComponentHealth::healthy(Some(format!(
                "Running normally: {} active agents, {} queued",
                running_count, queue_len
            )))
        };

        Ok(status
            .with_uptime(uptime)
            .with_metric("running_agents".to_string(), running_count.to_string())
            .with_metric("queued_agents".to_string(), queue_len.to_string())
            .with_metric("total_scheduled".to_string(), total_scheduled.to_string())
            .with_metric(
                "max_capacity".to_string(),
                self.config.max_concurrent_agents.to_string(),
            )
            .with_metric("load_factor".to_string(), format!("{:.2}", load_factor)))
    }

    async fn list_agents(&self) -> Vec<AgentId> {
        // Return all registered agents (running, queued, and completed)
        self.registered_agents
            .iter()
            .map(|entry| *entry.key())
            .collect()
    }

    #[cfg(feature = "http-api")]
    async fn update_agent(
        &self,
        agent_id: AgentId,
        request: crate::api::types::UpdateAgentRequest,
    ) -> Result<(), SchedulerError> {
        if !*self.is_running.read() {
            return Err(SchedulerError::ShuttingDown);
        }

        // Existing invocations retain their admitted source/configuration snapshot.
        let _dispatch = self.dispatch_lock.lock().await;
        let mut config = self
            .registered_agents
            .get_mut(&agent_id)
            .ok_or(SchedulerError::AgentNotFound { agent_id })?;
        if let Some(name) = request.name {
            config.name = name;
        }
        if let Some(dsl) = request.dsl {
            config.dsl_source = dsl;
        }
        Ok(())
    }

    fn has_agent(&self, agent_id: AgentId) -> bool {
        self.registered_agents.contains_key(&agent_id)
    }

    fn get_agent_config(&self, agent_id: AgentId) -> Option<AgentConfig> {
        self.registered_agents.get(&agent_id).map(|r| r.clone())
    }

    async fn delete_agent(&self, agent_id: AgentId) -> Result<(), SchedulerError> {
        let _dispatch = self.dispatch_lock.lock().await;
        self.cancel_agent_runs_locked(agent_id).await?;
        #[cfg(feature = "http-api")]
        self.external_agents.remove(&agent_id);
        self.registered_agents.remove(&agent_id);
        self.latest_runs.remove(&agent_id);
        self.suspended_agents.remove(&agent_id);
        Ok(())
    }

    #[cfg(feature = "http-api")]
    fn external_agents(&self) -> &Arc<DashMap<AgentId, crate::api::types::ExternalAgentState>> {
        &self.external_agents
    }

    #[cfg(feature = "http-api")]
    fn get_external_agent_state(
        &self,
        agent_id: AgentId,
    ) -> Option<crate::api::types::ExternalAgentState> {
        self.external_agents.get(&agent_id).map(|r| r.clone())
    }

    #[cfg(feature = "http-api")]
    fn check_unreachable_agents(&self) {
        let now = chrono::Utc::now();

        for mut entry in self.external_agents.iter_mut() {
            let agent_id = *entry.key();
            let ext_state = entry.value_mut();

            // Skip agents already marked unreachable or in terminal states
            if ext_state.reported_state == crate::types::AgentState::Unreachable {
                continue;
            }

            // Get heartbeat interval from config
            let interval_secs = self
                .registered_agents
                .get(&agent_id)
                .and_then(|config| match &config.execution_mode {
                    crate::types::agent::ExecutionMode::External {
                        heartbeat_interval_secs,
                        ..
                    } => Some(*heartbeat_interval_secs),
                    _ => None,
                })
                .unwrap_or(60);

            let threshold = chrono::Duration::seconds((interval_secs * 3) as i64);

            if let Some(last_hb) = ext_state.last_heartbeat {
                if now - last_hb > threshold {
                    tracing::warn!(
                        "External agent {} is unreachable (no heartbeat for {}s)",
                        agent_id,
                        (now - last_hb).num_seconds()
                    );
                    ext_state.reported_state = crate::types::AgentState::Unreachable;
                }
            }
            // If last_heartbeat is None: agent just registered, don't mark unreachable yet.
        }
    }
}

impl DefaultAgentScheduler {
    /// Start the periodic metrics export loop.
    async fn start_metrics_export_loop(&self) {
        let exporter = match self.metrics_exporter.clone() {
            Some(e) => e,
            None => return,
        };

        let priority_queue = self.priority_queue.clone();
        let running_agents = self.running_agents.clone();
        let suspended_agents = self.suspended_agents.clone();
        let system_metrics = self.system_metrics.clone();
        let task_manager = self.task_manager.clone();
        let load_balancer = self.load_balancer.clone();
        let shutdown_notify = self.shutdown_notify.clone();
        let is_running = self.is_running.clone();
        let max_concurrent = self.config.max_concurrent_agents;
        let interval_secs = self
            .config
            .metrics
            .as_ref()
            .map(|m| m.export_interval_seconds)
            .unwrap_or(60);

        tokio::spawn(async move {
            let mut tick = interval(Duration::from_secs(interval_secs));

            loop {
                tokio::select! {
                    _ = tick.tick() => {
                        if !*is_running.read() {
                            break;
                        }

                        let snapshot = Self::build_metrics_snapshot(
                            &priority_queue,
                            &running_agents,
                            &suspended_agents,
                            &system_metrics,
                            &task_manager,
                            &load_balancer,
                            max_concurrent,
                        )
                        .await;

                        if let Err(e) = exporter.export(&snapshot).await {
                            tracing::warn!("Periodic metrics export failed: {}", e);
                        }
                    }
                    _ = shutdown_notify.notified() => {
                        break;
                    }
                }
            }
        });
    }

    /// Build a point-in-time metrics snapshot from all scheduler components.
    async fn build_metrics_snapshot(
        priority_queue: &Arc<RwLock<PriorityQueue<ScheduledTask>>>,
        running_agents: &Arc<DashMap<uuid::Uuid, ScheduledTask>>,
        suspended_agents: &Arc<DashMap<AgentId, AgentSuspensionInfo>>,
        system_metrics: &Arc<RwLock<SystemMetrics>>,
        task_manager: &Arc<TaskManager>,
        load_balancer: &Arc<LoadBalancer>,
        max_concurrent: usize,
    ) -> MetricsSnapshot {
        let (total_scheduled, uptime) = {
            let metrics = system_metrics.read();
            let now = SystemTime::now();
            (metrics.total_scheduled, metrics.uptime_since(now))
        };
        let running_count = running_agents.len();
        let queued_count = priority_queue.read().len();
        let suspended_count = suspended_agents.len();
        let load_factor = running_count as f64 / max_concurrent as f64;

        let task_stats = task_manager.get_task_statistics().await;
        let lb_stats = load_balancer.get_statistics().await;
        let resource_usage = load_balancer.get_resource_utilization().await;

        MetricsSnapshot {
            timestamp: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            scheduler: SchedulerMetrics {
                total_scheduled,
                uptime_seconds: uptime.as_secs(),
                running_agents: running_count,
                queued_agents: queued_count,
                suspended_agents: suspended_count,
                max_capacity: max_concurrent,
                load_factor,
            },
            task_manager: TaskManagerMetrics {
                total_tasks: task_stats.total_tasks,
                healthy_tasks: task_stats.healthy_tasks,
                average_uptime_seconds: task_stats.average_uptime.as_secs_f64(),
                total_memory_usage: task_stats.total_memory_usage,
            },
            load_balancer: LoadBalancerMetrics {
                total_allocations: lb_stats.total_allocations,
                active_allocations: lb_stats.active_allocations,
                memory_utilization: lb_stats.memory_utilization as f64,
                cpu_utilization: lb_stats.cpu_utilization as f64,
                allocation_failures: lb_stats.allocation_failures,
                average_allocation_time_ms: lb_stats.average_allocation_time.as_secs_f64() * 1000.0,
            },
            system: SystemResourceMetrics {
                memory_usage_mb: resource_usage.memory_used as f64 / (1024.0 * 1024.0),
                cpu_usage_percent: resource_usage.cpu_utilization as f64,
            },
            compaction: None,
        }
    }

    /// Collect and export a final metrics snapshot, then shut down the exporter.
    async fn flush_metrics(&self) -> Result<(), SchedulerError> {
        tracing::debug!("Flushing scheduler metrics");

        if let Some(ref exporter) = self.metrics_exporter {
            let snapshot = Self::build_metrics_snapshot(
                &self.priority_queue,
                &self.running_agents,
                &self.suspended_agents,
                &self.system_metrics,
                &self.task_manager,
                &self.load_balancer,
                self.config.max_concurrent_agents,
            )
            .await;

            if let Err(e) = exporter.export(&snapshot).await {
                tracing::warn!("Final metrics export failed: {}", e);
            }

            if let Err(e) = exporter.shutdown().await {
                tracing::warn!("Metrics exporter shutdown failed: {}", e);
            }
        }

        // Log summary regardless of exporter presence.
        let (total_scheduled, uptime) = {
            let metrics = self.system_metrics.read();
            let now = SystemTime::now();
            (metrics.total_scheduled, metrics.uptime_since(now))
        };
        tracing::info!(
            "Scheduler shutdown metrics - total_scheduled={}, uptime={:?}, \
             running={}, queued={}, suspended={}",
            total_scheduled,
            uptime,
            self.running_agents.len(),
            self.priority_queue.read().len(),
            self.suspended_agents.len(),
        );

        Ok(())
    }

    /// Suspension cancels the current invocation; resume starts a distinct run.
    pub async fn suspend_agent(
        &self,
        agent_id: AgentId,
        reason: String,
    ) -> Result<(), SchedulerError> {
        let tasks: Vec<_> = self
            .running_agents
            .iter()
            .filter(|t| t.agent_id == agent_id)
            .map(|t| t.value().clone())
            .collect();
        if tasks.len() != 1 {
            return Err(SchedulerError::SchedulingFailed {
                agent_id,
                reason: "suspension requires exactly one active invocation".into(),
            });
        }
        self.cancel_agent_runs(agent_id).await?;
        self.suspended_agents.insert(
            agent_id,
            AgentSuspensionInfo {
                agent_id,
                suspended_at: SystemTime::now(),
                suspension_reason: reason,
                original_task: tasks.into_iter().next().unwrap(),
                can_resume: true,
            },
        );
        Ok(())
    }

    pub async fn resume_agent(&self, agent_id: AgentId) -> Result<(), SchedulerError> {
        let info = self
            .suspended_agents
            .get(&agent_id)
            .map(|r| r.clone())
            .ok_or(SchedulerError::AgentNotFound { agent_id })?;
        if !info.can_resume {
            return Err(SchedulerError::SchedulingFailed {
                agent_id,
                reason: "agent cannot be resumed".into(),
            });
        }
        self.schedule_invocation(info.original_task.config, info.original_task.input)
            .await?;
        self.suspended_agents.remove(&agent_id);
        Ok(())
    }

    /// Get list of suspended agents
    pub async fn list_suspended_agents(&self) -> Vec<AgentSuspensionInfo> {
        self.suspended_agents
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }
}

impl Drop for DefaultAgentScheduler {
    fn drop(&mut self) {
        *self.is_running.write() = false;
        self.shutdown_notify.notify_waiters();
        let mut queue = self.priority_queue.write();
        while let Some(task) = queue.pop() {
            task.handle.cancel();
            task.handle.finish(task_manager::TaskCompletion::new(
                &task,
                task_manager::TaskStatus::Terminated,
                Some("scheduler owner dropped before execution".into()),
            ));
        }
        for task in self.running_agents.iter() {
            task.handle.cancel();
        }
    }
}

/// System metrics for monitoring
#[derive(Debug, Clone)]
struct SystemMetrics {
    total_scheduled: usize,
    start_time: SystemTime,
}

impl SystemMetrics {
    fn new() -> Self {
        Self {
            total_scheduled: 0,
            start_time: SystemTime::now(),
        }
    }

    fn update(&mut self, running: usize, queued: usize) {
        self.total_scheduled = running + queued;
    }

    fn uptime_since(&self, now: SystemTime) -> Duration {
        now.duration_since(self.start_time).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_test_config() -> AgentConfig {
        AgentConfig {
            id: AgentId::new(),
            name: "test-agent".to_string(),
            dsl_source: "do something useful".to_string(),
            execution_mode: ExecutionMode::Ephemeral,
            security_tier: SecurityTier::Tier2,
            resource_limits: ResourceLimits::default(),
            capabilities: vec![
                Capability::FileSystem,
                Capability::Network,
                Capability::Computation,
            ],
            policies: vec![],
            metadata: HashMap::new(),
            priority: Priority::default(),
        }
    }

    #[test]
    fn test_routing_context_from_scheduled_task() {
        let config = make_test_config();
        let mut task = ScheduledTask::new(config);
        task.deadline = Some(SystemTime::now() + Duration::from_secs(300));

        let ctx = task.to_routing_context();

        assert_eq!(ctx.agent_id, task.agent_id);
        assert_eq!(ctx.agent_security_level, SecurityLevel::High);
        assert_eq!(ctx.prompt, "do something useful");
        assert_eq!(
            ctx.agent_capabilities,
            vec!["FileSystem", "Network", "Computation"]
        );
        assert!(ctx.max_execution_time.is_some());
        assert!(matches!(ctx.task_type, TaskType::Custom(ref s) if s == "general"));
    }

    #[test]
    fn test_routing_context_custom_task_type() {
        let mut config = make_test_config();
        config
            .metadata
            .insert("task_type".to_string(), "analysis".to_string());

        let task = ScheduledTask::new(config);
        let ctx = task.to_routing_context();

        assert!(matches!(ctx.task_type, TaskType::Analysis));
    }

    #[test]
    fn test_routing_context_default_task_type() {
        let config = make_test_config();
        let task = ScheduledTask::new(config);
        let ctx = task.to_routing_context();

        assert!(matches!(ctx.task_type, TaskType::Custom(ref s) if s == "general"));
    }

    #[test]
    fn test_scheduled_task_route_decision_default_none() {
        let config = make_test_config();
        let task = ScheduledTask::new(config);

        assert!(task.route_decision.is_none());
    }

    #[cfg(feature = "http-api")]
    #[tokio::test]
    async fn test_external_agent_not_queued() {
        let scheduler = DefaultAgentScheduler::new(SchedulerConfig::default())
            .await
            .unwrap();

        let mut config = make_test_config();
        config.execution_mode = ExecutionMode::External {
            endpoint: None,
            agentpin_domain: None,
            heartbeat_interval_secs: 60,
        };
        let agent_id = config.id;

        let result = scheduler.schedule_agent(config).await;
        assert!(result.is_ok());

        // Agent should be registered
        assert!(scheduler.has_agent(agent_id));

        // Agent should NOT be in the priority queue
        assert_eq!(scheduler.priority_queue.read().len(), 0);

        // Status should return the external state
        let status = scheduler.get_agent_status(agent_id).await.unwrap();
        assert_eq!(status.agent_id, agent_id);
    }

    #[cfg(feature = "http-api")]
    #[tokio::test]
    async fn test_unreachable_detection() {
        let scheduler = DefaultAgentScheduler::new(SchedulerConfig::default())
            .await
            .unwrap();

        let mut config = make_test_config();
        config.execution_mode = ExecutionMode::External {
            endpoint: None,
            agentpin_domain: None,
            heartbeat_interval_secs: 1, // 1 second interval -> 3s threshold
        };
        let agent_id = config.id;

        scheduler.schedule_agent(config).await.unwrap();

        // Set a heartbeat in the past (> 3 seconds ago)
        {
            let mut entry = scheduler.external_agents.get_mut(&agent_id).unwrap();
            entry.last_heartbeat = Some(chrono::Utc::now() - chrono::Duration::seconds(10));
            entry.reported_state = crate::types::AgentState::Running;
        }

        scheduler.check_unreachable_agents();

        let entry = scheduler.external_agents.get(&agent_id).unwrap();
        assert_eq!(entry.reported_state, crate::types::AgentState::Unreachable);
    }
    struct BlockingFixture {
        started: std::sync::atomic::AtomicUsize,
    }
    #[async_trait]
    impl execution::ScheduledAgentExecutor for BlockingFixture {
        async fn execute(
            &self,
            task: &ScheduledTask,
            _: Duration,
            cancellation: tokio_util::sync::CancellationToken,
        ) -> task_manager::TaskCompletion {
            self.started
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if task.input == serde_json::json!("wait") {
                cancellation.cancelled().await;
                task_manager::TaskCompletion::new(task, task_manager::TaskStatus::Terminated, None)
            } else {
                let mut result = task_manager::TaskCompletion::new(
                    task,
                    task_manager::TaskStatus::Completed,
                    None,
                );
                result.output = Some(task.input.to_string());
                result
            }
        }
    }
    async fn wait_started(executor: &BlockingFixture, expected: usize) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while executor.started.load(std::sync::atomic::Ordering::SeqCst) < expected {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn registration_has_no_effect_and_repeated_runs_release_exact_resources() {
        let executor = Arc::new(BlockingFixture {
            started: Default::default(),
        });
        let scheduler = DefaultAgentScheduler::new_with_executor(
            SchedulerConfig::default(),
            None,
            executor.clone(),
        )
        .await
        .unwrap();
        let config = make_test_config();
        scheduler.register_agent(config.clone()).await.unwrap();
        assert_eq!(
            scheduler.get_agent_status(config.id).await.unwrap().state,
            AgentState::Ready
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            executor.started.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
        let first = scheduler
            .schedule_invocation(config.clone(), serde_json::json!("wait"))
            .await
            .unwrap();
        let second = scheduler
            .schedule_invocation(config.clone(), serde_json::json!("wait"))
            .await
            .unwrap();
        wait_started(&executor, 2).await;
        assert_ne!(first.run_id(), second.run_id());
        assert_eq!(
            scheduler
                .get_agent_status(config.id)
                .await
                .unwrap()
                .active_tasks,
            2
        );
        assert_eq!(
            scheduler
                .load_balancer
                .get_statistics()
                .await
                .active_allocations,
            2
        );
        scheduler.terminate_agent(config.id).await.unwrap();
        assert_eq!(
            first.wait().await.status,
            task_manager::TaskStatus::Terminated
        );
        assert_eq!(
            second.wait().await.status,
            task_manager::TaskStatus::Terminated
        );
        assert_eq!(
            scheduler
                .load_balancer
                .get_statistics()
                .await
                .active_allocations,
            0
        );
        let final_run = scheduler
            .execute_agent(config.clone(), serde_json::json!("payload"))
            .await
            .unwrap();
        assert_eq!(final_run.output.as_deref(), Some("\"payload\""));
        scheduler.shutdown().await.unwrap();
        assert_eq!(
            scheduler
                .load_balancer
                .get_statistics()
                .await
                .active_allocations,
            0
        );
    }
    #[tokio::test]
    async fn queued_runs_expire_or_cancel_without_execution() {
        let executor = Arc::new(BlockingFixture {
            started: Default::default(),
        });
        let scheduler = DefaultAgentScheduler::new_with_executor(
            SchedulerConfig {
                max_concurrent_agents: 0,
                task_timeout: Duration::from_millis(100),
                ..Default::default()
            },
            None,
            executor.clone(),
        )
        .await
        .unwrap();
        let config = make_test_config();
        let first = scheduler
            .schedule_invocation(config.clone(), serde_json::Value::Null)
            .await
            .unwrap();
        let second = scheduler
            .schedule_invocation(config, serde_json::Value::Null)
            .await
            .unwrap();
        second.cancel();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), first.wait())
                .await
                .unwrap()
                .status,
            task_manager::TaskStatus::TimedOut
        );
        assert_eq!(
            second.wait().await.status,
            task_manager::TaskStatus::Terminated
        );
        assert_eq!(
            executor.started.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
        scheduler.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn deletion_resolves_every_queued_handle_and_prevents_dispatch() {
        let executor = Arc::new(BlockingFixture {
            started: Default::default(),
        });
        let scheduler = DefaultAgentScheduler::new_with_executor(
            SchedulerConfig {
                max_concurrent_agents: 0,
                ..Default::default()
            },
            None,
            executor.clone(),
        )
        .await
        .unwrap();
        let config = make_test_config();
        let first = scheduler
            .schedule_invocation(config.clone(), serde_json::Value::Null)
            .await
            .unwrap();
        let second = scheduler
            .schedule_invocation(config.clone(), serde_json::Value::Null)
            .await
            .unwrap();
        scheduler.delete_agent(config.id).await.unwrap();
        assert!(!scheduler.has_agent(config.id));
        for handle in [first, second] {
            assert_eq!(
                handle.wait().await.status,
                task_manager::TaskStatus::Terminated
            );
        }
        assert_eq!(
            executor.started.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
        scheduler.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn failed_execution_remains_failed_after_dequeue() {
        let scheduler = DefaultAgentScheduler::new(SchedulerConfig::default())
            .await
            .unwrap();
        let config = make_test_config();
        let result = scheduler
            .execute_agent(config.clone(), serde_json::Value::Null)
            .await
            .unwrap();
        assert_eq!(result.status, task_manager::TaskStatus::Failed);
        assert!(result.output.is_none());
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            scheduler.get_agent_status(config.id).await.unwrap().state,
            AgentState::Failed
        );
        assert_eq!(
            scheduler
                .load_balancer
                .get_statistics()
                .await
                .active_allocations,
            0
        );
        scheduler.shutdown().await.unwrap();
    }
}
