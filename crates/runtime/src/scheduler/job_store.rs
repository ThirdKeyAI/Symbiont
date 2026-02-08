//! Persistent job store backed by SQLite.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::cron_types::*;

/// Abstract job store for cron job persistence.
#[async_trait]
pub trait JobStore: Send + Sync {
    /// Persist a job definition (insert or update).
    async fn save_job(&self, job: &CronJobDefinition) -> Result<(), JobStoreError>;

    /// Retrieve a job by its ID.
    async fn get_job(&self, job_id: CronJobId) -> Result<Option<CronJobDefinition>, JobStoreError>;

    /// Delete a job.
    async fn delete_job(&self, job_id: CronJobId) -> Result<bool, JobStoreError>;

    /// List all jobs, optionally filtered by status.
    async fn list_jobs(
        &self,
        status_filter: Option<CronJobStatus>,
    ) -> Result<Vec<CronJobDefinition>, JobStoreError>;

    /// Return all enabled, active jobs whose `next_run <= now`.
    async fn get_due_jobs(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<CronJobDefinition>, JobStoreError>;

    /// Update the run-state fields after a tick (last_run, next_run, run_count, status, etc.).
    async fn update_run_state(
        &self,
        job_id: CronJobId,
        last_run: DateTime<Utc>,
        next_run: Option<DateTime<Utc>>,
        run_count: u64,
        status: CronJobStatus,
        enabled: bool,
    ) -> Result<(), JobStoreError>;

    /// Increment `failure_count` for a job.
    async fn record_failure(
        &self,
        job_id: CronJobId,
        failure_count: u64,
        status: CronJobStatus,
    ) -> Result<(), JobStoreError>;

    /// Append a run record to the history log.
    async fn save_run_record(&self, record: &JobRunRecord) -> Result<(), JobStoreError>;

    /// Query run history for a job, newest first.
    async fn get_run_history(
        &self,
        job_id: CronJobId,
        limit: usize,
    ) -> Result<Vec<JobRunRecord>, JobStoreError>;
}

/// Errors produced by the job store.
#[derive(Debug, thiserror::Error)]
pub enum JobStoreError {
    #[error("SQLite error: {0}")]
    Sqlite(String),
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("Job not found: {0}")]
    NotFound(CronJobId),
}

/// SQLite-backed persistent store for cron jobs.
pub struct SqliteJobStore {
    conn: tokio::sync::Mutex<rusqlite::Connection>,
}

