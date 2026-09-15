//! Frozen cron intent and atomic occurrence persistence. Mutable clock/counter
//! fields do not alter retry identity; source, input and authority do.

use super::{
    cron_types::*,
    invocations::InvocationIdentity,
    job_store::{JobStoreError, SqliteJobStore},
    task_manager::{TaskCompletion, TaskStatus},
};
use crate::{
    reasoning::{prepared::digest_json, run_audit::RunAuditReference},
    types::{AgentConfig, ExecutionMode},
};
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

const MAX_OCCURRENCES: i64 = 4096;
const MAX_OCCURRENCE_BYTES: usize = 1024 * 1024;
const MAX_STORE_BYTES: i64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Occurrence {
    pub job: CronJobDefinition,
    pub identity: InvocationIdentity,
    pub scheduled_for: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}
impl Occurrence {
    pub fn manual(job: CronJobDefinition, identity: InvocationIdentity) -> Self {
        Self {
            job,
            identity,
            scheduled_for: None,
            created_at: Utc::now(),
        }
    }
    pub fn timer(job: CronJobDefinition) -> Result<Self, String> {
        let due = job
            .next_run
            .ok_or("timer occurrence has no scheduled time")?;
        let digest = digest_json(&json!(["cron-occurrence:v1", job.job_id, due]))?;
        let bytes = hex::decode(digest.trim_start_matches("sha256:")).map_err(|e| e.to_string())?;
        let id = Uuid::from_bytes(bytes[..16].try_into().expect("SHA-256 prefix"));
        Ok(Self {
            job,
            identity: InvocationIdentity {
                id,
                context: json!({"caller":"cron.timer"}),
            },
            scheduled_for: Some(due),
            created_at: Utc::now(),
        })
    }
    pub fn admission(&self) -> (AgentConfig, Value, InvocationIdentity) {
        let mut config = self.job.agent_config.clone();
        if !matches!(config.execution_mode, ExecutionMode::External { .. }) {
            config.execution_mode = ExecutionMode::CronScheduled {
                cron_expression: self.job.cron_expression.clone(),
                timezone: self.job.timezone.clone(),
            };
        }
        config.metadata.insert(
            "trigger".into(),
            if self.scheduled_for.is_some() {
                "cron"
            } else {
                "cron_manual"
            }
            .into(),
        );
        config
            .metadata
            .insert("cron_job_id".into(), self.job.job_id.to_string());
        config
            .metadata
            .insert("session_id".into(), self.identity.id.to_string());
        config.metadata.insert(
            "session_mode".into(),
            format!("{:?}", self.job.session_mode),
        );
        let identity = InvocationIdentity {
            id: self.identity.id,
            context: json!({"caller":self.identity.context,
            "job_id":self.job.job_id,"job_name":self.job.name,"one_shot":self.job.one_shot,"max_concurrent":self.job.max_concurrent,"max_retries":self.job.max_retries,"scheduled_for":self.scheduled_for,"policy_ids":self.job.policy_ids,
            "audit_level":self.job.audit_level,"delivery":self.job.delivery_config,
            "agentpin_credential":self.job.agentpin_jwt,"jitter_max_secs":self.job.jitter_max_secs}),
        };
        (config, self.job.input.clone(), identity)
    }
    pub fn fingerprint(&self) -> Result<String, String> {
        let (config, input, identity) = self.admission();
        digest_json(&json!({"version":1,"context":identity.context,"target":config,"input":input}))
    }
}

#[derive(Debug, Clone)]
pub struct StoredOccurrence {
    pub occurrence: Occurrence,
    pub state: String,
}

fn sql(error: rusqlite::Error) -> JobStoreError {
    JobStoreError::Sqlite(error.to_string())
}
fn data(error: impl ToString) -> JobStoreError {
    JobStoreError::Serialization(error.to_string())
}
fn decode(raw: String, hash: String, state: String) -> Result<StoredOccurrence, JobStoreError> {
    if raw.len() > MAX_OCCURRENCE_BYTES {
        return Err(data("oversized occurrence record"));
    }
    let occurrence: Occurrence = serde_json::from_str(&raw).map_err(data)?;
    if occurrence.fingerprint().map_err(data)? != hash {
        return Err(data("occurrence fingerprint mismatch"));
    }
    if ![
        "prepared",
        "running",
        "succeeded",
        "failed",
        "unresolved",
        "reconciled",
        "timed_out",
    ]
    .contains(&state.as_str())
    {
        return Err(data("invalid occurrence state"));
    }
    Ok(StoredOccurrence { occurrence, state })
}

