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

use crate::routing::RoutingEngine;
use crate::types::*;

#[cfg(feature = "http-api")]
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
#[cfg(feature = "cron")]
pub mod policy_gate;

// use load_balancer::LoadBalancer;
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
    /// Schedule a new agent for execution
    async fn schedule_agent(&self, config: AgentConfig) -> Result<AgentId, SchedulerError>;

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
        }
    }
}

/// Scheduled task information
#[derive(Debug, Clone)]
pub struct ScheduledTask {
    pub agent_id: AgentId,
    pub config: AgentConfig,
    pub priority: Priority,
    pub scheduled_at: SystemTime,
    pub deadline: Option<SystemTime>,
    pub retry_count: u32,
    pub resource_requirements: ResourceRequirements,
}

impl ScheduledTask {
    pub fn new(config: AgentConfig) -> Self {
        let now = SystemTime::now();
        Self {
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
        }
    }
}

impl PartialEq for ScheduledTask {
    fn eq(&self, other: &Self) -> bool {
        self.agent_id == other.agent_id
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
    }
}

// Stub LoadBalancer type until the actual implementation is available
pub struct LoadBalancer;

impl LoadBalancer {
    pub fn new(_strategy: LoadBalancingStrategy) -> Self {
        Self
    }

    pub async fn allocate_resources(
        &self,
        _requirements: &ResourceRequirements,
    ) -> Result<ResourceAllocation, String> {
        // Stub implementation - always succeeds
        Ok(ResourceAllocation {
            agent_id: AgentId::new(),
            allocated_memory: 0,
            allocated_cpu_cores: 0.0,
            allocated_disk_io: 0,
            allocated_network_io: 0,
            allocation_time: SystemTime::now(),
        })
    }

    pub async fn deallocate_resources(&self, _allocation: ResourceAllocation) {
        // Stub implementation
    }

    pub async fn get_resource_utilization(&self) -> ResourceUsage {
        ResourceUsage {
            memory_used: 0,
            cpu_utilization: 0.0,
            disk_io_rate: 0,
            network_io_rate: 0,
            uptime: std::time::Duration::from_secs(0),
        }
    }

    pub async fn get_statistics(&self) -> serde_json::Value {
        serde_json::json!({})
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
    running_agents: Arc<DashMap<AgentId, ScheduledTask>>,
    suspended_agents: Arc<DashMap<AgentId, AgentSuspensionInfo>>,
    system_metrics: Arc<RwLock<SystemMetrics>>,
    shutdown_notify: Arc<Notify>,
    is_running: Arc<RwLock<bool>>,
    routing_engine: Option<Arc<dyn RoutingEngine>>,
}

impl DefaultAgentScheduler {
    /// Create a new scheduler instance
    pub async fn new(config: SchedulerConfig) -> Result<Self, SchedulerError> {
        Self::new_with_routing(config, None).await
    }

    /// Create a new scheduler instance with optional routing engine
    pub async fn new_with_routing(
        config: SchedulerConfig,
        routing_engine: Option<Arc<dyn RoutingEngine>>,
    ) -> Result<Self, SchedulerError> {
        let priority_queue = Arc::new(RwLock::new(PriorityQueue::new()));
        let load_balancer = Arc::new(LoadBalancer::new(config.load_balancing_strategy.clone()));
        let task_manager = Arc::new(TaskManager::new(config.task_timeout));
        let running_agents = Arc::new(DashMap::new());
        let suspended_agents = Arc::new(DashMap::new());
        let system_metrics = Arc::new(RwLock::new(SystemMetrics::new()));
        let shutdown_notify = Arc::new(Notify::new());
        let is_running = Arc::new(RwLock::new(true));

        let scheduler = Self {
            config,
            priority_queue,
            load_balancer,
            task_manager,
            running_agents,
            suspended_agents,
            system_metrics,
            shutdown_notify,
            is_running,
            routing_engine,
        };

        // Start background tasks
        scheduler.start_scheduler_loop().await;
        scheduler.start_health_check_loop().await;

        Ok(scheduler)
    }