impl SqliteJobStore {
    /// Open (or create) the store at the given path.
    pub fn open(path: &std::path::Path) -> Result<Self, JobStoreError> {
        // Ensure parent directory exists.
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| JobStoreError::Sqlite(format!("create dir: {e}")))?;
        }
        let conn =
            rusqlite::Connection::open(path).map_err(|e| JobStoreError::Sqlite(e.to_string()))?;

        // WAL mode for concurrent access.
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| JobStoreError::Sqlite(e.to_string()))?;

        // Create tables before wrapping in Mutex (avoids blocking_lock in async context).
        Self::init_schema(&conn)?;

        Ok(Self {
            conn: tokio::sync::Mutex::new(conn),
        })
    }

    /// Open an in-memory store (useful for tests).
    pub fn open_in_memory() -> Result<Self, JobStoreError> {
        let conn = rusqlite::Connection::open_in_memory()
            .map_err(|e| JobStoreError::Sqlite(e.to_string()))?;

        Self::init_schema(&conn)?;

        Ok(Self {
            conn: tokio::sync::Mutex::new(conn),
        })
    }

    /// Default database path: `$XDG_DATA_HOME/symbi/cron_jobs.db`
    pub fn default_path() -> std::path::PathBuf {
        let base = dirs::data_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        base.join("symbi").join("cron_jobs.db")
    }

    fn init_schema(conn: &rusqlite::Connection) -> Result<(), JobStoreError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER PRIMARY KEY
            );

            INSERT OR IGNORE INTO schema_version (version) VALUES (1);

            CREATE TABLE IF NOT EXISTS cron_jobs (
                job_id       TEXT PRIMARY KEY,
                name         TEXT NOT NULL,
                cron_expr    TEXT NOT NULL,
                timezone     TEXT NOT NULL,
                agent_json   TEXT NOT NULL,
                policy_ids   TEXT NOT NULL DEFAULT '[]',
                audit_level  TEXT NOT NULL DEFAULT 'None',
                status       TEXT NOT NULL DEFAULT 'Active',
                enabled      INTEGER NOT NULL DEFAULT 1,
                one_shot     INTEGER NOT NULL DEFAULT 0,
                created_at   TEXT NOT NULL,
                updated_at   TEXT NOT NULL,
                last_run     TEXT,
                next_run     TEXT,
                run_count    INTEGER NOT NULL DEFAULT 0,
                failure_count INTEGER NOT NULL DEFAULT 0,
                max_retries  INTEGER NOT NULL DEFAULT 3,
                max_concurrent INTEGER NOT NULL DEFAULT 1,
                delivery_json TEXT,
                jitter_max_secs INTEGER NOT NULL DEFAULT 0,
                session_mode TEXT NOT NULL DEFAULT '\"EphemeralWithSummary\"',
                agentpin_jwt TEXT
            );

            CREATE TABLE IF NOT EXISTS job_run_log (
                run_id        TEXT PRIMARY KEY,
                job_id        TEXT NOT NULL,
                agent_id      TEXT NOT NULL,
                started_at    TEXT NOT NULL,
                completed_at  TEXT,
                status        TEXT NOT NULL,
                error         TEXT,
                exec_time_ms  INTEGER,
                FOREIGN KEY (job_id) REFERENCES cron_jobs(job_id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_cron_jobs_next_run ON cron_jobs(next_run);
            CREATE INDEX IF NOT EXISTS idx_job_run_log_job_id ON job_run_log(job_id);
            CREATE INDEX IF NOT EXISTS idx_job_run_log_started ON job_run_log(started_at);",
        )
        .map_err(|e| JobStoreError::Sqlite(e.to_string()))?;
        Ok(())
    }
}

#[async_trait]
impl JobStore for SqliteJobStore {
    async fn save_job(&self, job: &CronJobDefinition) -> Result<(), JobStoreError> {
        let agent_json = serde_json::to_string(&job.agent_config)
            .map_err(|e| JobStoreError::Serialization(e.to_string()))?;
        let policy_json = serde_json::to_string(&job.policy_ids)
            .map_err(|e| JobStoreError::Serialization(e.to_string()))?;
        let delivery_json = job
            .delivery_config
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| JobStoreError::Serialization(e.to_string()))?;
        let audit_str = serde_json::to_string(&job.audit_level)
            .map_err(|e| JobStoreError::Serialization(e.to_string()))?;
        let status_str = serde_json::to_string(&job.status)
            .map_err(|e| JobStoreError::Serialization(e.to_string()))?;

