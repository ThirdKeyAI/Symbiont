//! Bounded concurrent task execution with exact completion and cancellation.

use super::execution::{GovernedAgentExecutor, ScheduledAgentExecutor};
use crate::types::*;
use futures::FutureExt;
use parking_lot::RwLock;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

const MAX_TASKS: usize = 2048;
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(25);

pub struct TaskManager {
    task_timeout: Duration,
    tasks: Arc<RwLock<HashMap<uuid::Uuid, TaskHandle>>>,
    executor: Arc<dyn ScheduledAgentExecutor>,
}

impl TaskManager {
    pub fn new(task_timeout: Duration) -> Self {
        Self::with_executor(task_timeout, Arc::new(GovernedAgentExecutor::default()))
    }

    /// Supply a trusted execution service, for embedding or deterministic tests.
    pub fn with_executor(
        task_timeout: Duration,
        executor: Arc<dyn ScheduledAgentExecutor>,
    ) -> Self {
        Self {
            task_timeout,
            tasks: Arc::new(RwLock::new(HashMap::new())),
            executor,
        }
    }

    #[cfg(unix)]
    pub(crate) fn invocation_project(&self) -> Result<&std::path::Path, String> {
        self.executor.invocation_project()
    }

    pub async fn start_task(&self, task: super::ScheduledTask) -> Result<(), SchedulerError> {
        let handle = task.handle.clone();
        if handle.completion().is_some() {
            return Ok(());
        }
        {
            let mut tasks = self.tasks.write();
            if tasks.contains_key(&handle.run_id()) {
                return Err(failure(task.agent_id, "execution is already admitted"));
            }
            if tasks.len() >= MAX_TASKS {
                if let Some(oldest) = tasks
                    .values()
                    .filter(|h| h.completion().is_some())
                    .min_by_key(|h| h.created)
                    .map(TaskHandle::run_id)
                {
                    tasks.remove(&oldest);
                } else {
                    return Err(failure(task.agent_id, "task capacity exhausted"));
                }
            }
            tasks.insert(handle.run_id(), handle.clone());
        }
        let executor = self.executor.clone();
        let budget = self
            .task_timeout
            .min(task.config.resource_limits.execution_timeout)
            .saturating_sub(handle.created.elapsed());
        tokio::spawn(async move {
            if budget.is_zero() {
                handle.finish(TaskCompletion::new(
                    &task,
                    TaskStatus::TimedOut,
                    Some("execution expired in queue".into()),
                ));
                return;
            }
            if !handle.mark_running() {
                handle.finish(TaskCompletion::new(
                    &task,
                    TaskStatus::Terminated,
                    Some("execution cancelled before start".into()),
                ));
                return;
            }
            let execution = std::panic::AssertUnwindSafe(executor.execute(
                &task,
                budget,
                handle.cancellation.clone(),
            ))
            .catch_unwind()
            .map(|result| {
                result.unwrap_or_else(|_| {
                    handle.cancel();
                    TaskCompletion::new(
                        &task,
                        TaskStatus::Failed,
                        Some("execution service panicked; cleanup is unconfirmed".into()),
                    )
                })
            });
            tokio::pin!(execution);
            let mut timed_out = false;
            let result = tokio::select! {
                biased;
                _ = handle.cancellation.cancelled() => None,
                result = &mut execution => Some(result),
                _ = tokio::time::sleep(budget) => {
                    timed_out = true;
                    handle.cancellation.cancel();
                    None
                },
            };
            let mut completion = match result {
                Some(completion) => completion,
                None => match tokio::time::timeout(CLEANUP_TIMEOUT, &mut execution).await {
                    Ok(completion) => completion,
                    Err(_) => TaskCompletion::new(&task, TaskStatus::Failed,
                        Some("execution cleanup did not acknowledge cancellation; completion is unconfirmed".into())),
                },
            };
            if timed_out && completion.status != TaskStatus::Failed {
                completion.status = TaskStatus::TimedOut;
                completion.error = Some("scheduled execution deadline exceeded".into());
                completion.output = None;
            } else if handle.cancellation.is_cancelled()
                && completion.status == TaskStatus::Completed
            {
                completion.status = TaskStatus::Terminated;
                completion.error = Some("scheduled execution was cancelled".into());
                completion.output = None;
            }
            if completion.agent_id != task.agent_id
                || completion.run_id != handle.run_id()
                || matches!(completion.status, TaskStatus::Pending | TaskStatus::Running)
            {
                completion = TaskCompletion::new(
                    &task,
                    TaskStatus::Failed,
                    Some(
                        "execution service returned an invalid completion identity or status"
                            .into(),
                    ),
                );
            }
            handle.finish(completion);
        });
        Ok(())
    }

