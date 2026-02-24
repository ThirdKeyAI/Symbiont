//! HTTP API specific data structures
//!
//! This module defines data structures used specifically for HTTP API communication.

#[cfg(feature = "http-api")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "http-api")]
use utoipa::ToSchema;

#[cfg(feature = "http-api")]
use crate::types::{AgentId, AgentState};

/// Request structure for workflow execution
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WorkflowExecutionRequest {
    /// The workflow definition or identifier
    pub workflow_id: String,
    /// Parameters to pass to the workflow
    pub parameters: serde_json::Value,
    /// Optional agent ID to execute the workflow
    pub agent_id: Option<AgentId>,
}

/// Response structure for agent status queries
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentStatusResponse {
    /// The agent identifier
    pub agent_id: AgentId,
    /// Current status of the agent
    pub state: AgentState,
    /// Last activity timestamp
    pub last_activity: chrono::DateTime<chrono::Utc>,
    /// Current resource usage
    pub resource_usage: ResourceUsage,
}

/// Resource usage information
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ResourceUsage {
    /// Memory usage in bytes
    pub memory_bytes: u64,
    /// CPU usage percentage
    pub cpu_percent: f64,
    /// Number of active tasks
    pub active_tasks: u32,
}

/// Health check response
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    /// Overall system status
    pub status: String,
    /// System uptime in seconds
    pub uptime_seconds: u64,
    /// Current timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Version information
    pub version: String,
}

/// Scheduler health response
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SchedulerHealthResponse {
    pub is_running: bool,
    pub store_accessible: bool,
    pub jobs_total: usize,
    pub jobs_active: usize,
    pub jobs_paused: usize,
    pub jobs_dead_letter: usize,
    pub global_active_runs: usize,
    pub max_concurrent: usize,
    pub runs_total: u64,
    pub runs_succeeded: u64,
    pub runs_failed: u64,
    pub average_execution_time_ms: f64,
    pub longest_run_ms: u64,
}

/// Request structure for creating a new agent
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateAgentRequest {
    /// Name of the agent
    pub name: String,
    /// DSL definition for the agent
    pub dsl: String,
}

/// Response structure for agent creation
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateAgentResponse {
    /// Unique identifier for the created agent
    pub id: String,
    /// Status of the agent creation
    pub status: String,
}

/// Request structure for updating an existing agent
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateAgentRequest {
    /// Optional name of the agent
    pub name: Option<String>,
    /// Optional DSL definition for the agent
    pub dsl: Option<String>,
}

/// Response structure for agent update
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateAgentResponse {
    /// Unique identifier for the updated agent
    pub id: String,
    /// Status of the agent update
    pub status: String,
}

/// Response structure for agent deletion
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DeleteAgentResponse {
    /// Unique identifier for the deleted agent
    pub id: String,
    /// Status of the agent deletion
    pub status: String,
}

/// Request structure for executing an agent
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ExecuteAgentRequest {
    // Empty struct for now as specified
}

/// Response structure for agent execution
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ExecuteAgentResponse {
    /// Unique identifier for the execution
    pub execution_id: String,
    /// Status of the agent execution
    pub status: String,
}

/// Agent execution record for history tracking
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentExecutionRecord {
    /// Unique identifier for the execution
    pub execution_id: String,
    /// Status of the execution
    pub status: String,
    /// Timestamp of the execution
    pub timestamp: String,
}

/// Response structure for agent execution history
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GetAgentHistoryResponse {
    /// List of execution records
    pub history: Vec<AgentExecutionRecord>,
}

/// Error response structure
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ErrorResponse {
    /// Error message
    pub error: String,
    /// Error code
    pub code: String,
    /// Optional details
    pub details: Option<serde_json::Value>,
}

// ── Schedule/Cron API types ──────────────────────────────────────────

/// Request to create a new scheduled job.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateScheduleRequest {
    /// Human-readable name for the job.
    pub name: String,
    /// Six-field cron expression: `sec min hour day month weekday`.
    ///
    /// An optional seventh field (year) is also accepted.
    /// Example: `"0 */5 * * * *"` = every 5 minutes.
    #[schema(example = "0 */5 * * * *")]
    pub cron_expression: String,
    /// IANA timezone (e.g. "America/New_York"). Defaults to "UTC".
    #[serde(default = "default_timezone")]
    pub timezone: String,
    /// Name of the agent to execute.
    pub agent_name: String,
    /// Policy IDs to attach.
    #[serde(default)]
    pub policy_ids: Vec<String>,
    /// Run once then disable.
    #[serde(default)]
    pub one_shot: bool,
}

