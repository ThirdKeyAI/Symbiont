//! Policy Enforcement Engine
//! 
//! Core implementation of resource access management and policy enforcement

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, Duration};
use async_trait::async_trait;
use parking_lot::RwLock;
use serde_yaml;
use tokio::time::Instant;

use crate::types::*;
use super::types::*;
use super::{PolicyEnforcementPoint, ResourceAccessConfig};

/// Default implementation of PolicyEnforcementPoint
pub struct DefaultPolicyEnforcementPoint {
    config: ResourceAccessConfig,
    policies: Arc<RwLock<Vec<ResourceAccessPolicy>>>,
    stats: Arc<RwLock<EnforcementStatistics>>,
    decision_cache: Arc<RwLock<HashMap<String, CachedDecision>>>,
}

/// Cached policy decision
#[derive(Debug, Clone)]
struct CachedDecision {
    decision: AccessDecision,
    expires_at: SystemTime,
}

impl DefaultPolicyEnforcementPoint {
    /// Create a new policy enforcement point
    pub async fn new(config: ResourceAccessConfig) -> Result<Self, PolicyError> {
        let policies = Arc::new(RwLock::new(Vec::new()));
        let stats = Arc::new(RwLock::new(EnforcementStatistics {
            total_requests: 0,
            decisions: HashMap::new(),
            resource_types: HashMap::new(),
            performance: PerformanceMetrics {
                avg_evaluation_time_ms: 0.0,
                p95_evaluation_time_ms: 0.0,
                cache_hit_rate: if config.enable_caching { Some(0.0) } else { None },
                policy_reloads: 0,
            },
            last_updated: SystemTime::now(),
        }));
        let decision_cache = Arc::new(RwLock::new(HashMap::new()));

        let enforcement_point = Self {
            config,
            policies,
            stats,
            decision_cache,
        };

        // Load default policies
        enforcement_point.load_default_policies().await?;

        Ok(enforcement_point)
    }

    /// Load default policies from embedded YAML
    async fn load_default_policies(&self) -> Result<(), PolicyError> {
        let policies_data: serde_yaml::Value = serde_yaml::from_str(DEFAULT_POLICIES_YAML)
            .map_err(|e| PolicyError::InvalidPolicy {
                reason: format!("Failed to parse default policies: {}", e),
            })?;

        let policies = self.parse_policies_from_yaml(&policies_data)?;
        *self.policies.write() = policies;

        // Update stats
        {
            let mut stats = self.stats.write();
            stats.performance.policy_reloads += 1;
            stats.last_updated = SystemTime::now();
        }

        Ok(())
    }

    /// Parse policies from YAML data
    fn parse_policies_from_yaml(&self, data: &serde_yaml::Value) -> Result<Vec<ResourceAccessPolicy>, PolicyError> {
        let policies_array = data.get("policies")
            .and_then(|v| v.as_sequence())
            .ok_or_else(|| PolicyError::InvalidPolicy {
                reason: "Missing 'policies' array in YAML".to_string(),
            })?;

        let mut policies = Vec::new();
        
        for policy_data in policies_array {
            let policy = self.parse_single_policy(policy_data)?;
            policies.push(policy);
        }

        // Sort by priority (higher priority first)
        policies.sort_by(|a, b| b.priority.cmp(&a.priority));

        Ok(policies)
    }

    /// Parse a single policy from YAML
    fn parse_single_policy(&self, data: &serde_yaml::Value) -> Result<ResourceAccessPolicy, PolicyError> {
        let id = data.get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PolicyError::InvalidPolicy {
                reason: "Policy missing 'id' field".to_string(),
            })?
            .to_string();

        let name = data.get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(&id)
            .to_string();

        let description = data.get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let version = data.get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("1.0.0")
            .to_string();

        let enabled = data.get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let priority = data.get("priority")
            .and_then(|v| v.as_u64())
            .unwrap_or(1000) as u32;

        let empty_rules = Vec::new();
        let rules_data = data.get("rules")
            .and_then(|v| v.as_sequence())
            .unwrap_or(&empty_rules);

        let mut rules = Vec::new();
        for rule_data in rules_data {
            let rule = self.parse_rule(rule_data)?;
            rules.push(rule);
        }

        // Sort rules by priority within policy
        rules.sort_by(|a, b| b.priority.cmp(&a.priority));

