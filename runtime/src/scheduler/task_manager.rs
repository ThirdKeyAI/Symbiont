//! Task manager for executing and monitoring agent tasks

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use parking_lot::RwLock;
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::types::*;

/// Task manager for agent execution
pub struct TaskManager {
    task_timeout: Duration,
    running_tasks: Arc<RwLock<HashMap<AgentId, TaskHandle>>>,
    task_sender: mpsc::UnboundedSender<TaskCommand>,
}

impl TaskManager {
    /// Create a new task manager
    pub fn new(task_timeout: Duration) -> Self {
        let running_tasks = Arc::new(RwLock::new(HashMap::new()));
        let (task_sender, task_receiver) = mpsc::unbounded_channel();

        let manager = Self {
            task_timeout,
            running_tasks: running_tasks.clone(),
            task_sender,
        };

        // Start the task execution loop
        manager.start_task_loop(task_receiver);

        manager
    }

    /// Start a new task
    pub async fn start_task(&self, task: super::ScheduledTask) -> Result<(), SchedulerError> {
        let handle = TaskHandle::new(task.clone());
        let agent_id = task.agent_id;
        
        // Store the handle
        self.running_tasks.write().insert(agent_id, handle.clone());

        // Send command to start the task
        self.task_sender.send(TaskCommand::Start { task: Box::new(task), handle })
            .map_err(|_| SchedulerError::SchedulingFailed {
                agent_id,
                reason: "Failed to send start command".to_string(),
            })?;

        Ok(())
    }

    /// Terminate a task
    pub async fn terminate_task(&self, agent_id: AgentId) -> Result<(), SchedulerError> {
        // Remove from running tasks
        let handle = self.running_tasks.write().remove(&agent_id);
        
        if let Some(handle) = handle {
            // Send termination command
            self.task_sender.send(TaskCommand::Terminate { agent_id, handle })
                .map_err(|_| SchedulerError::SchedulingFailed {
                    agent_id,
                    reason: "Failed to send terminate command".to_string(),
                })?;
        }

        Ok(())
    }

    /// Check task health
    pub async fn check_task_health(&self, agent_id: AgentId) -> Result<TaskHealth, SchedulerError> {
        let running_tasks = self.running_tasks.read();
        
        if let Some(handle) = running_tasks.get(&agent_id) {
            let health = handle.get_health();
            
            // Check if task has exceeded timeout
            if health.uptime > self.task_timeout {
                return Err(SchedulerError::SchedulingFailed {
                    agent_id,
                    reason: "Task timeout exceeded".to_string(),
                });
            }

            Ok(health)
        } else {
            Err(SchedulerError::AgentNotFound { agent_id })
        }
    }

    /// Get task statistics
    pub async fn get_task_statistics(&self) -> TaskStatistics {
        let running_tasks = self.running_tasks.read();
        let total_tasks = running_tasks.len();
        
        let mut healthy_tasks = 0;
        let mut total_uptime = Duration::from_secs(0);
        let mut total_memory_usage = 0;

        for handle in running_tasks.values() {
            let health = handle.get_health();
            if health.is_healthy {
                healthy_tasks += 1;
            }
            total_uptime += health.uptime;
            total_memory_usage += health.memory_usage;
        }

        TaskStatistics {
            total_tasks,
            healthy_tasks,
            average_uptime: if total_tasks > 0 {
                total_uptime / total_tasks as u32
            } else {
                Duration::from_secs(0)
            },
            total_memory_usage,
        }
    }

    /// Start the task execution loop
    fn start_task_loop(&self, mut task_receiver: mpsc::UnboundedReceiver<TaskCommand>) {
        let task_timeout = self.task_timeout;

        tokio::spawn(async move {
            while let Some(command) = task_receiver.recv().await {
                match command {
                    TaskCommand::Start { task, handle } => {
                        Self::execute_task(*task, handle, task_timeout).await;
                    }
                    TaskCommand::Terminate { agent_id, handle } => {
                        Self::terminate_task_execution(agent_id, handle).await;
                    }
                }
            }
        });
    }

    /// Execute a task
    async fn execute_task(task: super::ScheduledTask, handle: TaskHandle, task_timeout: Duration) {
        let agent_id = task.agent_id;
        
        tracing::info!("Starting execution of agent {}", agent_id);
        
        // Update handle status
        handle.set_status(TaskStatus::Running);

        // Simulate task execution (in real implementation, this would execute the agent)
        let execution_result = timeout(task_timeout, Self::simulate_agent_execution(task.clone())).await;

        match execution_result {
            Ok(Ok(())) => {
                tracing::info!("Agent {} completed successfully", agent_id);
                handle.set_status(TaskStatus::Completed);
            }
            Ok(Err(e)) => {
                tracing::error!("Agent {} failed: {}", agent_id, e);
                handle.set_status(TaskStatus::Failed);
            }
            Err(_) => {
                tracing::error!("Agent {} timed out", agent_id);
                handle.set_status(TaskStatus::TimedOut);
            }
        }
    }

    /// Terminate task execution
    async fn terminate_task_execution(agent_id: AgentId, handle: TaskHandle) {
        tracing::info!("Terminating agent {}", agent_id);
        handle.set_status(TaskStatus::Terminated);
    }

    /// Simulate agent execution (placeholder for real implementation)
    async fn simulate_agent_execution(task: super::ScheduledTask) -> Result<(), String> {
        // Simulate some work
        tokio::time::sleep(Duration::from_millis(100)).await;
        
        // Simulate random success/failure for testing
        if task.agent_id.0.as_u128() % 10 == 0 {
            Err("Simulated failure".to_string())
        } else {
            Ok(())
        }
    }
}

