//! HTTP API specific data structures
//! 
//! This module defines data structures used specifically for HTTP API communication.

#[cfg(feature = "http-api")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "http-api")]
use crate::types::{AgentId, AgentState};

/// Request structure for workflow execution
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Error response structure
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    /// Error message
    pub error: String,
    /// Error code
    pub code: String,
    /// Optional details
    pub details: Option<serde_json::Value>,
}