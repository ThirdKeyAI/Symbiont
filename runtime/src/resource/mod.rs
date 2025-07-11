//! Agent Resource Manager
//! 
//! Manages resource allocation, monitoring, and enforcement for agents

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use async_trait::async_trait;
use parking_lot::RwLock;
use tokio::sync::{mpsc, Notify};
use tokio::time::interval;

use crate::types::*;

/// Resource manager trait
#[async_trait]
pub trait ResourceManager {
    /// Allocate resources for an agent
    async fn allocate_resources(&self, agent_id: AgentId, requirements: ResourceRequirements) -> Result<ResourceAllocation, ResourceError>;
    
    /// Deallocate resources for an agent
    async fn deallocate_resources(&self, agent_id: AgentId) -> Result<(), ResourceError>;
    
    /// Update resource usage for an agent
    async fn update_usage(&self, agent_id: AgentId, usage: ResourceUsage) -> Result<(), ResourceError>;
    
    /// Get current resource usage for an agent
    async fn get_usage(&self, agent_id: AgentId) -> Result<ResourceUsage, ResourceError>;
    
    /// Get system resource status
    async fn get_system_status(&self) -> ResourceSystemStatus;
    
    /// Set resource limits for an agent
    async fn set_limits(&self, agent_id: AgentId, limits: ResourceLimits) -> Result<(), ResourceError>;
    
    /// Check if agent is within resource limits
    async fn check_limits(&self, agent_id: AgentId) -> Result<bool, ResourceError>;
    
    /// Shutdown the resource manager
    async fn shutdown(&self) -> Result<(), ResourceError>;
}

/// Resource manager configuration
#[derive(Debug, Clone)]
pub struct ResourceManagerConfig {
    pub total_memory: usize,
    pub total_cpu_cores: u32,
    pub total_disk_space: usize,
    pub total_network_bandwidth: usize,
    pub monitoring_interval: Duration,
    pub enforcement_enabled: bool,
    pub auto_scaling_enabled: bool,
    pub resource_reservation_percentage: f32,
}

impl Default for ResourceManagerConfig {
    fn default() -> Self {
        Self {
            total_memory: 16 * 1024 * 1024 * 1024, // 16GB
            total_cpu_cores: 8,
            total_disk_space: 1024 * 1024 * 1024 * 1024, // 1TB
            total_network_bandwidth: 1000 * 1024 * 1024, // 1Gbps
            monitoring_interval: Duration::from_secs(5),
            enforcement_enabled: true,
            auto_scaling_enabled: false,
            resource_reservation_percentage: 0.1, // 10% reserved
        }
    }
}

/// Default implementation of the resource manager
pub struct DefaultResourceManager {
    config: ResourceManagerConfig,
    allocations: Arc<RwLock<HashMap<AgentId, ResourceAllocation>>>,
    usage_tracker: Arc<RwLock<HashMap<AgentId, ResourceUsage>>>,
    system_resources: Arc<RwLock<SystemResources>>,
    monitoring_sender: mpsc::UnboundedSender<MonitoringEvent>,
    shutdown_notify: Arc<Notify>,
    is_running: Arc<RwLock<bool>>,
}

impl DefaultResourceManager {
    /// Create a new resource manager
    pub async fn new(config: ResourceManagerConfig) -> Result<Self, ResourceError> {
        let allocations = Arc::new(RwLock::new(HashMap::new()));
        let usage_tracker = Arc::new(RwLock::new(HashMap::new()));
        let system_resources = Arc::new(RwLock::new(SystemResources::new(&config)));
        let (monitoring_sender, monitoring_receiver) = mpsc::unbounded_channel();
        let shutdown_notify = Arc::new(Notify::new());
        let is_running = Arc::new(RwLock::new(true));

        let manager = Self {
            config,
            allocations,
            usage_tracker,
            system_resources,
            monitoring_sender,
            shutdown_notify,
            is_running,
        };

        // Start background tasks
        manager.start_monitoring_loop(monitoring_receiver).await;
        manager.start_enforcement_loop().await;

        Ok(manager)
    }

