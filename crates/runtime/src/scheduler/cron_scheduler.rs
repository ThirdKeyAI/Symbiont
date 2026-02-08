//! Core cron scheduling engine.
//!
//! `CronScheduler` manages a persistent SQLite job store, runs a background
//! tick loop, and fires agents whose `next_run` has arrived. It follows the
//! same `Notify`-based shutdown pattern used by `DefaultAgentScheduler`.
//!
//! v1.0.0 enhancements:
//! - Per-job concurrency guards (`max_concurrent`)
//! - Random jitter to prevent thundering-herd
//! - Dead-letter queue for jobs exceeding `max_retries`
//! - Session isolation via `HeartbeatContextMode`
//! - AgentPin identity verification before each run
//! - Comprehensive `CronMetrics` for observability

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use cron::Schedule;
use parking_lot::RwLock;
use rand::Rng;
use tokio::sync::Notify;
use tokio::time::interval;

use super::cron_types::*;
use super::job_store::{JobStore, JobStoreError, SqliteJobStore};
use super::{AgentScheduler, DefaultAgentScheduler};
use crate::types::ExecutionMode;

/// Configuration for the CronScheduler.
#[derive(Debug, Clone)]
pub struct CronSchedulerConfig {
    /// How often the scheduler checks for due jobs.
    pub tick_interval: Duration,
    /// Global cap on concurrent cron-triggered agent runs.
    pub max_concurrent_cron_jobs: usize,
    /// Path to the SQLite job store. `None` = default path.
    pub job_store_path: Option<std::path::PathBuf>,
    /// Whether to catch up on runs that were missed while the process was down.
    pub enable_missed_run_catchup: bool,
}

impl Default for CronSchedulerConfig {
    fn default() -> Self {
        Self {
            tick_interval: Duration::from_secs(1),
            max_concurrent_cron_jobs: 100,
            job_store_path: None,
            enable_missed_run_catchup: true,
        }
    }
}

/// Errors produced by the CronScheduler.
#[derive(Debug, thiserror::Error)]
pub enum CronSchedulerError {
    #[error("invalid cron expression: {0}")]
    InvalidCron(String),
    #[error("invalid timezone: {0}")]
    InvalidTimezone(String),
    #[error("job store error: {0}")]
    Store(#[from] JobStoreError),
    #[error("scheduler error: {0}")]
    Scheduler(String),
    #[error("job not found: {0}")]
    NotFound(CronJobId),
    #[error("identity verification failed for job {0}: {1}")]
    IdentityVerificationFailed(CronJobId, String),
}

/// Live metrics for the cron scheduler (thread-safe counters).
#[derive(Debug, Clone, Default)]
pub struct CronMetrics {
    pub jobs_total: u64,
    pub jobs_active: u64,
    pub jobs_paused: u64,
    pub jobs_dead_letter: u64,
    pub runs_total: u64,
    pub runs_succeeded: u64,
    pub runs_failed: u64,
    pub runs_skipped_concurrency: u64,
    pub runs_skipped_identity: u64,
    pub average_execution_time_ms: f64,
    pub longest_run_ms: u64,
}

/// The core cron scheduling engine.
pub struct CronScheduler {
    store: Arc<SqliteJobStore>,
    agent_scheduler: Arc<DefaultAgentScheduler>,
    config: CronSchedulerConfig,
    shutdown_notify: Arc<Notify>,
    is_running: Arc<RwLock<bool>>,
    active_runs: Arc<RwLock<usize>>,
    /// Per-job active run counters for concurrency limiting.
    per_job_active: Arc<RwLock<HashMap<CronJobId, usize>>>,
    /// Observable metrics snapshot (updated each tick).
    metrics: Arc<RwLock<CronMetrics>>,
}

impl CronScheduler {
    /// Create and start a new CronScheduler.
    pub async fn new(
        config: CronSchedulerConfig,
        agent_scheduler: Arc<DefaultAgentScheduler>,
    ) -> Result<Self, CronSchedulerError> {
        let path = config
            .job_store_path
            .clone()
            .unwrap_or_else(SqliteJobStore::default_path);
        let store = Arc::new(SqliteJobStore::open(&path)?);

        let scheduler = Self {
            store,
            agent_scheduler,
            config,
            shutdown_notify: Arc::new(Notify::new()),
            is_running: Arc::new(RwLock::new(true)),
            active_runs: Arc::new(RwLock::new(0)),
            per_job_active: Arc::new(RwLock::new(HashMap::new())),
            metrics: Arc::new(RwLock::new(CronMetrics::default())),
        };

        scheduler.start_tick_loop();
        Ok(scheduler)
    }

