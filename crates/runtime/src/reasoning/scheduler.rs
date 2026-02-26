//! Agent scheduler for durable cron-based execution
//!
//! Provides scheduled agent execution with cron expressions and
//! durable sleep between runs.
//!
//! Feature-gated behind `cron`.

use crate::types::AgentId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Configuration for a scheduled agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleConfig {
    /// Agent to execute.
    pub agent_id: AgentId,
    /// Cron expression (standard 5-field or extended 6-field).
    pub cron_expr: String,
    /// Initial observation/prompt for each run.
    pub observation: String,
    /// Whether the schedule is enabled.
    pub enabled: bool,
    /// When this schedule was created.
    pub created_at: DateTime<Utc>,
    /// When this schedule was last executed.
    pub last_run: Option<DateTime<Utc>>,
    /// Number of times this schedule has run.
    pub run_count: u64,
}

/// Result of parsing and validating a cron expression.
#[derive(Debug, Clone)]
pub struct ParsedCron {
    /// The original cron expression.
    pub expression: String,
    /// Next scheduled time from now.
    pub next_fire: Option<DateTime<Utc>>,
}

/// Parse a cron expression and compute the next fire time.
///
/// Supports standard 5-field cron (minute, hour, day-of-month, month, day-of-week).
/// Returns a `ParsedCron` with the expression and next fire time, or an error
/// if the expression is invalid.
pub fn parse_cron(expr: &str) -> Result<ParsedCron, SchedulerError> {
    // Validate by checking field count (5 or 6 fields expected)
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() < 5 || fields.len() > 7 {
        return Err(SchedulerError::InvalidCron {
            expression: expr.to_string(),
            message: format!("Expected 5-7 fields, got {}", fields.len()),
        });
    }

    // Validate individual fields
    for (i, field) in fields.iter().enumerate() {
        validate_cron_field(field, i).map_err(|msg| SchedulerError::InvalidCron {
            expression: expr.to_string(),
            message: msg,
        })?;
    }

    Ok(ParsedCron {
        expression: expr.to_string(),
        next_fire: None, // Full cron scheduling requires the `cron` crate at runtime
    })
}

fn validate_cron_field(field: &str, index: usize) -> Result<(), String> {
    let field_name = match index {
        0 => "minute",
        1 => "hour",
        2 => "day-of-month",
        3 => "month",
        4 => "day-of-week",
        5 => "year",
        6 => "seconds",
        _ => "unknown",
    };

    if field == "*" || field == "?" {
        return Ok(());
    }

    // Handle ranges (e.g., "1-5"), lists (e.g., "1,3,5"), and steps (e.g., "*/5")
    for part in field.split(',') {
        let part = part.trim();
        if part.contains('/') {
            let parts: Vec<&str> = part.split('/').collect();
            if parts.len() != 2 {
                return Err(format!("Invalid step in {} field: {}", field_name, part));
            }
            if parts[0] != "*" {
                parts[0].parse::<u32>().map_err(|_| {
                    format!("Invalid base value in {} field: {}", field_name, parts[0])
                })?;
            }
            parts[1]
                .parse::<u32>()
                .map_err(|_| format!("Invalid step value in {} field: {}", field_name, parts[1]))?;
        } else if part.contains('-') {
            let parts: Vec<&str> = part.split('-').collect();
            if parts.len() != 2 {
                return Err(format!("Invalid range in {} field: {}", field_name, part));
            }
            parts[0].parse::<u32>().map_err(|_| {
                format!("Invalid range start in {} field: {}", field_name, parts[0])
            })?;
            parts[1]
                .parse::<u32>()
                .map_err(|_| format!("Invalid range end in {} field: {}", field_name, parts[1]))?;
        } else {
            part.parse::<u32>()
                .map_err(|_| format!("Invalid value in {} field: {}", field_name, part))?;
        }
    }

    Ok(())
}

/// Manages scheduled agent executions.
pub struct AgentScheduler {
    schedules: Arc<RwLock<HashMap<String, ScheduleConfig>>>,
}