    /// Start the main scheduler loop
    async fn start_scheduler_loop(&self) {
        let priority_queue = self.priority_queue.clone();
        let load_balancer = self.load_balancer.clone();
        let task_manager = self.task_manager.clone();
        let running_agents = self.running_agents.clone();
        let system_metrics = self.system_metrics.clone();
        let shutdown_notify = self.shutdown_notify.clone();
        let is_running = self.is_running.clone();
        let _routing_engine = self.routing_engine.clone();
        let max_concurrent = self.config.max_concurrent_agents;

        tokio::spawn(async move {
            let mut interval = interval(Duration::from_millis(100));

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if !*is_running.read() {
                            break;
                        }

                        // Check if we can schedule more agents
                        if running_agents.len() < max_concurrent {
                            let task_opt = {
                                let mut queue = priority_queue.write();
                                queue.pop()
                            };

                            if let Some(task) = task_opt {
                                // Try to schedule the task
                                if let Ok(resource_allocation) = load_balancer.allocate_resources(&task.resource_requirements).await {
                                    running_agents.insert(task.agent_id, task.clone());

                                    // Start the task (routing integration would be handled in TaskManager if needed)
                                    if let Err(e) = task_manager.start_task(task.clone()).await {
                                        tracing::error!("Failed to start task for agent {}: {}", task.agent_id, e);
                                        running_agents.remove(&task.agent_id);
                                        load_balancer.deallocate_resources(resource_allocation).await;
                                    }
                                } else {
                                    // Put the task back in the queue if resources aren't available
                                    let mut queue = priority_queue.write();
                                    queue.push(task);
                                }
                            }
                        }

                        // Update system metrics
                        let (running_count, queue_len) = {
                            let queue = priority_queue.read();
                            (running_agents.len(), queue.len())
                        };
                        system_metrics.write().update(running_count, queue_len);
                    }
                    _ = shutdown_notify.notified() => {
                        break;
                    }
                }
            }
        });
    }

    /// Start the health check loop
    async fn start_health_check_loop(&self) {
        let task_manager = self.task_manager.clone();
        let running_agents = self.running_agents.clone();
        let shutdown_notify = self.shutdown_notify.clone();
        let is_running = self.is_running.clone();
        let health_check_interval = self.config.health_check_interval;

        tokio::spawn(async move {
            let mut interval = interval(health_check_interval);

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if !*is_running.read() {
                            break;
                        }

                        // Check health of running agents
                        let mut failed_agents = Vec::new();
                        for entry in running_agents.iter() {
                            let agent_id = *entry.key();
                            if (task_manager.check_task_health(agent_id).await).is_err() {
                                failed_agents.push(agent_id);
                            }
                        }

                        // Remove failed agents
                        for agent_id in failed_agents {
                            running_agents.remove(&agent_id);
                            if let Err(e) = task_manager.terminate_task(agent_id).await {
                                tracing::error!("Failed to terminate failed agent {}: {}", agent_id, e);
                            }
                        }
                    }
                    _ = shutdown_notify.notified() => {
                        break;
                    }
                }
            }
        });
    }
}

#[async_trait]
impl AgentScheduler for DefaultAgentScheduler {
    async fn schedule_agent(&self, config: AgentConfig) -> Result<AgentId, SchedulerError> {
        if !*self.is_running.read() {
            return Err(SchedulerError::ShuttingDown);
        }

        let task = ScheduledTask::new(config);
        let agent_id = task.agent_id;

        // Add to priority queue
        self.priority_queue.write().push(task);

        tracing::info!("Scheduled agent {} for execution", agent_id);
        Ok(agent_id)
    }

