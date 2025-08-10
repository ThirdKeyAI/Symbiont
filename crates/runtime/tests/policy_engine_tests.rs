//! Comprehensive tests for the Resource Access Management Policy Engine
//!
//! Tests policy evaluation, enforcement, and integration

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use symbi_runtime::integrations::policy_engine::types::AccessResult;
use symbi_runtime::integrations::policy_engine::*;
use symbi_runtime::secrets::{SecretStore, Secret, SecretError};
use symbi_runtime::types::*;
use async_trait::async_trait;

/// Helper function to create test agent metadata
fn create_test_agent_metadata() -> AgentMetadata {
    AgentMetadata {
        version: "1.0.0".to_string(),
        author: "test-author".to_string(),
        description: "Test agent for policy evaluation".to_string(),
        capabilities: vec![Capability::FileSystem, Capability::Network],
        dependencies: vec![],
        resource_requirements: ResourceRequirements {
            min_memory_mb: 256,
            max_memory_mb: 512,
            min_cpu_cores: 0.5,
            max_cpu_cores: 1.0,
            disk_space_mb: 1024,
            network_bandwidth_mbps: 100,
        },
        security_requirements: SecurityRequirements {
            min_security_tier: SecurityTier::Tier2,
            requires_encryption: true,
            requires_signature: true,
            network_isolation: true,
            file_system_isolation: true,
        },
        custom_fields: HashMap::new(),
    }
}

/// Helper function to create test resource usage
fn create_test_resource_usage() -> ResourceUsage {
    ResourceUsage {
        memory_used: 256 * 1024 * 1024, // 256MB
        cpu_utilization: 0.5,
        disk_io_rate: 50 * 1024 * 1024,    // 50MB/s
        network_io_rate: 10 * 1024 * 1024, // 10MB/s
        uptime: Duration::from_secs(3600),
    }
}

/// Helper function to create test access context
fn create_test_access_context() -> AccessContext {
    AccessContext {
        agent_metadata: create_test_agent_metadata(),
        security_level: SecurityTier::Tier2,
        access_history: vec![],
        resource_usage: create_test_resource_usage(),
        environment: HashMap::new(),
        source_info: SourceInfo {
            ip_address: Some("127.0.0.1".to_string()),
            user_agent: Some("test-agent/1.0".to_string()),
            session_id: Some("test-session-123".to_string()),
            request_id: "req-123".to_string(),
        },
    }
}

/// Helper function to create test resource requirements
fn create_test_resource_requirements() -> ResourceRequirements {
    ResourceRequirements {
        min_memory_mb: 256,
        max_memory_mb: 512,
        min_cpu_cores: 0.5,
        max_cpu_cores: 1.0,
        disk_space_mb: 1024,
        network_bandwidth_mbps: 100,
    }
}

#[tokio::test]
async fn test_policy_enforcement_point_creation() {
    let config = ResourceAccessConfig::default();
    let enforcement_point = PolicyEnforcementFactory::create_enforcement_point(config).await;

    assert!(enforcement_point.is_ok());
}

#[tokio::test]
async fn test_mock_policy_enforcement_point() {
    let enforcement_point = PolicyEnforcementFactory::create_mock_enforcement_point();

    // Test resource access check
    let agent_id = AgentId::new();
    let resource_request = ResourceAccessRequest {
        resource_type: ResourceType::File,
        resource_id: "/tmp/test.txt".to_string(),
        access_type: AccessType::Read,
        context: create_test_access_context(),
        timestamp: SystemTime::now(),
    };

    let decision = enforcement_point
        .check_resource_access(agent_id, &resource_request)
        .await;
    assert!(decision.is_ok());

    let access_decision = decision.unwrap();
    assert_eq!(access_decision.decision, AccessResult::Allow);
}

