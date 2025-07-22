//! Policy Engine Module
//! 
//! Provides resource access management through policy-based enforcement

pub mod types;
pub mod engine;

// Re-export existing policy engine interface
mod existing;
pub use existing::{PolicyEngine, Policy, PolicyDecision as ExistingPolicyDecision, MockPolicyEngine};

// Re-export new resource access management components
pub use types::{
    ResourceAccessRequest, ResourceType, AccessType, AccessContext, SourceInfo,
    AccessDecision, AllocationDecision, AllocationResult, EnforcementStatistics,
    ResourceAllocationRequest
};
// Re-export PolicyError from crate::types to avoid conflicts
pub use crate::types::PolicyError;
pub use engine::{DefaultPolicyEnforcementPoint, MockPolicyEnforcementPoint};

use std::sync::Arc;
use async_trait::async_trait;
use crate::types::*;

/// Resource Access Management Configuration
#[derive(Debug, Clone)]
pub struct ResourceAccessConfig {
    /// Default deny mode - if true, deny access by default when no policy matches
    pub default_deny: bool,
    /// Enable policy caching for performance
    pub enable_caching: bool,
    /// Cache TTL for policy decisions
    pub cache_ttl_secs: u64,
    /// Path to policy definition files
    pub policy_path: Option<String>,
    /// Enable audit logging for all access decisions
    pub enable_audit: bool,
}

impl Default for ResourceAccessConfig {
    fn default() -> Self {
        Self {
            default_deny: true,
            enable_caching: true,
            cache_ttl_secs: 300, // 5 minutes
            policy_path: None,
            enable_audit: true,
        }
    }
}

/// Main Policy Enforcement Point for Resource Access Management
#[async_trait]
pub trait PolicyEnforcementPoint: Send + Sync {
    /// Check if an agent can access a specific resource
    async fn check_resource_access(
        &self, 
        agent_id: AgentId, 
        resource: &ResourceAccessRequest
    ) -> Result<AccessDecision, PolicyError>;
    
    /// Validate a resource allocation request
    async fn validate_resource_allocation(
        &self,
        agent_id: AgentId,
        allocation: &ResourceAllocationRequest
    ) -> Result<AllocationDecision, PolicyError>;
    
    /// Load policies from configuration
    async fn load_policies(&self, config: &ResourceAccessConfig) -> Result<(), PolicyError>;
    
    /// Reload policies (e.g., after configuration changes)
    async fn reload_policies(&self) -> Result<(), PolicyError>;
    
    /// Get policy evaluation statistics
    async fn get_enforcement_stats(&self) -> Result<EnforcementStatistics, PolicyError>;
}

/// Factory for creating policy enforcement points
pub struct PolicyEnforcementFactory;

impl PolicyEnforcementFactory {
    /// Create a new policy enforcement point with the given configuration
    pub async fn create_enforcement_point(
        config: ResourceAccessConfig
    ) -> Result<Arc<dyn PolicyEnforcementPoint>, PolicyError> {
        let enforcement_point = DefaultPolicyEnforcementPoint::new(config).await?;
        Ok(Arc::new(enforcement_point))
    }
    
    /// Create a mock enforcement point for testing
    pub fn create_mock_enforcement_point() -> Arc<dyn PolicyEnforcementPoint> {
        Arc::new(MockPolicyEnforcementPoint::new())
    }
}