    pub async fn terminate_task(&self, agent_id: AgentId) -> Result<(), SchedulerError> {
        let handles: Vec<_> = self
            .tasks
            .read()
            .values()
            .filter(|h| h.agent_id == agent_id && h.completion().is_none())
            .cloned()
            .collect();
        for handle in &handles {
            handle.cancel();
        }
        for handle in handles {
            let result =
                tokio::time::timeout(CLEANUP_TIMEOUT + Duration::from_secs(1), handle.wait())
                    .await
                    .map_err(|_| failure(agent_id, "task cleanup timed out"))?;
            if result.status == TaskStatus::Failed {
                return Err(failure(
                    agent_id,
                    result.error.unwrap_or_else(|| "task cleanup failed".into()),
                ));
            }
        }
        Ok(())
    }

    pub fn active_count(&self) -> usize {
        self.tasks
            .read()
            .values()
            .filter(|h| h.completion().is_none())
            .count()
    }

    pub fn latest_completion(&self, agent_id: AgentId) -> Option<TaskCompletion> {
        self.tasks
            .read()
            .values()
            .filter(|h| h.agent_id == agent_id)
            .max_by_key(|h| h.created)
            .and_then(TaskHandle::completion)
    }

    pub async fn check_task_health(&self, agent_id: AgentId) -> Result<TaskHealth, SchedulerError> {
        self.tasks
            .read()
            .values()
            .filter(|h| h.agent_id == agent_id)
            .max_by_key(|h| (h.completion().is_none(), h.created))
            .map(TaskHandle::get_health)
            .ok_or(SchedulerError::AgentNotFound { agent_id })
    }

    pub async fn get_task_statistics(&self) -> TaskStatistics {
        let tasks = self.tasks.read();
        let count = tasks.len();
        let health: Vec<_> = tasks.values().map(TaskHandle::get_health).collect();
        TaskStatistics {
            total_tasks: count,
            healthy_tasks: health.iter().filter(|h| h.is_healthy).count(),
            average_uptime: if count == 0 {
                Duration::ZERO
            } else {
                health.iter().map(|h| h.uptime).sum::<Duration>() / count as u32
            },
            // Container measurements are not exposed by the execution service.
            total_memory_usage: 0,
        }
    }
}

impl Drop for TaskManager {
    fn drop(&mut self) {
        for handle in self.tasks.read().values() {
            if handle.completion().is_none() {
                handle.cancel();
            }
        }
    }
}

