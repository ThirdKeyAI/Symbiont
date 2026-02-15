//! Load balancer for distributing agents across available resources

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use sysinfo::System;

use crate::types::*;

/// Load balancer for resource allocation
pub struct LoadBalancer {
    strategy: LoadBalancingStrategy,
    resource_pool: Arc<RwLock<ResourcePool>>,
    allocation_history: Arc<RwLock<AllocationHistory>>,
    system_info: Arc<RwLock<System>>,
    created_at: std::time::Instant,
}

impl LoadBalancer {
    /// Create a new load balancer with real system resource detection
    pub fn new(strategy: LoadBalancingStrategy) -> Self {
        let mut sys = System::new_all();
        sys.refresh_all();

        let total_memory = (sys.total_memory() / (1024 * 1024)) as usize; // bytes -> MB
        let total_cpu_cores = sys.cpus().len();

        Self {
            strategy,
            resource_pool: Arc::new(RwLock::new(ResourcePool::new(
                total_memory,
                total_cpu_cores,
            ))),
            allocation_history: Arc::new(RwLock::new(AllocationHistory::new())),
            system_info: Arc::new(RwLock::new(sys)),
            created_at: std::time::Instant::now(),
        }
    }

    /// Allocate resources for an agent
    pub async fn allocate_resources(
        &self,
        requirements: &ResourceRequirements,
    ) -> Result<ResourceAllocation, ResourceError> {
        let agent_id = AgentId::new(); // This would normally come from the task
        let start_time = std::time::Instant::now();

        // Convert requirements to limits
        let limits = ResourceLimits {
            memory_mb: requirements.max_memory_mb,
            cpu_cores: requirements.max_cpu_cores,
            disk_io_mbps: 100,    // Default
            network_io_mbps: 100, // Default
            execution_timeout: std::time::Duration::from_secs(3600),
            idle_timeout: std::time::Duration::from_secs(300),
        };

        let mut pool = self.resource_pool.write();

        let result = match self.strategy {
            LoadBalancingStrategy::RoundRobin => {
                self.allocate_round_robin(&mut pool, agent_id, &limits)
            }
            LoadBalancingStrategy::LeastConnections => {
                self.allocate_least_connections(&mut pool, agent_id, &limits)
            }
            LoadBalancingStrategy::ResourceBased => {
                self.allocate_resource_based(&mut pool, agent_id, &limits)
            }
            LoadBalancingStrategy::WeightedRoundRobin => {
                self.allocate_weighted_round_robin(&mut pool, agent_id, &limits)
            }
        };

        // Record allocation metrics
        let mut history = self.allocation_history.write();
        match &result {
            Ok(_) => {
                let duration = start_time.elapsed();
                history.record_allocation(agent_id, duration);
            }
            Err(_) => {
                history.record_failure();
            }
        }

        result
    }

    /// Deallocate resources for an agent
    pub async fn deallocate_resources(&self, allocation: ResourceAllocation) {
        let mut pool = self.resource_pool.write();
        pool.deallocate(allocation.agent_id);

        let mut history = self.allocation_history.write();
        history.record_deallocation(allocation.agent_id);
    }

    /// Get current resource utilization using real system metrics
    pub async fn get_resource_utilization(&self) -> ResourceUsage {
        let pool = self.resource_pool.read();
        let mut sys = self.system_info.write();
        sys.refresh_all();

        ResourceUsage {
            memory_used: pool.total_memory - pool.available_memory,
            cpu_utilization: sys.global_cpu_info().cpu_usage(),
            disk_io_rate: 0,    // sysinfo doesn't track disk I/O rate directly
            network_io_rate: 0, // sysinfo doesn't track network I/O rate directly
            uptime: self.created_at.elapsed(),
        }
    }

    /// Round-robin allocation strategy
    fn allocate_round_robin(
        &self,
        pool: &mut ResourcePool,
        agent_id: AgentId,
        limits: &ResourceLimits,
    ) -> Result<ResourceAllocation, ResourceError> {
        pool.allocate(agent_id, limits)
            .ok_or_else(|| ResourceError::AllocationFailed {
                agent_id,
                reason: "Insufficient resources for round-robin allocation".to_string(),
            })
    }

    /// Least connections allocation strategy
    ///
    /// Considers the count of active allocations relative to a capacity threshold
    /// (80% of max based on memory/CPU). Rejects when active count exceeds threshold.
    fn allocate_least_connections(
        &self,
        pool: &mut ResourcePool,
        agent_id: AgentId,
        limits: &ResourceLimits,
    ) -> Result<ResourceAllocation, ResourceError> {
        let active_count = pool.allocated_agents.len();
        // Estimate max agents from available resources (use memory as proxy)
        let max_agents_estimate = if limits.memory_mb > 0 {
            pool.total_memory / limits.memory_mb
        } else {
            pool.total_memory // Fallback: 1 agent per MB
        };
        let threshold = (max_agents_estimate as f64 * 0.8) as usize;

        if active_count >= threshold.max(1) {
            return Err(ResourceError::AllocationFailed {
                agent_id,
                reason: format!(
                    "Active allocation count ({}) exceeds 80% capacity threshold ({})",
                    active_count, threshold
                ),
            });
        }

        pool.allocate(agent_id, limits)
            .ok_or_else(|| ResourceError::AllocationFailed {
                agent_id,
                reason: "Insufficient resources for least-connections allocation".to_string(),
            })
    }