/// Task command for the execution loop
#[derive(Debug)]
enum TaskCommand {
    Start {
        task: Box<super::ScheduledTask>,
        handle: TaskHandle,
    },
    Terminate {
        agent_id: AgentId,
        handle: TaskHandle,
    },
}

/// Handle for a running task
#[derive(Debug, Clone)]
pub struct TaskHandle {
    agent_id: AgentId,
    status: Arc<RwLock<TaskStatus>>,
    start_time: SystemTime,
    metrics: Arc<RwLock<TaskMetrics>>,
}

impl TaskHandle {
    fn new(task: super::ScheduledTask) -> Self {
        Self {
            agent_id: task.agent_id,
            status: Arc::new(RwLock::new(TaskStatus::Pending)),
            start_time: SystemTime::now(),
            metrics: Arc::new(RwLock::new(TaskMetrics::new())),
        }
    }

    fn set_status(&self, status: TaskStatus) {
        *self.status.write() = status.clone();
        self.metrics.write().update_status(&status);
    }

    fn get_health(&self) -> TaskHealth {
        let status = self.status.read().clone();
        let uptime = SystemTime::now().duration_since(self.start_time).unwrap_or_default();
        let metrics = self.metrics.read();

        TaskHealth {
            agent_id: self.agent_id,
            status: status.clone(),
            uptime,
            is_healthy: matches!(status, TaskStatus::Running | TaskStatus::Pending),
            memory_usage: metrics.memory_usage,
            cpu_usage: metrics.cpu_usage,
            last_activity: metrics.last_activity,
        }
    }
}

/// Task execution status
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    TimedOut,
    Terminated,
}

/// Task health information
#[derive(Debug, Clone)]
pub struct TaskHealth {
    pub agent_id: AgentId,
    pub status: TaskStatus,
    pub uptime: Duration,
    pub is_healthy: bool,
    pub memory_usage: usize,
    pub cpu_usage: f32,
    pub last_activity: SystemTime,
}

/// Task metrics for monitoring
#[derive(Debug, Clone)]
struct TaskMetrics {
    memory_usage: usize,
    cpu_usage: f32,
    last_activity: SystemTime,
    status_changes: u32,
}

impl TaskMetrics {
    fn new() -> Self {
        Self {
            memory_usage: 0,
            cpu_usage: 0.0,
            last_activity: SystemTime::now(),
            status_changes: 0,
        }
    }

    fn update_status(&mut self, _status: &TaskStatus) {
        self.status_changes += 1;
        self.last_activity = SystemTime::now();
        
        // Simulate resource usage updates
        self.memory_usage = (self.status_changes as usize * 1024) % (512 * 1024); // 0-512MB
        self.cpu_usage = (self.status_changes as f32 * 0.1) % 1.0; // 0-100%
    }
}

/// Task statistics for monitoring
#[derive(Debug, Clone)]
pub struct TaskStatistics {
    pub total_tasks: usize,
    pub healthy_tasks: usize,
    pub average_uptime: Duration,
    pub total_memory_usage: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AgentConfig, ExecutionMode, SecurityTier, ResourceLimits, Priority};
    use std::collections::HashMap;

    fn create_test_task() -> super::super::ScheduledTask {
        let agent_id = AgentId::new();
        let config = AgentConfig {
            id: agent_id,
            name: "test".to_string(),
            dsl_source: "test".to_string(),
            execution_mode: ExecutionMode::Ephemeral,
            security_tier: SecurityTier::Tier1,
            resource_limits: ResourceLimits::default(),
            capabilities: vec![],
            policies: vec![],
            metadata: HashMap::new(),
            priority: Priority::Normal,
        };
        super::super::ScheduledTask::new(config)
    }

    #[tokio::test]
    async fn test_task_start_and_health_check() {
        let task_manager = TaskManager::new(Duration::from_secs(60));
        let task = create_test_task();
        let agent_id = task.agent_id;

        // Start the task
        let result = task_manager.start_task(task).await;
        assert!(result.is_ok());

        // Give it a moment to start
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Check health
        let health = task_manager.check_task_health(agent_id).await;
        assert!(health.is_ok());
        
        let health = health.unwrap();
        assert_eq!(health.agent_id, agent_id);
        assert!(health.is_healthy);
    }

    #[tokio::test]
    async fn test_task_termination() {
        let task_manager = TaskManager::new(Duration::from_secs(60));
        let task = create_test_task();
        let agent_id = task.agent_id;

        // Start the task
        task_manager.start_task(task).await.unwrap();
        
        // Give it a moment to start
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Terminate the task
        let result = task_manager.terminate_task(agent_id).await;
        assert!(result.is_ok());

        // Check that health check fails for terminated task
        let health = task_manager.check_task_health(agent_id).await;
        assert!(health.is_err());
    }

    #[tokio::test]
    async fn test_task_statistics() {
        let task_manager = TaskManager::new(Duration::from_secs(60));
        
        // Start multiple tasks
        for _ in 0..3 {
            let task = create_test_task();
            task_manager.start_task(task).await.unwrap();
        }

        // Give them a moment to start
        tokio::time::sleep(Duration::from_millis(50)).await;

        let stats = task_manager.get_task_statistics().await;
        assert_eq!(stats.total_tasks, 3);
        assert!(stats.healthy_tasks <= 3); // Some might have completed already
    }
}