#[cfg(feature = "http-api")]
fn default_timezone() -> String {
    "UTC".to_string()
}

/// Response after creating a schedule.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateScheduleResponse {
    /// The UUID of the new job.
    pub job_id: String,
    /// Computed next run time (UTC ISO-8601).
    pub next_run: Option<String>,
    pub status: String,
}

/// Request to update an existing schedule.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateScheduleRequest {
    /// New cron expression (6-field: `sec min hour day month weekday`; optional 7th field: year).
    #[schema(example = "0 */10 * * * *")]
    pub cron_expression: Option<String>,
    /// New timezone.
    pub timezone: Option<String>,
    /// New policy IDs (replaces existing).
    pub policy_ids: Option<Vec<String>>,
    /// Change one-shot flag.
    pub one_shot: Option<bool>,
}

/// Summary of a scheduled job (for list endpoint).
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ScheduleSummary {
    pub job_id: String,
    pub name: String,
    pub cron_expression: String,
    pub timezone: String,
    pub status: String,
    pub enabled: bool,
    pub next_run: Option<String>,
    pub run_count: u64,
}

/// Detailed schedule information.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ScheduleDetail {
    pub job_id: String,
    pub name: String,
    pub cron_expression: String,
    pub timezone: String,
    pub status: String,
    pub enabled: bool,
    pub one_shot: bool,
    pub next_run: Option<String>,
    pub last_run: Option<String>,
    pub run_count: u64,
    pub failure_count: u64,
    pub created_at: String,
    pub updated_at: String,
}

/// Response for listing next N run times.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NextRunsResponse {
    pub job_id: String,
    pub next_runs: Vec<String>,
}

/// A single run history entry (API view).
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ScheduleRunEntry {
    pub run_id: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub execution_time_ms: Option<u64>,
}

/// Response for schedule history endpoint.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ScheduleHistoryResponse {
    pub job_id: String,
    pub history: Vec<ScheduleRunEntry>,
}

/// Generic status response for pause/resume/trigger.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ScheduleActionResponse {
    pub job_id: String,
    pub action: String,
    pub status: String,
}

/// Response for deleting a schedule.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DeleteScheduleResponse {
    pub job_id: String,
    pub deleted: bool,
}

// ── Channel API types ──────────────────────────────────────────

/// Request to register a new channel adapter.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RegisterChannelRequest {
    /// Human-readable name for this channel.
    pub name: String,
    /// Platform identifier ("slack", "teams", "mattermost").
    pub platform: String,
    /// Platform-specific configuration JSON.
    pub config: serde_json::Value,
}

/// Response after registering a channel.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RegisterChannelResponse {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub status: String,
}

/// Request to update an existing channel.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateChannelRequest {
    pub config: Option<serde_json::Value>,
}

/// Summary of a channel adapter (for list endpoint).
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChannelSummary {
    pub id: String,
    pub name: String,
    pub platform: String,
    /// Current status: "running", "stopped", "error".
    pub status: String,
}

/// Detailed channel adapter information.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChannelDetail {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub status: String,
    pub config: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

/// Generic action response for start/stop.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChannelActionResponse {
    pub id: String,
    pub action: String,
    pub status: String,
}

/// Response for deleting a channel.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DeleteChannelResponse {
    pub id: String,
    pub deleted: bool,
}

/// Channel health and connectivity information.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChannelHealthResponse {
    pub id: String,
    pub connected: bool,
    pub platform: String,
    pub workspace_name: Option<String>,
    pub channels_active: usize,
    pub last_message_at: Option<String>,
    pub uptime_secs: u64,
}

// ── Channel enterprise types (always compiled, gated at runtime) ────

/// Identity mapping between a platform user and a Symbiont user.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IdentityMappingEntry {
    pub platform_user_id: String,
    pub platform: String,
    pub symbiont_user_id: String,
    pub email: Option<String>,
    pub display_name: String,
    pub roles: Vec<String>,
    pub verified: bool,
    pub created_at: String,
}

/// Request to add an identity mapping.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AddIdentityMappingRequest {
    pub platform_user_id: String,
    pub symbiont_user_id: String,
    pub email: Option<String>,
    pub display_name: String,
    pub roles: Vec<String>,
}

/// A single channel audit log entry.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChannelAuditEntry {
    pub timestamp: String,
    pub event_type: String,
    pub user_id: Option<String>,
    pub channel_id: Option<String>,
    pub agent: Option<String>,
    pub details: serde_json::Value,
}

/// Response for channel audit log queries.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChannelAuditResponse {
    pub channel_id: String,
    pub entries: Vec<ChannelAuditEntry>,
}