        let session_mode_str = serde_json::to_string(&job.session_mode)
            .map_err(|e| JobStoreError::Serialization(e.to_string()))?;

        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT OR REPLACE INTO cron_jobs
                (job_id, name, cron_expr, timezone, agent_json, policy_ids,
                 audit_level, status, enabled, one_shot, created_at, updated_at,
                 last_run, next_run, run_count, failure_count, max_retries,
                 max_concurrent, delivery_json, jitter_max_secs, session_mode, agentpin_jwt)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22)",
            rusqlite::params![
                job.job_id.to_string(),
                job.name,
                job.cron_expression,
                job.timezone,
                agent_json,
                policy_json,
                audit_str,
                status_str,
                job.enabled as i32,
                job.one_shot as i32,
                job.created_at.to_rfc3339(),
                job.updated_at.to_rfc3339(),
                job.last_run.map(|t| t.to_rfc3339()),
                job.next_run.map(|t| t.to_rfc3339()),
                job.run_count as i64,
                job.failure_count as i64,
                job.max_retries as i32,
                job.max_concurrent as i32,
                delivery_json,
                job.jitter_max_secs as i32,
                session_mode_str,
                job.agentpin_jwt,
            ],
        )
        .map_err(|e| JobStoreError::Sqlite(e.to_string()))?;
        Ok(())
    }

    async fn get_job(&self, job_id: CronJobId) -> Result<Option<CronJobDefinition>, JobStoreError> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT job_id, name, cron_expr, timezone, agent_json, policy_ids,
                        audit_level, status, enabled, one_shot, created_at, updated_at,
                        last_run, next_run, run_count, failure_count, max_retries,
                        max_concurrent, delivery_json, jitter_max_secs, session_mode, agentpin_jwt
                 FROM cron_jobs WHERE job_id = ?1",
            )
            .map_err(|e| JobStoreError::Sqlite(e.to_string()))?;

        let result = stmt
            .query_row(rusqlite::params![job_id.to_string()], row_to_job)
            .optional()
            .map_err(|e| JobStoreError::Sqlite(e.to_string()))?;

        match result {
            Some(Ok(job)) => Ok(Some(job)),
            Some(Err(e)) => Err(e),
            None => Ok(None),
        }
    }

    async fn delete_job(&self, job_id: CronJobId) -> Result<bool, JobStoreError> {
        let conn = self.conn.lock().await;
        let rows = conn
            .execute(
                "DELETE FROM cron_jobs WHERE job_id = ?1",
                rusqlite::params![job_id.to_string()],
            )
            .map_err(|e| JobStoreError::Sqlite(e.to_string()))?;
        Ok(rows > 0)
    }

    async fn list_jobs(
        &self,
        status_filter: Option<CronJobStatus>,
    ) -> Result<Vec<CronJobDefinition>, JobStoreError> {
        let conn = self.conn.lock().await;
        let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match status_filter {
            Some(s) => {
                let status_str = serde_json::to_string(&s)
                    .map_err(|e| JobStoreError::Serialization(e.to_string()))?;
                (
                    "SELECT job_id, name, cron_expr, timezone, agent_json, policy_ids,
                            audit_level, status, enabled, one_shot, created_at, updated_at,
                            last_run, next_run, run_count, failure_count, max_retries,
                            max_concurrent, delivery_json, jitter_max_secs, session_mode, agentpin_jwt
                     FROM cron_jobs WHERE status = ?1 ORDER BY created_at",
                    vec![Box::new(status_str)],
                )
            }
            None => (
                "SELECT job_id, name, cron_expr, timezone, agent_json, policy_ids,
                        audit_level, status, enabled, one_shot, created_at, updated_at,
                        last_run, next_run, run_count, failure_count, max_retries,
                        max_concurrent, delivery_json, jitter_max_secs, session_mode, agentpin_jwt
                 FROM cron_jobs ORDER BY created_at",
                vec![],
            ),
        };

        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| JobStoreError::Sqlite(e.to_string()))?;

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), row_to_job)
            .map_err(|e| JobStoreError::Sqlite(e.to_string()))?;

        let mut jobs = Vec::new();
        for row_result in rows {
            let inner = row_result.map_err(|e| JobStoreError::Sqlite(e.to_string()))?;
            jobs.push(inner?);
        }
        Ok(jobs)
    }

    async fn get_due_jobs(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<CronJobDefinition>, JobStoreError> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT job_id, name, cron_expr, timezone, agent_json, policy_ids,
                        audit_level, status, enabled, one_shot, created_at, updated_at,
                        last_run, next_run, run_count, failure_count, max_retries,
                        max_concurrent, delivery_json, jitter_max_secs, session_mode, agentpin_jwt
                 FROM cron_jobs
                 WHERE enabled = 1
                   AND status = '\"Active\"'
                   AND next_run IS NOT NULL
                   AND next_run <= ?1
                 ORDER BY next_run",
            )
            .map_err(|e| JobStoreError::Sqlite(e.to_string()))?;

        let rows = stmt
            .query_map(rusqlite::params![now.to_rfc3339()], row_to_job)
            .map_err(|e| JobStoreError::Sqlite(e.to_string()))?;

        let mut jobs = Vec::new();
        for row_result in rows {
            let inner = row_result.map_err(|e| JobStoreError::Sqlite(e.to_string()))?;
            jobs.push(inner?);
        }
        Ok(jobs)
    }

    async fn update_run_state(
        &self,
        job_id: CronJobId,
        last_run: DateTime<Utc>,
        next_run: Option<DateTime<Utc>>,
        run_count: u64,
        status: CronJobStatus,
        enabled: bool,
    ) -> Result<(), JobStoreError> {
        let status_str = serde_json::to_string(&status)
            .map_err(|e| JobStoreError::Serialization(e.to_string()))?;
        let conn = self.conn.lock().await;
        let rows = conn
            .execute(
                "UPDATE cron_jobs
                 SET last_run = ?1, next_run = ?2, run_count = ?3,
                     status = ?4, enabled = ?5, updated_at = ?6
                 WHERE job_id = ?7",
                rusqlite::params![
                    last_run.to_rfc3339(),
                    next_run.map(|t| t.to_rfc3339()),
                    run_count as i64,
                    status_str,
                    enabled as i32,
                    Utc::now().to_rfc3339(),
                    job_id.to_string(),
                ],
            )
            .map_err(|e| JobStoreError::Sqlite(e.to_string()))?;
        if rows == 0 {
            return Err(JobStoreError::NotFound(job_id));
        }
        Ok(())
    }

    async fn record_failure(
        &self,
        job_id: CronJobId,
        failure_count: u64,
        status: CronJobStatus,
    ) -> Result<(), JobStoreError> {
        let status_str = serde_json::to_string(&status)
            .map_err(|e| JobStoreError::Serialization(e.to_string()))?;
        let conn = self.conn.lock().await;
        let rows = conn
            .execute(
                "UPDATE cron_jobs SET failure_count = ?1, status = ?2, updated_at = ?3 WHERE job_id = ?4",
                rusqlite::params![
                    failure_count as i64,
                    status_str,
                    Utc::now().to_rfc3339(),
                    job_id.to_string(),
                ],
            )
            .map_err(|e| JobStoreError::Sqlite(e.to_string()))?;
        if rows == 0 {
            return Err(JobStoreError::NotFound(job_id));
        }
        Ok(())
    }

    async fn save_run_record(&self, record: &JobRunRecord) -> Result<(), JobStoreError> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO job_run_log
                (run_id, job_id, agent_id, started_at, completed_at, status, error, exec_time_ms)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            rusqlite::params![
                record.run_id.to_string(),
                record.job_id.to_string(),
                record.agent_id.to_string(),
                record.started_at.to_rfc3339(),
                record.completed_at.map(|t| t.to_rfc3339()),
                record.status.to_string(),
                record.error,
                record.execution_time_ms.map(|v| v as i64),
            ],
        )
        .map_err(|e| JobStoreError::Sqlite(e.to_string()))?;
        Ok(())
    }

    async fn get_run_history(
        &self,
        job_id: CronJobId,
        limit: usize,
    ) -> Result<Vec<JobRunRecord>, JobStoreError> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT run_id, job_id, agent_id, started_at, completed_at, status, error, exec_time_ms
                 FROM job_run_log
                 WHERE job_id = ?1
                 ORDER BY started_at DESC
                 LIMIT ?2",
            )
            .map_err(|e| JobStoreError::Sqlite(e.to_string()))?;

        let rows = stmt
            .query_map(
                rusqlite::params![job_id.to_string(), limit as i64],
                row_to_run_record,
            )
            .map_err(|e| JobStoreError::Sqlite(e.to_string()))?;

        let mut records = Vec::new();
        for row_result in rows {
            let inner = row_result.map_err(|e| JobStoreError::Sqlite(e.to_string()))?;
            records.push(inner?);
        }
        Ok(records)
    }
}