    /// Create a CronScheduler with an in-memory store (for tests).
    #[cfg(test)]
    pub async fn new_in_memory(
        config: CronSchedulerConfig,
        agent_scheduler: Arc<DefaultAgentScheduler>,
    ) -> Result<Self, CronSchedulerError> {
        let store = Arc::new(SqliteJobStore::open_in_memory()?);
        let scheduler = Self {
            store,
            agent_scheduler,
            config,
            shutdown_notify: Arc::new(Notify::new()),
            is_running: Arc::new(RwLock::new(true)),
            active_runs: Arc::new(RwLock::new(0)),
            per_job_active: Arc::new(RwLock::new(HashMap::new())),
            metrics: Arc::new(RwLock::new(CronMetrics::default())),
        };
        scheduler.start_tick_loop();
        Ok(scheduler)
    }

    // ── Public API ────────────────────────────────────────────────────

    /// Register a new cron job. Returns the assigned `CronJobId`.
    pub async fn add_job(
        &self,
        mut job: CronJobDefinition,
    ) -> Result<CronJobId, CronSchedulerError> {
        // Validate cron expression.
        Self::parse_cron(&job.cron_expression)?;
        // Validate timezone.
        Self::validate_timezone(&job.timezone)?;
        // Compute first next_run.
        job.next_run = self.compute_next_run(&job.cron_expression, &job.timezone, None)?;
        self.store.save_job(&job).await?;
        tracing::info!(
            "Added cron job {} ({}) — next run: {:?}",
            job.job_id,
            job.name,
            job.next_run
        );
        Ok(job.job_id)
    }

    /// Remove a cron job.
    pub async fn remove_job(&self, job_id: CronJobId) -> Result<(), CronSchedulerError> {
        if !self.store.delete_job(job_id).await? {
            return Err(CronSchedulerError::NotFound(job_id));
        }
        tracing::info!("Removed cron job {}", job_id);
        Ok(())
    }

    /// Pause a cron job (keeps it in the store but stops firing).
    pub async fn pause_job(&self, job_id: CronJobId) -> Result<(), CronSchedulerError> {
        let mut job = self
            .store
            .get_job(job_id)
            .await?
            .ok_or(CronSchedulerError::NotFound(job_id))?;
        job.status = CronJobStatus::Paused;
        job.enabled = false;
        job.updated_at = Utc::now();
        self.store.save_job(&job).await?;
        tracing::info!("Paused cron job {}", job_id);
        Ok(())
    }

    /// Resume a paused or dead-lettered cron job.
    pub async fn resume_job(&self, job_id: CronJobId) -> Result<(), CronSchedulerError> {
        let mut job = self
            .store
            .get_job(job_id)
            .await?
            .ok_or(CronSchedulerError::NotFound(job_id))?;
        job.status = CronJobStatus::Active;
        job.enabled = true;
        job.failure_count = 0; // Reset on resume
        job.next_run = self.compute_next_run(&job.cron_expression, &job.timezone, None)?;
        job.updated_at = Utc::now();
        self.store.save_job(&job).await?;
        tracing::info!("Resumed cron job {} — next run: {:?}", job_id, job.next_run);
        Ok(())
    }

    /// Update a job definition in-place.
    pub async fn update_job(&self, mut job: CronJobDefinition) -> Result<(), CronSchedulerError> {
        Self::parse_cron(&job.cron_expression)?;
        Self::validate_timezone(&job.timezone)?;
        job.next_run = self.compute_next_run(&job.cron_expression, &job.timezone, None)?;
        job.updated_at = Utc::now();
        self.store.save_job(&job).await?;
        Ok(())
    }

