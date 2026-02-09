//! Shared types for the cron scheduling subsystem.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use uuid::Uuid;

use crate::types::AgentConfig;

use super::heartbeat::HeartbeatContextMode;

/// Unique identifier for a cron job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CronJobId(Uuid);

impl CronJobId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for CronJobId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for CronJobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for CronJobId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// How much audit detail to record for a cron job's runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AuditLevel {
    /// No audit logging.
    #[default]
    None,
    /// Only log errors and failures.
    ErrorsOnly,
    /// Log every operation.
    AllOperations,
}

/// Lifecycle status of a cron job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CronJobStatus {
    /// Job is active and will fire on schedule.
    #[default]
    Active,
    /// Job is paused; will not fire until resumed.
    Paused,
    /// Job completed (one-shot that has already run).
    Completed,
    /// Job entered dead-letter state after exceeding max retries.
    Failed,
    /// Job is in dead-letter queue — requires manual intervention.
    DeadLetter,
}

/// Configuration for routing scheduled job output to one or more channels.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryConfig {
    /// Channels to deliver results to (evaluated in order).
    pub channels: Vec<DeliveryChannel>,
    /// If true, delivery failure on any channel aborts remaining channels.
    #[serde(default)]
    pub fail_fast: bool,
}

/// A delivery channel that receives scheduled job output.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeliveryChannel {
    /// Print to stdout (useful for dev/debug).
    Stdout,
    /// Append to a log file.
    LogFile { path: String },
    /// POST results to an HTTP endpoint.
    Webhook {
        url: String,
        #[serde(default = "default_webhook_method")]
        method: String,
        #[serde(default)]
        headers: HashMap<String, String>,
        #[serde(default = "default_retry_count")]
        retry_count: u32,
        /// Timeout per request in seconds.
        #[serde(default = "default_webhook_timeout")]
        timeout_secs: u64,
    },
    /// Send to a Slack webhook.
    Slack {
        webhook_url: String,
        #[serde(default)]
        channel: Option<String>,
    },
    /// Send via email (SMTP).
    Email {
        smtp_host: String,
        smtp_port: u16,
        to: Vec<String>,
        from: String,
        #[serde(default)]
        subject_template: Option<String>,
    },
    /// Delegate to a named custom handler registered at runtime.
    Custom {
        handler_name: String,
        #[serde(default)]
        config: HashMap<String, String>,
    },
    /// Deliver through a registered chat channel adapter.
    ChannelAdapter {
        adapter_name: String,
        channel_id: String,
        #[serde(default)]
        thread_id: Option<String>,
    },
}

fn default_webhook_method() -> String {
    "POST".to_string()
}

fn default_retry_count() -> u32 {
    3
}

fn default_webhook_timeout() -> u64 {
    30
}

/// Receipt proving delivery was attempted / succeeded on a single channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryReceipt {
    /// Which channel was used.
    pub channel_description: String,
    /// When delivery completed (or failed).
    pub delivered_at: DateTime<Utc>,
    /// Whether delivery succeeded.
    pub success: bool,
    /// HTTP status code or other channel-specific status.
    pub status_code: Option<u16>,
    /// Error message if delivery failed.
    pub error: Option<String>,
}

/// Full definition of a scheduled cron job persisted in the job store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJobDefinition {
    pub job_id: CronJobId,
    pub name: String,
    pub cron_expression: String,
    pub timezone: String,
    pub agent_config: AgentConfig,
    pub policy_ids: Vec<String>,
    pub audit_level: AuditLevel,
    pub status: CronJobStatus,
    pub enabled: bool,
    pub one_shot: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_run: Option<DateTime<Utc>>,
    pub next_run: Option<DateTime<Utc>>,
    pub run_count: u64,
    pub failure_count: u64,
    pub max_retries: u32,
    pub max_concurrent: u32,
    pub delivery_config: Option<DeliveryConfig>,
    /// Maximum jitter in seconds applied before firing (0 = no jitter).
    #[serde(default)]
    pub jitter_max_secs: u32,
    /// Session isolation mode for each run.
    #[serde(default)]
    pub session_mode: HeartbeatContextMode,
    /// Optional AgentPin JWT for identity verification before each run.
    #[serde(default)]
    pub agentpin_jwt: Option<String>,
}