        Ok(ResourceAccessPolicy {
            id,
            name,
            description,
            version,
            enabled,
            priority,
            rules,
            metadata: HashMap::new(),
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
        })
    }

    /// Parse a rule from YAML
    fn parse_rule(&self, data: &serde_yaml::Value) -> Result<ResourceAccessRule, PolicyError> {
        let id = data.get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PolicyError::InvalidPolicy {
                reason: "Rule missing 'id' field".to_string(),
            })?
            .to_string();

        let name = data.get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(&id)
            .to_string();

        let description = data.get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let priority = data.get("priority")
            .and_then(|v| v.as_u64())
            .unwrap_or(100) as u32;

        // Parse conditions (simplified for now)
        let conditions = Vec::new();

        // Parse effect
        let effect = self.parse_rule_effect(data.get("effect"))?;

        Ok(ResourceAccessRule {
            id,
            name,
            description,
            conditions,
            effect,
            priority,
            metadata: HashMap::new(),
        })
    }

    /// Parse rule effect from YAML
    fn parse_rule_effect(&self, data: Option<&serde_yaml::Value>) -> Result<RuleEffect, PolicyError> {
        let effect_data = data.ok_or_else(|| PolicyError::InvalidPolicy {
            reason: "Rule missing 'effect' field".to_string(),
        })?;

        let effect_type = effect_data.get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PolicyError::InvalidPolicy {
                reason: "Effect missing 'type' field".to_string(),
            })?;

        match effect_type {
            "Allow" => Ok(RuleEffect::Allow {
                conditions: Vec::new(),
            }),
            "Deny" => {
                let reason = effect_data.get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Access denied by policy")
                    .to_string();
                Ok(RuleEffect::Deny { reason })
            },
            "Limit" => {
                // Simplified limit parsing
                Ok(RuleEffect::Limit {
                    limits: ResourceConstraints {
                        max_concurrent_access: Some(10),
                        rate_limit: None,
                        transfer_limits: None,
                        time_restrictions: None,
                    },
                })
            },
            "Audit" => Ok(RuleEffect::Audit {
                level: AuditLevel::Info,
            }),
            "Escalate" => {
                let to = effect_data.get("to")
                    .and_then(|v| v.as_str())
                    .unwrap_or("administrator")
                    .to_string();
                let reason = effect_data.get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Manual review required")
                    .to_string();
                Ok(RuleEffect::Escalate { to, reason })
            },
            _ => Err(PolicyError::InvalidPolicy {
                reason: format!("Unknown effect type: {}", effect_type),
            })
        }
    }

    /// Evaluate access request against policies
    async fn evaluate_access(&self, request: &ResourceAccessRequest) -> Result<AccessDecision, PolicyError> {
        let start_time = Instant::now();

        // Check cache if enabled
        if self.config.enable_caching {
            let cache_key = self.generate_cache_key(request);
            if let Some(cached) = self.check_cache(&cache_key) {
                self.update_stats_cache_hit();
                return Ok(cached.decision);
            }
        }

        // Evaluate against policies
        let policies = self.policies.read();
        let mut decision = if self.config.default_deny {
            AccessDecision {
                decision: AccessResult::Deny,
                reason: "Default deny policy".to_string(),
                applied_rule: None,
                conditions: Vec::new(),
                expires_at: None,
                metadata: HashMap::new(),
            }
        } else {
            AccessDecision {
                decision: AccessResult::Allow,
                reason: "Default allow policy".to_string(),
                applied_rule: None,
                conditions: Vec::new(),
                expires_at: None,
                metadata: HashMap::new(),
            }
        };

        // Apply policies in priority order
        for policy in policies.iter() {
            if !policy.enabled {
                continue;
            }

            for rule in &policy.rules {
                if self.rule_matches(rule, request) {
                    decision = self.apply_rule_effect(&rule.effect, &rule.id);
                    break;
                }
            }

            // If we got a definitive decision, stop processing
            if matches!(decision.decision, AccessResult::Allow | AccessResult::Deny) {
                break;
            }
        }

        // Cache the decision if enabled
        if self.config.enable_caching {
            let cache_key = self.generate_cache_key(request);
            self.cache_decision(cache_key, decision.clone());
        }

        // Update statistics
        let eval_time = start_time.elapsed().as_millis() as f64;
        self.update_stats(request, &decision, eval_time);

        Ok(decision)
    }

    /// Check if a rule matches the request
    fn rule_matches(&self, rule: &ResourceAccessRule, _request: &ResourceAccessRequest) -> bool {
        // Simplified rule matching - in a real implementation, this would
        // evaluate the rule conditions against the request
        // For now, just return true for first matching rule
        !rule.conditions.is_empty() || rule.conditions.is_empty()
    }

    /// Apply rule effect to generate decision
    fn apply_rule_effect(&self, effect: &RuleEffect, rule_id: &str) -> AccessDecision {
        match effect {
            RuleEffect::Allow { conditions } => AccessDecision {
                decision: AccessResult::Allow,
                reason: "Access granted by policy rule".to_string(),
                applied_rule: Some(rule_id.to_string()),
                conditions: conditions.clone(),
                expires_at: None,
                metadata: HashMap::new(),
            },
            RuleEffect::Deny { reason } => AccessDecision {
                decision: AccessResult::Deny,
                reason: reason.clone(),
                applied_rule: Some(rule_id.to_string()),
                conditions: Vec::new(),
                expires_at: None,
                metadata: HashMap::new(),
            },
            RuleEffect::Limit { .. } => AccessDecision {
                decision: AccessResult::Conditional,
                reason: "Access granted with limits".to_string(),
                applied_rule: Some(rule_id.to_string()),
                conditions: Vec::new(),
                expires_at: None,
                metadata: HashMap::new(),
            },
            RuleEffect::Audit { .. } => AccessDecision {
                decision: AccessResult::Allow,
                reason: "Access granted with audit requirement".to_string(),
                applied_rule: Some(rule_id.to_string()),
                conditions: vec![AccessCondition {
                    condition_type: ConditionType::AuditRequired,
                    parameters: HashMap::new(),
                    timeout: None,
                    blocking: false,
                }],
                expires_at: None,
                metadata: HashMap::new(),
            },
            RuleEffect::Escalate { to, reason } => AccessDecision {
                decision: AccessResult::Escalate,
                reason: reason.clone(),
                applied_rule: Some(rule_id.to_string()),
                conditions: Vec::new(),
                expires_at: None,
                metadata: {
                    let mut map = HashMap::new();
                    map.insert("escalate_to".to_string(), to.clone());
                    map
                },
            },
        }
    }

    /// Generate cache key for request
    fn generate_cache_key(&self, request: &ResourceAccessRequest) -> String {
        let resource_type_id = match request.resource_type {
            ResourceType::File => 0u8,
            ResourceType::Network => 1u8,
            ResourceType::Command => 2u8,
            ResourceType::Database => 3u8,
            ResourceType::Environment => 4u8,
            ResourceType::Agent => 5u8,
            ResourceType::Custom(_) => 6u8,
        };
        let access_type_id = match request.access_type {
            AccessType::Read => 0u8,
            AccessType::Write => 1u8,
            AccessType::Execute => 2u8,
            AccessType::Delete => 3u8,
            AccessType::Create => 4u8,
            AccessType::Modify => 5u8,
            AccessType::List => 6u8,
            AccessType::Connect => 7u8,
        };
        format!("{}:{}:{}:{:?}",
            resource_type_id,
            request.resource_id,
            access_type_id,
            request.context.security_level
        )
    }

    /// Check decision cache
    fn check_cache(&self, key: &str) -> Option<CachedDecision> {
        let cache = self.decision_cache.read();
        if let Some(cached) = cache.get(key) {
            if cached.expires_at > SystemTime::now() {
                return Some(cached.clone());
            }
        }
        None
    }

    /// Cache a decision
    fn cache_decision(&self, key: String, decision: AccessDecision) {
        let expires_at = SystemTime::now() + Duration::from_secs(self.config.cache_ttl_secs);
        let cached = CachedDecision {
            decision,
            expires_at,
        };
        self.decision_cache.write().insert(key, cached);
    }

    /// Update statistics
    fn update_stats(&self, request: &ResourceAccessRequest, decision: &AccessDecision, eval_time_ms: f64) {
        let mut stats = self.stats.write();
        stats.total_requests += 1;
        *stats.decisions.entry(decision.decision.clone()).or_insert(0) += 1;
        *stats.resource_types.entry(request.resource_type.clone()).or_insert(0) += 1;
        
        // Update average evaluation time
        let total_time = stats.performance.avg_evaluation_time_ms * (stats.total_requests - 1) as f64 + eval_time_ms;
        stats.performance.avg_evaluation_time_ms = total_time / stats.total_requests as f64;
        
        stats.last_updated = SystemTime::now();
    }

    /// Update cache hit statistics
    fn update_stats_cache_hit(&self) {
        let mut stats = self.stats.write();
        if let Some(ref mut hit_rate) = stats.performance.cache_hit_rate {
            // Simple approximation - in reality you'd track hits vs misses
            *hit_rate = (*hit_rate * 0.9) + (1.0 * 0.1);
        }
    }
}

