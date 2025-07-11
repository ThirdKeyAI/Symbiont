//! Agent Runtime Scheduler
//! 
//! The central orchestrator responsible for managing agent execution across the system.


use std::sync::Arc;
use std::time::{Duration, SystemTime};
use async_trait::async_trait;
use dashmap::DashMap;
use parking_lot::RwLock;
use tokio::sync::Notify;
use tokio::time::interval;

use crate::types::*;

pub mod priority_queue;
pub mod load_balancer;
pub mod task_manager;

use priority_queue::PriorityQueue;
use load_balancer::LoadBalancer;
use task_manager::TaskManager;

/// Agent scheduler trait
#[async_trait]
pub trait AgentScheduler {
    /// Schedule a new agent for execution
    async fn schedule_agent(&self, config: AgentConfig) -> Result<AgentId, SchedulerError>;
    
    /// Reschedule an existing agent with new priority
    async fn reschedule_agent(&self, agent_id: AgentId, priority: Priority) -> Result<(), SchedulerError>;
    
    /// Terminate an agent
    async fn terminate_agent(&self, agent_id: AgentId) -> Result<(), SchedulerError>;
    
    /// Get current system status
    async fn get_system_status(&self) -> SystemStatus;
    
    /// Shutdown the scheduler
    async fn shutdown(&self) -> Result<(), SchedulerError>;
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
            resource_requirements: config.metadata.get("resource_requirements")
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
        self.priority.cmp(&other.priority)
            .then_with(|| other.scheduled_at.cmp(&self.scheduled_at))
    }
}

/// Default implementation of the Agent Scheduler
pub struct DefaultAgentScheduler {
    config: SchedulerConfig,
    priority_queue: Arc<RwLock<PriorityQueue<ScheduledTask>>>,
    load_balancer: Arc<LoadBalancer>,
    task_manager: Arc<TaskManager>,
    running_agents: Arc<DashMap<AgentId, ScheduledTask>>,
    system_metrics: Arc<RwLock<SystemMetrics>>,
    shutdown_notify: Arc<Notify>,
    is_running: Arc<RwLock<bool>>,
}

impl DefaultAgentScheduler {
    /// Create a new scheduler instance
    pub async fn new(config: SchedulerConfig) -> Result<Self, SchedulerError> {
        let priority_queue = Arc::new(RwLock::new(PriorityQueue::new()));
        let load_balancer = Arc::new(LoadBalancer::new(config.load_balancing_strategy.clone()));
        let task_manager = Arc::new(TaskManager::new(config.task_timeout));
        let running_agents = Arc::new(DashMap::new());
        let system_metrics = Arc::new(RwLock::new(SystemMetrics::new()));
        let shutdown_notify = Arc::new(Notify::new());
        let is_running = Arc::new(RwLock::new(true));

        let scheduler = Self {
            config,
            priority_queue,
            load_balancer,
            task_manager,
            running_agents,
            system_metrics,
            shutdown_notify,
            is_running,
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
                                    
                                    // Start the task
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
                            if let Err(_) = task_manager.check_task_health(agent_id).await {
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

    async fn reschedule_agent(&self, agent_id: AgentId, priority: Priority) -> Result<(), SchedulerError> {
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
            self.task_manager.terminate_task(agent_id).await
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

    async fn get_system_status(&self) -> SystemStatus {
        let (total_scheduled, start_time) = {
            let metrics = self.system_metrics.read();
            (metrics.total_scheduled, metrics.start_time)
        };
        let now = SystemTime::now();
        let resource_utilization = self.load_balancer.get_resource_utilization().await;
        
        SystemStatus {
            total_agents: total_scheduled,
            running_agents: self.running_agents.len(),
            suspended_agents: 0, // TODO: Track suspended agents
            resource_utilization,
            uptime: now.duration_since(start_time).unwrap_or_default(),
            last_updated: now,
        }
    }

    async fn shutdown(&self) -> Result<(), SchedulerError> {
        tracing::info!("Shutting down scheduler");
        
        *self.is_running.write() = false;
        self.shutdown_notify.notify_waiters();

        // Terminate all running agents
        let running_agent_ids: Vec<AgentId> = self.running_agents.iter()
            .map(|entry| *entry.key())
            .collect();

        for agent_id in running_agent_ids {
            if let Err(e) = self.terminate_agent(agent_id).await {
                tracing::error!("Failed to terminate agent {} during shutdown: {}", agent_id, e);
            }
        }

        Ok(())
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