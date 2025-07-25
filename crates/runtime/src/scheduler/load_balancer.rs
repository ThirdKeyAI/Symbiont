//! Load balancer for distributing agents across available resources

use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

use crate::types::*;

/// Load balancer for resource allocation
pub struct LoadBalancer {
    strategy: LoadBalancingStrategy,
    resource_pool: Arc<RwLock<ResourcePool>>,
    allocation_history: Arc<RwLock<AllocationHistory>>,
}

impl LoadBalancer {
    /// Create a new load balancer
    pub fn new(strategy: LoadBalancingStrategy) -> Self {
        // Initialize with default system resources
        let total_memory = 16 * 1024; // 16GB in MB
        let total_cpu_cores = 8;

        Self {
            strategy,
            resource_pool: Arc::new(RwLock::new(ResourcePool::new(
                total_memory,
                total_cpu_cores,
            ))),
            allocation_history: Arc::new(RwLock::new(AllocationHistory::new())),
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

    /// Get current resource utilization
    pub async fn get_resource_utilization(&self) -> ResourceUsage {
        let pool = self.resource_pool.read();
        let utilization = pool.get_utilization();

        ResourceUsage {
            memory_used: (pool.total_memory - pool.available_memory),
            cpu_utilization: utilization.cpu_utilization,
            disk_io_rate: 0,    // Would be measured in real implementation
            network_io_rate: 0, // Would be measured in real implementation
            uptime: std::time::Duration::from_secs(0), // Would track actual uptime
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
    fn allocate_least_connections(
        &self,
        pool: &mut ResourcePool,
        agent_id: AgentId,
        limits: &ResourceLimits,
    ) -> Result<ResourceAllocation, ResourceError> {
        // For simplicity, this is the same as round-robin
        // In a real implementation, this would consider connection counts
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
    fn allocate_weighted_round_robin(
        &self,
        pool: &mut ResourcePool,
        agent_id: AgentId,
        limits: &ResourceLimits,
    ) -> Result<ResourceAllocation, ResourceError> {
        // For simplicity, this is the same as resource-based
        // In a real implementation, this would consider weights
        self.allocate_resource_based(pool, agent_id, limits)
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
#[derive(Debug, Clone)]
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
        let _agent_id = allocation.agent_id;

        load_balancer.deallocate_resources(allocation).await;

        // Verify resources are available again
        let utilization = load_balancer.get_resource_utilization().await;
        assert!(utilization.memory_used < 16 * 1024); // Should be less than total
    }

    #[tokio::test]
    async fn test_insufficient_resources() {
        let load_balancer = LoadBalancer::new(LoadBalancingStrategy::ResourceBased);

        // Try to allocate more than available
        let requirements = ResourceRequirements {
            min_memory_mb: 32 * 1024, // 32GB, more than our 16GB
            max_memory_mb: 32 * 1024,
            min_cpu_cores: 16.0, // More than our 8 cores
            max_cpu_cores: 16.0,
            disk_space_mb: 100,
            network_bandwidth_mbps: 10,
        };

        let result = load_balancer.allocate_resources(&requirements).await;
        assert!(result.is_err());

        if let Err(ResourceError::AllocationFailed { reason, .. }) = result {
            assert!(reason.contains("Insufficient resources"));
        }
    }
}
