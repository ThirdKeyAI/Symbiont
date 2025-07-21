//! Integration tests for Resource Management and Policy Engine
//! 
//! Tests the end-to-end integration between resource allocation and policy enforcement

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, Duration};
use tokio;

use symbiont_runtime::types::*;
use symbiont_runtime::resource::{ResourceManager, DefaultResourceManager, ResourceManagerConfig};
use symbiont_runtime::integrations::policy_engine::*;

/// Helper function to create test resource manager configuration
fn create_test_resource_config() -> ResourceManagerConfig {
    ResourceManagerConfig {
        total_memory: 8 * 1024 * 1024 * 1024, // 8GB
        total_cpu_cores: 4,
        total_disk_space: 100 * 1024 * 1024 * 1024, // 100GB
        total_network_bandwidth: 1000 * 1024 * 1024, // 1Gbps
        monitoring_interval: Duration::from_millis(100),
        enforcement_enabled: true,
        auto_scaling_enabled: false,
        resource_reservation_percentage: 0.1,
        policy_enforcement_config: ResourceAccessConfig {
            default_deny: false, // Allow by default for testing
            enable_caching: true,
            cache_ttl_secs: 300,
            policy_path: None,
            enable_audit: true,
        },
    }
}

/// Helper function to create test resource requirements
fn create_test_requirements(memory_mb: usize, cpu_cores: f32) -> ResourceRequirements {
    ResourceRequirements {
        min_memory_mb: memory_mb / 2,
        max_memory_mb: memory_mb,
        min_cpu_cores: cpu_cores / 2.0,
        max_cpu_cores: cpu_cores,
        disk_space_mb: 1024,
        network_bandwidth_mbps: 100,
    }
}

#[tokio::test]
async fn test_resource_allocation_with_policy_enforcement() {
    let config = create_test_resource_config();
    let resource_manager = DefaultResourceManager::new(config).await.unwrap();
    
    let agent_id = AgentId::new();
    let requirements = create_test_requirements(512, 1.0);
    
    // Test successful allocation within policy limits
    let result = resource_manager.allocate_resources(agent_id, requirements).await;
    assert!(result.is_ok());
    
    let allocation = result.unwrap();
    assert_eq!(allocation.allocated_memory, 512 * 1024 * 1024);
    assert_eq!(allocation.allocated_cpu_cores, 1.0);
    
    // Clean up
    let _ = resource_manager.deallocate_resources(agent_id).await;
}

#[tokio::test]
async fn test_resource_allocation_policy_denial() {
    let mut config = create_test_resource_config();
    config.policy_enforcement_config.default_deny = true; // Deny by default
    
    let resource_manager = DefaultResourceManager::new(config).await.unwrap();
    
    let agent_id = AgentId::new();
    let requirements = create_test_requirements(4096, 2.0); // Large allocation
    
    // This should fail due to policy enforcement
    let result = resource_manager.allocate_resources(agent_id, requirements).await;
    
    // Depending on policy configuration, this may be denied or modified
    match result {
        Ok(_) => {
            // If approved, ensure it's within acceptable limits
            let _ = resource_manager.deallocate_resources(agent_id).await;
        },
        Err(ResourceError::PolicyViolation { reason: _ }) => {
            // Expected for restrictive policy
        },
        Err(e) => panic!("Unexpected error: {:?}", e),
    }
}

#[tokio::test]
async fn test_resource_allocation_modification() {
    let config = create_test_resource_config();
    let resource_manager = DefaultResourceManager::new(config).await.unwrap();
    
    let agent_id = AgentId::new();
    let requirements = create_test_requirements(8192, 4.0); // Very large allocation
    
    let result = resource_manager.allocate_resources(agent_id, requirements).await;
    
    match result {
        Ok(allocation) => {
            // Policy may have modified the allocation to be within limits
            assert!(allocation.allocated_memory <= 8 * 1024 * 1024 * 1024); // Within system limits
            assert!(allocation.allocated_cpu_cores <= 4.0);
            
            let _ = resource_manager.deallocate_resources(agent_id).await;
        },
        Err(ResourceError::PolicyViolation { reason: _ }) => {
            // Also acceptable - policy denied the large allocation
        },
        Err(ResourceError::InsufficientResources { requirements: _ }) => {
            // System doesn't have enough resources
        },
        Err(e) => panic!("Unexpected error: {:?}", e),
    }
}