// ── Row-mapping helpers ───────────────────────────────────────────────

fn row_to_job(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<Result<CronJobDefinition, JobStoreError>> {
    let job_id_str: String = row.get(0)?;
    let name: String = row.get(1)?;
    let cron_expr: String = row.get(2)?;
    let timezone: String = row.get(3)?;
    let agent_json: String = row.get(4)?;
    let policy_json: String = row.get(5)?;
    let audit_str: String = row.get(6)?;
    let status_str: String = row.get(7)?;
    let enabled: i32 = row.get(8)?;
    let one_shot: i32 = row.get(9)?;
    let created_str: String = row.get(10)?;
    let updated_str: String = row.get(11)?;
    let last_run_str: Option<String> = row.get(12)?;
    let next_run_str: Option<String> = row.get(13)?;
    let run_count: i64 = row.get(14)?;
    let failure_count: i64 = row.get(15)?;
    let max_retries: i32 = row.get(16)?;
    let max_concurrent: i32 = row.get(17)?;
    let delivery_json: Option<String> = row.get(18)?;
    let jitter_max_secs: i32 = row.get(19)?;
    let session_mode_str: String = row.get(20)?;
    let agentpin_jwt: Option<String> = row.get(21)?;

    Ok((|| -> Result<CronJobDefinition, JobStoreError> {
        let job_id: CronJobId = job_id_str
            .parse()
            .map_err(|e: uuid::Error| JobStoreError::Serialization(e.to_string()))?;
        let agent_config = serde_json::from_str(&agent_json)
            .map_err(|e| JobStoreError::Serialization(e.to_string()))?;
        let policy_ids = serde_json::from_str(&policy_json)
            .map_err(|e| JobStoreError::Serialization(e.to_string()))?;
        let audit_level = serde_json::from_str(&audit_str)
            .map_err(|e| JobStoreError::Serialization(e.to_string()))?;
        let status = serde_json::from_str(&status_str)
            .map_err(|e| JobStoreError::Serialization(e.to_string()))?;
        let created_at = DateTime::parse_from_rfc3339(&created_str)
            .map_err(|e| JobStoreError::Serialization(e.to_string()))?
            .with_timezone(&Utc);
        let updated_at = DateTime::parse_from_rfc3339(&updated_str)
            .map_err(|e| JobStoreError::Serialization(e.to_string()))?
            .with_timezone(&Utc);
        let last_run = last_run_str
            .map(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .map(|dt| dt.with_timezone(&Utc))
                    .map_err(|e| JobStoreError::Serialization(e.to_string()))
            })
            .transpose()?;
        let next_run = next_run_str
            .map(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .map(|dt| dt.with_timezone(&Utc))
                    .map_err(|e| JobStoreError::Serialization(e.to_string()))
            })
            .transpose()?;
        let delivery_config = delivery_json
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| JobStoreError::Serialization(e.to_string()))?;
        let session_mode = serde_json::from_str(&session_mode_str)
            .map_err(|e| JobStoreError::Serialization(e.to_string()))?;

        Ok(CronJobDefinition {
            job_id,
            name,
            cron_expression: cron_expr,
            timezone,
            agent_config,
            policy_ids,
            audit_level,
            status,
            enabled: enabled != 0,
            one_shot: one_shot != 0,
            created_at,
            updated_at,
            last_run,
            next_run,
            run_count: run_count as u64,
            failure_count: failure_count as u64,
            max_retries: max_retries as u32,
            max_concurrent: max_concurrent as u32,
            delivery_config,
            jitter_max_secs: jitter_max_secs as u32,
            session_mode,
            agentpin_jwt,
        })
    })())
}