    async fn reschedule_agent(
        &self,
        agent_id: AgentId,
        priority: Priority,
    ) -> Result<(), SchedulerError> {
        if !*self.is_running.read() {
            return Err(SchedulerError::ShuttingDown);
        }

        // Check if agent is currently running
        if let Some(mut entry) = self.running_agents.get_mut(&agent_id) {
            entry.priority = priority;
            return Ok(());
        }

        // Check if agent is in the queue
        let mut queue = self.priority_queue.write();
        if let Some(mut task) = queue.remove(&agent_id) {
            task.priority = priority;
            queue.push(task);
            return Ok(());
        }

        Err(SchedulerError::AgentNotFound { agent_id })
    }

    async fn terminate_agent(&self, agent_id: AgentId) -> Result<(), SchedulerError> {
        // Remove from running agents
        if let Some((_, _task)) = self.running_agents.remove(&agent_id) {
            self.task_manager
                .terminate_task(agent_id)
                .await
                .map_err(|e| SchedulerError::SchedulingFailed {
                    agent_id,
                    reason: format!("Failed to terminate task: {}", e),
                })?;

            tracing::info!("Terminated agent {}", agent_id);
            return Ok(());
        }

        // Remove from queue
        let mut queue = self.priority_queue.write();
        if queue.remove(&agent_id).is_some() {
            tracing::info!("Removed agent {} from queue", agent_id);
            return Ok(());
        }

        Err(SchedulerError::AgentNotFound { agent_id })
    }

    async fn shutdown_agent(&self, agent_id: AgentId) -> Result<(), SchedulerError> {
        // Check if agent is currently running
        if let Some((_, _task)) = self.running_agents.remove(&agent_id) {
            // For graceful shutdown, we use the same task manager termination
            // but could potentially add graceful shutdown signals in the future
            self.task_manager
                .terminate_task(agent_id)
                .await
                .map_err(|e| SchedulerError::SchedulingFailed {
                    agent_id,
                    reason: format!("Failed to shutdown task: {}", e),
                })?;

            tracing::info!("Gracefully shutdown agent {}", agent_id);
            return Ok(());
        }

        // Remove from queue if not running
        let mut queue = self.priority_queue.write();
        if queue.remove(&agent_id).is_some() {
            tracing::info!("Removed agent {} from queue during shutdown", agent_id);
            return Ok(());
        }

        Err(SchedulerError::AgentNotFound { agent_id })
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
        // Check if agent is currently running
        if let Some(entry) = self.running_agents.get(&agent_id) {
            let scheduled_task = entry.value();

            // Get detailed health information from task manager
            match self.task_manager.check_task_health(agent_id).await {
                Ok(task_health) => {
                    // Map TaskStatus to AgentState
                    let state = match task_health.status {
                        task_manager::TaskStatus::Pending => AgentState::Ready,
                        task_manager::TaskStatus::Running => AgentState::Running,
                        task_manager::TaskStatus::Completed => AgentState::Completed,
                        task_manager::TaskStatus::Failed => AgentState::Failed,
                        task_manager::TaskStatus::TimedOut => AgentState::Failed,
                        task_manager::TaskStatus::Terminated => AgentState::Terminated,
                    };

                    let active_tasks = if matches!(state, AgentState::Running) {
                        1
                    } else {
                        0
                    };

                    Ok(AgentStatus {
                        agent_id,
                        state,
                        last_activity: task_health.last_activity,
                        memory_usage: task_health.memory_usage as u64,
                        cpu_usage: task_health.cpu_usage as f64,
                        active_tasks,
                        scheduled_at: scheduled_task.scheduled_at,
                    })
                }
                Err(_) => {
                    // Agent exists but health check failed - might be in error state
                    Ok(AgentStatus {
                        agent_id,
                        state: AgentState::Failed,
                        last_activity: scheduled_task.scheduled_at,
                        memory_usage: 0,
                        cpu_usage: 0.0,
                        active_tasks: 0,
                        scheduled_at: scheduled_task.scheduled_at,
                    })
                }
            }
        } else {
            // Check if agent is in the queue
            let queue = self.priority_queue.read();
            if let Some(task) = queue.find(&agent_id) {
                // Agent is queued but not yet running
                Ok(AgentStatus {
                    agent_id,
                    state: AgentState::Waiting,
                    last_activity: task.scheduled_at,
                    memory_usage: 0,
                    cpu_usage: 0.0,
                    active_tasks: 0,
                    scheduled_at: task.scheduled_at,
                })
            } else {
                // Agent not found anywhere
                Err(SchedulerError::AgentNotFound { agent_id })
            }
        }
    }

