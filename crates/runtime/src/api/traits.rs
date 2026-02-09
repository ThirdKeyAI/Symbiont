//! API abstraction traits for the HTTP API
//!
//! This module defines traits that provide abstraction for core runtime functionalities
//! that can be exposed via the HTTP API.

#[cfg(feature = "http-api")]
use async_trait::async_trait;

#[cfg(feature = "http-api")]
use crate::types::{AgentId, RuntimeError};

#[cfg(feature = "http-api")]
use super::types::{
    AddIdentityMappingRequest, AgentStatusResponse, ChannelActionResponse, ChannelAuditResponse,
    ChannelDetail, ChannelHealthResponse, ChannelSummary, CreateAgentRequest, CreateAgentResponse,
    CreateScheduleRequest, CreateScheduleResponse, DeleteAgentResponse, DeleteChannelResponse,
    DeleteScheduleResponse, ExecuteAgentRequest, ExecuteAgentResponse, GetAgentHistoryResponse,
    IdentityMappingEntry, NextRunsResponse, RegisterChannelRequest, RegisterChannelResponse,
    ScheduleActionResponse, ScheduleDetail, ScheduleHistoryResponse, ScheduleSummary,
    SchedulerHealthResponse, UpdateAgentRequest, UpdateAgentResponse, UpdateChannelRequest,
    UpdateScheduleRequest, WorkflowExecutionRequest,
};

/// Trait providing API access to core runtime functionalities
#[cfg(feature = "http-api")]
#[async_trait]
pub trait RuntimeApiProvider: Send + Sync {
    /// Execute a workflow with the given parameters
    async fn execute_workflow(
        &self,
        request: WorkflowExecutionRequest,
    ) -> Result<serde_json::Value, RuntimeError>;

    /// Get the current status of a specific agent
    async fn get_agent_status(
        &self,
        agent_id: AgentId,
    ) -> Result<AgentStatusResponse, RuntimeError>;

    /// Get system health information
    async fn get_system_health(&self) -> Result<serde_json::Value, RuntimeError>;

    /// List all active agents
    async fn list_agents(&self) -> Result<Vec<AgentId>, RuntimeError>;

    /// Shutdown an agent gracefully
    async fn shutdown_agent(&self, agent_id: AgentId) -> Result<(), RuntimeError>;

    /// Get runtime system metrics
    async fn get_metrics(&self) -> Result<serde_json::Value, RuntimeError>;

    /// Create a new agent with the given configuration
    async fn create_agent(
        &self,
        request: CreateAgentRequest,
    ) -> Result<CreateAgentResponse, RuntimeError>;

    /// Update an existing agent with the given configuration
    async fn update_agent(
        &self,
        agent_id: AgentId,
        request: UpdateAgentRequest,
    ) -> Result<UpdateAgentResponse, RuntimeError>;

    /// Delete an existing agent
    async fn delete_agent(&self, agent_id: AgentId) -> Result<DeleteAgentResponse, RuntimeError>;

    /// Execute an agent with the given request
    async fn execute_agent(
        &self,
        agent_id: AgentId,
        request: ExecuteAgentRequest,
    ) -> Result<ExecuteAgentResponse, RuntimeError>;

    /// Get execution history for an agent
    async fn get_agent_history(
        &self,
        agent_id: AgentId,
    ) -> Result<GetAgentHistoryResponse, RuntimeError>;

    // ── Schedule endpoints ──────────────────────────────────────────

    /// List all scheduled jobs.
    async fn list_schedules(&self) -> Result<Vec<ScheduleSummary>, RuntimeError>;

    /// Create a new scheduled job.
    async fn create_schedule(
        &self,
        request: CreateScheduleRequest,
    ) -> Result<CreateScheduleResponse, RuntimeError>;

    /// Get details of a scheduled job.
    async fn get_schedule(&self, job_id: &str) -> Result<ScheduleDetail, RuntimeError>;

    /// Update a scheduled job.
    async fn update_schedule(
        &self,
        job_id: &str,
        request: UpdateScheduleRequest,
    ) -> Result<ScheduleDetail, RuntimeError>;

    /// Delete a scheduled job.
    async fn delete_schedule(&self, job_id: &str) -> Result<DeleteScheduleResponse, RuntimeError>;

    /// Pause a scheduled job.
    async fn pause_schedule(&self, job_id: &str) -> Result<ScheduleActionResponse, RuntimeError>;

    /// Resume a paused scheduled job.
    async fn resume_schedule(&self, job_id: &str) -> Result<ScheduleActionResponse, RuntimeError>;

    /// Force-trigger a scheduled job immediately.
    async fn trigger_schedule(&self, job_id: &str) -> Result<ScheduleActionResponse, RuntimeError>;

    /// Get run history for a scheduled job.
    async fn get_schedule_history(
        &self,
        job_id: &str,
        limit: usize,
    ) -> Result<ScheduleHistoryResponse, RuntimeError>;

    /// Get next N computed run times for a job.
    async fn get_schedule_next_runs(
        &self,
        job_id: &str,
        count: usize,
    ) -> Result<NextRunsResponse, RuntimeError>;

    /// Get scheduler health and metrics.
    async fn get_scheduler_health(&self) -> Result<SchedulerHealthResponse, RuntimeError>;

    // ── Channel endpoints ──────────────────────────────────────────

    /// List all registered channel adapters.
    async fn list_channels(&self) -> Result<Vec<ChannelSummary>, RuntimeError>;

    /// Register a new channel adapter.
    async fn register_channel(
        &self,
        request: RegisterChannelRequest,
    ) -> Result<RegisterChannelResponse, RuntimeError>;

    /// Get details of a channel adapter.
    async fn get_channel(&self, id: &str) -> Result<ChannelDetail, RuntimeError>;

    /// Update a channel adapter configuration.
    async fn update_channel(
        &self,
        id: &str,
        request: UpdateChannelRequest,
    ) -> Result<ChannelDetail, RuntimeError>;

    /// Delete a channel adapter.
    async fn delete_channel(&self, id: &str) -> Result<DeleteChannelResponse, RuntimeError>;

    /// Start a channel adapter.
    async fn start_channel(&self, id: &str) -> Result<ChannelActionResponse, RuntimeError>;

    /// Stop a channel adapter.
    async fn stop_channel(&self, id: &str) -> Result<ChannelActionResponse, RuntimeError>;

    /// Get health and connectivity info for a channel adapter.
    async fn get_channel_health(&self, id: &str) -> Result<ChannelHealthResponse, RuntimeError>;

    // ── Channel enterprise endpoints (return NotImplemented for community) ──

    /// List identity mappings for a channel.
    async fn list_channel_mappings(
        &self,
        id: &str,
    ) -> Result<Vec<IdentityMappingEntry>, RuntimeError>;

    /// Add an identity mapping to a channel.
    async fn add_channel_mapping(
        &self,
        id: &str,
        request: AddIdentityMappingRequest,
    ) -> Result<IdentityMappingEntry, RuntimeError>;

    /// Remove an identity mapping from a channel.
    async fn remove_channel_mapping(&self, id: &str, user_id: &str) -> Result<(), RuntimeError>;

    /// Get audit log entries for a channel.
    async fn get_channel_audit(
        &self,
        id: &str,
        limit: usize,
    ) -> Result<ChannelAuditResponse, RuntimeError>;
}