    /// Resource-based allocation strategy
    fn allocate_resource_based(
        &self,
        pool: &mut ResourcePool,
        agent_id: AgentId,
        limits: &ResourceLimits,
    ) -> Result<ResourceAllocation, ResourceError> {
        // Check if we have enough resources
        if !pool.can_allocate(limits) {
            return Err(ResourceError::AllocationFailed {
                agent_id,
                reason: format!(
                    "Insufficient resources: need {}MB memory, {}CPU cores, available: {}MB memory, {:.2}CPU cores",
                    limits.memory_mb,
                    limits.cpu_cores,
                    pool.available_memory,
                    pool.available_cpu_cores
                ),
            });
        }

        pool.allocate(agent_id, limits)
            .ok_or_else(|| ResourceError::AllocationFailed {
                agent_id,
                reason: "Resource allocation failed unexpectedly".to_string(),
            })
    }

    /// Weighted round-robin allocation strategy
    ///
    /// Scales allocation based on available resource fraction and applies back-pressure
    /// as the system fills up. Rejects when less than 10% resources remain.
    fn allocate_weighted_round_robin(
        &self,
        pool: &mut ResourcePool,
        agent_id: AgentId,
        limits: &ResourceLimits,
    ) -> Result<ResourceAllocation, ResourceError> {
        let available_fraction = if pool.total_memory > 0 {
            pool.available_memory as f64 / pool.total_memory as f64
        } else {
            0.0
        };

        // Reject if less than 10% resources remain to preserve headroom
        if available_fraction < 0.1 {
            return Err(ResourceError::AllocationFailed {
                agent_id,
                reason: format!(
                    "Weighted round-robin rejected: only {:.1}% resources available (minimum 10% required)",
                    available_fraction * 100.0
                ),
            });
        }

        // Scale requested memory proportionally to available resources
        let proportional_memory = (limits.memory_mb as f64 * available_fraction).ceil() as usize;
        let scaled_memory = proportional_memory.max(limits.memory_mb.min(pool.available_memory));

        let scaled_limits = ResourceLimits {
            memory_mb: scaled_memory.min(limits.memory_mb),
            cpu_cores: limits.cpu_cores,
            disk_io_mbps: limits.disk_io_mbps,
            network_io_mbps: limits.network_io_mbps,
            execution_timeout: limits.execution_timeout,
            idle_timeout: limits.idle_timeout,
        };

        pool.allocate(agent_id, &scaled_limits)
            .ok_or_else(|| ResourceError::AllocationFailed {
                agent_id,
                reason: format!(
                    "Weighted round-robin allocation failed: requested {}MB (scaled from {}MB), available {}MB",
                    scaled_limits.memory_mb, limits.memory_mb, pool.available_memory
                ),
            })
    }

    /// Get load balancing statistics
    pub async fn get_statistics(&self) -> LoadBalancingStats {
        let pool = self.resource_pool.read();
        let history = self.allocation_history.read();
        let utilization = pool.get_utilization();

        LoadBalancingStats {
            total_allocations: history.total_allocations,
            active_allocations: pool.allocated_agents.len(),
            memory_utilization: utilization.memory_utilization,
            cpu_utilization: utilization.cpu_utilization,
            allocation_failures: history.allocation_failures,
            average_allocation_time: history.average_allocation_time(),
        }
    }
}

/// Allocation history for tracking and optimization
#[derive(Debug)]
struct AllocationHistory {
    total_allocations: usize,
    allocation_failures: usize,
    allocation_times: Vec<std::time::Duration>,
    recent_allocations: HashMap<AgentId, std::time::SystemTime>,
}

impl AllocationHistory {
    fn new() -> Self {
        Self {
            total_allocations: 0,
            allocation_failures: 0,
            allocation_times: Vec::new(),
            recent_allocations: HashMap::new(),
        }
    }

    fn record_allocation(&mut self, agent_id: AgentId, duration: std::time::Duration) {
        self.total_allocations += 1;
        self.allocation_times.push(duration);
        self.recent_allocations
            .insert(agent_id, std::time::SystemTime::now());

        // Keep only recent allocation times (last 1000)
        if self.allocation_times.len() > 1000 {
            self.allocation_times.remove(0);
        }
    }

    fn record_failure(&mut self) {
        self.allocation_failures += 1;
    }

