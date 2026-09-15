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
use tokio::sync::Notify;
use tokio::time::interval;
use tokio_util::sync::CancellationToken;

use super::cron_types::*;
use super::job_store::{JobStore, JobStoreError, SqliteJobStore};
use super::policy_gate::{PolicyGate, ScheduleContext, SchedulePolicyDecision};
use super::task_manager::{TaskCompletion, TaskStatus};
use super::AgentScheduler;
#[cfg(test)]
use super::DefaultAgentScheduler;
#[cfg(test)]
use crate::types::ExecutionMode;

#[cfg(unix)]
#[path = "cron_runs.rs"]
mod runs;

struct RunReservation {
    active: Arc<RwLock<usize>>,
    per_job: Arc<RwLock<HashMap<CronJobId, usize>>>,
    job_id: CronJobId,
}
impl Drop for RunReservation {
    fn drop(&mut self) {
        let mut active = self.active.write();
        let mut per_job = self.per_job.write();
        *active = active.saturating_sub(1);
        if let Some(count) = per_job.get_mut(&self.job_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                per_job.remove(&self.job_id);
            }
        }
    }
}

/// Configuration for the CronScheduler.
#[derive(Debug, Clone)]
pub struct CronSchedulerConfig {
    /// How often the scheduler checks for due jobs.
    pub tick_interval: Duration,
    /// Global cap on concurrent cron-triggered agent runs.
    pub max_concurrent_cron_jobs: usize,
    /// SQLite store path. `None` uses the project's private `.symbiont/cron_jobs.db`.
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
    #[error("invalid cron expression (expected 6-field: sec min hour day month weekday): {0}")]
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
    #[error("cron job {0} refused by schedule policy: {1}")]
    PolicyDenied(CronJobId, String),
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
    pub runs_skipped_policy: u64,
    pub average_execution_time_ms: f64,
    pub longest_run_ms: u64,
}

/// The core cron scheduling engine.
#[derive(Clone)]
pub struct CronScheduler {
    store: Arc<SqliteJobStore>,
    agent_scheduler: Arc<dyn AgentScheduler + Send + Sync>,
    config: CronSchedulerConfig,
    shutdown_notify: Arc<Notify>,
    stopping: CancellationToken,
    is_running: Arc<RwLock<bool>>,
    active_runs: Arc<RwLock<usize>>,
    /// Per-job active run counters for concurrency limiting.
    per_job_active: Arc<RwLock<HashMap<CronJobId, usize>>>,
    #[cfg(unix)]
    pending_attempts: Arc<RwLock<std::collections::HashSet<uuid::Uuid>>>,
    /// Observable metrics snapshot (updated each tick).
    metrics: Arc<RwLock<CronMetrics>>,
    /// Optional AgentPin verifier. When present, every cron-triggered
    /// run verifies `job.agentpin_jwt` before invoking the agent scheduler.
    /// When the verifier is present but the job carries no JWT, the run
    /// is refused so "AgentPin enabled at the scheduler" cannot be
    /// bypassed by leaving the JWT column null.
    agentpin_verifier: Arc<RwLock<Option<Arc<dyn crate::integrations::AgentPinVerifier>>>>,
    /// Optional schedule policy gate. When present, every cron-triggered run
    /// (both `trigger_now` and the tick loop) is evaluated against it before
    /// the agent scheduler is invoked; a `Deny` or `RequiresApproval`
    /// decision refuses the run. `None` means no gate is installed and every
    /// run proceeds — the same "explicit opt-in" shape as `agentpin_verifier`,
    /// not a fail-closed default. Every cron invocation passes this gate
    /// before the governed runtime separately authorizes its tool effects.
    ///
    /// The shared lock makes later guard changes visible to timer tasks.
    /// Use `new_with_guards` to install startup authority before recovery begins.
    policy_gate: Arc<RwLock<Option<Arc<PolicyGate>>>>,
}

impl CronScheduler {
    /// Create and start a new CronScheduler.
    pub async fn new(
        config: CronSchedulerConfig,
        agent_scheduler: Arc<dyn AgentScheduler + Send + Sync>,
    ) -> Result<Self, CronSchedulerError> {
        Self::new_with_guards(config, agent_scheduler, None, None).await
    }

    /// Install execution guards before starting timer dispatch or recovery.
    pub async fn new_with_guards(
        config: CronSchedulerConfig,
        agent_scheduler: Arc<dyn AgentScheduler + Send + Sync>,
        agentpin_verifier: Option<Arc<dyn crate::integrations::AgentPinVerifier>>,
        policy_gate: Option<Arc<PolicyGate>>,
    ) -> Result<Self, CronSchedulerError> {
        #[cfg(unix)]
        let project = agent_scheduler
            .invocation_project()
            .map_err(CronSchedulerError::Scheduler)?
            .canonicalize()
            .map_err(|e| CronSchedulerError::Scheduler(e.to_string()))?;
        #[cfg(not(unix))]
        let project: std::path::PathBuf = return Err(CronSchedulerError::Scheduler(
            "protected cron execution requires Unix".into(),
        ));
        let path = config
            .job_store_path
            .clone()
            .unwrap_or_else(|| project.join(".symbiont/cron_jobs.db"));
        let store = Arc::new(SqliteJobStore::open(&path)?);
        store.bind_project(&project).await?;

        let scheduler = Self {
            store,
            agent_scheduler,
            config,
            shutdown_notify: Arc::new(Notify::new()),
            stopping: CancellationToken::new(),
            is_running: Arc::new(RwLock::new(true)),
            active_runs: Arc::new(RwLock::new(0)),
            per_job_active: Arc::new(RwLock::new(HashMap::new())),
            #[cfg(unix)]
            pending_attempts: Arc::new(RwLock::new(std::collections::HashSet::new())),
            metrics: Arc::new(RwLock::new(CronMetrics::default())),
            agentpin_verifier: Arc::new(RwLock::new(agentpin_verifier)),
            policy_gate: Arc::new(RwLock::new(policy_gate)),
        };

        scheduler.start_tick_loop();
        Ok(scheduler)
    }

    /// Install an AgentPin verifier. After this call, every cron run
    /// requires `job.agentpin_jwt` to verify successfully — jobs without
    /// a JWT are refused (see AP-3). Returns the scheduler builder-style
    /// for chaining.
    pub fn with_agentpin_verifier(
        self,
        verifier: Arc<dyn crate::integrations::AgentPinVerifier>,
    ) -> Self {
        *self.agentpin_verifier.write() = Some(verifier);
        self
    }

    /// Install a schedule policy gate. After this call, every cron-triggered
    /// run — `trigger_now` and the tick loop alike — is evaluated against it
    /// before the agent scheduler is invoked; `Deny` and `RequiresApproval`
    /// both refuse the run (see [`Self::evaluate_schedule_policy`]). Returns
    /// the scheduler builder-style for chaining, matching
    /// [`Self::with_agentpin_verifier`].
    pub fn with_policy_gate(self, gate: Arc<PolicyGate>) -> Self {
        *self.policy_gate.write() = Some(gate);
        self
    }