fn row_to_run_record(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<Result<JobRunRecord, JobStoreError>> {
    let run_id_str: String = row.get(0)?;
    let job_id_str: String = row.get(1)?;
    let agent_id_str: String = row.get(2)?;
    let started_str: String = row.get(3)?;
    let completed_str: Option<String> = row.get(4)?;
    let status_str: String = row.get(5)?;
    let error: Option<String> = row.get(6)?;
    let exec_time: Option<i64> = row.get(7)?;

    Ok((|| -> Result<JobRunRecord, JobStoreError> {
        let run_id = Uuid::parse_str(&run_id_str)
            .map_err(|e| JobStoreError::Serialization(e.to_string()))?;
        let job_id: CronJobId = job_id_str
            .parse()
            .map_err(|e: uuid::Error| JobStoreError::Serialization(e.to_string()))?;
        let agent_id: crate::types::AgentId = agent_id_str
            .parse()
            .map_err(|e| JobStoreError::Serialization(format!("agent_id: {e}")))?;
        let started_at = DateTime::parse_from_rfc3339(&started_str)
            .map_err(|e| JobStoreError::Serialization(e.to_string()))?
            .with_timezone(&Utc);
        let completed_at = completed_str
            .map(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .map(|dt| dt.with_timezone(&Utc))
                    .map_err(|e| JobStoreError::Serialization(e.to_string()))
            })
            .transpose()?;
        let status: JobRunStatus = status_str
            .parse()
            .map_err(|e: String| JobStoreError::Serialization(e))?;

        Ok(JobRunRecord {
            run_id,
            job_id,
            agent_id,
            started_at,
            completed_at,
            status,
            error,
            execution_time_ms: exec_time.map(|v| v as u64),
        })
    })())
}

