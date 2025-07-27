//! Core types and data structures for the Agent Runtime System

use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};
use uuid::Uuid;

pub mod agent;
pub mod communication;
pub mod error;
pub mod resource;
pub mod security;

pub use agent::*;
pub use communication::*;
pub use error::*;
pub use resource::*;
pub use security::*;

/// Unique identifier for agents
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub Uuid);

impl AgentId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for AgentId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for messages
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(pub Uuid);

impl std::fmt::Display for MessageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl MessageId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for MessageId {
    fn default() -> Self {
        Self::new()
    }
}

/// Unique identifier for requests
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RequestId(pub Uuid);

impl RequestId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for RequestId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for policies
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PolicyId(pub Uuid);

impl std::fmt::Display for PolicyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl PolicyId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for PolicyId {
    fn default() -> Self {
        Self::new()
    }
}

/// Unique identifier for audit events
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AuditId(pub Uuid);

impl AuditId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for AuditId {
    fn default() -> Self {
        Self::new()
    }
}

/// Priority levels for agent scheduling
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum Priority {
    Low = 0,
    #[default]
    Normal = 1,
    High = 2,
    Critical = 3,
}

/// System status information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemStatus {
    pub total_agents: usize,
    pub running_agents: usize,
    pub suspended_agents: usize,
    pub resource_utilization: ResourceUsage,
    pub uptime: Duration,
    pub last_updated: SystemTime,
}

/// Agent capabilities
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    FileSystem,
    Network,
    Database,
    Computation,
    Communication,
    Custom(String),
}

/// Agent dependencies
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    pub name: String,
    pub version: String,
    pub required: bool,
}

/// Scheduling algorithms
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SchedulingAlgorithm {
    FirstComeFirstServe,
    #[default]
    PriorityBased,
    RoundRobin,
    ShortestJobFirst,
    WeightedFairQueuing,
}

/// Load balancing strategies
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LoadBalancingStrategy {
    RoundRobin,
    LeastConnections,
    #[default]
    ResourceBased,
    WeightedRoundRobin,
}