    /// Start the resource monitoring loop
    async fn start_monitoring_loop(&self, mut monitoring_receiver: mpsc::UnboundedReceiver<MonitoringEvent>) {
        let usage_tracker = self.usage_tracker.clone();
        let allocations = self.allocations.clone();
        let system_resources = self.system_resources.clone();
        let shutdown_notify = self.shutdown_notify.clone();
        let is_running = self.is_running.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    event = monitoring_receiver.recv() => {
                        if let Some(event) = event {
                            Self::process_monitoring_event(event, &usage_tracker, &allocations, &system_resources).await;
                        } else {
                            break;
                        }
                    }
                    _ = shutdown_notify.notified() => {
                        break;
                    }
                }
            }
        });
    }

    /// Start the resource enforcement loop
    async fn start_enforcement_loop(&self) {
        let usage_tracker = self.usage_tracker.clone();
        let allocations = self.allocations.clone();
        let monitoring_sender = self.monitoring_sender.clone();
        let shutdown_notify = self.shutdown_notify.clone();
        let is_running = self.is_running.clone();
        let monitoring_interval = self.config.monitoring_interval;
        let enforcement_enabled = self.config.enforcement_enabled;

        tokio::spawn(async move {
            let mut interval = interval(monitoring_interval);
            
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if !*is_running.read() {
                            break;
                        }

                        if enforcement_enabled {
                            Self::enforce_resource_limits(&usage_tracker, &allocations, &monitoring_sender).await;
                        }
                    }
                    _ = shutdown_notify.notified() => {
                        break;
                    }
                }
            }
        });
    }

    /// Process a monitoring event
    async fn process_monitoring_event(
        event: MonitoringEvent,
        usage_tracker: &Arc<RwLock<HashMap<AgentId, ResourceUsage>>>,
        allocations: &Arc<RwLock<HashMap<AgentId, ResourceAllocation>>>,
        system_resources: &Arc<RwLock<SystemResources>>,
    ) {
        match event {
            MonitoringEvent::UsageUpdate { agent_id, usage } => {
                usage_tracker.write().insert(agent_id, usage.clone());
                
                // Update system resource usage
                system_resources.write().update_usage(&usage);
                
                tracing::debug!("Updated resource usage for agent {}: {:?}", agent_id, usage);
            }
            MonitoringEvent::AllocationRequest { agent_id, requirements } => {
                let mut system = system_resources.write();
                if system.can_allocate(&requirements) {
                    let allocation = system.allocate(&requirements);
                    allocations.write().insert(agent_id, allocation.clone());
                    
                    tracing::info!("Allocated resources for agent {}: {:?}", agent_id, allocation);
                } else {
                    tracing::warn!("Cannot allocate resources for agent {}: insufficient resources", agent_id);
                }
            }
            MonitoringEvent::DeallocationRequest { agent_id } => {
                if let Some(allocation) = allocations.write().remove(&agent_id) {
                    system_resources.write().deallocate(&allocation);
                    usage_tracker.write().remove(&agent_id);
                    
                    tracing::info!("Deallocated resources for agent {}", agent_id);
                }
            }
            MonitoringEvent::LimitViolation { agent_id, violations } => {
                // Handle limit violation event - this is typically sent by the enforcement loop
                // and processed by external systems, so we just log it here
                tracing::warn!("Resource limit violation detected for agent {}: {:?}", agent_id, violations);
            }
        }
    }

    /// Enforce resource limits
    async fn enforce_resource_limits(
        usage_tracker: &Arc<RwLock<HashMap<AgentId, ResourceUsage>>>,
        allocations: &Arc<RwLock<HashMap<AgentId, ResourceAllocation>>>,
        monitoring_sender: &mpsc::UnboundedSender<MonitoringEvent>,
    ) {
        let usage_map = usage_tracker.read();
        let allocations_map = allocations.read();
        
        for (agent_id, usage) in usage_map.iter() {
            if let Some(allocation) = allocations_map.get(agent_id) {
                // Create limits from allocation for violation checking
                let limits = ResourceLimits {
                    memory_mb: allocation.allocated_memory / (1024 * 1024),
                    cpu_cores: allocation.allocated_cpu_cores,
                    disk_io_mbps: allocation.allocated_disk_io / (1024 * 1024),
                    network_io_mbps: allocation.allocated_network_io / (1024 * 1024),
                    execution_timeout: Duration::from_secs(3600),
                    idle_timeout: Duration::from_secs(300),
                };
                let violations = Self::check_resource_violations(usage, &limits);
                
                if !violations.is_empty() {
                    tracing::warn!("Agent {} violated resource limits: {:?}", agent_id, violations);
                    
                    // Send violation event (could trigger throttling, suspension, etc.)
                    let _ = monitoring_sender.send(MonitoringEvent::LimitViolation {
                        agent_id: *agent_id,
                        violations,
                    });
                }
            }
        }
    }

    /// Check for resource limit violations
    fn check_resource_violations(usage: &ResourceUsage, limits: &ResourceLimits) -> Vec<ResourceViolation> {
        let mut violations = Vec::new();
        
        if usage.memory_used > limits.memory_mb * 1024 * 1024 {
            violations.push(ResourceViolation::Memory {
                used: usage.memory_used,
                limit: limits.memory_mb * 1024 * 1024,
            });
        }
        
        if usage.cpu_utilization > limits.cpu_cores {
            violations.push(ResourceViolation::Cpu {
                used: usage.cpu_utilization,
                limit: limits.cpu_cores,
            });
        }
        
        if usage.disk_io_rate > limits.disk_io_mbps * 1024 * 1024 {
            violations.push(ResourceViolation::DiskIo {
                used: usage.disk_io_rate,
                limit: limits.disk_io_mbps * 1024 * 1024,
            });
        }
        
        if usage.network_io_rate > limits.network_io_mbps * 1024 * 1024 {
            violations.push(ResourceViolation::NetworkIo {
                used: usage.network_io_rate,
                limit: limits.network_io_mbps * 1024 * 1024,
            });
        }
        
        violations
    }

    /// Send a monitoring event
    fn send_monitoring_event(&self, event: MonitoringEvent) -> Result<(), ResourceError> {
        self.monitoring_sender.send(event)
            .map_err(|_| ResourceError::MonitoringFailed("Failed to send monitoring event".to_string()))
    }
}