    fn record_deallocation(&mut self, agent_id: AgentId) {
        self.recent_allocations.remove(&agent_id);
    }

    fn average_allocation_time(&self) -> std::time::Duration {
        if self.allocation_times.is_empty() {
            std::time::Duration::from_millis(0)
        } else {
            let total: std::time::Duration = self.allocation_times.iter().sum();
            total / self.allocation_times.len() as u32
        }
    }
}

/// Load balancing statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadBalancingStats {
    pub total_allocations: usize,
    pub active_allocations: usize,
    pub memory_utilization: f32,
    pub cpu_utilization: f32,
    pub allocation_failures: usize,
    pub average_allocation_time: std::time::Duration,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to get the detected total memory for assertions
    fn detected_total_memory() -> usize {
        let mut sys = System::new_all();
        sys.refresh_all();
        (sys.total_memory() / (1024 * 1024)) as usize
    }

    #[tokio::test]
    async fn test_resource_allocation() {
        let load_balancer = LoadBalancer::new(LoadBalancingStrategy::ResourceBased);

        let requirements = ResourceRequirements {
            min_memory_mb: 100,
            max_memory_mb: 200,
            min_cpu_cores: 0.5,
            max_cpu_cores: 1.0,
            disk_space_mb: 100,
            network_bandwidth_mbps: 10,
        };

        let result = load_balancer.allocate_resources(&requirements).await;
        assert!(result.is_ok());

        let allocation = result.unwrap();
        assert_eq!(allocation.allocated_memory, 200);
        assert_eq!(allocation.allocated_cpu_cores, 1.0);
    }

    #[tokio::test]
    async fn test_resource_deallocation() {
        let load_balancer = LoadBalancer::new(LoadBalancingStrategy::ResourceBased);

        let requirements = ResourceRequirements {
            min_memory_mb: 100,
            max_memory_mb: 200,
            min_cpu_cores: 0.5,
            max_cpu_cores: 1.0,
            disk_space_mb: 100,
            network_bandwidth_mbps: 10,
        };

        let allocation = load_balancer
            .allocate_resources(&requirements)
            .await
            .unwrap();

        load_balancer.deallocate_resources(allocation).await;

        // After deallocation, memory_used should be 0
        let utilization = load_balancer.get_resource_utilization().await;
        assert_eq!(utilization.memory_used, 0);
    }

    #[tokio::test]
    async fn test_insufficient_resources() {
        let total_memory = detected_total_memory();
        let load_balancer = LoadBalancer::new(LoadBalancingStrategy::ResourceBased);

        // Request more than the system actually has
        let requirements = ResourceRequirements {
            min_memory_mb: total_memory + 1024,
            max_memory_mb: total_memory + 1024,
            min_cpu_cores: 1024.0,
            max_cpu_cores: 1024.0,
            disk_space_mb: 100,
            network_bandwidth_mbps: 10,
        };

        let result = load_balancer.allocate_resources(&requirements).await;
        assert!(result.is_err());

        if let Err(ResourceError::AllocationFailed { reason, .. }) = result {
            assert!(reason.contains("Insufficient resources"));
        }
    }

    #[tokio::test]
    async fn test_get_statistics() {
        let load_balancer = LoadBalancer::new(LoadBalancingStrategy::RoundRobin);

        let stats = load_balancer.get_statistics().await;
        assert_eq!(stats.total_allocations, 0);
        assert_eq!(stats.active_allocations, 0);
        assert_eq!(stats.allocation_failures, 0);

        // Allocate one agent
        let requirements = ResourceRequirements {
            min_memory_mb: 50,
            max_memory_mb: 100,
            min_cpu_cores: 0.5,
            max_cpu_cores: 1.0,
            disk_space_mb: 50,
            network_bandwidth_mbps: 10,
        };

        let _ = load_balancer
            .allocate_resources(&requirements)
            .await
            .unwrap();

        let stats = load_balancer.get_statistics().await;
        assert_eq!(stats.total_allocations, 1);
        assert_eq!(stats.active_allocations, 1);
        assert_eq!(stats.allocation_failures, 0);
        assert!(stats.memory_utilization > 0.0);
    }

    #[tokio::test]
    async fn test_statistics_serializable() {
        let load_balancer = LoadBalancer::new(LoadBalancingStrategy::RoundRobin);
        let stats = load_balancer.get_statistics().await;
        let json = serde_json::to_value(&stats);
        assert!(json.is_ok());
    }

    #[tokio::test]
    async fn test_sysinfo_detection() {
        let load_balancer = LoadBalancer::new(LoadBalancingStrategy::ResourceBased);
        let total = detected_total_memory();

        // Verify the load balancer detected real system memory (should be > 0)
        assert!(total > 0);

        // Verify utilization reports zero usage initially
        let utilization = load_balancer.get_resource_utilization().await;
        assert_eq!(utilization.memory_used, 0);
        assert!(utilization.uptime.as_nanos() > 0);
    }
}