    async fn shutdown(&self) -> Result<(), SchedulerError> {
        // Check if already shutting down (idempotent)
        {
            let is_running = self.is_running.read();
            if !*is_running {
                tracing::debug!("Scheduler already shutdown");
                return Ok(());
            }
        }

        tracing::info!("Initiating graceful scheduler shutdown");

        // Set shutdown flag and notify background tasks
        *self.is_running.write() = false;
        self.shutdown_notify.notify_waiters();

        // Step 1: Stop accepting new agents (already done by setting is_running=false)

        // Step 2: Gracefully shutdown all running agents with timeout
        let running_agent_ids: Vec<AgentId> = self
            .running_agents
            .iter()
            .map(|entry| *entry.key())
            .collect();

        tracing::info!(
            "Shutting down {} running agents gracefully",
            running_agent_ids.len()
        );

        // First pass: attempt graceful shutdown
        let graceful_timeout = Duration::from_secs(30);
        let graceful_start = std::time::Instant::now();

        for agent_id in &running_agent_ids {
            if graceful_start.elapsed() >= graceful_timeout {
                tracing::warn!(
                    "Graceful shutdown timeout reached, switching to forced termination"
                );
                break;
            }

            // Use graceful shutdown method first
            if let Err(e) = self.shutdown_agent(*agent_id).await {
                tracing::warn!(
                    "Failed to gracefully shutdown agent {}: {}, will force terminate",
                    agent_id,
                    e
                );
            }
        }

        // Wait a bit for agents to terminate gracefully
        tokio::time::sleep(Duration::from_secs(5)).await;

        // Step 3: Force terminate any remaining agents
        let remaining_agent_ids: Vec<AgentId> = self
            .running_agents
            .iter()
            .map(|entry| *entry.key())
            .collect();

        if !remaining_agent_ids.is_empty() {
            tracing::warn!(
                "Force terminating {} remaining agents",
                remaining_agent_ids.len()
            );

            for agent_id in remaining_agent_ids {
                if let Err(e) = self.terminate_agent(agent_id).await {
                    tracing::error!(
                        "Failed to force terminate agent {} during shutdown: {}",
                        agent_id,
                        e
                    );
                }
            }
        }

        // Step 4: Flush metrics to persistent storage
        self.flush_metrics().await?;

        // Step 5: Release all allocated resources
        self.cleanup_resources().await?;

        // Step 6: Final cleanup of queued agents
        {
            let mut queue = self.priority_queue.write();
            let queued_count = queue.len();
            if queued_count > 0 {
                tracing::info!("Clearing {} queued agents", queued_count);
                queue.clear();
            }
        }

        tracing::info!("Scheduler shutdown completed successfully");
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
        let load_factor = running_count as f64 / self.config.max_concurrent_agents as f64;

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
        let mut agent_ids = Vec::new();

        // Collect running agents
        for entry in self.running_agents.iter() {
            agent_ids.push(*entry.key());
        }

        // Collect queued agents
        let queue = self.priority_queue.read();
        let queued_tasks = queue.to_vec();
        for task in queued_tasks {
            agent_ids.push(task.agent_id);
        }

        agent_ids
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

        // Check if agent is currently running
        if let Some(mut entry) = self.running_agents.get_mut(&agent_id) {
            let task = entry.value_mut();

            // Update the agent configuration
            if let Some(name) = request.name {
                task.config.name = name;
            }

            if let Some(dsl) = request.dsl {
                task.config.dsl_source = dsl;
            }

            tracing::info!("Updated running agent {}", agent_id);
            return Ok(());
        }

        // Check if agent is in the queue
        let mut queue = self.priority_queue.write();
        if let Some(mut task) = queue.remove(&agent_id) {
            // Update the agent configuration
            if let Some(name) = request.name {
                task.config.name = name;
            }

            if let Some(dsl) = request.dsl {
                task.config.dsl_source = dsl;
            }

            // Put it back in the queue
            queue.push(task);
            tracing::info!("Updated queued agent {}", agent_id);
            return Ok(());
        }

        Err(SchedulerError::AgentNotFound { agent_id })
    }
}