#[async_trait]
impl ResourceManager for DefaultResourceManager {
    async fn allocate_resources(&self, agent_id: AgentId, requirements: ResourceRequirements) -> Result<ResourceAllocation, ResourceError> {
        if !*self.is_running.read() {
            return Err(ResourceError::ShuttingDown);
        }

        // Check if agent already has allocation
        if self.allocations.read().contains_key(&agent_id) {
            return Err(ResourceError::AllocationExists { agent_id });
        }

        // Send allocation request
        self.send_monitoring_event(MonitoringEvent::AllocationRequest {
            agent_id,
            requirements: requirements.clone(),
        })?;

        // Give the monitoring loop time to process
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Check if allocation was successful
        self.allocations.read().get(&agent_id)
            .cloned()
            .ok_or(ResourceError::InsufficientResources { 
                requirements: "Insufficient system resources".to_string() 
            })
    }

    async fn deallocate_resources(&self, agent_id: AgentId) -> Result<(), ResourceError> {
        self.send_monitoring_event(MonitoringEvent::DeallocationRequest { agent_id })?;
        
        // Give the monitoring loop time to process
        tokio::time::sleep(Duration::from_millis(10)).await;
        
        Ok(())
    }

    async fn update_usage(&self, agent_id: AgentId, usage: ResourceUsage) -> Result<(), ResourceError> {
        self.send_monitoring_event(MonitoringEvent::UsageUpdate { agent_id, usage })?;
        Ok(())
    }

    async fn get_usage(&self, agent_id: AgentId) -> Result<ResourceUsage, ResourceError> {
        self.usage_tracker.read().get(&agent_id)
            .cloned()
            .ok_or(ResourceError::AgentNotFound { agent_id })
    }

    async fn get_system_status(&self) -> ResourceSystemStatus {
        let system = self.system_resources.read();
        let allocations_count = self.allocations.read().len();
        
        ResourceSystemStatus {
            total_memory: self.config.total_memory,
            available_memory: system.available_memory,
            total_cpu_cores: self.config.total_cpu_cores,
            available_cpu_cores: system.available_cpu_cores,
            total_disk_space: self.config.total_disk_space,
            available_disk_space: system.available_disk_space,
            total_network_bandwidth: self.config.total_network_bandwidth,
            available_network_bandwidth: system.available_network_bandwidth,
            active_allocations: allocations_count,
            last_updated: SystemTime::now(),
        }
    }

    async fn set_limits(&self, agent_id: AgentId, limits: ResourceLimits) -> Result<(), ResourceError> {
        let mut allocations = self.allocations.write();
        if let Some(allocation) = allocations.get_mut(&agent_id) {
            // Update allocation fields based on limits
            allocation.allocated_memory = limits.memory_mb * 1024 * 1024;
            allocation.allocated_cpu_cores = limits.cpu_cores;
            allocation.allocated_disk_io = limits.disk_io_mbps * 1024 * 1024;
            allocation.allocated_network_io = limits.network_io_mbps * 1024 * 1024;
            Ok(())
        } else {
            Err(ResourceError::AgentNotFound { agent_id })
        }
    }