impl SqliteJobStore {
    /// For timers, intent, initial history and clock advancement commit together.
    /// A stale timer snapshot returns None without creating any occurrence.
    pub async fn prepare_occurrence(
        &self,
        occurrence: &Occurrence,
        next: Option<DateTime<Utc>>,
        global_limit: usize,
    ) -> Result<Option<StoredOccurrence>, JobStoreError> {
        let raw = serde_json::to_string(occurrence).map_err(data)?;
        if raw.len() > MAX_OCCURRENCE_BYTES {
            return Err(data("occurrence exceeds 1 MiB"));
        }
        let hash = occurrence.fingerprint().map_err(data)?;
        let id = occurrence.identity.id.to_string();
        let job = &occurrence.job;
        let mut conn = self.conn.lock().await;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let existing=tx.query_row("SELECT request_json,request_hash,state FROM cron_occurrences WHERE invocation_id=?1",[&id],
            |r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional().map_err(sql)?;
        if let Some((raw, saved_hash, state)) = existing {
            if saved_hash != hash {
                return Err(JobStoreError::InvocationConflict);
            }
            return decode(raw, saved_hash, state).map(Some);
        }
        let (count, bytes): (i64, i64) = tx
            .query_row(
                "SELECT COUNT(*),COALESCE(SUM(length(CAST(request_json AS BLOB))),0) FROM cron_occurrences",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(sql)?;
        if count >= MAX_OCCURRENCES || bytes.saturating_add(raw.len() as i64) > MAX_STORE_BYTES {
            return Err(data("cron occurrence storage capacity exhausted"));
        }
        let (active,per_job):(i64,i64)=tx.query_row("SELECT COUNT(*),COALESCE(SUM(job_id=?1),0) FROM cron_occurrences WHERE state IN ('prepared','running')",[job.job_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?))).map_err(sql)?;
        if active >= i64::try_from(global_limit).unwrap_or(i64::MAX)
            || per_job >= i64::from(job.max_concurrent)
        {
            return Ok(None);
        }
        if let Some(due) = occurrence.scheduled_for {
            let changed=tx.execute("UPDATE cron_jobs SET last_run=?1,next_run=?2,run_count=run_count+1,enabled=?3,updated_at=?1
                WHERE job_id=?4 AND next_run=?5 AND updated_at=?6 AND enabled=1 AND status='\"Active\"' AND NOT EXISTS(SELECT 1 FROM cron_occurrences WHERE job_id=?4 AND state='unresolved') AND NOT EXISTS(SELECT 1 FROM job_run_log WHERE job_id=?4 AND status='unresolved')",
                params![occurrence.created_at.to_rfc3339(),next.map(|t|t.to_rfc3339()),!job.one_shot,job.job_id.to_string(),due.to_rfc3339(),job.updated_at.to_rfc3339()]).map_err(sql)?;
            if changed != 1 {
                return Ok(None);
            }
        } else {
            let exists: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM cron_jobs WHERE job_id=?1)",
                    [job.job_id.to_string()],
                    |r| r.get(0),
                )
                .map_err(sql)?;
            if !exists {
                return Err(JobStoreError::NotFound(job.job_id));
            }
        }
        tx.execute("INSERT INTO cron_occurrences (invocation_id,job_id,request_hash,request_json,state,created_at) VALUES (?1,?2,?3,?4,'prepared',?5)",
            params![id,job.job_id.to_string(),hash,raw,occurrence.created_at.to_rfc3339()]).map_err(sql)?;
        tx.execute("INSERT INTO job_run_log (run_id,job_id,agent_id,started_at,status) VALUES (?1,?2,?3,?4,'pending')",
            params![id,job.job_id.to_string(),job.agent_config.id.to_string(),occurrence.created_at.to_rfc3339()]).map_err(sql)?;
        tx.commit().map_err(sql)?;
        Ok(Some(StoredOccurrence {
            occurrence: occurrence.clone(),
            state: "prepared".into(),
        }))
    }

    pub async fn job_has_unresolved_occurrence(
        &self,
        job_id: CronJobId,
    ) -> Result<bool, JobStoreError> {
        self.conn.lock().await.query_row("SELECT EXISTS(SELECT 1 FROM cron_occurrences WHERE job_id=?1 AND state='unresolved') OR EXISTS(SELECT 1 FROM job_run_log WHERE job_id=?1 AND status='unresolved')",[job_id.to_string()],|r|r.get(0)).map_err(sql)
    }

    #[cfg(test)]
    pub async fn pending_occurrences(&self) -> Result<Vec<StoredOccurrence>, JobStoreError> {
        self.pending_occurrences_after(None).await
    }

    /// Bounded pages rotate by stable ID so long-running owners cannot starve
    /// recovery of later intents. The caller wraps after the last page.
    pub async fn pending_occurrences_after(
        &self,
        after: Option<Uuid>,
    ) -> Result<Vec<StoredOccurrence>, JobStoreError> {
        let conn = self.conn.lock().await;
        let mut query = conn.prepare("SELECT request_json,request_hash,state FROM cron_occurrences WHERE state IN ('prepared','running','unresolved') AND invocation_id > ?1 ORDER BY invocation_id LIMIT 32").map_err(sql)?;
        let rows = query
            .query_map([after.map(|id| id.to_string()).unwrap_or_default()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(sql)?;
        rows.map(|row| {
            let (raw, hash, state) = row.map_err(sql)?;
            decode(raw, hash, state)
        })
        .collect()
    }

    pub async fn occurrence_running(
        &self,
        occurrence: &Occurrence,
        audit: &RunAuditReference,
    ) -> Result<(), JobStoreError> {
        let mut conn = self.conn.lock().await;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let id = occurrence.identity.id.to_string();
        let changed=tx.execute("UPDATE cron_occurrences SET state='running' WHERE invocation_id=?1 AND request_hash=?2 AND state='prepared'",params![id,occurrence.fingerprint().map_err(data)?]).map_err(sql)?;
        if changed != 1 {
            return Err(data("occurrence is no longer pending admission"));
        }
        let history_changed = tx.execute("UPDATE job_run_log SET status='running',admission_audit_json=?1 WHERE run_id=?2 AND status='pending'",params![serde_json::to_string(audit).map_err(data)?,id]).map_err(sql)?;
        if history_changed != 1 {
            return Err(data("occurrence history is missing or inconsistent"));
        }
        tx.commit().map_err(sql)
    }

    /// Apply a verified operator receipt to matching cron history. The retained
    /// execution identity remains closed and the job stays paused for an explicit
    /// resume. A crash between receipt publication and this transaction is repaired
    /// by repeating reconciliation or by the scheduler's bounded recovery scan.
    pub async fn reconcile_occurrence(
        &self,
        project: &std::path::Path,
        id: Uuid,
    ) -> Result<bool, JobStoreError> {
        self.bind_project(project).await?;
        let stored = {
            let conn = self.conn.lock().await;
            conn.query_row("SELECT request_json,request_hash,state FROM cron_occurrences WHERE invocation_id=?1", [id.to_string()],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
                .optional().map_err(sql)?
        };
        let Some((raw, fingerprint, state)) = stored else {
            return Ok(false);
        };
        let stored = decode(raw, fingerprint, state)?;
        let project = project.to_owned();
        let inspection = tokio::task::spawn_blocking(move || {
            crate::reasoning::invocation::reconciliation::inspect_invocation(
                &project,
                "scheduler:v1",
                id,
            )
        })
        .await
        .map_err(data)?
        .map_err(data)?;
        let receipt = inspection
            .resolution
            .ok_or_else(|| data("invocation has no verified operator resolution"))?;
        if receipt.snapshot.request_hash != stored.occurrence.fingerprint().map_err(data)?
            || receipt.snapshot.agent_id != stored.occurrence.job.agent_config.id
        {
            return Err(data(
                "operator resolution does not match the frozen cron occurrence",
            ));
        }
        let encoded = serde_json::to_string(&receipt).map_err(data)?;
        let mut conn = self.conn.lock().await;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let current: String = tx
            .query_row(
                "SELECT state FROM cron_occurrences WHERE invocation_id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .map_err(sql)?;
        if current == "reconciled" {
            let saved: Option<String> = tx.query_row("SELECT resolution_json FROM job_run_log WHERE run_id=?1 AND status='reconciled'", [id.to_string()], |r| r.get(0)).optional().map_err(sql)?.flatten();
            return if saved.as_deref() == Some(encoded.as_str()) {
                Ok(false)
            } else {
                Err(data("reconciled cron history is inconsistent"))
            };
        }
        let changed = tx.execute("UPDATE cron_occurrences SET state='reconciled' WHERE invocation_id=?1 AND request_hash=?2 AND state IN ('prepared','running','unresolved')",
            params![id.to_string(),receipt.snapshot.request_hash]).map_err(sql)?;
        if changed != 1 {
            return Err(data(
                "cron occurrence is already terminal or changed during reconciliation",
            ));
        }
        let now = Utc::now().to_rfc3339();
        let changed = tx.execute("UPDATE job_run_log SET status='reconciled',completed_at=COALESCE(completed_at,?1),resolution_json=?2,admission_audit_json=?3 WHERE run_id=?4 AND job_id=?5 AND status IN ('pending','running','unresolved')",
            params![now,encoded,serde_json::to_string(&receipt.snapshot.audit).map_err(data)?,id.to_string(),stored.occurrence.job.job_id.to_string()]).map_err(sql)?;
        if changed != 1 {
            return Err(data("cron history is missing or inconsistent"));
        }
        tx.execute(
            "UPDATE cron_jobs SET status='\"Paused\"',enabled=0,updated_at=?1 WHERE job_id=?2",
            params![now, stored.occurrence.job.job_id.to_string()],
        )
        .map_err(sql)?;
        tx.commit().map_err(sql)?;
        Ok(true)
    }

    /// Reconcile bookkeeping from the retained execution result, without dispatch.
    /// Unknown outcomes stop future timer occurrences until operator review.
    pub async fn finish_occurrence(
        &self,
        occurrence: &Occurrence,
        completion: Option<&TaskCompletion>,
        audit: Option<&RunAuditReference>,
        error: Option<&str>,
        unknown: bool,
    ) -> Result<bool, JobStoreError> {
        let status = if unknown || completion.is_some_and(|c| c.status == TaskStatus::Unresolved) {
            "unresolved"
        } else {
            match completion.map(|c| &c.status) {
                Some(TaskStatus::Completed) => "succeeded",
                Some(TaskStatus::TimedOut) => "timed_out",
                _ => "failed",
            }
        };
        let mut conn = self.conn.lock().await;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let id = occurrence.identity.id.to_string();
        let changed=tx.execute("UPDATE cron_occurrences SET state=?1 WHERE invocation_id=?2 AND request_hash=?3 AND state IN ('prepared','running')",
            params![status,id,occurrence.fingerprint().map_err(data)?]).map_err(sql)?;
        if changed == 0 {
            return Ok(false);
        }
        let now = Utc::now();
        let elapsed = (now - occurrence.created_at).num_milliseconds().max(0);
        let history_changed = tx.execute("UPDATE job_run_log SET status=?1,completed_at=?2,error=?3,exec_time_ms=?4,execution_json=?5,admission_audit_json=COALESCE(?6,admission_audit_json) WHERE run_id=?7 AND status IN ('pending','running')",
            params![status,now.to_rfc3339(),error.or_else(||completion.and_then(|c|c.error.as_deref())),elapsed,
                completion.map(serde_json::to_string).transpose().map_err(data)?,audit.map(serde_json::to_string).transpose().map_err(data)?,id]).map_err(sql)?;
        if history_changed != 1 {
            return Err(data("occurrence history is missing or inconsistent"));
        }
        let job = &occurrence.job;
        if status == "unresolved" {
            tx.execute("UPDATE cron_jobs SET status='\"DeadLetter\"',enabled=0,failure_count=failure_count+1,updated_at=?1 WHERE job_id=?2",params![now.to_rfc3339(),job.job_id.to_string()]).map_err(sql)?;
        } else if status != "succeeded" {
            tx.execute("UPDATE cron_jobs SET failure_count=failure_count+1,status=CASE WHEN one_shot=1 OR failure_count+1>=max_retries THEN '\"DeadLetter\"' ELSE status END,updated_at=?1 WHERE job_id=?2",params![now.to_rfc3339(),job.job_id.to_string()]).map_err(sql)?;
        } else if job.one_shot && occurrence.scheduled_for.is_some() {
            tx.execute("UPDATE cron_jobs SET status='\"Completed\"',enabled=0,next_run=NULL,updated_at=?1 WHERE job_id=?2 AND enabled=0 AND status='\"Active\"' AND updated_at=?3",params![now.to_rfc3339(),job.job_id.to_string(),occurrence.created_at.to_rfc3339()]).map_err(sql)?;
        }
        tx.commit().map_err(sql)?;
        Ok(true)
    }
}

#[cfg(test)]
#[path = "occurrence_tests.rs"]
mod tests;