#[async_trait]
impl PolicyEnforcementPoint for DefaultPolicyEnforcementPoint {
    async fn check_resource_access(
        &self, 
        _agent_id: AgentId, 
        resource: &ResourceAccessRequest
    ) -> Result<AccessDecision, PolicyError> {
        self.evaluate_access(resource).await
    }
    
    async fn validate_resource_allocation(
        &self,
        _agent_id: AgentId,
        allocation: &ResourceAllocationRequest
    ) -> Result<AllocationDecision, PolicyError> {
        // Simplified allocation validation with updated thresholds
        let decision = if allocation.requirements.max_memory_mb > 16384 {
            AllocationResult::Escalate
        } else if allocation.requirements.max_memory_mb > 4096 {
            AllocationResult::Modified
        } else {
            AllocationResult::Approve
        };

        Ok(AllocationDecision {
            decision,
            reason: "Resource allocation validated".to_string(),
            modified_requirements: None,
            conditions: Vec::new(),
            expires_at: None,
            metadata: HashMap::new(),
        })
    }
    
    async fn load_policies(&self, config: &ResourceAccessConfig) -> Result<(), PolicyError> {
        if let Some(policy_path) = &config.policy_path {
            // In a real implementation, load from file
            let _ = policy_path;
            self.load_default_policies().await
        } else {
            self.load_default_policies().await
        }
    }
    
