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

/// Lightweight agent summary for list endpoints
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentSummary {
    /// The agent identifier
    pub id: AgentId,
    /// Agent name (from DSL definition)
    pub name: String,
    /// Current state
    pub state: AgentState,
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
    /// Agent metadata (present for external agents).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<std::collections::HashMap<String, String>>,
    /// Last result summary (present for external agents).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_result: Option<String>,
    /// Recent events (present for external agents).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recent_events: Option<Vec<AgentEvent>>,
    /// Execution mode label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<String>,
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
    /// DSL definition for the agent. Required for all modes except External.
    pub dsl: Option<String>,
    /// Execution mode. Defaults to Ephemeral if omitted.
    #[schema(value_type = Object)]
    pub execution_mode: Option<crate::types::agent::ExecutionMode>,
    /// Agent capabilities.
    pub capabilities: Option<Vec<String>>,
    /// Agent metadata key-value pairs.
    pub metadata: Option<std::collections::HashMap<String, String>>,
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

// ── External Agent API types ─────────────────────────────────────────

/// Event types reported by external agents.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub enum AgentEventType {
    RunStarted,
    RunCompleted,
    RunFailed,
}

/// A timestamped event from an external agent.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentEvent {
    pub event_type: AgentEventType,
    pub payload: serde_json::Value,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Per-external-agent state tracked by the scheduler.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone)]
pub struct ExternalAgentState {
    pub last_heartbeat: Option<chrono::DateTime<chrono::Utc>>,
    pub reported_state: crate::types::AgentState,
    pub metadata: std::collections::HashMap<String, String>,
    pub last_result: Option<String>,
    /// Ring buffer of recent events (max 100).
    pub events: std::collections::VecDeque<AgentEvent>,
}

#[cfg(feature = "http-api")]
impl Default for ExternalAgentState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "http-api")]
impl ExternalAgentState {
    pub fn new() -> Self {
        Self {
            last_heartbeat: None,
            reported_state: crate::types::AgentState::Created,
            metadata: std::collections::HashMap::new(),
            last_result: None,
            events: std::collections::VecDeque::new(),
        }
    }

    /// Push an event, keeping the ring buffer at most 100 entries.
    pub fn push_event(&mut self, event: AgentEvent) {
        if self.events.len() >= 100 {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }
}

/// Heartbeat request from an external agent.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HeartbeatRequest {
    /// Current agent state (e.g. Running, Completed, Failed).
    pub state: crate::types::AgentState,
    /// Optional metadata update.
    pub metadata: Option<std::collections::HashMap<String, String>>,
    /// Optional last result summary.
    pub last_result: Option<String>,
    /// Optional AgentPin JWT. Required when the runtime has AgentPin
    /// verification enabled; the JWT's `sub` must match the agent in
    /// the URL path.
    #[serde(default)]
    pub agentpin_jwt: Option<String>,
}

/// Push event request from an external agent.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PushEventRequest {
    pub event_type: AgentEventType,
    pub payload: serde_json::Value,
    /// Optional AgentPin JWT. Required when the runtime has AgentPin
    /// verification enabled; the JWT's `sub` must match the agent in
    /// the URL path.
    #[serde(default)]
    pub agentpin_jwt: Option<String>,
}

// ══════════════════════════════════════════════════════════════════
// Inter-agent messaging DTOs
// ══════════════════════════════════════════════════════════════════

/// Request to send a message to an agent.
///
/// The shell or another runtime instance POSTs this to
/// `/api/v1/agents/:id/messages` to deliver a message to agent `id`.
/// The payload is plaintext; the receiving runtime handles encryption
/// internally when placing the message in the agent's queue.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SendMessageRequest {
    /// The sender agent's identifier. Use the system agent ID when
    /// originated by an external caller without an agent context.
    pub sender: AgentId,
    /// Plaintext message payload.
    pub payload: String,
    /// Optional TTL in seconds. Defaults to 300 if omitted.
    #[serde(default)]
    pub ttl_seconds: Option<u64>,
    /// Optional topic for pub/sub messages. If present, publishes
    /// to the topic instead of direct delivery.
    #[serde(default)]
    pub topic: Option<String>,
    /// Optional AgentPin JWT.
    ///
    /// When the receiving runtime has AgentPin verification enabled, this
    /// is required and must verify against the runtime's configured
    /// discovery / trust bundle, and must carry a `sub` claim covering
    /// `sender`. When the receiving runtime has AgentPin disabled, this
    /// field is accepted (and logged) but not enforced. See
    /// `docs/security-model.md` for the full cross-runtime trust model.
    #[serde(default)]
    pub agentpin_jwt: Option<String>,
}

/// Response returned after successfully queuing a message.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SendMessageResponse {
    /// Unique identifier of the queued message.
    pub message_id: String,
    /// Current delivery status (typically "pending").
    pub status: String,
}

/// A message envelope returned to clients polling for messages.
///
/// Strips the encrypted payload — clients receive plaintext and the
/// bus-level encryption is opaque to them.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MessageEnvelope {
    /// Unique message identifier.
    pub message_id: String,
    /// Sender agent identifier.
    pub sender: AgentId,
    /// Optional recipient (None for broadcast).
    #[serde(default)]
    pub recipient: Option<AgentId>,
    /// Optional topic (present for pub/sub messages).
    #[serde(default)]
    pub topic: Option<String>,
    /// Plaintext payload.
    pub payload: String,
    /// Message type: "direct", "broadcast", "publish", "subscribe", "request", "response".
    pub message_type: String,
    /// Unix epoch seconds when the message was created.
    pub timestamp_secs: u64,
    /// TTL in seconds.
    pub ttl_seconds: u64,
}

/// Response with pending messages for an agent.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReceiveMessagesResponse {
    pub messages: Vec<MessageEnvelope>,
}

/// Delivery status response for a specific message.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MessageStatusResponse {
    pub message_id: String,
    /// One of: "pending", "delivered", "failed", "expired".
    pub status: String,
}

#[cfg(test)]
mod external_agent_tests {
    use super::*;

    #[test]
    fn test_external_agent_state_new() {
        let state = ExternalAgentState::new();
        assert!(state.last_heartbeat.is_none());
        assert_eq!(state.reported_state, crate::types::AgentState::Created);
        assert!(state.events.is_empty());
    }

    #[test]
    fn test_push_event_ring_buffer() {
        let mut state = ExternalAgentState::new();
        for i in 0..110 {
            state.push_event(AgentEvent {
                event_type: AgentEventType::RunStarted,
                payload: serde_json::json!({ "run": i }),
                timestamp: chrono::Utc::now(),
            });
        }
        assert_eq!(state.events.len(), 100);
        // Oldest event should be run 10 (0-9 evicted)
        assert_eq!(state.events.front().unwrap().payload["run"], 10);
    }
}