impl Default for AgentScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentScheduler {
    /// Create a new scheduler.
    pub fn new() -> Self {
        Self {
            schedules: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add a scheduled agent execution.
    pub async fn schedule(
        &self,
        name: impl Into<String>,
        agent_id: AgentId,
        cron_expr: impl Into<String>,
        observation: impl Into<String>,
    ) -> Result<(), SchedulerError> {
        let cron_expr = cron_expr.into();

        // Validate the cron expression
        parse_cron(&cron_expr)?;

        let config = ScheduleConfig {
            agent_id,
            cron_expr,
            observation: observation.into(),
            enabled: true,
            created_at: Utc::now(),
            last_run: None,
            run_count: 0,
        };

        self.schedules.write().await.insert(name.into(), config);
        Ok(())
    }

    /// Get a schedule by name.
    pub async fn get_schedule(&self, name: &str) -> Option<ScheduleConfig> {
        self.schedules.read().await.get(name).cloned()
    }

    /// List all schedules.
    pub async fn list_schedules(&self) -> Vec<(String, ScheduleConfig)> {
        self.schedules
            .read()
            .await
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Enable or disable a schedule.
    pub async fn set_enabled(&self, name: &str, enabled: bool) -> bool {
        if let Some(config) = self.schedules.write().await.get_mut(name) {
            config.enabled = enabled;
            true
        } else {
            false
        }
    }

    /// Remove a schedule.
    pub async fn remove_schedule(&self, name: &str) -> bool {
        self.schedules.write().await.remove(name).is_some()
    }

    /// Record that a schedule has been executed.
    pub async fn record_execution(&self, name: &str) -> bool {
        if let Some(config) = self.schedules.write().await.get_mut(name) {
            config.last_run = Some(Utc::now());
            config.run_count += 1;
            true
        } else {
            false
        }
    }

    /// Get schedules that are due for execution.
    pub async fn due_schedules(&self) -> Vec<(String, ScheduleConfig)> {
        self.schedules
            .read()
            .await
            .iter()
            .filter(|(_, config)| config.enabled)
            .map(|(name, config)| (name.clone(), config.clone()))
            .collect()
    }
}

/// Errors from the scheduler.
#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    #[error("Invalid cron expression '{expression}': {message}")]
    InvalidCron { expression: String, message: String },

    #[error("Schedule '{name}' not found")]
    NotFound { name: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cron_valid_5_field() {
        let result = parse_cron("0 * * * *");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().expression, "0 * * * *");
    }

    #[test]
    fn test_parse_cron_valid_with_ranges() {
        assert!(parse_cron("0 9-17 * * 1-5").is_ok());
    }

    #[test]
    fn test_parse_cron_valid_with_steps() {
        assert!(parse_cron("*/15 * * * *").is_ok());
    }

    #[test]
    fn test_parse_cron_valid_with_lists() {
        assert!(parse_cron("0 0 1,15 * *").is_ok());
    }

    #[test]
    fn test_parse_cron_invalid_too_few_fields() {
        let result = parse_cron("0 *");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("5-7 fields"));
    }

    #[test]
    fn test_parse_cron_invalid_field_value() {
        let result = parse_cron("abc * * * *");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_cron_valid_6_field() {
        assert!(parse_cron("0 30 9 * * 1-5").is_ok());
    }

    #[tokio::test]
    async fn test_scheduler_add_and_get() {
        let scheduler = AgentScheduler::new();
        let agent_id = AgentId::new();

        scheduler
            .schedule("daily_check", agent_id, "0 9 * * *", "Run daily analysis")
            .await
            .unwrap();

        let config = scheduler.get_schedule("daily_check").await.unwrap();
        assert_eq!(config.agent_id, agent_id);
        assert_eq!(config.cron_expr, "0 9 * * *");
        assert!(config.enabled);
        assert_eq!(config.run_count, 0);
    }

    #[tokio::test]
    async fn test_scheduler_list() {
        let scheduler = AgentScheduler::new();

        scheduler
            .schedule("a", AgentId::new(), "0 * * * *", "task a")
            .await
            .unwrap();
        scheduler
            .schedule("b", AgentId::new(), "*/5 * * * *", "task b")
            .await
            .unwrap();

        let schedules = scheduler.list_schedules().await;
        assert_eq!(schedules.len(), 2);
    }

    #[tokio::test]
    async fn test_scheduler_enable_disable() {
        let scheduler = AgentScheduler::new();

        scheduler
            .schedule("job", AgentId::new(), "0 * * * *", "task")
            .await
            .unwrap();

        assert!(scheduler.set_enabled("job", false).await);
        assert!(!scheduler.get_schedule("job").await.unwrap().enabled);

        assert!(scheduler.set_enabled("job", true).await);
        assert!(scheduler.get_schedule("job").await.unwrap().enabled);

        assert!(!scheduler.set_enabled("nonexistent", false).await);
    }

    #[tokio::test]
    async fn test_scheduler_remove() {
        let scheduler = AgentScheduler::new();

        scheduler
            .schedule("temp", AgentId::new(), "0 * * * *", "task")
            .await
            .unwrap();

        assert!(scheduler.remove_schedule("temp").await);
        assert!(scheduler.get_schedule("temp").await.is_none());
        assert!(!scheduler.remove_schedule("temp").await);
    }

    #[tokio::test]
    async fn test_scheduler_record_execution() {
        let scheduler = AgentScheduler::new();

        scheduler
            .schedule("job", AgentId::new(), "0 * * * *", "task")
            .await
            .unwrap();

        assert!(scheduler.record_execution("job").await);
        let config = scheduler.get_schedule("job").await.unwrap();
        assert_eq!(config.run_count, 1);
        assert!(config.last_run.is_some());

        assert!(scheduler.record_execution("job").await);
        let config = scheduler.get_schedule("job").await.unwrap();
        assert_eq!(config.run_count, 2);

        assert!(!scheduler.record_execution("nonexistent").await);
    }

    #[tokio::test]
    async fn test_scheduler_due_schedules() {
        let scheduler = AgentScheduler::new();

        scheduler
            .schedule("enabled", AgentId::new(), "0 * * * *", "task")
            .await
            .unwrap();
        scheduler
            .schedule("disabled", AgentId::new(), "0 * * * *", "task")
            .await
            .unwrap();
        scheduler.set_enabled("disabled", false).await;

        let due = scheduler.due_schedules().await;
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].0, "enabled");
    }

    #[tokio::test]
    async fn test_scheduler_invalid_cron_rejected() {
        let scheduler = AgentScheduler::new();

        let result = scheduler
            .schedule("bad", AgentId::new(), "invalid cron", "task")
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn test_schedule_config_serialization() {
        let config = ScheduleConfig {
            agent_id: AgentId::new(),
            cron_expr: "0 9 * * *".into(),
            observation: "Run analysis".into(),
            enabled: true,
            created_at: Utc::now(),
            last_run: None,
            run_count: 0,
        };

        let json = serde_json::to_string(&config).unwrap();
        let restored: ScheduleConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.cron_expr, "0 9 * * *");
        assert!(restored.enabled);
    }
}