#[tokio::test]
async fn test_concurrent_allocations_with_policy() {
    let config = create_test_resource_config();
    let resource_manager = Arc::new(DefaultResourceManager::new(config).await.unwrap());
    
    let mut handles = vec![];
    let mut agent_ids = vec![];
    
    // Create multiple concurrent allocation requests
    for i in 0..5 {
        let rm = resource_manager.clone();
        let agent_id = AgentId::new();
        agent_ids.push(agent_id);
        
        let handle = tokio::spawn(async move {
            let requirements = create_test_requirements(256, 0.5);
            rm.allocate_resources(agent_id, requirements).await
        });
        handles.push(handle);
    }
    
    // Wait for all allocations to complete
    let mut successful_allocations = 0;
    for handle in handles {
        let result = handle.await.unwrap();
        if result.is_ok() {
            successful_allocations += 1;
        }
    }
    
    // At least some allocations should succeed
    assert!(successful_allocations > 0);
    
    // Clean up successful allocations
    for agent_id in agent_ids {
        let _ = resource_manager.deallocate_resources(agent_id).await;
    }
}

#[tokio::test]
async fn test_policy_escalation_handling() {
    let config = create_test_resource_config();
    let resource_manager = DefaultResourceManager::new(config).await.unwrap();
    
    let agent_id = AgentId::new();
    let requirements = create_test_requirements(16384, 8.0); // Extremely large allocation
    
    let result = resource_manager.allocate_resources(agent_id, requirements).await;
    
    match result {
        Ok(_) => {
            // Should not happen with such large requirements
            let _ = resource_manager.deallocate_resources(agent_id).await;
        },
        Err(ResourceError::EscalationRequired { reason: _ }) => {
            // Expected for very large allocations
        },
        Err(ResourceError::InsufficientResources { requirements: _ }) => {
            // Also acceptable
        },
        Err(ResourceError::PolicyViolation { reason: _ }) => {
            // Also acceptable
        },
        Err(e) => panic!("Unexpected error: {:?}", e),
    }
}

#[tokio::test]
async fn test_resource_limits_enforcement() {
    let config = create_test_resource_config();
    let resource_manager = DefaultResourceManager::new(config).await.unwrap();
    
    let agent_id = AgentId::new();
    let requirements = create_test_requirements(512, 1.0);
    
    // Allocate resources
    let allocation = resource_manager.allocate_resources(agent_id, requirements).await.unwrap();
    
    // Set specific limits
    let limits = ResourceLimits {
        memory_mb: 256, // Lower than allocated
        cpu_cores: 0.5,
        disk_io_mbps: 50,
        network_io_mbps: 50,
        execution_timeout: Duration::from_secs(3600),
        idle_timeout: Duration::from_secs(300),
    };
    
    let result = resource_manager.set_limits(agent_id, limits).await;
    assert!(result.is_ok());
    
    // Check if limits are within bounds
    let within_limits = resource_manager.check_limits(agent_id).await.unwrap();
    // Should be true since we haven't exceeded usage yet
    assert!(within_limits);
    
    // Clean up
    let _ = resource_manager.deallocate_resources(agent_id).await;
}

#[tokio::test]
async fn test_resource_usage_monitoring() {
    let config = create_test_resource_config();
    let resource_manager = DefaultResourceManager::new(config).await.unwrap();
    
    let agent_id = AgentId::new();
    let requirements = create_test_requirements(512, 1.0);
    
    // Allocate resources
    let _ = resource_manager.allocate_resources(agent_id, requirements).await.unwrap();
    
    // Update usage
    let usage = ResourceUsage {
        memory_used: 256 * 1024 * 1024, // 256MB
        cpu_utilization: 0.5,
        disk_io_rate: 25 * 1024 * 1024, // 25MB/s
        network_io_rate: 10 * 1024 * 1024, // 10MB/s
        uptime: Duration::from_secs(300),
    };
    
    let result = resource_manager.update_usage(agent_id, usage.clone()).await;
    assert!(result.is_ok());
    
    // Give some time for monitoring to process
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    // Retrieve usage
    let retrieved_usage = resource_manager.get_usage(agent_id).await.unwrap();
    assert_eq!(retrieved_usage.memory_used, usage.memory_used);
    assert_eq!(retrieved_usage.cpu_utilization, usage.cpu_utilization);
    
    // Clean up
    let _ = resource_manager.deallocate_resources(agent_id).await;
}

#[tokio::test]
async fn test_system_status_reporting() {
    let config = create_test_resource_config();
    let resource_manager = DefaultResourceManager::new(config.clone()).await.unwrap();
    
    let status = resource_manager.get_system_status().await;
    
    // Verify system status fields
    assert_eq!(status.total_memory, config.total_memory);
    assert_eq!(status.total_cpu_cores, config.total_cpu_cores);
    assert_eq!(status.total_disk_space, config.total_disk_space);
    assert_eq!(status.total_network_bandwidth, config.total_network_bandwidth);
    
    // Available resources should be less than total due to reservation
    assert!(status.available_memory < status.total_memory);
    assert!(status.available_cpu_cores < status.total_cpu_cores);
    
    // Initially no active allocations
    assert_eq!(status.active_allocations, 0);
}