fn failure(agent_id: AgentId, reason: impl Into<String>) -> SchedulerError {
    SchedulerError::SchedulingFailed {
        agent_id,
        reason: reason.into().into(),
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskAudit {
    pub path: std::path::PathBuf,
    pub public_key: String,
}

/// An outcome is published after execution and cleanup, never at queue admission.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskCompletion {
    pub run_id: uuid::Uuid,
    pub agent_id: AgentId,
    pub status: TaskStatus,
    pub output: Option<String>,
    pub error: Option<String>,
    pub duration: Duration,
    pub audit: Option<TaskAudit>,
    #[serde(default)]
    pub total_usage: Option<crate::reasoning::inference::Usage>,
    #[serde(default)]
    pub budget: Option<crate::reasoning::budget::BudgetSnapshot>,
}
impl TaskCompletion {
    pub fn new(task: &super::ScheduledTask, status: TaskStatus, error: Option<String>) -> Self {
        Self {
            run_id: task.handle.run_id(),
            agent_id: task.agent_id,
            status,
            output: None,
            error,
            duration: task.handle.created.elapsed(),
            audit: None,
            total_usage: None,
            budget: None,
        }
    }
}

#[derive(Debug, Clone)]
struct TaskState {
    status: TaskStatus,
    updated: SystemTime,
    completion: Option<TaskCompletion>,
}

/// One run's identity and result remain distinct from every other run of an agent.
#[derive(Debug, Clone)]
pub struct TaskHandle {
    agent_id: AgentId,
    run_id: uuid::Uuid,
    created: Instant,
    state: watch::Sender<TaskState>,
    cancellation: CancellationToken,
}
impl TaskHandle {
    pub(crate) fn new(agent_id: AgentId) -> Self {
        Self::with_run_id(agent_id, uuid::Uuid::new_v4())
    }
    pub(crate) fn with_run_id(agent_id: AgentId, run_id: uuid::Uuid) -> Self {
        let (state, _) = watch::channel(TaskState {
            status: TaskStatus::Pending,
            updated: SystemTime::now(),
            completion: None,
        });
        Self {
            agent_id,
            run_id,
            created: Instant::now(),
            state,
            cancellation: CancellationToken::new(),
        }
    }
    pub(crate) fn elapsed(&self) -> Duration {
        self.created.elapsed()
    }
    pub fn agent_id(&self) -> AgentId {
        self.agent_id
    }
    pub fn run_id(&self) -> uuid::Uuid {
        self.run_id
    }
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }
    #[cfg(unix)]
    pub(crate) async fn cancelled(&self) {
        self.cancellation.cancelled().await;
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
    pub fn completion(&self) -> Option<TaskCompletion> {
        self.state.borrow().completion.clone()
    }
    fn mark_running(&self) -> bool {
        let mut running = false;
        self.state.send_modify(|state| {
            if state.completion.is_none() && !self.is_cancelled() {
                state.status = TaskStatus::Running;
                state.updated = SystemTime::now();
                running = true;
            }
        });
        running
    }
    pub(crate) fn finish(&self, mut completion: TaskCompletion) {
        self.state.send_modify(|state| {
            if state.completion.is_none() {
                completion.duration = self.created.elapsed();
                state.status = completion.status.clone();
                state.updated = SystemTime::now();
                state.completion = Some(completion);
            }
        });
    }
    pub async fn wait(&self) -> TaskCompletion {
        let mut state = self.state.subscribe();
        loop {
            if let Some(completion) = state.borrow().completion.clone() {
                return completion;
            }
            // This handle owns a sender, so it cannot close during this wait.
            let _ = state.changed().await;
        }
    }
    pub fn get_health(&self) -> TaskHealth {
        let state = self.state.borrow();
        TaskHealth {
            agent_id: self.agent_id,
            status: state.status.clone(),
            uptime: state
                .completion
                .as_ref()
                .map_or_else(|| self.created.elapsed(), |c| c.duration),
            is_healthy: matches!(
                state.status,
                TaskStatus::Pending | TaskStatus::Running | TaskStatus::Completed
            ),
            memory_usage: 0,
            cpu_usage: 0.0,
            last_activity: state.updated,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    /// No verified durable result exists; repeating the invocation is refused.
    Unresolved,
    TimedOut,
    Terminated,
}
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
#[derive(Debug, Clone, serde::Serialize)]
pub struct TaskStatistics {
    pub total_tasks: usize,
    pub healthy_tasks: usize,
    pub average_uptime: Duration,
    pub total_memory_usage: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct ControlledExecutor;
    #[async_trait]
    impl ScheduledAgentExecutor for ControlledExecutor {
        async fn execute(
            &self,
            task: &super::super::ScheduledTask,
            _: Duration,
            cancellation: CancellationToken,
        ) -> TaskCompletion {
            if task.input == serde_json::json!("wait") {
                cancellation.cancelled().await;
                TaskCompletion::new(
                    task,
                    TaskStatus::Terminated,
                    Some("cancelled fixture".into()),
                )
            } else {
                let mut result = TaskCompletion::new(task, TaskStatus::Completed, None);
                result.output = Some(task.input.to_string());
                result
            }
        }
    }

    fn task(input: &str) -> super::super::ScheduledTask {
        let config = AgentConfig {
            id: AgentId::new(),
            name: "fixture".into(),
            dsl_source: "agent fixture() {}".into(),
            execution_mode: ExecutionMode::Ephemeral,
            security_tier: SecurityTier::Tier1,
            resource_limits: ResourceLimits::default(),
            capabilities: vec![],
            policies: vec![],
            metadata: HashMap::new(),
            priority: Priority::Normal,
        };
        let mut task = super::super::ScheduledTask::new(config);
        task.input = serde_json::json!(input);
        task
    }

    #[tokio::test]
    async fn blocked_execution_does_not_block_another_run_or_cancellation() {
        let manager =
            TaskManager::with_executor(Duration::from_secs(20), Arc::new(ControlledExecutor));
        let slow = task("wait");
        let fast = task("payload");
        manager.start_task(slow.clone()).await.unwrap();
        manager.start_task(fast.clone()).await.unwrap();
        let completed = tokio::time::timeout(Duration::from_secs(1), fast.handle.wait())
            .await
            .unwrap();
        assert_eq!(completed.status, TaskStatus::Completed);
        assert_eq!(completed.output.as_deref(), Some("\"payload\""));
        manager.terminate_task(slow.agent_id).await.unwrap();
        assert_eq!(slow.handle.wait().await.status, TaskStatus::Terminated);
        assert_eq!(manager.active_count(), 0);
    }

    #[tokio::test]
    async fn each_run_of_one_principal_retains_its_own_result() {
        let manager =
            TaskManager::with_executor(Duration::from_secs(5), Arc::new(ControlledExecutor));
        let first = task("first");
        let mut second = super::super::ScheduledTask::new(first.config.clone());
        second.input = serde_json::json!("second");
        manager.start_task(first.clone()).await.unwrap();
        manager.start_task(second.clone()).await.unwrap();
        let a = first.handle.wait().await;
        let b = second.handle.wait().await;
        assert_eq!(a.agent_id, b.agent_id);
        assert_ne!(a.run_id, b.run_id);
        assert_ne!(a.output, b.output);
    }

    #[tokio::test]
    async fn deadline_requests_cleanup_and_publishes_timeout() {
        let manager =
            TaskManager::with_executor(Duration::from_millis(20), Arc::new(ControlledExecutor));
        let task = task("wait");
        manager.start_task(task.clone()).await.unwrap();
        let completed = task.handle.wait().await;
        assert_eq!(completed.status, TaskStatus::TimedOut);
        assert!(completed.output.is_none());
    }

    #[tokio::test]
    async fn invalid_default_execution_never_claims_success() {
        let manager = TaskManager::new(Duration::from_secs(10));
        let mut task = task("input");
        task.config.dsl_source = "not a valid agent".into();
        manager.start_task(task.clone()).await.unwrap();
        let completed = task.handle.wait().await;
        assert_eq!(completed.status, TaskStatus::Failed);
        assert!(completed.output.is_none());
    }
    struct PanickingFixture;
    #[async_trait]
    impl ScheduledAgentExecutor for PanickingFixture {
        async fn execute(
            &self,
            _: &super::super::ScheduledTask,
            _: Duration,
            _: CancellationToken,
        ) -> TaskCompletion {
            panic!("fixture backend panicked")
        }
    }
    #[tokio::test]
    async fn backend_panic_publishes_failure_instead_of_hanging_a_run() {
        let manager =
            TaskManager::with_executor(Duration::from_secs(2), Arc::new(PanickingFixture));
        let task = task("input");
        manager.start_task(task.clone()).await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(1), task.handle.wait())
            .await
            .unwrap();
        assert_eq!(result.status, TaskStatus::Failed);
        assert!(result.error.unwrap().contains("cleanup is unconfirmed"));
        assert!(result.output.is_none());
    }
}