    async fn check_limits(&self, agent_id: AgentId) -> Result<bool, ResourceError> {
        let usage_map = self.usage_tracker.read();
        let allocations_map = self.allocations.read();
        
        if let (Some(usage), Some(allocation)) = (usage_map.get(&agent_id), allocations_map.get(&agent_id)) {
            // Create limits from allocation for violation checking
            let limits = ResourceLimits {
                memory_mb: allocation.allocated_memory / (1024 * 1024),
                cpu_cores: allocation.allocated_cpu_cores,
                disk_io_mbps: allocation.allocated_disk_io / (1024 * 1024),
                network_io_mbps: allocation.allocated_network_io / (1024 * 1024),
                execution_timeout: Duration::from_secs(3600),
                idle_timeout: Duration::from_secs(300),
            };
            let violations = Self::check_resource_violations(usage, &limits);
            Ok(violations.is_empty())
        } else {
            Err(ResourceError::AgentNotFound { agent_id })
        }
    }

    async fn shutdown(&self) -> Result<(), ResourceError> {
        tracing::info!("Shutting down resource manager");
        
        *self.is_running.write() = false;
        self.shutdown_notify.notify_waiters();

        // Deallocate all resources
        let agent_ids: Vec<AgentId> = self.allocations.read().keys().copied().collect();
        
        for agent_id in agent_ids {
            if let Err(e) = self.deallocate_resources(agent_id).await {
                tracing::error!("Failed to deallocate resources for agent {} during shutdown: {}", agent_id, e);
            }
        }

        Ok(())
    }
}

/// System resources tracking
#[derive(Debug, Clone)]
struct SystemResources {
    available_memory: usize,
    available_cpu_cores: u32,
    available_disk_space: usize,
    available_network_bandwidth: usize,
    reserved_memory: usize,
    reserved_cpu_cores: u32,
    reserved_disk_space: usize,
    reserved_network_bandwidth: usize,
}

impl SystemResources {
    fn new(config: &ResourceManagerConfig) -> Self {
        let reservation_factor = config.resource_reservation_percentage;
        
        Self {
            available_memory: config.total_memory - (config.total_memory as f32 * reservation_factor) as usize,
            available_cpu_cores: config.total_cpu_cores - (config.total_cpu_cores as f32 * reservation_factor) as u32,
            available_disk_space: config.total_disk_space - (config.total_disk_space as f32 * reservation_factor) as usize,
            available_network_bandwidth: config.total_network_bandwidth - (config.total_network_bandwidth as f32 * reservation_factor) as usize,
            reserved_memory: (config.total_memory as f32 * reservation_factor) as usize,
            reserved_cpu_cores: (config.total_cpu_cores as f32 * reservation_factor) as u32,
            reserved_disk_space: (config.total_disk_space as f32 * reservation_factor) as usize,
            reserved_network_bandwidth: (config.total_network_bandwidth as f32 * reservation_factor) as usize,
        }
    }

    fn can_allocate(&self, requirements: &ResourceRequirements) -> bool {
        self.available_memory >= requirements.max_memory_mb * 1024 * 1024 &&
        self.available_cpu_cores >= requirements.max_cpu_cores as u32 &&
        self.available_disk_space >= requirements.disk_space_mb * 1024 * 1024 &&
        self.available_network_bandwidth >= requirements.network_bandwidth_mbps * 1024 * 1024
    }

    fn allocate(&mut self, requirements: &ResourceRequirements) -> ResourceAllocation {
        let memory_bytes = requirements.max_memory_mb * 1024 * 1024;
        let disk_bytes = requirements.disk_space_mb * 1024 * 1024;
        let network_bytes = requirements.network_bandwidth_mbps * 1024 * 1024;
        
        self.available_memory -= memory_bytes;
        self.available_cpu_cores -= requirements.max_cpu_cores as u32;
        self.available_disk_space -= disk_bytes;
        self.available_network_bandwidth -= network_bytes;

        ResourceAllocation {
            agent_id: AgentId::new(), // Will be set by caller
            allocated_memory: memory_bytes,
            allocated_cpu_cores: requirements.max_cpu_cores,
            allocated_disk_io: disk_bytes,
            allocated_network_io: network_bytes,
            allocation_time: SystemTime::now(),
        }
    }

    fn deallocate(&mut self, allocation: &ResourceAllocation) {
        self.available_memory += allocation.allocated_memory;
        self.available_cpu_cores += allocation.allocated_cpu_cores as u32;
        self.available_disk_space += allocation.allocated_disk_io;
        self.available_network_bandwidth += allocation.allocated_network_io;
    }

    fn update_usage(&mut self, _usage: &ResourceUsage) {
        // In a real implementation, this would update current usage metrics
        // For now, we just track allocations vs available resources
    }
}