    async fn reload_policies(&self) -> Result<(), PolicyError> {
        self.load_policies(&self.config.clone()).await
    }
    
    async fn get_enforcement_stats(&self) -> Result<EnforcementStatistics, PolicyError> {
        Ok(self.stats.read().clone())
    }
}

/// Mock implementation for testing
pub struct MockPolicyEnforcementPoint {
    stats: Arc<RwLock<EnforcementStatistics>>,
}

impl Default for MockPolicyEnforcementPoint {
    fn default() -> Self {
        Self::new()
    }
}

impl MockPolicyEnforcementPoint {
    pub fn new() -> Self {
        Self {
            stats: Arc::new(RwLock::new(EnforcementStatistics {
                total_requests: 0,
                decisions: HashMap::new(),
                resource_types: HashMap::new(),
                performance: PerformanceMetrics {
                    avg_evaluation_time_ms: 1.0,
                    p95_evaluation_time_ms: 5.0,
                    cache_hit_rate: Some(0.95),
                    policy_reloads: 0,
                },
                last_updated: SystemTime::now(),
            })),
        }
    }
}

#[async_trait]
impl PolicyEnforcementPoint for MockPolicyEnforcementPoint {
    async fn check_resource_access(
        &self, 
        _agent_id: AgentId, 
        resource: &ResourceAccessRequest
    ) -> Result<AccessDecision, PolicyError> {
        // Update stats
        {
            let mut stats = self.stats.write();
            stats.total_requests += 1;
            *stats.resource_types.entry(resource.resource_type.clone()).or_insert(0) += 1;
        }

        // Simple mock logic
        let decision = match &resource.resource_type {
            ResourceType::File => {
                if resource.resource_id.contains("/etc/") || resource.resource_id.contains("/sys/") {
                    AccessResult::Deny
                } else {
                    AccessResult::Allow
                }
            },
            ResourceType::Network => {
                if resource.resource_id.contains("malicious") {
                    AccessResult::Deny
                } else {
                    AccessResult::Allow
                }
            },
            _ => AccessResult::Allow,
        };

        {
            let mut stats = self.stats.write();
            *stats.decisions.entry(decision.clone()).or_insert(0) += 1;
            stats.last_updated = SystemTime::now();
        }

        Ok(AccessDecision {
            decision,
            reason: "Mock policy evaluation".to_string(),
            applied_rule: Some("mock-rule".to_string()),
            conditions: Vec::new(),
            expires_at: None,
            metadata: HashMap::new(),
        })
    }
    