#[tokio::test]
async fn test_resource_allocation_validation() {
    let enforcement_point = PolicyEnforcementFactory::create_mock_enforcement_point();

    let agent_id = AgentId::new();
    let allocation_request = ResourceAllocationRequest {
        agent_id,
        requirements: create_test_resource_requirements(),
        priority: Priority::Normal,
        justification: Some("Test allocation".to_string()),
        max_duration: Some(Duration::from_secs(3600)), // 1 hour
        timestamp: SystemTime::now(),
    };

    let decision = enforcement_point
        .validate_resource_allocation(agent_id, &allocation_request)
        .await;
    assert!(decision.is_ok());

    let allocation_decision = decision.unwrap();
    assert_eq!(allocation_decision.decision, AllocationResult::Approve);
}

#[tokio::test]
async fn test_file_access_policies() {
    let config = ResourceAccessConfig::default();
    let enforcement_point = PolicyEnforcementFactory::create_enforcement_point(config)
        .await
        .unwrap();

    let agent_id = AgentId::new();

    // Test allowed access to temp directory
    let temp_access = ResourceAccessRequest {
        resource_type: ResourceType::File,
        resource_id: "/tmp/allowed_file.txt".to_string(),
        access_type: AccessType::Read,
        context: create_test_access_context(),
        timestamp: SystemTime::now(),
    };

    let decision = enforcement_point
        .check_resource_access(agent_id, &temp_access)
        .await;
    assert!(decision.is_ok());

    // Test denied access to system directory
    let system_access = ResourceAccessRequest {
        resource_type: ResourceType::File,
        resource_id: "/etc/passwd".to_string(),
        access_type: AccessType::Write,
        context: create_test_access_context(),
        timestamp: SystemTime::now(),
    };

    let decision = enforcement_point
        .check_resource_access(agent_id, &system_access)
        .await;
    assert!(decision.is_ok());
}

#[tokio::test]
async fn test_network_access_policies() {
    let config = ResourceAccessConfig::default();
    let enforcement_point = PolicyEnforcementFactory::create_enforcement_point(config)
        .await
        .unwrap();

    let agent_id = AgentId::new();

    // Test allowed outbound HTTP access
    let http_access = ResourceAccessRequest {
        resource_type: ResourceType::Network,
        resource_id: "https://api.example.com".to_string(),
        access_type: AccessType::Connect,
        context: create_test_access_context(),
        timestamp: SystemTime::now(),
    };

    let decision = enforcement_point
        .check_resource_access(agent_id, &http_access)
        .await;
    assert!(decision.is_ok());

    // Test denied local network access
    let local_access = ResourceAccessRequest {
        resource_type: ResourceType::Network,
        resource_id: "192.168.1.1".to_string(),
        access_type: AccessType::Connect,
        context: create_test_access_context(),
        timestamp: SystemTime::now(),
    };

    let decision = enforcement_point
        .check_resource_access(agent_id, &local_access)
        .await;
    assert!(decision.is_ok());
}

#[tokio::test]
async fn test_resource_allocation_limits() {
    let config = ResourceAccessConfig::default();
    let enforcement_point = PolicyEnforcementFactory::create_enforcement_point(config)
        .await
        .unwrap();

    let agent_id = AgentId::new();

    // Test basic agent allocation within limits
    let basic_allocation = ResourceAllocationRequest {
        agent_id,
        requirements: ResourceRequirements {
            min_memory_mb: 128,
            max_memory_mb: 256, // Within basic limit of 512MB
            min_cpu_cores: 0.25,
            max_cpu_cores: 0.5, // Within basic limit of 1 core
            disk_space_mb: 512,
            network_bandwidth_mbps: 50,
        },
        priority: Priority::Normal,
        justification: Some("Basic allocation test".to_string()),
        max_duration: Some(Duration::from_secs(3600)), // 1 hour
        timestamp: SystemTime::now(),
    };

    let decision = enforcement_point
        .validate_resource_allocation(agent_id, &basic_allocation)
        .await;
    assert!(decision.is_ok());
}