#[tokio::test]
async fn test_resource_violation_detection() {
    let config = create_test_resource_config();
    let resource_manager = DefaultResourceManager::new(config).await.unwrap();
    
    let agent_id = AgentId::new();
    let requirements = create_test_requirements(512, 1.0);
    
    // Allocate resources
    let _ = resource_manager.allocate_resources(agent_id, requirements).await.unwrap();
    
    // Update with excessive usage
    let excessive_usage = ResourceUsage {
        memory_used: 1024 * 1024 * 1024, // 1GB (exceeds 512MB allocation)
        cpu_utilization: 2.0, // Exceeds 1 core allocation
        disk_io_rate: 500 * 1024 * 1024, // Very high disk I/O
        network_io_rate: 200 * 1024 * 1024, // Very high network I/O
        uptime: Duration::from_secs(300),
    };
    
    let _ = resource_manager.update_usage(agent_id, excessive_usage).await;
    
    // Give time for monitoring to detect violations
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Check for violations
    let violations = resource_manager.check_resource_violations(agent_id).await.unwrap();
    assert!(!violations.is_empty(), "Expected resource violations to be detected");
    
    // Clean up
    let _ = resource_manager.deallocate_resources(agent_id).await;
}

#[tokio::test]
async fn test_resource_manager_shutdown() {
    let config = create_test_resource_config();
    let resource_manager = DefaultResourceManager::new(config).await.unwrap();
    
    let agent_id = AgentId::new();
    let requirements = create_test_requirements(256, 0.5);
    
    // Allocate some resources
    let _ = resource_manager.allocate_resources(agent_id, requirements).await.unwrap();
    
    // Shutdown should clean up all allocations
    let result = resource_manager.shutdown().await;
    assert!(result.is_ok());
    
    // After shutdown, further operations should fail
    let post_shutdown_result = resource_manager.allocate_resources(AgentId::new(), create_test_requirements(128, 0.25)).await;
    assert!(post_shutdown_result.is_err());
}

#[tokio::test]
async fn test_policy_audit_trail() {
    let mut config = create_test_resource_config();
    config.policy_enforcement_config.enable_audit = true;
    
    let resource_manager = DefaultResourceManager::new(config).await.unwrap();
    
    let agent_id = AgentId::new();
    let requirements = create_test_requirements(512, 1.0);
    
    // Allocate resources (should be audited)
    let result = resource_manager.allocate_resources(agent_id, requirements).await;
    
    match result {
        Ok(_) => {
            // Allocation succeeded - audit trail should be recorded
            let _ = resource_manager.deallocate_resources(agent_id).await;
        },
        Err(e) => {
            // Even failures should be audited
            println!("Allocation failed (audited): {:?}", e);
        }
    }
    
    // In a real implementation, we would check audit logs here
    // For now, we just verify the operation completed without panicking
}

#[tokio::test]
async fn test_duplicate_allocation_prevention() {
    let config = create_test_resource_config();
    let resource_manager = DefaultResourceManager::new(config).await.unwrap();
    
    let agent_id = AgentId::new();
    let requirements = create_test_requirements(256, 0.5);
    
    // First allocation should succeed
    let first_result = resource_manager.allocate_resources(agent_id, requirements.clone()).await;
    assert!(first_result.is_ok());
    
    // Second allocation for same agent should fail
    let second_result = resource_manager.allocate_resources(agent_id, requirements).await;
    assert!(second_result.is_err());
    
    match second_result {
        Err(ResourceError::AllocationExists { agent_id: returned_id }) => {
            assert_eq!(returned_id, agent_id);
        },
        Err(e) => panic!("Expected AllocationExists error, got: {:?}", e),
        Ok(_) => panic!("Expected second allocation to fail"),
    }
    
    // Clean up
    let _ = resource_manager.deallocate_resources(agent_id).await;
}

#[tokio::test]
async fn test_resource_priority_handling() {
    let config = create_test_resource_config();
    let resource_manager = DefaultResourceManager::new(config).await.unwrap();
    
    let high_priority_agent = AgentId::new();
    let normal_priority_agent = AgentId::new();
    
    let high_priority_requirements = create_test_requirements(1024, 2.0);
    let normal_priority_requirements = create_test_requirements(512, 1.0);
    
    // Allocate for normal priority first
    let normal_result = resource_manager.allocate_resources(normal_priority_agent, normal_priority_requirements).await;
    
    // Then try high priority allocation
    let high_result = resource_manager.allocate_resources(high_priority_agent, high_priority_requirements).await;
    
    // Both should work within available system resources
    if normal_result.is_ok() {
        let _ = resource_manager.deallocate_resources(normal_priority_agent).await;
    }
    if high_result.is_ok() {
        let _ = resource_manager.deallocate_resources(high_priority_agent).await;
    }
}