impl CronJobDefinition {
    /// Create a new job definition with sensible defaults.
    pub fn new(
        name: String,
        cron_expression: String,
        timezone: String,
        agent_config: AgentConfig,
    ) -> Self {
        let now = Utc::now();
        Self {
            job_id: CronJobId::new(),
            name,
            cron_expression,
            timezone,
            agent_config,
            policy_ids: Vec::new(),
            audit_level: AuditLevel::default(),
            status: CronJobStatus::Active,
            enabled: true,
            one_shot: false,
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
            session_mode: HeartbeatContextMode::default(),
            agentpin_jwt: None,
        }
    }
}

/// Record of a single cron job execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRunRecord {
    pub run_id: Uuid,
    pub job_id: CronJobId,
    pub agent_id: crate::types::AgentId,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub status: JobRunStatus,
    pub error: Option<String>,
    pub execution_time_ms: Option<u64>,
}

/// Status of a single run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobRunStatus {
    Running,
    Succeeded,
    Failed,
    TimedOut,
    Skipped,
}

impl fmt::Display for JobRunStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JobRunStatus::Running => write!(f, "running"),
            JobRunStatus::Succeeded => write!(f, "succeeded"),
            JobRunStatus::Failed => write!(f, "failed"),
            JobRunStatus::TimedOut => write!(f, "timed_out"),
            JobRunStatus::Skipped => write!(f, "skipped"),
        }
    }
}

impl std::str::FromStr for JobRunStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "running" => Ok(JobRunStatus::Running),
            "succeeded" => Ok(JobRunStatus::Succeeded),
            "failed" => Ok(JobRunStatus::Failed),
            "timed_out" => Ok(JobRunStatus::TimedOut),
            "skipped" => Ok(JobRunStatus::Skipped),
            other => Err(format!("unknown job run status: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cron_job_id_roundtrip() {
        let id = CronJobId::new();
        let s = id.to_string();
        let parsed: CronJobId = s.parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn cron_job_id_default() {
        let a = CronJobId::default();
        let b = CronJobId::default();
        assert_ne!(a, b);
    }

    #[test]
    fn audit_level_default_is_none() {
        assert_eq!(AuditLevel::default(), AuditLevel::None);
    }

    #[test]
    fn cron_job_status_default_is_active() {
        assert_eq!(CronJobStatus::default(), CronJobStatus::Active);
    }

    #[test]
    fn job_run_status_display_roundtrip() {
        for status in [
            JobRunStatus::Running,
            JobRunStatus::Succeeded,
            JobRunStatus::Failed,
            JobRunStatus::TimedOut,
            JobRunStatus::Skipped,
        ] {
            let s = status.to_string();
            let parsed: JobRunStatus = s.parse().unwrap();
            assert_eq!(status, parsed);
        }
    }

    #[test]
    fn cron_job_definition_new_defaults() {
        use crate::types::{
            AgentConfig, AgentId, ExecutionMode, Priority, ResourceLimits, SecurityTier,
        };
        use std::collections::HashMap;

        let config = AgentConfig {
            id: AgentId::new(),
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
        let job = CronJobDefinition::new(
            "test_job".to_string(),
            "0 * * * *".to_string(),
            "UTC".to_string(),
            config,
        );
        assert!(job.enabled);
        assert!(!job.one_shot);
        assert_eq!(job.run_count, 0);
        assert_eq!(job.max_retries, 3);
        assert_eq!(job.max_concurrent, 1);
        assert_eq!(job.status, CronJobStatus::Active);
    }

    #[test]
    fn cron_job_definition_serialization() {
        use crate::types::{
            AgentConfig, AgentId, ExecutionMode, Priority, ResourceLimits, SecurityTier,
        };
        use std::collections::HashMap;

        let config = AgentConfig {
            id: AgentId::new(),
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
        let job = CronJobDefinition::new(
            "ser_test".to_string(),
            "*/5 * * * *".to_string(),
            "US/Eastern".to_string(),
            config,
        );
        let json = serde_json::to_string(&job).unwrap();
        let deserialized: CronJobDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "ser_test");
        assert_eq!(deserialized.cron_expression, "*/5 * * * *");
        assert_eq!(deserialized.timezone, "US/Eastern");
    }
}