impl DefaultAgentScheduler {
    /// Flush system metrics to persistent storage
    async fn flush_metrics(&self) -> Result<(), SchedulerError> {
        tracing::debug!("Flushing scheduler metrics");

        let (total_scheduled, uptime, running_count, queued_count) = {
            let metrics = self.system_metrics.read();
            let queue = self.priority_queue.read();
            let now = SystemTime::now();
            (
                metrics.total_scheduled,
                metrics.uptime_since(now),
                self.running_agents.len(),
                queue.len(),
            )
        };

        // Get task manager statistics
        let task_stats = self.task_manager.get_task_statistics().await;

        // Get load balancer statistics
        let lb_stats = self.load_balancer.get_statistics().await;

        // In a real implementation, this would write to persistent storage
        // For now, we log comprehensive shutdown metrics
        tracing::info!(
            "Scheduler shutdown metrics - Total scheduled: {}, Uptime: {:?}, \
             Running agents: {}, Queued agents: {}, Task stats: {:?}, \
             Load balancer stats: {:?}",
            total_scheduled,
            uptime,
            running_count,
            queued_count,
            task_stats,
            lb_stats
        );

        // Implement actual persistence to metrics system
        self.persist_metrics_to_storage(
            total_scheduled,
            uptime,
            running_count,
            queued_count,
            serde_json::to_value(&task_stats).unwrap_or(serde_json::json!({})),
            lb_stats,
        )
        .await?;

        Ok(())
    }

    /// Clean up all allocated resources
    async fn cleanup_resources(&self) -> Result<(), SchedulerError> {
        tracing::debug!("Cleaning up allocated resources");

        // Get all allocated agents and their resource allocations
        let allocated_agents: Vec<AgentId> = self
            .running_agents
            .iter()
            .map(|entry| *entry.key())
            .collect();

        // For each agent, ensure resources are properly deallocated
        for agent_id in allocated_agents {
            // Create a dummy allocation for cleanup
            // In a real implementation, we'd track actual allocations
            let allocation = ResourceAllocation {
                agent_id,
                allocated_memory: 0, // Would be tracked from actual allocation
                allocated_cpu_cores: 0.0,
                allocated_disk_io: 0,
                allocated_network_io: 0,
                allocation_time: SystemTime::now(),
            };

            self.load_balancer.deallocate_resources(allocation).await;
        }

        // Additional cleanup for task manager resources
        // The task manager will handle process cleanup in its own termination methods

        tracing::debug!("Resource cleanup completed");
        Ok(())
    }

