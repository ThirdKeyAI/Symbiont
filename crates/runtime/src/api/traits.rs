//! API abstraction traits for the HTTP API
//!
//! This module defines traits that provide abstraction for core runtime functionalities
//! that can be exposed via the HTTP API.

#[cfg(feature = "http-api")]
use async_trait::async_trait;

#[cfg(feature = "http-api")]
use crate::types::{AgentId, RuntimeError};

#[cfg(feature = "http-api")]
use super::types::{AgentStatusResponse, CreateAgentRequest, CreateAgentResponse, DeleteAgentResponse, ExecuteAgentRequest, ExecuteAgentResponse, GetAgentHistoryResponse, UpdateAgentRequest, UpdateAgentResponse, WorkflowExecutionRequest};

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
        agent_id: String,
        request: UpdateAgentRequest,
    ) -> Result<UpdateAgentResponse, RuntimeError>;

    /// Delete an existing agent
    async fn delete_agent(
        &self,
        agent_id: String,
    ) -> Result<DeleteAgentResponse, RuntimeError>;

    /// Execute an agent with the given request
    async fn execute_agent(
        &self,
        agent_id: String,
        request: ExecuteAgentRequest,
    ) -> Result<ExecuteAgentResponse, RuntimeError>;

    /// Get execution history for an agent
    async fn get_agent_history(
        &self,
        agent_id: String,
    ) -> Result<GetAgentHistoryResponse, RuntimeError>;
}