    /// Build the [`ScheduleContext`] for a job about to run and evaluate it
    /// against `policy_gate`, if one is installed.
    ///
    /// `None` (no gate installed) always evaluates to `Allow` — this method
    /// takes an `Option` rather than being a `&self` method so it can also be
    /// called from the tick loop's spawned task, which clones the gate out
    /// of `self` before spawning (see `start_tick_loop`) rather than
    /// capturing `&self` across an `.await`.
    ///
    /// Context fields are populated only from data the job/scheduler
    /// actually track: `consecutive_failures` and `total_runs` come straight
    /// from the job record. `system_load` is left at its default (`0.0`) —
    /// no system-level load metric is wired into the cron scheduler today,
    /// so a `SystemLoadExceeds` rule cannot fire from cron yet; that is
    /// stated here rather than fabricating a reading.
    fn evaluate_schedule_policy(
        policy_gate: Option<&PolicyGate>,
        job: &CronJobDefinition,
    ) -> SchedulePolicyDecision {
        let Some(gate) = policy_gate else {
            return SchedulePolicyDecision::Allow;
        };
        let mut extra = HashMap::new();
        extra.insert("job_id".to_string(), job.job_id.to_string());
        let context = ScheduleContext {
            consecutive_failures: job.failure_count,
            total_runs: job.run_count,
            system_load: 0.0,
            extra,
        };
        gate.evaluate(job, &context)
    }

    /// Format a human-readable refusal reason for a non-`Allow` schedule
    /// policy decision. `RequiresApproval` is treated as a refusal, not an
    /// automatic allow: the cron scheduler has no approval-queue wiring to
    /// route it to, and silently allowing would defeat the point of
    /// installing a gate in the first place.
    fn describe_policy_refusal(decision: &SchedulePolicyDecision) -> String {
        match decision {
            SchedulePolicyDecision::Allow => {
                unreachable!("describe_policy_refusal called with an Allow decision")
            }
            SchedulePolicyDecision::Deny { reason, policy_id } => {
                format!("denied by policy '{}': {}", policy_id, reason)
            }
            SchedulePolicyDecision::RequiresApproval {
                approver,
                reason,
                policy_id,
            } => format!(
                "requires approval from '{}' per policy '{}' ({}) — cron has no approval \
                 routing yet, refusing rather than auto-running",
                approver, policy_id, reason
            ),
        }
    }

    /// Verify that a job's AgentPin credential is valid before firing.
    ///
    /// Returns `Ok(())` when no verifier is configured. When a verifier
    /// IS configured, both absence of `agentpin_jwt` and verification
    /// failure produce an `IdentityVerificationFailed` error and the run
    /// is skipped (and counted in `runs_skipped_identity`).
    async fn verify_job_credential(
        &self,
        job: &CronJobDefinition,
    ) -> Result<(), CronSchedulerError> {
        let verifier = self.agentpin_verifier.read().clone();
        let Some(verifier) = verifier else {
            return Ok(());
        };
        let jwt = job.agentpin_jwt.as_deref().ok_or_else(|| {
            CronSchedulerError::IdentityVerificationFailed(
                job.job_id,
                "AgentPin verification is enabled but job has no agentpin_jwt".to_string(),
            )
        })?;
        let result = tokio::select! {
            biased;
            _ = self.stopping.cancelled() => return Err(CronSchedulerError::Scheduler("cron scheduler is shutting down".into())),
            result = tokio::time::timeout(Duration::from_secs(10), verifier.verify_credential(jwt)) => result,
        }.map_err(|_| CronSchedulerError::IdentityVerificationFailed(job.job_id, "verification timed out".into()))?
            .map_err(|e| CronSchedulerError::IdentityVerificationFailed(job.job_id, e.to_string()))?;
        if !result.valid {
            return Err(CronSchedulerError::IdentityVerificationFailed(
                job.job_id,
                result
                    .error_message
                    .unwrap_or_else(|| "invalid credential".to_string()),
            ));
        }
        // The job's agent_config.id must be covered by the JWT's subject.
        // This prevents a JWT for one agent from being reused to run a
        // cron job that claims a different agent identity.
        let expected = job.agent_config.id.0.to_string();
        let sub_matches = result
            .agent_id
            .as_deref()
            .map(|sub| sub == expected)
            .unwrap_or(false);
        if !sub_matches {
            return Err(CronSchedulerError::IdentityVerificationFailed(
                job.job_id,
                format!(
                    "JWT subject {:?} does not match job agent {}",
                    result.agent_id, expected
                ),
            ));
        }
        Ok(())
    }

