//! Resource management types and data structures

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

use super::AgentId;

/// Resource limits for an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    pub memory_mb: usize,
    pub cpu_cores: f32,
    pub disk_io_mbps: usize,
    pub network_io_mbps: usize,
    pub execution_timeout: Duration,
    pub idle_timeout: Duration,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            memory_mb: 512,
            cpu_cores: 1.0,
            disk_io_mbps: 100,
            network_io_mbps: 100,
            execution_timeout: Duration::from_secs(3600), // 1 hour
            idle_timeout: Duration::from_secs(300),       // 5 minutes
        }
    }
}

/// Current resource usage by an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceUsage {
    pub memory_used: usize,
    pub cpu_utilization: f32,
    pub disk_io_rate: usize,
    pub network_io_rate: usize,
    pub uptime: Duration,
}

impl Default for ResourceUsage {
    fn default() -> Self {
        Self {
            memory_used: 0,
            cpu_utilization: 0.0,
            disk_io_rate: 0,
            network_io_rate: 0,
            uptime: Duration::from_secs(0),
        }
    }
}

/// Resource allocation for an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceAllocation {
    pub agent_id: AgentId,
    pub allocated_memory: usize,
    pub allocated_cpu_cores: f32,
    pub allocated_disk_io: usize,
    pub allocated_network_io: usize,
    pub allocation_time: std::time::SystemTime,
}

/// System-wide resource pool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourcePool {
    pub total_memory: usize,
    pub total_cpu_cores: usize,
    pub available_memory: usize,
    pub available_cpu_cores: usize,
    pub allocated_agents: HashMap<AgentId, ResourceAllocation>,
}

impl ResourcePool {
    pub fn new(total_memory: usize, total_cpu_cores: usize) -> Self {
        Self {
            total_memory,
            total_cpu_cores,
            available_memory: total_memory,
            available_cpu_cores: total_cpu_cores,
            allocated_agents: HashMap::new(),
        }
    }

    pub fn can_allocate(&self, limits: &ResourceLimits) -> bool {
        self.available_memory >= limits.memory_mb
            && self.available_cpu_cores >= limits.cpu_cores as usize
    }

    pub fn allocate(
        &mut self,
        agent_id: AgentId,
        limits: &ResourceLimits,
    ) -> Option<ResourceAllocation> {
        if !self.can_allocate(limits) {
            return None;
        }

        let allocation = ResourceAllocation {
            agent_id,
            allocated_memory: limits.memory_mb,
            allocated_cpu_cores: limits.cpu_cores,
            allocated_disk_io: limits.disk_io_mbps,
            allocated_network_io: limits.network_io_mbps,
            allocation_time: std::time::SystemTime::now(),
        };

        self.available_memory -= limits.memory_mb;
        self.available_cpu_cores -= limits.cpu_cores as usize;
        self.allocated_agents.insert(agent_id, allocation.clone());

        Some(allocation)
    }

    pub fn deallocate(&mut self, agent_id: AgentId) -> Option<ResourceAllocation> {
        if let Some(allocation) = self.allocated_agents.remove(&agent_id) {
            self.available_memory += allocation.allocated_memory;
            self.available_cpu_cores += allocation.allocated_cpu_cores as usize;
            Some(allocation)
        } else {
            None
        }
    }

    pub fn get_utilization(&self) -> ResourceUtilization {
        let memory_utilization = if self.total_memory > 0 {
            (self.total_memory - self.available_memory) as f32 / self.total_memory as f32
        } else {
            0.0
        };

        let cpu_utilization = if self.total_cpu_cores > 0 {
            (self.total_cpu_cores as f32 - self.available_cpu_cores as f32)
                / self.total_cpu_cores as f32
        } else {
            0.0
        };

        ResourceUtilization {
            memory_utilization,
            cpu_utilization,
            active_agents: self.allocated_agents.len(),
        }
    }
}

/// Resource utilization metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceUtilization {
    pub memory_utilization: f32,
    pub cpu_utilization: f32,
    pub active_agents: usize,
}

/// Resource allocation strategies
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AllocationStrategy {
    /// First available resource slot
    FirstFit,
    /// Optimal resource utilization
    #[default]
    BestFit,
    /// Load balancing across resources
    WorstFit,
    /// Priority-based allocation
    Priority,
}

/// Resource request from an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceRequest {
    pub agent_id: AgentId,
    pub requested_limits: ResourceLimits,
    pub priority: super::Priority,
    pub justification: Option<String>,
}

/// Alert thresholds for resource monitoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertThresholds {
    pub memory_warning: f32,  // 80%
    pub memory_critical: f32, // 95%
    pub cpu_warning: f32,     // 80%
    pub cpu_critical: f32,    // 95%
}

impl Default for AlertThresholds {
    fn default() -> Self {
        Self {
            memory_warning: 0.8,
            memory_critical: 0.95,
            cpu_warning: 0.8,
            cpu_critical: 0.95,
        }
    }
}

/// Resource monitoring configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceMonitorConfig {
    pub collection_interval: Duration,
    pub alert_thresholds: AlertThresholds,
    pub enable_detailed_metrics: bool,
    pub metrics_retention_duration: Duration,
}

impl Default for ResourceMonitorConfig {
    fn default() -> Self {
        Self {
            collection_interval: Duration::from_secs(30),
            alert_thresholds: AlertThresholds::default(),
            enable_detailed_metrics: true,
            metrics_retention_duration: Duration::from_secs(86400), // 24 hours
        }
    }
}