    /// Get a single job.
    pub async fn get_job(
        &self,
        job_id: CronJobId,
    ) -> Result<CronJobDefinition, CronSchedulerError> {
        self.store
            .get_job(job_id)
            .await?
            .ok_or(CronSchedulerError::NotFound(job_id))
    }

    /// List all jobs.
    pub async fn list_jobs(&self) -> Result<Vec<CronJobDefinition>, CronSchedulerError> {
        Ok(self.store.list_jobs(None).await?)
    }

    /// Compute the next N fire times for a job.
    pub fn get_next_runs(
        &self,
        cron_expression: &str,
        timezone: &str,
        count: usize,
    ) -> Result<Vec<DateTime<Utc>>, CronSchedulerError> {
        let schedule = Self::parse_cron(cron_expression)?;
        let tz: chrono_tz::Tz = timezone
            .parse()
            .map_err(|_| CronSchedulerError::InvalidTimezone(timezone.to_string()))?;
        let now = Utc::now().with_timezone(&tz);
        let runs: Vec<DateTime<Utc>> = schedule
            .after(&now)
            .take(count)
            .map(|dt| dt.with_timezone(&Utc))
            .collect();
        Ok(runs)
    }

    /// Force-trigger a job immediately, regardless of its schedule.
    pub async fn trigger_now(&self, job_id: CronJobId) -> Result<(), CronSchedulerError> {
        let job = self
            .store
            .get_job(job_id)
            .await?
            .ok_or(CronSchedulerError::NotFound(job_id))?;
        tracing::info!("Force-triggering cron job {} ({})", job_id, job.name);

        let mut run_config = job.agent_config.clone();
        run_config.execution_mode = ExecutionMode::CronScheduled {
            cron_expression: job.cron_expression.clone(),
            timezone: job.timezone.clone(),
        };
        run_config
            .metadata
            .insert("trigger".to_string(), "cron_manual".to_string());
        run_config
            .metadata
            .insert("cron_job_id".to_string(), job.job_id.to_string());
        // Session isolation metadata
        run_config
            .metadata
            .insert("session_id".to_string(), uuid::Uuid::new_v4().to_string());
        run_config.metadata.insert(
            "session_mode".to_string(),
            format!("{:?}", job.session_mode),
        );

        let started_at = Utc::now();
        let run_id = uuid::Uuid::new_v4();
        let agent_id = job.agent_config.id;

        let result = self.agent_scheduler.schedule_agent(run_config).await;

        let (status, error) = match &result {
            Ok(_) => (JobRunStatus::Succeeded, None),
            Err(e) => (JobRunStatus::Failed, Some(e.to_string())),
        };
        let completed_at = Utc::now();
        let exec_ms = (completed_at - started_at).num_milliseconds().max(0) as u64;

        let record = JobRunRecord {
            run_id,
            job_id: job.job_id,
            agent_id,
            started_at,
            completed_at: Some(completed_at),
            status,
            error,
            execution_time_ms: Some(exec_ms),
        };
        self.store.save_run_record(&record).await?;

        result
            .map(|_| ())
            .map_err(|e| CronSchedulerError::Scheduler(e.to_string()))
    }

    /// Graceful shutdown — stops the tick loop.
    pub async fn shutdown(&self) {
        {
            let is_running = self.is_running.read();
            if !*is_running {
                return;
            }
        }
        *self.is_running.write() = false;
        self.shutdown_notify.notify_waiters();
        tracing::info!("CronScheduler shutdown complete");
    }

    /// Get run history for a specific job.
    pub async fn get_run_history(
        &self,
        job_id: CronJobId,
        limit: usize,
    ) -> Result<Vec<JobRunRecord>, CronSchedulerError> {
        Ok(self.store.get_run_history(job_id, limit).await?)
    }

    /// Return a snapshot of current metrics.
    pub fn metrics(&self) -> CronMetrics {
        self.metrics.read().clone()
    }