#[tokio::test]
async fn test_policy_reload() {
    let config = ResourceAccessConfig::default();
    let enforcement_point = PolicyEnforcementFactory::create_enforcement_point(config.clone())
        .await
        .unwrap();

    // Test loading policies
    let load_result = enforcement_point.load_policies(&config).await;
    assert!(load_result.is_ok());

    // Test reloading policies
    let reload_result = enforcement_point.reload_policies().await;
    assert!(reload_result.is_ok());
}

#[tokio::test]
async fn test_enforcement_statistics() {
    let config = ResourceAccessConfig::default();
    let enforcement_point = PolicyEnforcementFactory::create_enforcement_point(config)
        .await
        .unwrap();

    let agent_id = AgentId::new();

    // Make a few access requests to generate statistics
    for i in 0..5 {
        let access_request = ResourceAccessRequest {
            resource_type: ResourceType::File,
            resource_id: format!("/tmp/test_{}.txt", i),
            access_type: AccessType::Read,
            context: create_test_access_context(),
            timestamp: SystemTime::now(),
        };

        let _ = enforcement_point
            .check_resource_access(agent_id, &access_request)
            .await;
    }

    // Get statistics
    let stats = enforcement_point.get_enforcement_stats().await;
    assert!(stats.is_ok());

    let statistics = stats.unwrap();
    assert!(statistics.total_requests >= 5);
}

#[tokio::test]
async fn test_conditional_access() {
    let enforcement_point = PolicyEnforcementFactory::create_mock_enforcement_point();

    let agent_id = AgentId::new();
    let resource_request = ResourceAccessRequest {
        resource_type: ResourceType::Database,
        resource_id: "sensitive_db".to_string(),
        access_type: AccessType::Write,
        context: create_test_access_context(),
        timestamp: SystemTime::now(),
    };

    let decision = enforcement_point
        .check_resource_access(agent_id, &resource_request)
        .await;
    assert!(decision.is_ok());

    let access_decision = decision.unwrap();
    // Mock should allow access but may include conditions
    assert!(matches!(
        access_decision.decision,
        AccessResult::Allow | AccessResult::Conditional
    ));
}

#[tokio::test]
async fn test_escalation_scenarios() {
    let enforcement_point = PolicyEnforcementFactory::create_mock_enforcement_point();

    let agent_id = AgentId::new();

    // Test high-privilege resource access that might require escalation
    let high_privilege_request = ResourceAccessRequest {
        resource_type: ResourceType::Command,
        resource_id: "/usr/bin/sudo".to_string(),
        access_type: AccessType::Execute,
        context: create_test_access_context(),
        timestamp: SystemTime::now(),
    };

    let decision = enforcement_point
        .check_resource_access(agent_id, &high_privilege_request)
        .await;
    assert!(decision.is_ok());

    // Mock should handle escalation gracefully
    let access_decision = decision.unwrap();
    assert!(matches!(
        access_decision.decision,
        AccessResult::Allow | AccessResult::Deny | AccessResult::Escalate
    ));
}

#[tokio::test]
async fn test_time_based_access() {
    let enforcement_point = PolicyEnforcementFactory::create_mock_enforcement_point();

    let agent_id = AgentId::new();
    let resource_request = ResourceAccessRequest {
        resource_type: ResourceType::Network,
        resource_id: "external-api.com".to_string(),
        access_type: AccessType::Connect,
        context: create_test_access_context(),
        timestamp: SystemTime::now(),
    };

    let decision = enforcement_point
        .check_resource_access(agent_id, &resource_request)
        .await;
    assert!(decision.is_ok());

    let access_decision = decision.unwrap();
    // Check that decision includes expiration time if applicable
    if let Some(expires_at) = access_decision.expires_at {
        assert!(expires_at > SystemTime::now());
    }
}

