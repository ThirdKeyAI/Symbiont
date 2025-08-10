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