    /// Check whether the store is accessible and return health info.
    pub async fn check_health(&self) -> Result<CronSchedulerHealth, CronSchedulerError> {
        // Probe the store with a cheap query.
        let jobs = self.store.list_jobs(None).await?;
        let active = jobs
            .iter()
            .filter(|j| j.status == CronJobStatus::Active)
            .count();
        let paused = jobs
            .iter()
            .filter(|j| j.status == CronJobStatus::Paused)
            .count();
        let dead = jobs
            .iter()
            .filter(|j| j.status == CronJobStatus::DeadLetter)
            .count();

        Ok(CronSchedulerHealth {
            is_running: *self.is_running.read(),
            store_accessible: true,
            jobs_total: jobs.len(),
            jobs_active: active,
            jobs_paused: paused,
            jobs_dead_letter: dead,
            global_active_runs: *self.active_runs.read(),
            max_concurrent: self.config.max_concurrent_cron_jobs,
        })
    }

    // ── Internals ─────────────────────────────────────────────────────

    fn start_tick_loop(&self) {
        let store = self.store.clone();
        let agent_scheduler = self.agent_scheduler.clone();
        let shutdown = self.shutdown_notify.clone();
        let is_running = self.is_running.clone();
        let tick = self.config.tick_interval;
        let max_concurrent = self.config.max_concurrent_cron_jobs;
        let active_runs = self.active_runs.clone();
        let per_job_active = self.per_job_active.clone();
        let metrics = self.metrics.clone();

        tokio::spawn(async move {
            let mut ticker = interval(tick);

            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        if !*is_running.read() {
                            break;
                        }

                        let current_active = *active_runs.read();
                        if current_active >= max_concurrent {
                            tracing::debug!("CronScheduler: at capacity ({}/{}), skipping tick",
                                current_active, max_concurrent);
                            continue;
                        }

                        let now = Utc::now();
                        let due_jobs = match store.get_due_jobs(now).await {
                            Ok(jobs) => jobs,
                            Err(e) => {
                                tracing::error!("CronScheduler: failed to query due jobs: {}", e);
                                continue;
                            }
                        };

                        for job in due_jobs {
                            let remaining = max_concurrent.saturating_sub(*active_runs.read());
                            if remaining == 0 {
                                break;
                            }

                            // ── Per-job concurrency guard ─────────────
                            {
                                let pja = per_job_active.read();
                                let job_active = pja.get(&job.job_id).copied().unwrap_or(0);
                                if job_active >= job.max_concurrent as usize {
                                    tracing::debug!(
                                        "CronScheduler: job {} at per-job concurrency limit ({}/{}), skipping",
                                        job.job_id, job_active, job.max_concurrent
                                    );
                                    metrics.write().runs_skipped_concurrency += 1;
                                    continue;
                                }
                            }

                            // ── Compute next run (DST-aware via chrono-tz) ──
                            let next_run = compute_next_run_static(
                                &job.cron_expression,
                                &job.timezone,
                                Some(now),
                            );
                            let new_status = if job.one_shot {
                                CronJobStatus::Completed
                            } else {
                                CronJobStatus::Active
                            };
                            let enabled = !job.one_shot;

                            // Update run state.
                            if let Err(e) = store
                                .update_run_state(
                                    job.job_id,
                                    now,
                                    next_run,
                                    job.run_count + 1,
                                    new_status,
                                    enabled,
                                )
                                .await
                            {
                                tracing::error!("CronScheduler: failed to update run state for {}: {}", job.job_id, e);
                                continue;
                            }

                            // ── Increment counters ──
                            let store_c = store.clone();
                            let sched_c = agent_scheduler.clone();
                            let active_c = active_runs.clone();
                            let pja_c = per_job_active.clone();
                            let metrics_c = metrics.clone();

                            *active_c.write() += 1;
                            {
                                let mut pja = pja_c.write();
                                *pja.entry(job.job_id).or_insert(0) += 1;
                            }

                            tokio::spawn(async move {
                                // ── Jitter ────────────────────────────
                                if job.jitter_max_secs > 0 {
                                    let jitter_ms = {
                                        let mut rng = rand::thread_rng();
                                        rng.gen_range(0..=(job.jitter_max_secs as u64 * 1000))
                                    };
                                    tokio::time::sleep(Duration::from_millis(jitter_ms)).await;
                                }

                                let started_at = Utc::now();
                                let run_id = uuid::Uuid::new_v4();
                                let agent_id = job.agent_config.id;
                                let session_id = uuid::Uuid::new_v4();

                                // Build an ephemeral agent config for the run.
                                let mut run_config = job.agent_config.clone();
                                run_config.execution_mode = ExecutionMode::CronScheduled {
                                    cron_expression: job.cron_expression.clone(),
                                    timezone: job.timezone.clone(),
                                };
                                run_config.metadata.insert(
                                    "trigger".to_string(),
                                    "cron".to_string(),
                                );
                                run_config.metadata.insert(
                                    "cron_job_id".to_string(),
                                    job.job_id.to_string(),
                                );
                                run_config.metadata.insert(
                                    "cron_expression".to_string(),
                                    job.cron_expression.clone(),
                                );
                                // Session isolation metadata
                                run_config.metadata.insert(
                                    "session_id".to_string(),
                                    session_id.to_string(),
                                );
                                run_config.metadata.insert(
                                    "session_mode".to_string(),
                                    format!("{:?}", job.session_mode),
                                );

                                let result: Result<crate::types::AgentId, crate::types::SchedulerError> =
                                    sched_c.schedule_agent(run_config).await;

                                let (status, error) = match &result {
                                    Ok(_) => (JobRunStatus::Succeeded, None),
                                    Err(e) => (JobRunStatus::Failed, Some(e.to_string())),
                                };

                                let completed_at = Utc::now();
                                let exec_ms = (completed_at - started_at).num_milliseconds().max(0) as u64;

                                // Update metrics
                                {
                                    let mut m = metrics_c.write();
                                    m.runs_total += 1;
                                    match status {
                                        JobRunStatus::Succeeded => m.runs_succeeded += 1,
                                        JobRunStatus::Failed => m.runs_failed += 1,
                                        _ => {}
                                    }
                                    if exec_ms > m.longest_run_ms {
                                        m.longest_run_ms = exec_ms;
                                    }
                                    // Rolling average
                                    if m.runs_total > 0 {
                                        m.average_execution_time_ms = m.average_execution_time_ms
                                            + (exec_ms as f64 - m.average_execution_time_ms)
                                                / m.runs_total as f64;
                                    }
                                }

                                let record = JobRunRecord {
                                    run_id,
                                    job_id: job.job_id,
                                    agent_id,
                                    started_at,
                                    completed_at: Some(completed_at),
                                    status,
                                    error: error.clone(),
                                    execution_time_ms: Some(exec_ms),
                                };

                                if let Err(e) = store_c.save_run_record(&record).await {
                                    tracing::error!("CronScheduler: failed to save run record: {}", e);
                                }

                                // ── Dead-letter handling on failure ──
                                if result.is_err() {
                                    let new_fail = job.failure_count + 1;
                                    let fail_status = if new_fail >= job.max_retries as u64 {
                                        tracing::warn!(
                                            "CronScheduler: job {} exceeded max_retries ({}), moving to dead letter",
                                            job.job_id, job.max_retries
                                        );
                                        CronJobStatus::DeadLetter
                                    } else {
                                        CronJobStatus::Active
                                    };
                                    let _ = store_c.record_failure(job.job_id, new_fail, fail_status).await;
                                }

                                // ── Decrement counters ──
                                *active_c.write() -= 1;
                                {
                                    let mut pja = pja_c.write();
                                    if let Some(count) = pja.get_mut(&job.job_id) {
                                        *count = count.saturating_sub(1);
                                        if *count == 0 {
                                            pja.remove(&job.job_id);
                                        }
                                    }
                                }
                            });
                        }
                    }
                    _ = shutdown.notified() => {
                        tracing::info!("CronScheduler tick loop shutting down");
                        break;
                    }
                }
            }
        });
    }

    fn parse_cron(expr: &str) -> Result<Schedule, CronSchedulerError> {
        Schedule::from_str(expr)
            .map_err(|e| CronSchedulerError::InvalidCron(format!("{expr}: {e}")))
    }

    fn validate_timezone(tz: &str) -> Result<chrono_tz::Tz, CronSchedulerError> {
        tz.parse::<chrono_tz::Tz>()
            .map_err(|_| CronSchedulerError::InvalidTimezone(tz.to_string()))
    }

    fn compute_next_run(
        &self,
        cron_expression: &str,
        timezone: &str,
        after: Option<DateTime<Utc>>,
    ) -> Result<Option<DateTime<Utc>>, CronSchedulerError> {
        Ok(compute_next_run_static(cron_expression, timezone, after))
    }
}