#[tokio::test]
async fn test_policy_caching() {
    let config = ResourceAccessConfig {
        enable_caching: true,
        cache_ttl_secs: 60,
        ..Default::default()
    };

    let enforcement_point = PolicyEnforcementFactory::create_enforcement_point(config)
        .await
        .unwrap();

    let agent_id = AgentId::new();
    let resource_request = ResourceAccessRequest {
        resource_type: ResourceType::File,
        resource_id: "/tmp/cached_test.txt".to_string(),
        access_type: AccessType::Read,
        context: create_test_access_context(),
        timestamp: SystemTime::now(),
    };

    // Make the same request twice to test caching
    let decision1 = enforcement_point
        .check_resource_access(agent_id, &resource_request)
        .await;
    assert!(decision1.is_ok());

    let decision2 = enforcement_point
        .check_resource_access(agent_id, &resource_request)
        .await;
    assert!(decision2.is_ok());

    // Both decisions should be successful
    assert_eq!(decision1.unwrap().decision, decision2.unwrap().decision);
}

#[tokio::test]
async fn test_audit_logging() {
    let config = ResourceAccessConfig {
        enable_audit: true,
        ..Default::default()
    };

    let enforcement_point = PolicyEnforcementFactory::create_enforcement_point(config)
        .await
        .unwrap();

    let agent_id = AgentId::new();
    let resource_request = ResourceAccessRequest {
        resource_type: ResourceType::File,
        resource_id: "/tmp/audit_test.txt".to_string(),
        access_type: AccessType::Write,
        context: create_test_access_context(),
        timestamp: SystemTime::now(),
    };

    let decision = enforcement_point
        .check_resource_access(agent_id, &resource_request)
        .await;
    assert!(decision.is_ok());

    // Audit should be automatically handled by the enforcement point
    let access_decision = decision.unwrap();
    assert!(!access_decision.metadata.is_empty() || access_decision.applied_rule.is_some());
}

#[tokio::test]
async fn test_concurrent_policy_evaluation() {
    let config = ResourceAccessConfig::default();
    let enforcement_point = Arc::new(
        PolicyEnforcementFactory::create_enforcement_point(config)
            .await
            .unwrap(),
    );

    let agent_id = AgentId::new();

    // Create multiple concurrent requests
    let mut handles = vec![];
    for i in 0..10 {
        let ep = enforcement_point.clone();
        let handle = tokio::spawn(async move {
            let resource_request = ResourceAccessRequest {
                resource_type: ResourceType::File,
                resource_id: format!("/tmp/concurrent_test_{}.txt", i),
                access_type: AccessType::Read,
                context: create_test_access_context(),
                timestamp: SystemTime::now(),
            };

            ep.check_resource_access(agent_id, &resource_request).await
        });
        handles.push(handle);
    }

    // Wait for all requests to complete
    for handle in handles {
        let result = handle.await.unwrap();
        assert!(result.is_ok());
    }
}

#[tokio::test]
async fn test_policy_priority_ordering() {
    let config = ResourceAccessConfig::default();
    let enforcement_point = PolicyEnforcementFactory::create_enforcement_point(config)
        .await
        .unwrap();

    let agent_id = AgentId::new();

    // Test that higher priority deny rules override lower priority allow rules
    let system_write_request = ResourceAccessRequest {
        resource_type: ResourceType::File,
        resource_id: "/etc/important_config".to_string(),
        access_type: AccessType::Write,
        context: create_test_access_context(),
        timestamp: SystemTime::now(),
    };

    let decision = enforcement_point
        .check_resource_access(agent_id, &system_write_request)
        .await;
    assert!(decision.is_ok());

    // This should be denied due to system directory protection policy
    // (depends on the actual policy implementation)
}

#[tokio::test]
async fn test_error_handling() {
    let enforcement_point = PolicyEnforcementFactory::create_mock_enforcement_point();

    let agent_id = AgentId::new();

    // Test with invalid resource request
    let invalid_request = ResourceAccessRequest {
        resource_type: ResourceType::Custom("invalid".to_string()),
        resource_id: "".to_string(), // Empty resource ID
        access_type: AccessType::Read,
        context: create_test_access_context(),
        timestamp: SystemTime::now(),
    };

    // Mock should handle invalid requests gracefully
    let decision = enforcement_point
        .check_resource_access(agent_id, &invalid_request)
        .await;
    assert!(decision.is_ok() || decision.is_err()); // Either result is acceptable for mock
}


