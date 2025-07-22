//! Agent-related types and data structures

use std::collections::HashMap;
use std::time::{Duration, SystemTime};
use serde::{Deserialize, Serialize};

use super::{AgentId, Capability, Dependency, PolicyId, Priority};
use crate::types::resource::{ResourceLimits, ResourceAllocation};
use crate::types::security::SecurityTier;

/// Agent configuration for initialization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub id: AgentId,
    pub name: String,
    pub dsl_source: String,
    pub execution_mode: ExecutionMode,
    pub security_tier: SecurityTier,
    pub resource_limits: ResourceLimits,
    pub capabilities: Vec<Capability>,
    pub policies: Vec<PolicyId>,
    pub metadata: HashMap<String, String>,
    pub priority: Priority,
}

/// Agent execution modes
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[derive(Default)]
pub enum ExecutionMode {
    /// Long-lived agent that persists across tasks
    Persistent,
    /// Task-based execution that terminates after completion
    #[default]
    Ephemeral,
    /// Cron-like scheduling with periodic execution
    Scheduled { interval: Duration },
    /// Reactive to events and messages
    EventDriven,
}


/// Agent state in the lifecycle
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[derive(Default)]
pub enum AgentState {
    #[default]
    Created,
    Initializing,
    Ready,
    Running,
    Suspended,
    Waiting,
    Completed,
    Failed,
    Terminating,
    Terminated,
}


/// Agent metadata structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMetadata {
    pub version: String,
    pub author: String,
    pub description: String,
    pub capabilities: Vec<Capability>,
    pub dependencies: Vec<Dependency>,
    pub resource_requirements: ResourceRequirements,
    pub security_requirements: SecurityRequirements,
    pub custom_fields: HashMap<String, String>,
}

/// Resource requirements for an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceRequirements {
    pub min_memory_mb: usize,
    pub max_memory_mb: usize,
    pub min_cpu_cores: f32,
    pub max_cpu_cores: f32,
    pub disk_space_mb: usize,
    pub network_bandwidth_mbps: usize,
}

impl Default for ResourceRequirements {
    fn default() -> Self {
        Self {
            min_memory_mb: 64,
            max_memory_mb: 512,
            min_cpu_cores: 0.1,
            max_cpu_cores: 1.0,
            disk_space_mb: 100,
            network_bandwidth_mbps: 10,
        }
    }
}

/// Security requirements for an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityRequirements {
    pub min_security_tier: SecurityTier,
    pub requires_encryption: bool,
    pub requires_signature: bool,
    pub network_isolation: bool,
    pub file_system_isolation: bool,
}

impl Default for SecurityRequirements {
    fn default() -> Self {
        Self {
            min_security_tier: SecurityTier::Tier1,
            requires_encryption: true,
            requires_signature: true,
            network_isolation: true,
            file_system_isolation: true,
        }
    }
}

/// Runtime instance of an agent
#[derive(Debug, Clone)]
pub struct AgentInstance {
    pub id: AgentId,
    pub config: AgentConfig,
    pub state: AgentState,
    pub resource_allocation: Option<ResourceAllocation>,
    pub sandbox_handle: Option<SandboxHandle>,
    pub created_at: SystemTime,
    pub last_activity: SystemTime,
    pub execution_count: u64,
    pub last_state_change: SystemTime,
    pub error_count: u32,
    pub restart_count: u32,
    pub last_error: Option<String>,
}

impl AgentInstance {
    pub fn new(config: AgentConfig) -> Self {
        let now = SystemTime::now();
        Self {
            id: config.id,
            config,
            state: AgentState::Created,
            resource_allocation: None,
            sandbox_handle: None,
            created_at: now,
            last_activity: now,
            execution_count: 0,
            last_state_change: now,
            error_count: 0,
            restart_count: 0,
            last_error: None,
        }
    }

    pub fn update_activity(&mut self) {
        self.last_activity = SystemTime::now();
    }

    pub fn increment_execution(&mut self) {
        self.execution_count += 1;
        self.update_activity();
    }
}

/// Handle to a sandbox environment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxHandle {
    pub id: String,
    pub security_tier: SecurityTier,
    pub created_at: SystemTime,
    pub process_id: Option<u32>,
    pub container_id: Option<String>,
}

/// Reasons for agent termination
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminationReason {
    Completed,
    Failed(String),
    PolicyViolation(String),
    ResourceExhaustion,
    Timeout,
    ManualTermination,
    SystemShutdown,
}

impl std::fmt::Display for TerminationReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TerminationReason::Completed => write!(f, "Completed successfully"),
            TerminationReason::Failed(reason) => write!(f, "Failed: {}", reason),
            TerminationReason::PolicyViolation(policy) => write!(f, "Policy violation: {}", policy),
            TerminationReason::ResourceExhaustion => write!(f, "Resource exhaustion"),
            TerminationReason::Timeout => write!(f, "Execution timeout"),
            TerminationReason::ManualTermination => write!(f, "Manual termination"),
            TerminationReason::SystemShutdown => write!(f, "System shutdown"),
        }
    }
}