    async fn validate_resource_allocation(
        &self,
        _agent_id: AgentId,
        _allocation: &ResourceAllocationRequest
    ) -> Result<AllocationDecision, PolicyError> {
        Ok(AllocationDecision {
            decision: AllocationResult::Approve,
            reason: "Mock allocation approval".to_string(),
            modified_requirements: None,
            conditions: Vec::new(),
            expires_at: None,
            metadata: HashMap::new(),
        })
    }
    
    async fn load_policies(&self, _config: &ResourceAccessConfig) -> Result<(), PolicyError> {
        let mut stats = self.stats.write();
        stats.performance.policy_reloads += 1;
        Ok(())
    }
    
    async fn reload_policies(&self) -> Result<(), PolicyError> {
        let mut stats = self.stats.write();
        stats.performance.policy_reloads += 1;
        Ok(())
    }
    
    async fn get_enforcement_stats(&self) -> Result<EnforcementStatistics, PolicyError> {
        Ok(self.stats.read().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_default_enforcement_point_creation() {
        let config = ResourceAccessConfig::default();
        let enforcement_point = DefaultPolicyEnforcementPoint::new(config).await;
        assert!(enforcement_point.is_ok());
    }

    #[tokio::test]
    async fn test_mock_enforcement_point() {
        let enforcement_point = MockPolicyEnforcementPoint::new();
        let agent_id = AgentId::new();
        
        let request = ResourceAccessRequest {
            resource_type: ResourceType::File,
            resource_id: "/tmp/test.txt".to_string(),
            access_type: AccessType::Read,
            context: AccessContext {
                agent_metadata: AgentMetadata {
                    version: "1.0.0".to_string(),
                    author: "test".to_string(),
                    description: "Test agent".to_string(),
                    capabilities: vec![],
                    dependencies: vec![],
                    resource_requirements: crate::types::agent::ResourceRequirements::default(),
                    security_requirements: crate::types::agent::SecurityRequirements::default(),
                    custom_fields: HashMap::new(),
                },
                security_level: SecurityTier::Tier1,
                access_history: Vec::new(),
                resource_usage: ResourceUsage::default(),
                environment: HashMap::new(),
                source_info: SourceInfo {
                    ip_address: None,
                    user_agent: None,
                    session_id: None,
                    request_id: "test-request".to_string(),
                },
            },
            timestamp: SystemTime::now(),
        };

        let decision = enforcement_point.check_resource_access(agent_id, &request).await.unwrap();
        assert_eq!(decision.decision, AccessResult::Allow);
    }

    #[tokio::test]
    async fn test_mock_enforcement_point_deny() {
        let enforcement_point = MockPolicyEnforcementPoint::new();
        let agent_id = AgentId::new();
        
        let request = ResourceAccessRequest {
            resource_type: ResourceType::File,
            resource_id: "/etc/passwd".to_string(),
            access_type: AccessType::Read,
            context: AccessContext {
                agent_metadata: AgentMetadata {
                    version: "1.0.0".to_string(),
                    author: "test".to_string(),
                    description: "Test agent".to_string(),
                    capabilities: vec![],
                    dependencies: vec![],
                    resource_requirements: crate::types::agent::ResourceRequirements::default(),
                    security_requirements: crate::types::agent::SecurityRequirements::default(),
                    custom_fields: HashMap::new(),
                },
                security_level: SecurityTier::Tier1,
                access_history: Vec::new(),
                resource_usage: ResourceUsage::default(),
                environment: HashMap::new(),
                source_info: SourceInfo {
                    ip_address: None,
                    user_agent: None,
                    session_id: None,
                    request_id: "test-request".to_string(),
                },
            },
            timestamp: SystemTime::now(),
        };

        let decision = enforcement_point.check_resource_access(agent_id, &request).await.unwrap();
        assert_eq!(decision.decision, AccessResult::Deny);
    }
}