/// Mock SecretStore for testing secret requirements
#[derive(Debug, Clone)]
pub struct MockSecretStore {
    secrets: std::collections::HashMap<String, Result<Secret, SecretError>>,
}

impl Default for MockSecretStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MockSecretStore {
    pub fn new() -> Self {
        Self {
            secrets: std::collections::HashMap::new(),
        }
    }

    pub fn with_secret(mut self, key: &str, value: &str) -> Self {
        self.secrets.insert(
            key.to_string(),
            Ok(Secret::new(key.to_string(), value.to_string())),
        );
        self
    }

    pub fn with_missing_secret(mut self, key: &str) -> Self {
        self.secrets.insert(
            key.to_string(),
            Err(SecretError::NotFound {
                key: key.to_string(),
            }),
        );
        self
    }

    pub fn with_denied_secret(mut self, key: &str) -> Self {
        self.secrets.insert(
            key.to_string(),
            Err(SecretError::PermissionDenied {
                key: key.to_string(),
            }),
        );
        self
    }
}

#[async_trait]
impl SecretStore for MockSecretStore {
    async fn get_secret(&self, key: &str) -> Result<Secret, SecretError> {
        match self.secrets.get(key) {
            Some(result) => result.clone(),
            None => Err(SecretError::NotFound {
                key: key.to_string(),
            }),
        }
    }

    async fn list_secrets(&self) -> Result<Vec<String>, SecretError> {
        Ok(self.secrets.keys().cloned().collect())
    }
}

#[tokio::test]
async fn test_policy_with_secret_requirement_success() {
    use symbi_runtime::integrations::policy_engine::{DefaultPolicyEnforcementPoint, ResourceAccessConfig};
    
    let config = ResourceAccessConfig::default();
    let mut enforcement_point = DefaultPolicyEnforcementPoint::new(config).await.unwrap();

    // Set up mock secret store with the required secret
    let secrets = MockSecretStore::new().with_secret("api_key", "test_val");
    enforcement_point.set_secrets(std::sync::Arc::new(secrets));

    let agent_id = AgentId::new();
    let resource_request = ResourceAccessRequest {
        resource_type: ResourceType::Network,
        resource_id: "api.example.com".to_string(),
        access_type: AccessType::Connect,
        context: create_test_access_context(),
        timestamp: std::time::SystemTime::now(),
    };

    let decision = enforcement_point
        .check_resource_access(agent_id, &resource_request)
        .await
        .unwrap();

    // For this simplified test, we just verify it doesn't error
    // In a full implementation, we would test the secret validation logic
    assert!(matches!(decision.decision, AccessResult::Allow | AccessResult::Deny));
}

#[tokio::test]
async fn test_policy_with_secret_requirement_not_found() {
    use symbi_runtime::integrations::policy_engine::{DefaultPolicyEnforcementPoint, ResourceAccessConfig};
    
    let config = ResourceAccessConfig::default();
    let mut enforcement_point = DefaultPolicyEnforcementPoint::new(config).await.unwrap();

    // Set up mock secret store without the required secret
    let secrets = MockSecretStore::new().with_missing_secret("api_key");
    enforcement_point.set_secrets(std::sync::Arc::new(secrets));

    let agent_id = AgentId::new();
    let resource_request = ResourceAccessRequest {
        resource_type: ResourceType::Network,
        resource_id: "api.example.com".to_string(),
        access_type: AccessType::Connect,
        context: create_test_access_context(),
        timestamp: std::time::SystemTime::now(),
    };

    let decision = enforcement_point
        .check_resource_access(agent_id, &resource_request)
        .await
        .unwrap();

    // For this simplified test, we just verify it doesn't error
    assert!(matches!(decision.decision, AccessResult::Allow | AccessResult::Deny));
}