    /// Create a CronScheduler with an in-memory store (for tests).
    #[cfg(test)]
    pub async fn new_in_memory(
        config: CronSchedulerConfig,
        agent_scheduler: Arc<dyn AgentScheduler + Send + Sync>,
    ) -> Result<Self, CronSchedulerError> {
        let store = Arc::new(SqliteJobStore::open_in_memory()?);
        let scheduler = Self {
            store,
            agent_scheduler,
            config,
            shutdown_notify: Arc::new(Notify::new()),
            stopping: CancellationToken::new(),
            is_running: Arc::new(RwLock::new(true)),
            active_runs: Arc::new(RwLock::new(0)),
            per_job_active: Arc::new(RwLock::new(HashMap::new())),
            #[cfg(unix)]
            pending_attempts: Arc::new(RwLock::new(std::collections::HashSet::new())),
            metrics: Arc::new(RwLock::new(CronMetrics::default())),
            agentpin_verifier: Arc::new(RwLock::new(None)),
            policy_gate: Arc::new(RwLock::new(None)),
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

    /// Register a source schedule once. Restarts preserve its identity, clock,
    /// terminal state and history; changed definitions require explicit review.
    #[cfg(unix)]
    pub async fn add_source_job(
        &self,
        source_key: &str,
        mut job: CronJobDefinition,
    ) -> Result<CronJobId, CronSchedulerError> {
        use super::{invocations::InvocationIdentity, occurrences::Occurrence};
        use crate::reasoning::prepared::digest_json;
        if source_key.is_empty() || source_key.len() > 4096 {
            return Err(CronSchedulerError::Scheduler(
                "invalid schedule source identity".into(),
            ));
        }
        let hash = digest_json(&serde_json::json!([
            "dsl-schedule:v1",
            source_key,
            job.name
        ]))
        .map_err(CronSchedulerError::Scheduler)?;
        let bytes = hex::decode(hash.trim_start_matches("sha256:"))
            .map_err(|e| CronSchedulerError::Scheduler(e.to_string()))?;
        let id = uuid::Uuid::from_bytes(bytes[..16].try_into().expect("SHA-256 prefix"));
        job.job_id = CronJobId::from_uuid(id);
        job.agent_config.id = crate::types::AgentId(id);
        Self::parse_cron(&job.cron_expression)?;
        Self::validate_timezone(&job.timezone)?;
        job.next_run = self.compute_next_run(&job.cron_expression, &job.timezone, None)?;
        self.store.persist_job(&job, false).await?;
        let saved = self
            .store
            .get_job(job.job_id)
            .await?
            .ok_or(CronSchedulerError::NotFound(job.job_id))?;
        let identity = InvocationIdentity {
            id,
            context: serde_json::json!({"source":source_key}),
        };
        if Occurrence::manual(job.clone(), identity.clone())
            .fingerprint()
            .map_err(CronSchedulerError::Scheduler)?
            != Occurrence::manual(saved, identity)
                .fingerprint()
                .map_err(CronSchedulerError::Scheduler)?
        {
            return Err(CronSchedulerError::Scheduler("persisted source schedule differs; inspect and update its stored definition explicitly".into()));
        }
        Ok(job.job_id)
    }

    #[cfg(not(unix))]
    pub async fn add_source_job(
        &self,
        source_key: &str,
        job: CronJobDefinition,
    ) -> Result<CronJobId, CronSchedulerError> {
        let _ = (source_key, job);
        Err(CronSchedulerError::Scheduler(
            "protected cron execution requires Unix".into(),
        ))
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
        #[cfg(unix)]
        if self.store.job_has_unresolved_occurrence(job_id).await? {
            return Err(CronSchedulerError::Scheduler(
                "unresolved occurrences require reconciliation before resume".into(),
            ));
        }
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

    /// Start a new trusted SDK trigger. Use trigger_identified for caller retries.
    pub async fn trigger_now(&self, job_id: CronJobId) -> Result<(), CronSchedulerError> {
        #[cfg(unix)]
        {
            use super::invocations::{recorded_completion, Admission, InvocationIdentity};
            use crate::reasoning::invocation::ExistingInvocation;
            let admission=self.trigger_identified(job_id,InvocationIdentity {id:uuid::Uuid::new_v4(),context:serde_json::json!({"caller":"trusted-sdk","route":"schedule.trigger"})}).await?;
            let completion = match admission {
                Admission::Queued { handle, .. } => {
                    struct CancelOnDrop(super::task_manager::TaskHandle);
                    impl Drop for CancelOnDrop {
                        fn drop(&mut self) {
                            self.0.cancel();
                        }
                    }
                    let guard = CancelOnDrop(handle);
                    guard.0.wait().await
                }
                Admission::Existing(ExistingInvocation::Recorded { audit, result }) => {
                    recorded_completion(&audit, result).map_err(CronSchedulerError::Scheduler)?
                }
                _ => {
                    return Err(CronSchedulerError::Scheduler(
                        "cron invocation is active or requires reconciliation".into(),
                    ))
                }
            };
            if completion.status == TaskStatus::Completed {
                Ok(())
            } else {
                Err(CronSchedulerError::Scheduler(
                    completion
                        .error
                        .unwrap_or_else(|| "cron invocation did not complete".into()),
                ))
            }
        }
        #[cfg(not(unix))]
        {
            let _ = job_id;
            Err(CronSchedulerError::Scheduler(
                "protected cron execution requires Unix".into(),
            ))
        }
    }

    /// Stop dispatch and cancel active cron runs, waiting for their cleanup.
    pub async fn shutdown(&self) {
        *self.is_running.write() = false;
        self.stopping.cancel();
        self.shutdown_notify.notify_waiters();
        let stopped = tokio::time::timeout(Duration::from_secs(30), async {
            while *self.active_runs.read() > 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        if stopped.is_err() {
            tracing::error!("cron execution cleanup remains unconfirmed after shutdown");
        }
    }

    /// Get persisted run history, including actual runtime outcomes.
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

    fn reserve(&self, job: &CronJobDefinition) -> Option<RunReservation> {
        let running = self.is_running.read();
        if !*running {
            return None;
        }
        let mut active = self.active_runs.write();
        let mut per_job = self.per_job_active.write();
        if *active >= self.config.max_concurrent_cron_jobs
            || per_job.get(&job.job_id).copied().unwrap_or(0) >= job.max_concurrent as usize
        {
            self.metrics.write().runs_skipped_concurrency += 1;
            return None;
        }
        *active += 1;
        *per_job.entry(job.job_id).or_default() += 1;
        Some(RunReservation {
            active: self.active_runs.clone(),
            per_job: self.per_job_active.clone(),
            job_id: job.job_id,
        })
    }

    fn start_tick_loop(&self) {
        #[cfg(unix)]
        self.start_occurrence_loop();
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
    use crate::scheduler::policy_gate::{
        SchedulePolicyCondition, SchedulePolicyEffect, SchedulePolicyRule,
    };
    use crate::scheduler::SchedulerConfig;
    use crate::types::{AgentConfig, AgentId, Priority, ResourceLimits, SecurityTier};
    use std::collections::HashMap;

    fn test_agent_config() -> AgentConfig {
        AgentConfig {
            id: AgentId::new(),
            name: "cron_agent".to_string(),
            dsl_source: "agent cron_agent() {}".to_string(),
            execution_mode: ExecutionMode::Ephemeral,
            security_tier: SecurityTier::Tier1,
            resource_limits: ResourceLimits::default(),
            capabilities: vec![],
            policies: vec![],
            metadata: HashMap::new(),
            priority: Priority::Normal,
        }
    }

    struct ScheduleProvider;
    #[async_trait::async_trait]
    impl crate::reasoning::inference::InferenceProvider for ScheduleProvider {
        async fn complete(
            &self,
            _: &crate::reasoning::conversation::Conversation,
            _: &crate::reasoning::inference::InferenceOptions,
        ) -> Result<
            crate::reasoning::inference::InferenceResponse,
            crate::reasoning::inference::InferenceError,
        > {
            use crate::reasoning::inference::*;
            Ok(InferenceResponse {
                content: "scheduled result".into(),
                tool_calls: vec![],
                finish_reason: FinishReason::Stop,
                model: "fixture".into(),
                usage: Usage {
                    prompt_tokens: 1,
                    completion_tokens: 1,
                    total_tokens: 2,
                },
            })
        }
        fn provider_name(&self) -> &str {
            "fixture"
        }
        fn default_model(&self) -> &str {
            "fixture"
        }
        fn supports_native_tools(&self) -> bool {
            true
        }
        fn supports_structured_output(&self) -> bool {
            false
        }
    }
    struct ScheduleFixture {
        _root: tempfile::TempDir,
        inner: super::super::execution::GovernedAgentExecutor,
    }
    impl ScheduleFixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let inner = super::super::execution::GovernedAgentExecutor::new(root.path())
                .unwrap()
                .with_provider(Arc::new(ScheduleProvider))
                .with_policy_gate(Arc::new(
                    crate::reasoning::policy_bridge::DefaultPolicyGate::new(),
                ));
            Self { _root: root, inner }
        }
    }
    #[async_trait::async_trait]
    impl super::super::execution::ScheduledAgentExecutor for ScheduleFixture {
        #[cfg(unix)]
        fn invocation_project(&self) -> Result<&std::path::Path, String> {
            self.inner.invocation_project()
        }
        async fn execute(
            &self,
            task: &super::super::ScheduledTask,
            budget: Duration,
            cancellation: tokio_util::sync::CancellationToken,
        ) -> TaskCompletion {
            self.inner.execute(task, budget, cancellation).await
        }
    }

    async fn make_scheduler() -> (CronScheduler, Arc<DefaultAgentScheduler>) {
        let sched = Arc::new(
            DefaultAgentScheduler::new_with_executor(
                SchedulerConfig::default(),
                None,
                Arc::new(ScheduleFixture::new()),
            )
            .await
            .unwrap(),
        );
        let config = CronSchedulerConfig {
            tick_interval: Duration::from_millis(100),
            ..Default::default()
        };
        let store = Arc::new(SqliteJobStore::open_in_memory().unwrap());
        store
            .bind_project(sched.invocation_project().unwrap())
            .await
            .unwrap();
        let cron = CronScheduler {
            store,
            agent_scheduler: sched.clone(),
            config,
            shutdown_notify: Arc::new(Notify::new()),
            stopping: CancellationToken::new(),
            is_running: Arc::new(RwLock::new(true)),
            active_runs: Arc::new(RwLock::new(0)),
            per_job_active: Arc::new(RwLock::new(HashMap::new())),
            #[cfg(unix)]
            pending_attempts: Arc::new(RwLock::new(std::collections::HashSet::new())),
            metrics: Arc::new(RwLock::new(CronMetrics::default())),
            agentpin_verifier: Arc::new(RwLock::new(None)),
            policy_gate: Arc::new(RwLock::new(None)),
        };
        cron.start_tick_loop();
        (cron, sched)
    }

    #[test]
    fn parse_valid_seven_field_cron_expressions() {
        // 7-field: sec min hour dom month dow year
        assert!(CronScheduler::parse_cron("0 * * * * * *").is_ok());
        // Every 5 minutes (7-field)
        assert!(CronScheduler::parse_cron("0 */5 * * * * *").is_ok());
        // Specific year
        assert!(CronScheduler::parse_cron("0 0 12 * * Mon 2027").is_ok());
    }

    #[test]
    fn parse_valid_six_field_cron_expressions() {
        // 6-field: sec min hour dom month dow (no year)
        assert!(CronScheduler::parse_cron("0 * * * * *").is_ok());
        // Every 5 minutes (6-field)
        assert!(CronScheduler::parse_cron("0 */5 * * * *").is_ok());
        // Every 30 seconds
        assert!(CronScheduler::parse_cron("*/30 * * * * *").is_ok());
        // Weekdays at 9 AM
        assert!(CronScheduler::parse_cron("0 0 9 * * Mon-Fri").is_ok());
        // First of month at midnight
        assert!(CronScheduler::parse_cron("0 0 0 1 * *").is_ok());
    }

    #[test]
    fn reject_invalid_cron() {
        assert!(CronScheduler::parse_cron("not a cron").is_err());
    }

    #[test]
    fn reject_five_field_unix_cron() {
        // Standard 5-field Unix cron (min hour dom month dow) is NOT supported
        // because the `cron` crate requires a leading seconds field.
        assert!(CronScheduler::parse_cron("*/5 * * * *").is_err());
        assert!(CronScheduler::parse_cron("0 12 * * Mon").is_err());
    }

    #[test]
    fn reject_empty_and_whitespace_cron() {
        assert!(CronScheduler::parse_cron("").is_err());
        assert!(CronScheduler::parse_cron("   ").is_err());
    }

    #[test]
    fn error_message_includes_format_hint() {
        let err = CronScheduler::parse_cron("bad").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("6-field"),
            "error should mention 6-field format, got: {msg}"
        );
        assert!(
            msg.contains("sec min hour"),
            "error should include field names, got: {msg}"
        );
    }

    #[test]
    fn validate_timezone() {
        assert!(CronScheduler::validate_timezone("UTC").is_ok());
        assert!(CronScheduler::validate_timezone("America/New_York").is_ok());
        assert!(CronScheduler::validate_timezone("Asia/Kathmandu").is_ok());
        assert!(CronScheduler::validate_timezone("Bogus/Zone").is_err());
    }

    #[test]
    fn compute_next_run_returns_future_time_seven_field() {
        let next = compute_next_run_static("0 * * * * * *", "UTC", None);
        assert!(next.is_some());
        assert!(next.unwrap() > Utc::now());
    }

    #[test]
    fn compute_next_run_returns_future_time_six_field() {
        let next = compute_next_run_static("0 * * * * *", "UTC", None);
        assert!(next.is_some());
        assert!(next.unwrap() > Utc::now());
    }

    #[test]
    fn compute_next_run_respects_after_parameter() {
        let reference = Utc::now() + chrono::Duration::hours(1);
        let next = compute_next_run_static("0 * * * * *", "UTC", Some(reference));
        assert!(next.is_some());
        assert!(next.unwrap() > reference);
    }

    #[test]
    fn compute_next_run_with_different_timezones() {
        let utc_next = compute_next_run_static("0 0 12 * * *", "UTC", None);
        let eastern_next = compute_next_run_static("0 0 12 * * *", "America/New_York", None);
        assert!(utc_next.is_some());
        assert!(eastern_next.is_some());
        // Same cron expression in different timezones should produce different UTC times
        // (unless we happen to be at exactly the boundary, which is astronomically unlikely).
        assert_ne!(utc_next.unwrap(), eastern_next.unwrap());
    }

    #[test]
    fn compute_next_run_returns_none_for_invalid_expression() {
        let next = compute_next_run_static("bad", "UTC", None);
        assert!(next.is_none());
    }

    #[test]
    fn compute_next_run_returns_none_for_invalid_timezone() {
        let next = compute_next_run_static("0 * * * * *", "Mars/Olympus", None);
        assert!(next.is_none());
    }

    #[test]
    fn get_next_runs_returns_multiple() {
        let runs = {
            let schedule = Schedule::from_str("0 * * * * *").unwrap();
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

    #[test]
    fn six_and_seven_field_produce_equivalent_schedules() {
        // "0 */5 * * * *" (6-field) and "0 */5 * * * * *" (7-field) should
        // produce the same next fire time.
        let now = Some(Utc::now());
        let six = compute_next_run_static("0 */5 * * * *", "UTC", now);
        let seven = compute_next_run_static("0 */5 * * * * *", "UTC", now);
        assert_eq!(six, seven);
    }

    #[tokio::test]
    async fn add_and_list_jobs() {
        let (cron, _sched) = make_scheduler().await;

        let job = CronJobDefinition::new(
            "test_job".to_string(),
            "0 * * * * *".to_string(), // 6-field: every minute
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
            "0 * * * * *".to_string(), // 6-field
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
            "0 * * * * *".to_string(), // 6-field
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
    async fn reject_five_field_cron_on_add() {
        let (cron, _sched) = make_scheduler().await;

        let job = CronJobDefinition::new(
            "five_field".to_string(),
            "*/5 * * * *".to_string(), // 5-field Unix cron — not supported
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
            "0 * * * * *".to_string(),
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

    // ── 6-field end-to-end tests ─────────────────────────────────────

    #[tokio::test]
    async fn add_job_with_six_field_cron() {
        let (cron, _sched) = make_scheduler().await;

        let job = CronJobDefinition::new(
            "six_field_job".to_string(),
            "0 */5 * * * *".to_string(), // 6-field: every 5 minutes
            "UTC".to_string(),
            test_agent_config(),
        );
        let id = cron.add_job(job).await.unwrap();

        let loaded = cron.get_job(id).await.unwrap();
        assert_eq!(loaded.cron_expression, "0 */5 * * * *");
        assert!(loaded.next_run.is_some());
        cron.shutdown().await;
    }

    #[tokio::test]
    async fn trigger_now_with_six_field_cron() {
        let (cron, _sched) = make_scheduler().await;

        let job = CronJobDefinition::new(
            "trigger_six".to_string(),
            "0 0 0 1 1 *".to_string(), // Far future — 6-field
            "UTC".to_string(),
            test_agent_config(),
        );
        let id = cron.add_job(job).await.unwrap();
        cron.trigger_now(id).await.unwrap();

        let history = cron.get_run_history(id, 10).await.unwrap();
        assert!(!history.is_empty());
        assert_eq!(history[0].status, JobRunStatus::Succeeded);
        cron.shutdown().await;
    }

    // ── update_job tests ─────────────────────────────────────────────

    #[tokio::test]
    async fn update_job_changes_cron_expression() {
        let (cron, _sched) = make_scheduler().await;

        let job = CronJobDefinition::new(
            "update_cron".to_string(),
            "0 * * * * *".to_string(),
            "UTC".to_string(),
            test_agent_config(),
        );
        let id = cron.add_job(job).await.unwrap();
        let original = cron.get_job(id).await.unwrap();

        let mut updated = original.clone();
        updated.cron_expression = "0 */10 * * * *".to_string();
        cron.update_job(updated).await.unwrap();

        let loaded = cron.get_job(id).await.unwrap();
        assert_eq!(loaded.cron_expression, "0 */10 * * * *");
        // next_run should have been recomputed
        assert!(loaded.next_run.is_some());
        assert!(loaded.updated_at > original.updated_at);
        cron.shutdown().await;
    }

    #[tokio::test]
    async fn update_job_rejects_invalid_cron() {
        let (cron, _sched) = make_scheduler().await;

        let job = CronJobDefinition::new(
            "update_bad".to_string(),
            "0 * * * * *".to_string(),
            "UTC".to_string(),
            test_agent_config(),
        );
        let id = cron.add_job(job).await.unwrap();
        let mut bad = cron.get_job(id).await.unwrap();
        bad.cron_expression = "nope".to_string();

        assert!(cron.update_job(bad).await.is_err());
        // Original should be unchanged
        let loaded = cron.get_job(id).await.unwrap();
        assert_eq!(loaded.cron_expression, "0 * * * * *");
        cron.shutdown().await;
    }

    #[tokio::test]
    async fn update_job_rejects_invalid_timezone() {
        let (cron, _sched) = make_scheduler().await;

        let job = CronJobDefinition::new(
            "update_bad_tz".to_string(),
            "0 * * * * *".to_string(),
            "UTC".to_string(),
            test_agent_config(),
        );
        let id = cron.add_job(job).await.unwrap();
        let mut bad = cron.get_job(id).await.unwrap();
        bad.timezone = "Fake/Zone".to_string();

        assert!(cron.update_job(bad).await.is_err());
        cron.shutdown().await;
    }

    #[tokio::test]
    async fn update_job_changes_timezone() {
        let (cron, _sched) = make_scheduler().await;

        let job = CronJobDefinition::new(
            "tz_update".to_string(),
            "0 0 12 * * *".to_string(),
            "UTC".to_string(),
            test_agent_config(),
        );
        let id = cron.add_job(job).await.unwrap();
        let original_next = cron.get_job(id).await.unwrap().next_run;

        let mut updated = cron.get_job(id).await.unwrap();
        updated.timezone = "America/New_York".to_string();
        cron.update_job(updated).await.unwrap();

        let loaded = cron.get_job(id).await.unwrap();
        assert_eq!(loaded.timezone, "America/New_York");
        // next_run should differ because timezone changed
        assert_ne!(loaded.next_run, original_next);
        cron.shutdown().await;
    }

    // ── get_next_runs API tests ──────────────────────────────────────

    #[tokio::test]
    async fn get_next_runs_api_six_field() {
        let (cron, _sched) = make_scheduler().await;
        let runs = cron.get_next_runs("0 * * * * *", "UTC", 3).unwrap();
        assert_eq!(runs.len(), 3);
        for pair in runs.windows(2) {
            assert!(pair[1] > pair[0]);
        }
        cron.shutdown().await;
    }

    #[tokio::test]
    async fn get_next_runs_api_rejects_bad_expression() {
        let (cron, _sched) = make_scheduler().await;
        assert!(cron.get_next_runs("bad", "UTC", 3).is_err());
        cron.shutdown().await;
    }

    #[tokio::test]
    async fn get_next_runs_api_rejects_bad_timezone() {
        let (cron, _sched) = make_scheduler().await;
        assert!(cron.get_next_runs("0 * * * * *", "Bogus/Tz", 3).is_err());
        cron.shutdown().await;
    }

    // ── Multiple jobs / timezone isolation ────────────────────────────

    #[tokio::test]
    async fn multiple_jobs_coexist() {
        let (cron, _sched) = make_scheduler().await;

        let job_a = CronJobDefinition::new(
            "job_a".to_string(),
            "0 * * * * *".to_string(),
            "UTC".to_string(),
            test_agent_config(),
        );
        let job_b = CronJobDefinition::new(
            "job_b".to_string(),
            "0 */10 * * * *".to_string(),
            "America/Chicago".to_string(),
            test_agent_config(),
        );
        let job_c = CronJobDefinition::new(
            "job_c".to_string(),
            "0 0 9 * * Mon-Fri".to_string(),
            "Europe/London".to_string(),
            test_agent_config(),
        );

        let id_a = cron.add_job(job_a).await.unwrap();
        let id_b = cron.add_job(job_b).await.unwrap();
        let id_c = cron.add_job(job_c).await.unwrap();

        let all = cron.list_jobs().await.unwrap();
        assert_eq!(all.len(), 3);

        // Removing one doesn't affect others
        cron.remove_job(id_b).await.unwrap();
        let remaining = cron.list_jobs().await.unwrap();
        assert_eq!(remaining.len(), 2);
        let ids: Vec<_> = remaining.iter().map(|j| j.job_id).collect();
        assert!(ids.contains(&id_a));
        assert!(ids.contains(&id_c));
        cron.shutdown().await;
    }

    // ── Health check with jobs ───────────────────────────────────────

    #[tokio::test]
    async fn health_check_reflects_job_states() {
        let (cron, _sched) = make_scheduler().await;

        // Add an active job
        let active_job = CronJobDefinition::new(
            "active".to_string(),
            "0 * * * * *".to_string(),
            "UTC".to_string(),
            test_agent_config(),
        );
        cron.add_job(active_job).await.unwrap();

        // Add and pause a job
        let pause_job = CronJobDefinition::new(
            "paused".to_string(),
            "0 * * * * *".to_string(),
            "UTC".to_string(),
            test_agent_config(),
        );
        let pid = cron.add_job(pause_job).await.unwrap();
        cron.pause_job(pid).await.unwrap();

        // Add and dead-letter a job
        let dl_job = CronJobDefinition::new(
            "dead_letter".to_string(),
            "0 * * * * *".to_string(),
            "UTC".to_string(),
            test_agent_config(),
        );
        let did = cron.add_job(dl_job).await.unwrap();
        cron.store
            .record_failure(did, 10, CronJobStatus::DeadLetter)
            .await
            .unwrap();

        let health = cron.check_health().await.unwrap();
        assert_eq!(health.jobs_total, 3);
        assert_eq!(health.jobs_active, 1);
        assert_eq!(health.jobs_paused, 1);
        assert_eq!(health.jobs_dead_letter, 1);
        cron.shutdown().await;
    }

    // ── Pause / resume / remove edge cases ───────────────────────────

    #[tokio::test]
    async fn pause_nonexistent_job_returns_not_found() {
        let (cron, _sched) = make_scheduler().await;
        let bogus_id = CronJobId::new();
        assert!(matches!(
            cron.pause_job(bogus_id).await,
            Err(CronSchedulerError::NotFound(_))
        ));
        cron.shutdown().await;
    }

    #[tokio::test]
    async fn resume_nonexistent_job_returns_not_found() {
        let (cron, _sched) = make_scheduler().await;
        let bogus_id = CronJobId::new();
        assert!(matches!(
            cron.resume_job(bogus_id).await,
            Err(CronSchedulerError::NotFound(_))
        ));
        cron.shutdown().await;
    }

    #[tokio::test]
    async fn remove_nonexistent_job_returns_not_found() {
        let (cron, _sched) = make_scheduler().await;
        let bogus_id = CronJobId::new();
        assert!(matches!(
            cron.remove_job(bogus_id).await,
            Err(CronSchedulerError::NotFound(_))
        ));
        cron.shutdown().await;
    }

    #[tokio::test]
    async fn trigger_now_nonexistent_job_returns_not_found() {
        let (cron, _sched) = make_scheduler().await;
        let bogus_id = CronJobId::new();
        assert!(matches!(
            cron.trigger_now(bogus_id).await,
            Err(CronSchedulerError::NotFound(_))
        ));
        cron.shutdown().await;
    }

    #[tokio::test]
    async fn double_pause_is_idempotent() {
        let (cron, _sched) = make_scheduler().await;
        let job = CronJobDefinition::new(
            "double_pause".to_string(),
            "0 * * * * *".to_string(),
            "UTC".to_string(),
            test_agent_config(),
        );
        let id = cron.add_job(job).await.unwrap();
        cron.pause_job(id).await.unwrap();
        cron.pause_job(id).await.unwrap(); // Should not panic or error
        let loaded = cron.get_job(id).await.unwrap();
        assert_eq!(loaded.status, CronJobStatus::Paused);
        cron.shutdown().await;
    }

    // ── Metrics after trigger ────────────────────────────────────────

    #[tokio::test]
    async fn metrics_update_after_trigger_now() {
        let (cron, _sched) = make_scheduler().await;

        let job = CronJobDefinition::new(
            "metrics_trigger".to_string(),
            "0 0 0 1 1 * 2099".to_string(),
            "UTC".to_string(),
            test_agent_config(),
        );
        let id = cron.add_job(job).await.unwrap();

        let before = cron.metrics();
        assert_eq!(before.runs_total, 0);

        cron.trigger_now(id).await.unwrap();
        assert_eq!(cron.metrics().runs_total, 1);
        assert_eq!(cron.metrics().runs_succeeded, 1);
        let history = cron.get_run_history(id, 10).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].status, JobRunStatus::Succeeded);
        assert!(history[0].execution_time_ms.is_some());
        cron.shutdown().await;
    }

    // ── Health check after shutdown ──────────────────────────────────

    #[tokio::test]
    async fn health_check_after_shutdown() {
        let (cron, _sched) = make_scheduler().await;
        cron.shutdown().await;
        let health = cron.check_health().await.unwrap();
        assert!(!health.is_running);
        assert!(health.store_accessible);
    }

    // ── AP-3: AgentPin verification on cron runs ──────────────────────

    #[tokio::test]
    async fn trigger_now_refused_when_verifier_installed_without_jwt() {
        let (cron, _sched) = make_scheduler().await;
        let cron = cron.with_agentpin_verifier(std::sync::Arc::new(
            crate::integrations::MockAgentPinVerifier::new_success(),
        ));
        let id = cron
            .add_job(CronJobDefinition::new(
                "needs-jwt".to_string(),
                "0 * * * * *".to_string(),
                "UTC".to_string(),
                test_agent_config(),
            ))
            .await
            .unwrap();
        let err = cron.trigger_now(id).await.expect_err("must refuse");
        assert!(
            matches!(err, CronSchedulerError::IdentityVerificationFailed(_, _)),
            "expected IdentityVerificationFailed, got {:?}",
            err
        );
        let metrics = cron.metrics();
        assert!(metrics.runs_skipped_identity >= 1);
        cron.shutdown().await;
    }

    #[tokio::test]
    async fn trigger_now_refused_when_verifier_rejects() {
        let (cron, _sched) = make_scheduler().await;
        let cron = cron.with_agentpin_verifier(std::sync::Arc::new(
            crate::integrations::MockAgentPinVerifier::new_failure(),
        ));
        let mut job = CronJobDefinition::new(
            "bad-jwt".to_string(),
            "0 * * * * *".to_string(),
            "UTC".to_string(),
            test_agent_config(),
        );
        job.agentpin_jwt = Some("any.jwt.here".to_string());
        let id = cron.add_job(job).await.unwrap();
        let err = cron.trigger_now(id).await.expect_err("must refuse");
        assert!(matches!(
            err,
            CronSchedulerError::IdentityVerificationFailed(_, _)
        ));
        cron.shutdown().await;
    }

    #[tokio::test]
    async fn trigger_now_allowed_when_verifier_subject_matches_job_agent() {
        let (cron, _sched) = make_scheduler().await;
        let mut job = CronJobDefinition::new(
            "good-jwt".to_string(),
            "0 * * * * *".to_string(),
            "UTC".to_string(),
            test_agent_config(),
        );
        job.agentpin_jwt = Some("any.jwt.here".to_string());
        let expected_sub = job.agent_config.id.0.to_string();
        let cron = cron.with_agentpin_verifier(std::sync::Arc::new(
            crate::integrations::MockAgentPinVerifier::with_identity(
                expected_sub,
                "test.example.com".to_string(),
                vec![],
            ),
        ));
        let id = cron.add_job(job).await.unwrap();
        cron.trigger_now(id).await.expect("must succeed");
        cron.shutdown().await;
    }

    #[tokio::test]
    async fn trigger_now_refused_when_subject_does_not_match_job_agent() {
        let (cron, _sched) = make_scheduler().await;
        let mut job = CronJobDefinition::new(
            "wrong-sub".to_string(),
            "0 * * * * *".to_string(),
            "UTC".to_string(),
            test_agent_config(),
        );
        job.agentpin_jwt = Some("any.jwt.here".to_string());
        let cron = cron.with_agentpin_verifier(std::sync::Arc::new(
            crate::integrations::MockAgentPinVerifier::with_identity(
                "someone-else".to_string(),
                "test.example.com".to_string(),
                vec![],
            ),
        ));
        let id = cron.add_job(job).await.unwrap();
        let err = cron.trigger_now(id).await.expect_err("must refuse");
        assert!(matches!(
            err,
            CronSchedulerError::IdentityVerificationFailed(_, _)
        ));
        cron.shutdown().await;
    }

    // ── Schedule policy gate wiring ──────────────────────────────────

    fn deny_all_gate() -> PolicyGate {
        PolicyGate::new(
            vec![SchedulePolicyRule {
                id: "deny-all".to_string(),
                name: "Deny all cron runs".to_string(),
                condition: SchedulePolicyCondition::Always,
                effect: SchedulePolicyEffect::Deny {
                    reason: "cron disabled by test policy".to_string(),
                },
                priority: 100,
                enabled: true,
            }],
            true,
        )
    }

    fn allow_all_gate() -> PolicyGate {
        PolicyGate::new(
            vec![SchedulePolicyRule {
                id: "allow-all".to_string(),
                name: "Allow all cron runs".to_string(),
                condition: SchedulePolicyCondition::Always,
                effect: SchedulePolicyEffect::Allow,
                priority: 100,
                enabled: true,
            }],
            true,
        )
    }

    #[tokio::test]
    async fn trigger_now_refused_when_policy_denies() {
        let (cron, _sched) = make_scheduler().await;
        let cron = cron.with_policy_gate(Arc::new(deny_all_gate()));

        let job = CronJobDefinition::new(
            "policy-denied".to_string(),
            "0 0 0 1 1 * 2099".to_string(), // far future — trigger_now bypasses the schedule
            "UTC".to_string(),
            test_agent_config(),
        );
        let id = cron.add_job(job).await.unwrap();

        let err = cron.trigger_now(id).await.expect_err("must refuse");
        assert!(
            matches!(err, CronSchedulerError::PolicyDenied(_, _)),
            "expected PolicyDenied, got {:?}",
            err
        );

        let metrics = cron.metrics();
        assert!(metrics.runs_skipped_policy >= 1);

        let history = cron.get_run_history(id, 10).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].status, JobRunStatus::Unresolved);
        assert!(history[0].error.as_deref().unwrap_or("").contains("policy"));
        cron.shutdown().await;
    }

    #[tokio::test]
    async fn trigger_now_allowed_when_policy_permits() {
        let (cron, _sched) = make_scheduler().await;
        let cron = cron.with_policy_gate(Arc::new(allow_all_gate()));

        let job = CronJobDefinition::new(
            "policy-allowed".to_string(),
            "0 0 0 1 1 * 2099".to_string(),
            "UTC".to_string(),
            test_agent_config(),
        );
        let id = cron.add_job(job).await.unwrap();

        cron.trigger_now(id).await.expect("must succeed");

        let metrics = cron.metrics();
        assert_eq!(metrics.runs_skipped_policy, 0);

        let history = cron.get_run_history(id, 10).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].status, JobRunStatus::Succeeded);
        cron.shutdown().await;
    }

    #[tokio::test]
    async fn tick_loop_refuses_run_when_policy_denies() {
        let (cron, _sched) = make_scheduler().await;
        let cron = cron.with_policy_gate(Arc::new(deny_all_gate()));

        let mut job = CronJobDefinition::new(
            "tick-denied".to_string(),
            "* * * * * * *".to_string(), // fires every second (7-field)
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

        tokio::time::sleep(Duration::from_secs(2)).await;

        let history = cron.get_run_history(id, 10).await.unwrap();
        assert_eq!(history.len(), 1, "denied run must still be recorded");
        assert_eq!(history[0].status, JobRunStatus::Unresolved);
        assert!(history[0].error.as_deref().unwrap_or("").contains("policy"));

        let metrics = cron.metrics();
        assert!(metrics.runs_skipped_policy >= 1);
        assert_eq!(
            metrics.runs_succeeded, 0,
            "a policy-denied run must never reach the agent scheduler"
        );
        cron.shutdown().await;
    }

    #[tokio::test]
    async fn tick_loop_runs_when_policy_allows() {
        let (cron, _sched) = make_scheduler().await;
        let cron = cron.with_policy_gate(Arc::new(allow_all_gate()));

        let mut job = CronJobDefinition::new(
            "tick-allowed".to_string(),
            "* * * * * * *".to_string(),
            "UTC".to_string(),
            test_agent_config(),
        );
        job.one_shot = true;
        let id = cron.add_job(job).await.unwrap();

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

        tokio::time::sleep(Duration::from_secs(2)).await;

        let loaded = cron.get_job(id).await.unwrap();
        assert_eq!(loaded.status, CronJobStatus::Completed);

        let metrics = cron.metrics();
        assert_eq!(metrics.runs_skipped_policy, 0);
        assert!(metrics.runs_succeeded >= 1);
        cron.shutdown().await;
    }
    #[tokio::test]
    async fn timer_observes_identity_verifier_installed_after_construction() {
        let (cron, scheduler) = make_scheduler().await;
        let cron = cron.with_agentpin_verifier(Arc::new(
            crate::integrations::MockAgentPinVerifier::new_failure(),
        ));
        let config = test_agent_config();
        let agent_id = config.id;
        let mut job = CronJobDefinition::new(
            "identity-timer".into(),
            "* * * * * *".into(),
            "UTC".into(),
            config,
        );
        job.agentpin_jwt = Some("synthetic.jwt.fixture".into());
        job.one_shot = true;
        let id = cron.add_job(job).await.unwrap();
        let history = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let rows = cron.get_run_history(id, 10).await.unwrap();
                if rows.first().is_some_and(|r| {
                    !matches!(r.status, JobRunStatus::Pending | JobRunStatus::Running)
                }) {
                    break rows;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].status, JobRunStatus::Unresolved);
        assert!(history[0].execution.is_none());
        assert!(!scheduler.has_agent(agent_id));
        assert_eq!(cron.metrics().runs_skipped_identity, 1);
        cron.shutdown().await;
        scheduler.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn cron_cannot_reinterpret_external_registration_as_local_execution() {
        let (cron, scheduler) = make_scheduler().await;
        let mut config = test_agent_config();
        let agent_id = config.id;
        config.execution_mode = ExecutionMode::External {
            endpoint: None,
            agentpin_domain: None,
            heartbeat_interval_secs: 60,
        };
        scheduler.register_agent(config.clone()).await.unwrap();
        let job = CronJobDefinition::new(
            "external".into(),
            "0 0 * * * *".into(),
            "UTC".into(),
            config,
        );
        let id = cron.add_job(job).await.unwrap();
        assert!(cron
            .trigger_now(id)
            .await
            .unwrap_err()
            .to_string()
            .contains("external agents require their own execution transport"));
        let rows = cron.get_run_history(id, 10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, JobRunStatus::Unresolved);
        assert!(rows[0].execution.is_none());
        assert!(matches!(
            scheduler.get_agent_config(agent_id).unwrap().execution_mode,
            ExecutionMode::External { .. }
        ));
        cron.shutdown().await;
        scheduler.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn refused_identity_cannot_be_replayed_after_policy_changes() {
        use crate::scheduler::invocations::{Admission, InvocationIdentity};
        let (cron, sched) = make_scheduler().await;
        let cron = cron.with_policy_gate(Arc::new(deny_all_gate()));
        let job = CronJobDefinition::new(
            "refused".into(),
            "0 0 0 1 1 * 2099".into(),
            "UTC".into(),
            test_agent_config(),
        );
        let agent_id = job.agent_config.id;
        let id = cron.add_job(job).await.unwrap();
        let identity = InvocationIdentity {
            id: uuid::Uuid::new_v4(),
            context: serde_json::json!({"caller":"fixture"}),
        };
        assert!(matches!(
            cron.trigger_identified(id, identity.clone()).await,
            Err(CronSchedulerError::PolicyDenied(_, _))
        ));
        let cron = cron.with_policy_gate(Arc::new(allow_all_gate()));
        assert!(matches!(
            cron.trigger_identified(id, identity).await.unwrap(),
            Admission::Existing(
                crate::reasoning::invocation::ExistingInvocation::Unresolved { .. }
            )
        ));
        assert!(!sched.has_agent(agent_id));
        assert_eq!(cron.get_run_history(id, 10).await.unwrap().len(), 1);
        assert_eq!(cron.metrics().runs_failed, 1);
        assert!(cron.resume_job(id).await.is_err());
        cron.shutdown().await;
        sched.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn prepared_intent_and_stale_history_recover_without_duplicate_execution() {
        use crate::scheduler::{invocations::InvocationIdentity, occurrences::Occurrence};
        let (original, sched) = make_scheduler().await;
        original.shutdown().await;
        let path = sched
            .invocation_project()
            .unwrap()
            .join(".symbiont/cron_jobs.db");
        let store = SqliteJobStore::open(&path).unwrap();
        let job = CronJobDefinition::new(
            "recover".into(),
            "0 0 0 1 1 * 2099".into(),
            "UTC".into(),
            test_agent_config(),
        );
        let id = job.job_id;
        store.save_job(&job).await.unwrap();
        let occurrence = Occurrence::manual(
            job,
            InvocationIdentity {
                id: uuid::Uuid::new_v4(),
                context: serde_json::json!({"caller":"fixture"}),
            },
        );
        store
            .prepare_occurrence(&occurrence, None, 10)
            .await
            .unwrap()
            .unwrap();
        let config = CronSchedulerConfig {
            tick_interval: Duration::from_millis(20),
            ..Default::default()
        };
        let recovered = CronScheduler::new(config.clone(), sched.clone())
            .await
            .unwrap();
        async fn terminal(cron: &CronScheduler, id: CronJobId) -> JobRunRecord {
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let rows = cron.get_run_history(id, 10).await.unwrap();
                    if let Some(row) = rows
                        .into_iter()
                        .find(|r| r.status == JobRunStatus::Succeeded)
                    {
                        break row;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap()
        }
        let first = terminal(&recovered, id).await;
        assert_eq!(
            first.execution.as_ref().unwrap().output.as_deref(),
            Some("scheduled result")
        );
        recovered.shutdown().await;
        // Simulate loss of the cron bookkeeping commit after core completion.
        store.conn.lock().await.execute_batch("UPDATE cron_occurrences SET state='running'; UPDATE job_run_log SET status='running',execution_json=NULL,completed_at=NULL;").unwrap();
        let recovered = CronScheduler::new(config, sched.clone()).await.unwrap();
        let second = terminal(&recovered, id).await;
        assert_eq!(
            serde_json::to_value(&second.admission_audit).unwrap(),
            serde_json::to_value(&first.admission_audit).unwrap()
        );
        assert_eq!(
            second.execution.unwrap().run_id,
            first.execution.unwrap().run_id
        );
        let journals = std::fs::read_dir(
            sched
                .invocation_project()
                .unwrap()
                .join(".symbiont/governed"),
        )
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|e| e == "jsonl"))
        .count();
        assert_eq!(journals, 1);
        recovered.shutdown().await;
        sched.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn startup_guard_applies_before_recovering_persisted_intent() {
        use crate::scheduler::{invocations::InvocationIdentity, occurrences::Occurrence};
        let (original, sched) = make_scheduler().await;
        original.shutdown().await;
        let path = sched
            .invocation_project()
            .unwrap()
            .join(".symbiont/cron_jobs.db");
        let store = SqliteJobStore::open(&path).unwrap();
        let job = CronJobDefinition::new(
            "recover-denied".into(),
            "0 0 0 1 1 * 2099".into(),
            "UTC".into(),
            test_agent_config(),
        );
        let id = job.job_id;
        let agent_id = job.agent_config.id;
        store.save_job(&job).await.unwrap();
        let occurrence = Occurrence::manual(
            job,
            InvocationIdentity {
                id: uuid::Uuid::new_v4(),
                context: serde_json::json!({"caller":"fixture"}),
            },
        );
        store
            .prepare_occurrence(&occurrence, None, 10)
            .await
            .unwrap()
            .unwrap();
        let cron = CronScheduler::new_with_guards(
            CronSchedulerConfig::default(),
            sched.clone(),
            Some(Arc::new(
                crate::integrations::MockAgentPinVerifier::new_failure(),
            )),
            None,
        )
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if store.get_run_history(id, 1).await.unwrap()[0].status == JobRunStatus::Unresolved
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert!(!sched.has_agent(agent_id));
        assert_eq!(cron.metrics().runs_skipped_identity, 1);
        assert!(cron.resume_job(id).await.is_err());
        cron.shutdown().await;
        sched.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn source_schedule_restart_preserves_terminal_state_and_rejects_changed_source() {
        let (cron, sched) = make_scheduler().await;
        let make = || {
            CronJobDefinition::new(
                "source".into(),
                "0 0 0 1 1 * 2099".into(),
                "UTC".into(),
                test_agent_config(),
            )
        };
        let id = cron
            .add_source_job("agents/fixture.symbi", make())
            .await
            .unwrap();
        let mut saved = cron.get_job(id).await.unwrap();
        saved.run_count = 3;
        saved.enabled = false;
        saved.status = CronJobStatus::Completed;
        saved.next_run = None;
        cron.store.save_job(&saved).await.unwrap();
        assert_eq!(
            cron.add_source_job("agents/fixture.symbi", make())
                .await
                .unwrap(),
            id
        );
        let restored = cron.get_job(id).await.unwrap();
        assert_eq!(restored.status, CronJobStatus::Completed);
        assert!(!restored.enabled);
        assert_eq!(restored.run_count, 3);
        assert!(restored.next_run.is_none());
        let mut changed = make();
        changed.agent_config.dsl_source.push_str("// changed");
        assert!(cron
            .add_source_job("agents/fixture.symbi", changed)
            .await
            .is_err());
        assert_eq!(cron.list_jobs().await.unwrap().len(), 1);
        cron.shutdown().await;
        sched.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn corrupt_protected_claim_is_visible_as_unresolved_history() {
        use crate::reasoning::invocation::{open_invocation, OpenInvocation};
        use crate::scheduler::{invocations::InvocationIdentity, occurrences::Occurrence};
        let (cron, sched) = make_scheduler().await;
        cron.stopping.cancel();
        let job = CronJobDefinition::new(
            "corrupt".into(),
            "0 0 0 1 1 * 2099".into(),
            "UTC".into(),
            test_agent_config(),
        );
        let id = cron.add_job(job).await.unwrap();
        let identity = InvocationIdentity {
            id: uuid::Uuid::new_v4(),
            context: serde_json::json!({"caller":"fixture"}),
        };
        let occurrence = Occurrence::manual(cron.get_job(id).await.unwrap(), identity.clone());
        cron.store
            .prepare_occurrence(&occurrence, None, 10)
            .await
            .unwrap()
            .unwrap();
        let (config, input, bound) = occurrence.admission();
        let request =
            serde_json::json!({"version":1,"context":bound.context,"target":config,"input":input});
        let claim = open_invocation(
            sched.invocation_project().unwrap(),
            "scheduler:v1",
            bound.id,
            &request,
            config.id,
        )
        .await
        .unwrap();
        assert!(matches!(&claim, OpenInvocation::Fresh(_)));
        drop(claim);
        let path = std::fs::read_dir(
            sched
                .invocation_project()
                .unwrap()
                .join(".symbiont/invocations"),
        )
        .unwrap()
        .filter_map(Result::ok)
        .find(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .unwrap()
        .path();
        std::fs::write(path, "invalid claim\n").unwrap();
        assert!(cron.trigger_identified(id, identity).await.is_err());
        let history = cron.get_run_history(id, 10).await.unwrap();
        assert_eq!(history[0].status, JobRunStatus::Unresolved);
        assert!(history[0].error.as_ref().unwrap().contains("lookup failed"));
        assert!(!sched.has_agent(config.id));
        assert!(cron.resume_job(id).await.is_err());
        cron.shutdown().await;
        sched.shutdown().await.unwrap();
    }
}