/// Extension trait so we can use `optional()` on rusqlite queries.
trait OptionalExt<T> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error>;
}

impl<T> OptionalExt<T> for Result<T, rusqlite::Error> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        AgentConfig, AgentId, ExecutionMode, Priority, ResourceLimits, SecurityTier,
    };
    use std::collections::HashMap;

    fn test_agent_config() -> AgentConfig {
        AgentConfig {
            id: AgentId::new(),
            name: "cron_test_agent".to_string(),
            dsl_source: "agent cron_test {}".to_string(),
            execution_mode: ExecutionMode::Ephemeral,
            security_tier: SecurityTier::Tier1,
            resource_limits: ResourceLimits::default(),
            capabilities: vec![],
            policies: vec![],
            metadata: HashMap::new(),
            priority: Priority::Normal,
        }
    }

    fn test_job() -> CronJobDefinition {
        CronJobDefinition::new(
            "hourly_check".to_string(),
            "0 * * * *".to_string(),
            "UTC".to_string(),
            test_agent_config(),
        )
    }

    #[tokio::test]
    async fn save_and_get_job() {
        let store = SqliteJobStore::open_in_memory().unwrap();
        let job = test_job();
        store.save_job(&job).await.unwrap();

        let loaded = store.get_job(job.job_id).await.unwrap().unwrap();
        assert_eq!(loaded.name, "hourly_check");
        assert_eq!(loaded.cron_expression, "0 * * * *");
        assert!(loaded.enabled);
    }

    #[tokio::test]
    async fn delete_job() {
        let store = SqliteJobStore::open_in_memory().unwrap();
        let job = test_job();
        store.save_job(&job).await.unwrap();

        assert!(store.delete_job(job.job_id).await.unwrap());
        assert!(store.get_job(job.job_id).await.unwrap().is_none());
        // Deleting again returns false.
        assert!(!store.delete_job(job.job_id).await.unwrap());
    }

    #[tokio::test]
    async fn list_jobs_with_filter() {
        let store = SqliteJobStore::open_in_memory().unwrap();

        let mut job1 = test_job();
        job1.name = "job1".to_string();
        store.save_job(&job1).await.unwrap();

        let mut job2 = test_job();
        job2.name = "job2".to_string();
        job2.status = CronJobStatus::Paused;
        store.save_job(&job2).await.unwrap();

        let all = store.list_jobs(None).await.unwrap();
        assert_eq!(all.len(), 2);

        let active = store.list_jobs(Some(CronJobStatus::Active)).await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].name, "job1");

        let paused = store.list_jobs(Some(CronJobStatus::Paused)).await.unwrap();
        assert_eq!(paused.len(), 1);
        assert_eq!(paused[0].name, "job2");
    }

    #[tokio::test]
    async fn get_due_jobs_filtering() {
        let store = SqliteJobStore::open_in_memory().unwrap();
        let now = Utc::now();

        // Job due in the past → should be returned.
        let mut due_job = test_job();
        due_job.name = "due".to_string();
        due_job.next_run = Some(now - chrono::Duration::minutes(5));
        store.save_job(&due_job).await.unwrap();

        // Job due in the future → should not be returned.
        let mut future_job = test_job();
        future_job.name = "future".to_string();
        future_job.next_run = Some(now + chrono::Duration::hours(1));
        store.save_job(&future_job).await.unwrap();

        // Disabled job → should not be returned.
        let mut disabled_job = test_job();
        disabled_job.name = "disabled".to_string();
        disabled_job.next_run = Some(now - chrono::Duration::minutes(1));
        disabled_job.enabled = false;
        store.save_job(&disabled_job).await.unwrap();

        let due = store.get_due_jobs(now).await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].name, "due");
    }

    #[tokio::test]
    async fn update_run_state() {
        let store = SqliteJobStore::open_in_memory().unwrap();
        let job = test_job();
        store.save_job(&job).await.unwrap();

        let now = Utc::now();
        let next = now + chrono::Duration::hours(1);
        store
            .update_run_state(job.job_id, now, Some(next), 1, CronJobStatus::Active, true)
            .await
            .unwrap();

        let loaded = store.get_job(job.job_id).await.unwrap().unwrap();
        assert_eq!(loaded.run_count, 1);
        assert!(loaded.last_run.is_some());
        assert!(loaded.next_run.is_some());
    }

    #[tokio::test]
    async fn record_failure_and_dead_letter() {
        let store = SqliteJobStore::open_in_memory().unwrap();
        let job = test_job();
        store.save_job(&job).await.unwrap();

        store
            .record_failure(job.job_id, 4, CronJobStatus::Failed)
            .await
            .unwrap();

        let loaded = store.get_job(job.job_id).await.unwrap().unwrap();
        assert_eq!(loaded.failure_count, 4);
        assert_eq!(loaded.status, CronJobStatus::Failed);
    }

    #[tokio::test]
    async fn run_history_crud() {
        let store = SqliteJobStore::open_in_memory().unwrap();
        let job = test_job();
        store.save_job(&job).await.unwrap();

        let record = JobRunRecord {
            run_id: Uuid::new_v4(),
            job_id: job.job_id,
            agent_id: job.agent_config.id,
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
            status: JobRunStatus::Succeeded,
            error: None,
            execution_time_ms: Some(1234),
        };
        store.save_run_record(&record).await.unwrap();

        let history = store.get_run_history(job.job_id, 10).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].status, JobRunStatus::Succeeded);
        assert_eq!(history[0].execution_time_ms, Some(1234));
    }

    #[tokio::test]
    async fn concurrent_reads() {
        let store = std::sync::Arc::new(SqliteJobStore::open_in_memory().unwrap());
        let job = test_job();
        store.save_job(&job).await.unwrap();

        let mut handles = vec![];
        for _ in 0..10 {
            let s = store.clone();
            let jid = job.job_id;
            handles.push(tokio::spawn(async move {
                s.get_job(jid).await.unwrap().unwrap()
            }));
        }
        for h in handles {
            let loaded = h.await.unwrap();
            assert_eq!(loaded.name, "hourly_check");
        }
    }
}