/// Resource system status
#[derive(Debug, Clone)]
pub struct ResourceSystemStatus {
    pub total_memory: usize,
    pub available_memory: usize,
    pub total_cpu_cores: u32,
    pub available_cpu_cores: u32,
    pub total_disk_space: usize,
    pub available_disk_space: usize,
    pub total_network_bandwidth: usize,
    pub available_network_bandwidth: usize,
    pub active_allocations: usize,
    pub last_updated: SystemTime,
}

/// Resource violations
#[derive(Debug, Clone)]
pub enum ResourceViolation {
    Memory { used: usize, limit: usize },
    Cpu { used: f32, limit: f32 },
    DiskIo { used: usize, limit: usize },
    NetworkIo { used: usize, limit: usize },
}

/// Monitoring events for internal processing
#[derive(Debug, Clone)]
enum MonitoringEvent {
    UsageUpdate {
        agent_id: AgentId,
        usage: ResourceUsage,
    },
    AllocationRequest {
        agent_id: AgentId,
        requirements: ResourceRequirements,
    },
    DeallocationRequest {
        agent_id: AgentId,
    },
    LimitViolation {
        agent_id: AgentId,
        violations: Vec<ResourceViolation>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_requirements() -> ResourceRequirements {
        ResourceRequirements {
            min_memory_mb: 1,
            max_memory_mb: 1,
            min_cpu_cores: 1.0,
            max_cpu_cores: 1.0,
            disk_space_mb: 1,
            network_bandwidth_mbps: 1,
        }
    }

    #[tokio::test]
    async fn test_resource_allocation() {
        let manager = DefaultResourceManager::new(ResourceManagerConfig::default()).await.unwrap();
        let agent_id = AgentId::new();
        let requirements = create_test_requirements();

        let allocation = manager.allocate_resources(agent_id, requirements).await.unwrap();
        assert_eq!(allocation.allocated_memory, 1024 * 1024);
        assert_eq!(allocation.allocated_cpu_cores, 1.0);
    }

    #[tokio::test]
    async fn test_resource_deallocation() {
        let manager = DefaultResourceManager::new(ResourceManagerConfig::default()).await.unwrap();
        let agent_id = AgentId::new();
        let requirements = create_test_requirements();

        manager.allocate_resources(agent_id, requirements).await.unwrap();
        let result = manager.deallocate_resources(agent_id).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_usage_tracking() {
        let manager = DefaultResourceManager::new(ResourceManagerConfig::default()).await.unwrap();
        let agent_id = AgentId::new();
        let requirements = create_test_requirements();

        manager.allocate_resources(agent_id, requirements).await.unwrap();

        let usage = ResourceUsage {
            memory_used: 512 * 1024, // 512KB
            cpu_utilization: 0.5,
            disk_io_rate: 512 * 1024,
            network_io_rate: 512,
            uptime: Duration::from_secs(60),
        };

        manager.update_usage(agent_id, usage.clone()).await.unwrap();
        
        tokio::time::sleep(Duration::from_millis(20)).await;
        
        let retrieved_usage = manager.get_usage(agent_id).await.unwrap();
        assert_eq!(retrieved_usage.memory_used, usage.memory_used);
        assert_eq!(retrieved_usage.cpu_utilization, usage.cpu_utilization);
    }

    #[tokio::test]
    async fn test_system_status() {
        let manager = DefaultResourceManager::new(ResourceManagerConfig::default()).await.unwrap();
        let status = manager.get_system_status().await;
        
        assert!(status.total_memory > 0);
        assert!(status.available_memory <= status.total_memory);
        assert!(status.total_cpu_cores > 0);
        assert!(status.available_cpu_cores <= status.total_cpu_cores);
    }

    #[test]
    fn test_resource_violations() {
        let usage = ResourceUsage {
            memory_used: 2 * 1024 * 1024, // 2MB
            cpu_utilization: 2.0,
            disk_io_rate: 2 * 1024 * 1024,
            network_io_rate: 2 * 1024 * 1024, // 2MB to exceed 1MB limit
            uptime: Duration::from_secs(60),
        };

        let limits = ResourceLimits {
            memory_mb: 1,
            cpu_cores: 1.0,
            disk_io_mbps: 1,
            network_io_mbps: 1,
            execution_timeout: Duration::from_secs(3600),
            idle_timeout: Duration::from_secs(300),
        };

        let violations = DefaultResourceManager::check_resource_violations(&usage, &limits);
        assert_eq!(violations.len(), 4); // All resources exceeded
    }
}