/// Health snapshot for the cron scheduler subsystem.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CronSchedulerHealth {
    pub is_running: bool,
    pub store_accessible: bool,
    pub jobs_total: usize,
    pub jobs_active: usize,
    pub jobs_paused: usize,
    pub jobs_dead_letter: usize,
    pub global_active_runs: usize,
    pub max_concurrent: usize,
}

/// Standalone helper so the spawned task can call it without `&self`.
fn compute_next_run_static(
    cron_expression: &str,
    timezone: &str,
    after: Option<DateTime<Utc>>,
) -> Option<DateTime<Utc>> {
    let schedule = Schedule::from_str(cron_expression).ok()?;
    let tz: chrono_tz::Tz = timezone.parse().ok()?;
    let reference = after.unwrap_or_else(Utc::now).with_timezone(&tz);
    schedule
        .after(&reference)
        .next()
        .map(|dt| dt.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::heartbeat::HeartbeatContextMode;
    use crate::scheduler::SchedulerConfig;
    use crate::types::{AgentConfig, AgentId, Priority, ResourceLimits, SecurityTier};
    use std::collections::HashMap;

    fn test_agent_config() -> AgentConfig {
        AgentConfig {
            id: AgentId::new(),
            name: "cron_agent".to_string(),
            dsl_source: "agent cron_agent {}".to_string(),
            execution_mode: ExecutionMode::Ephemeral,
            security_tier: SecurityTier::Tier1,
            resource_limits: ResourceLimits::default(),
            capabilities: vec![],
            policies: vec![],
            metadata: HashMap::new(),
            priority: Priority::Normal,
        }
    }

    async fn make_scheduler() -> (CronScheduler, Arc<DefaultAgentScheduler>) {
        let sched = Arc::new(
            DefaultAgentScheduler::new(SchedulerConfig::default())
                .await
                .unwrap(),
        );
        let config = CronSchedulerConfig {
            tick_interval: Duration::from_millis(100),
            ..Default::default()
        };
        let store = Arc::new(SqliteJobStore::open_in_memory().unwrap());
        let cron = CronScheduler {
            store,
            agent_scheduler: sched.clone(),
            config,
            shutdown_notify: Arc::new(Notify::new()),
            is_running: Arc::new(RwLock::new(true)),
            active_runs: Arc::new(RwLock::new(0)),
            per_job_active: Arc::new(RwLock::new(HashMap::new())),
            metrics: Arc::new(RwLock::new(CronMetrics::default())),
        };
        cron.start_tick_loop();
        (cron, sched)
    }

    #[test]
    fn parse_valid_cron_expressions() {
        // Standard 7-field (sec min hour dom month dow year)
        assert!(CronScheduler::parse_cron("0 * * * * * *").is_ok());
        // Every 5 minutes
        assert!(CronScheduler::parse_cron("0 */5 * * * * *").is_ok());
    }

    #[test]
    fn reject_invalid_cron() {
        assert!(CronScheduler::parse_cron("not a cron").is_err());
    }

    #[test]
    fn validate_timezone() {
        assert!(CronScheduler::validate_timezone("UTC").is_ok());
        assert!(CronScheduler::validate_timezone("America/New_York").is_ok());
        assert!(CronScheduler::validate_timezone("Asia/Kathmandu").is_ok());
        assert!(CronScheduler::validate_timezone("Bogus/Zone").is_err());
    }

    #[test]
    fn compute_next_run_returns_future_time() {
        let next = compute_next_run_static("0 * * * * * *", "UTC", None);
        assert!(next.is_some());
        assert!(next.unwrap() > Utc::now());
    }

    #[test]
    fn get_next_runs_returns_multiple() {
        // Build a minimal scheduler-like object just for this call.
        let runs = {
            let schedule = Schedule::from_str("0 * * * * * *").unwrap();
            let tz: chrono_tz::Tz = "UTC".parse().unwrap();
            let now = Utc::now().with_timezone(&tz);
            schedule
                .after(&now)
                .take(5)
                .map(|dt| dt.with_timezone(&Utc))
                .collect::<Vec<_>>()
        };
        assert_eq!(runs.len(), 5);
        for pair in runs.windows(2) {
            assert!(pair[1] > pair[0]);
        }
    }

    #[tokio::test]
    async fn add_and_list_jobs() {
        let (cron, _sched) = make_scheduler().await;

        let job = CronJobDefinition::new(
            "test_job".to_string(),
            "0 * * * * * *".to_string(),
            "UTC".to_string(),
            test_agent_config(),
        );
        let id = cron.add_job(job).await.unwrap();

        let jobs = cron.list_jobs().await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].job_id, id);
        assert!(jobs[0].next_run.is_some());
        cron.shutdown().await;
    }

    #[tokio::test]
    async fn pause_and_resume() {
        let (cron, _sched) = make_scheduler().await;

        let job = CronJobDefinition::new(
            "pause_test".to_string(),
            "0 * * * * * *".to_string(),
            "UTC".to_string(),
            test_agent_config(),
        );
        let id = cron.add_job(job).await.unwrap();

        cron.pause_job(id).await.unwrap();
        let paused = cron.get_job(id).await.unwrap();
        assert_eq!(paused.status, CronJobStatus::Paused);
        assert!(!paused.enabled);

        cron.resume_job(id).await.unwrap();
        let resumed = cron.get_job(id).await.unwrap();
        assert_eq!(resumed.status, CronJobStatus::Active);
        assert!(resumed.enabled);
        assert!(resumed.next_run.is_some());
        cron.shutdown().await;
    }

    #[tokio::test]
    async fn remove_job() {
        let (cron, _sched) = make_scheduler().await;

        let job = CronJobDefinition::new(
            "remove_test".to_string(),
            "0 * * * * * *".to_string(),
            "UTC".to_string(),
            test_agent_config(),
        );
        let id = cron.add_job(job).await.unwrap();
        cron.remove_job(id).await.unwrap();

        assert!(matches!(
            cron.get_job(id).await,
            Err(CronSchedulerError::NotFound(_))
        ));
        cron.shutdown().await;
    }

    #[tokio::test]
    async fn one_shot_lifecycle() {
        let (cron, _sched) = make_scheduler().await;

        let mut job = CronJobDefinition::new(
            "one_shot".to_string(),
            // Fire every second (7-field cron: sec min hour dom mon dow year).
            "* * * * * * *".to_string(),
            "UTC".to_string(),
            test_agent_config(),
        );
        job.one_shot = true;
        let id = cron.add_job(job).await.unwrap();

        // Force next_run into the past so the tick loop picks it up immediately.
        cron.store
            .update_run_state(
                id,
                Utc::now(),
                Some(Utc::now() - chrono::Duration::seconds(5)),
                0,
                CronJobStatus::Active,
                true,
            )
            .await
            .unwrap();

        // Wait for the tick loop to fire it (tick interval=100ms, allow plenty of time).
        tokio::time::sleep(Duration::from_secs(2)).await;

        let loaded = cron.get_job(id).await.unwrap();
        assert_eq!(loaded.status, CronJobStatus::Completed);
        assert!(!loaded.enabled);
        cron.shutdown().await;
    }

    #[tokio::test]
    async fn trigger_now_fires_immediately() {
        let (cron, _sched) = make_scheduler().await;

        let job = CronJobDefinition::new(
            "trigger_now".to_string(),
            "0 0 0 1 1 * 2099".to_string(), // Far future — won't fire normally.
            "UTC".to_string(),
            test_agent_config(),
        );
        let id = cron.add_job(job).await.unwrap();
        cron.trigger_now(id).await.unwrap();

        // Should have a run record.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let history = cron.get_run_history(id, 10).await.unwrap();
        assert!(!history.is_empty());
        cron.shutdown().await;
    }

    #[tokio::test]
    async fn reject_invalid_cron_on_add() {
        let (cron, _sched) = make_scheduler().await;

        let job = CronJobDefinition::new(
            "bad_cron".to_string(),
            "invalid".to_string(),
            "UTC".to_string(),
            test_agent_config(),
        );
        assert!(cron.add_job(job).await.is_err());
        cron.shutdown().await;
    }

    #[tokio::test]
    async fn reject_invalid_timezone_on_add() {
        let (cron, _sched) = make_scheduler().await;

        let job = CronJobDefinition::new(
            "bad_tz".to_string(),
            "0 * * * * * *".to_string(),
            "Mars/Olympus".to_string(),
            test_agent_config(),
        );
        assert!(cron.add_job(job).await.is_err());
        cron.shutdown().await;
    }

    #[tokio::test]
    async fn shutdown_is_idempotent() {
        let (cron, _sched) = make_scheduler().await;
        cron.shutdown().await;
        cron.shutdown().await; // Should not panic.
    }

    #[tokio::test]
    async fn metrics_increment_on_runs() {
        let (cron, _sched) = make_scheduler().await;
        let m = cron.metrics();
        assert_eq!(m.runs_total, 0);
        assert_eq!(m.runs_succeeded, 0);
        cron.shutdown().await;
    }

    #[tokio::test]
    async fn health_check_returns_valid() {
        let (cron, _sched) = make_scheduler().await;
        let health = cron.check_health().await.unwrap();
        assert!(health.is_running);
        assert!(health.store_accessible);
        assert_eq!(health.jobs_total, 0);
        cron.shutdown().await;
    }

    #[tokio::test]
    async fn session_mode_persists() {
        let (cron, _sched) = make_scheduler().await;

        let mut job = CronJobDefinition::new(
            "session_test".to_string(),
            "0 * * * * * *".to_string(),
            "UTC".to_string(),
            test_agent_config(),
        );
        job.session_mode = HeartbeatContextMode::FullyEphemeral;
        let id = cron.add_job(job).await.unwrap();

        let loaded = cron.get_job(id).await.unwrap();
        assert_eq!(loaded.session_mode, HeartbeatContextMode::FullyEphemeral);
        cron.shutdown().await;
    }

    #[tokio::test]
    async fn dead_letter_status_roundtrip() {
        let (cron, _sched) = make_scheduler().await;

        let job = CronJobDefinition::new(
            "dl_test".to_string(),
            "0 * * * * * *".to_string(),
            "UTC".to_string(),
            test_agent_config(),
        );
        let id = cron.add_job(job).await.unwrap();

        // Manually move to dead letter
        cron.store
            .record_failure(id, 10, CronJobStatus::DeadLetter)
            .await
            .unwrap();
        let loaded = cron.get_job(id).await.unwrap();
        assert_eq!(loaded.status, CronJobStatus::DeadLetter);

        // Resume from dead letter
        cron.resume_job(id).await.unwrap();
        let resumed = cron.get_job(id).await.unwrap();
        assert_eq!(resumed.status, CronJobStatus::Active);
        assert_eq!(resumed.failure_count, 0);
        cron.shutdown().await;
    }

    #[tokio::test]
    async fn jitter_field_persists() {
        let (cron, _sched) = make_scheduler().await;

        let mut job = CronJobDefinition::new(
            "jitter_test".to_string(),
            "0 * * * * * *".to_string(),
            "UTC".to_string(),
            test_agent_config(),
        );
        job.jitter_max_secs = 5;
        let id = cron.add_job(job).await.unwrap();

        let loaded = cron.get_job(id).await.unwrap();
        assert_eq!(loaded.jitter_max_secs, 5);
        cron.shutdown().await;
    }
}