    /// Persist metrics to storage system (replaces TODO placeholder)
    async fn persist_metrics_to_storage(
        &self,
        total_scheduled: usize,
        uptime: Duration,
        running_count: usize,
        queued_count: usize,
        task_stats: serde_json::Value,
        lb_stats: serde_json::Value,
    ) -> Result<(), SchedulerError> {
        // Create comprehensive metrics payload
        let metrics_data = serde_json::json!({
            "timestamp": SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            "scheduler": {
                "total_scheduled": total_scheduled,
                "uptime_seconds": uptime.as_secs(),
                "running_agents": running_count,
                "queued_agents": queued_count,
                "suspended_agents": self.suspended_agents.len(),
                "max_capacity": self.config.max_concurrent_agents,
                "load_factor": running_count as f64 / self.config.max_concurrent_agents as f64
            },
            "task_manager": task_stats,
            "load_balancer": lb_stats,
            "system": {
                "memory_usage": self.get_system_memory_usage().await,
                "cpu_usage": self.get_system_cpu_usage().await
            }
        });

        // In a production environment, this would:
        // 1. Write to InfluxDB for time-series metrics
        // 2. Send to Prometheus via pushgateway
        // 3. Store in a relational database for queries
        // 4. Send to monitoring services like DataDog, New Relic, etc.

        // For now, we'll write to a local metrics file as a basic implementation
        let metrics_file = std::env::temp_dir().join("symbiont_scheduler_metrics.json");

        match tokio::fs::write(&metrics_file, serde_json::to_string_pretty(&metrics_data)?).await {
            Ok(_) => {
                tracing::debug!(
                    "Successfully persisted scheduler metrics to {}",
                    metrics_file.display()
                );
            }
            Err(e) => {
                tracing::error!("Failed to persist metrics to file: {}", e);
                // Don't fail the shutdown process due to metrics persistence failure
            }
        }

        Ok(())
    }

    /// Get current system memory usage
    async fn get_system_memory_usage(&self) -> f64 {
        // In a real implementation, this would query system memory usage
        // For now, estimate based on running agents
        let running_count = self.running_agents.len() as f64;
        let estimated_memory_per_agent = 50.0; // MB
        running_count * estimated_memory_per_agent
    }

    /// Get current system CPU usage
    async fn get_system_cpu_usage(&self) -> f64 {
        // In a real implementation, this would query actual CPU usage
        // For now, estimate based on running agents and max capacity
        let load_factor =
            self.running_agents.len() as f64 / self.config.max_concurrent_agents as f64;
        (load_factor * 100.0).min(100.0) // Convert to percentage, cap at 100%
    }

    /// Suspend an agent (moves from running to suspended state)
    pub async fn suspend_agent(
        &self,
        agent_id: AgentId,
        reason: String,
    ) -> Result<(), SchedulerError> {
        if let Some((_, task)) = self.running_agents.remove(&agent_id) {
            // Stop the task
            if let Err(e) = self.task_manager.terminate_task(agent_id).await {
                tracing::error!("Failed to terminate task during suspension: {}", e);
                // Put the agent back in running state if we can't stop it
                self.running_agents.insert(agent_id, task);
                return Err(SchedulerError::SchedulingFailed {
                    agent_id,
                    reason: format!("Failed to suspend agent: {}", e),
                });
            }

            // Create suspension info
            let suspension_info = AgentSuspensionInfo {
                agent_id,
                suspended_at: SystemTime::now(),
                suspension_reason: reason.clone(),
                original_task: task,
                can_resume: true,
            };

            // Store in suspended agents
            self.suspended_agents.insert(agent_id, suspension_info);

            tracing::info!("Suspended agent {} with reason: {}", agent_id, reason);
            Ok(())
        } else {
            Err(SchedulerError::AgentNotFound { agent_id })
        }
    }

    /// Resume a suspended agent
    pub async fn resume_agent(&self, agent_id: AgentId) -> Result<(), SchedulerError> {
        if let Some((_, suspension_info)) = self.suspended_agents.remove(&agent_id) {
            if !suspension_info.can_resume {
                return Err(SchedulerError::SchedulingFailed {
                    agent_id,
                    reason: "Agent cannot be resumed".to_string(),
                });
            }

            // Add back to priority queue for scheduling
            let mut task = suspension_info.original_task;
            task.scheduled_at = SystemTime::now(); // Update schedule time

            self.priority_queue.write().push(task);

            tracing::info!("Resumed agent {} from suspension", agent_id);
            Ok(())
        } else {
            Err(SchedulerError::AgentNotFound { agent_id })
        }
    }

    /// Get list of suspended agents
    pub async fn list_suspended_agents(&self) -> Vec<AgentSuspensionInfo> {
        self.suspended_agents
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
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
