//! Policy Engine Integration Interface
//! 
//! Provides interface for integrating with external policy enforcement engines

use std::collections::HashMap;
use std::time::SystemTime;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::types::*;

/// Policy engine trait for external policy enforcement
#[async_trait]
pub trait PolicyEngine: Send + Sync {
    /// Evaluate a policy for an agent action
    async fn evaluate_policy(&self, request: PolicyRequest) -> Result<PolicyDecision, PolicyError>;
    
    /// Register a new policy
    async fn register_policy(&self, policy: Policy) -> Result<PolicyId, PolicyError>;
    
    /// Update an existing policy
    async fn update_policy(&self, policy_id: PolicyId, policy: Policy) -> Result<(), PolicyError>;
    
    /// Delete a policy
    async fn delete_policy(&self, policy_id: PolicyId) -> Result<(), PolicyError>;
    
    /// List all policies
    async fn list_policies(&self) -> Result<Vec<PolicyInfo>, PolicyError>;
    
    /// Get policy by ID
    async fn get_policy(&self, policy_id: PolicyId) -> Result<Policy, PolicyError>;
    
    /// Validate policy syntax
    async fn validate_policy(&self, policy: &Policy) -> Result<ValidationResult, PolicyError>;
    
    /// Get policy evaluation statistics
    async fn get_policy_stats(&self) -> Result<PolicyStatistics, PolicyError>;
}

/// Policy request for evaluation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRequest {
    pub agent_id: AgentId,
    pub action: AgentAction,
    pub context: PolicyContext,
    pub timestamp: SystemTime,
}

/// Agent action being evaluated
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentAction {
    Execute {
        command: String,
        args: Vec<String>,
    },
    NetworkAccess {
        destination: String,
        port: u16,
        protocol: NetworkProtocol,
    },
    FileAccess {
        path: String,
        operation: FileOperation,
    },
    ResourceAllocation {
        resource_type: String, // Resource type as string
        amount: u64,
    },
    Communication {
        target: AgentId,
        message_type: String,
    },
    StateTransition {
        from_state: AgentState,
        to_state: AgentState,
    },
}

/// Network protocol types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetworkProtocol {
    TCP,
    UDP,
    HTTP,
    HTTPS,
    WebSocket,
}

/// File operation types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FileOperation {
    Read,
    Write,
    Execute,
    Delete,
    Create,
    Modify,
}

/// Policy context for evaluation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyContext {
    pub agent_metadata: AgentMetadata,
    pub resource_usage: ResourceUsage,
    pub security_level: SecurityTier,
    pub environment: HashMap<String, String>,
    pub previous_actions: Vec<AgentAction>,
}

/// Policy decision result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyDecision {
    pub decision: Decision,
    pub reason: String,
    pub conditions: Vec<PolicyCondition>,
    pub metadata: HashMap<String, String>,
    pub expires_at: Option<SystemTime>,
}

/// Policy decision types
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Decision {
    Allow,
    Deny,
    Conditional,
    Defer,
}

/// Policy condition for conditional decisions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyCondition {
    pub condition_type: ConditionType,
    pub parameters: HashMap<String, String>,
    pub timeout: Option<Duration>,
}

/// Condition types for policy decisions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConditionType {
    ResourceLimit,
    TimeWindow,
    ApprovalRequired,
    AuditRequired,
    SecurityScan,
    RateLimited,
}

/// Policy definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub id: Option<PolicyId>,
    pub name: String,
    pub description: String,
    pub version: String,
    pub rules: Vec<PolicyRule>,
    pub priority: u32,
    pub enabled: bool,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
    pub tags: Vec<String>,
}

/// Policy rule definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub id: String,
    pub condition: RuleCondition,
    pub action: RuleAction,
    pub metadata: HashMap<String, String>,
}

/// Rule condition for policy evaluation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RuleCondition {
    AgentMatch {
        patterns: Vec<String>,
    },
    ActionMatch {
        action_types: Vec<String>,
    },
    ResourceMatch {
        resource_patterns: Vec<String>,
    },
    TimeMatch {
        time_windows: Vec<TimeWindow>,
    },
    SecurityLevelMatch {
        levels: Vec<SecurityTier>,
    },
    And {
        conditions: Vec<RuleCondition>,
    },
    Or {
        conditions: Vec<RuleCondition>,
    },
    Not {
        condition: Box<RuleCondition>,
    },
}

/// Time window for policy rules
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeWindow {
    pub start_time: String, // HH:MM format
    pub end_time: String,   // HH:MM format
    pub days: Vec<Weekday>,
    pub timezone: String,
}

/// Days of the week
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Weekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

/// Rule action for policy decisions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RuleAction {
    Allow {
        conditions: Vec<PolicyCondition>,
    },
    Deny {
        reason: String,
    },
    Require {
        requirements: Vec<String>,
    },
    Limit {
        limits: HashMap<String, u64>,
    },
    Audit {
        level: AuditLevel,
    },
    Escalate {
        to: String,
    },
}

/// Audit levels for policy actions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditLevel {
    Info,
    Warning,
    Critical,
}

/// Policy information for listing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyInfo {
    pub id: PolicyId,
    pub name: String,
    pub description: String,
    pub version: String,
    pub priority: u32,
    pub enabled: bool,
    pub rule_count: u32,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

/// Policy validation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    pub valid: bool,
    pub errors: Vec<ValidationError>,
    pub warnings: Vec<ValidationWarning>,
}

/// Policy validation error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    pub rule_id: Option<String>,
    pub error_type: String,
    pub message: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

/// Policy validation warning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationWarning {
    pub rule_id: Option<String>,
    pub warning_type: String,
    pub message: String,
    pub suggestion: Option<String>,
}

/// Policy evaluation statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyStatistics {
    pub total_evaluations: u64,
    pub decisions: HashMap<Decision, u64>,
    pub policy_usage: HashMap<PolicyId, u64>,
    pub average_evaluation_time: Duration,
    pub error_rate: f64,
    pub last_updated: SystemTime,
}

/// Policy identifier (re-export from types)
pub use crate::types::PolicyId;

/// Duration type alias
pub type Duration = std::time::Duration;

/// Mock policy engine for testing and development
pub struct MockPolicyEngine {
    policies: std::sync::RwLock<HashMap<PolicyId, Policy>>,
    stats: std::sync::RwLock<PolicyStatistics>,
}

impl MockPolicyEngine {
    pub fn new() -> Self {
        Self {
            policies: std::sync::RwLock::new(HashMap::new()),
            stats: std::sync::RwLock::new(PolicyStatistics {
                total_evaluations: 0,
                decisions: HashMap::new(),
                policy_usage: HashMap::new(),
                average_evaluation_time: Duration::from_millis(10),
                error_rate: 0.0,
                last_updated: SystemTime::now(),
            }),
        }
    }

    fn create_default_policy() -> Policy {
        Policy {
            id: Some(PolicyId::new()),
            name: "Default Allow Policy".to_string(),
            description: "Default policy that allows most actions".to_string(),
            version: "1.0.0".to_string(),
            rules: vec![PolicyRule {
                id: "default-allow".to_string(),
                condition: RuleCondition::AgentMatch {
                    patterns: vec!["*".to_string()],
                },
                action: RuleAction::Allow {
                    conditions: vec![],
                },
                metadata: HashMap::new(),
            }],
            priority: 1000,
            enabled: true,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            tags: vec!["default".to_string()],
        }
    }
}

impl Default for MockPolicyEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PolicyEngine for MockPolicyEngine {
    async fn evaluate_policy(&self, request: PolicyRequest) -> Result<PolicyDecision, PolicyError> {
        // Update statistics
        {
            let mut stats = self.stats.write().unwrap();
            stats.total_evaluations += 1;
            *stats.decisions.entry(Decision::Allow).or_insert(0) += 1;
            stats.last_updated = SystemTime::now();
        }

        // Simple mock evaluation - allow most actions
        let decision = match &request.action {
            AgentAction::Execute { command, .. } => {
                if command.contains("rm") || command.contains("delete") {
                    Decision::Conditional
                } else {
                    Decision::Allow
                }
            }
            AgentAction::NetworkAccess { destination, .. } => {
                if destination.contains("malicious") {
                    Decision::Deny
                } else {
                    Decision::Allow
                }
            }
            AgentAction::FileAccess { operation, .. } => {
                match operation {
                    FileOperation::Delete => Decision::Conditional,
                    _ => Decision::Allow,
                }
            }
            _ => Decision::Allow,
        };

        let conditions = if decision == Decision::Conditional {
            vec![PolicyCondition {
                condition_type: ConditionType::ApprovalRequired,
                parameters: HashMap::new(),
                timeout: Some(Duration::from_secs(300)),
            }]
        } else {
            vec![]
        };

        Ok(PolicyDecision {
            decision,
            reason: "Mock policy evaluation".to_string(),
            conditions,
            metadata: HashMap::new(),
            expires_at: None,
        })
    }

    async fn register_policy(&self, mut policy: Policy) -> Result<PolicyId, PolicyError> {
        let policy_id = PolicyId::new();
        policy.id = Some(policy_id);
        policy.created_at = SystemTime::now();
        policy.updated_at = SystemTime::now();

        self.policies.write().unwrap().insert(policy_id, policy);
        Ok(policy_id)
    }

    async fn update_policy(&self, policy_id: PolicyId, mut policy: Policy) -> Result<(), PolicyError> {
        policy.id = Some(policy_id);
        policy.updated_at = SystemTime::now();

        let mut policies = self.policies.write().unwrap();
        if policies.contains_key(&policy_id) {
            policies.insert(policy_id, policy);
            Ok(())
        } else {
            Err(PolicyError::PolicyNotFound { id: policy_id })
        }
    }

    async fn delete_policy(&self, policy_id: PolicyId) -> Result<(), PolicyError> {
        let mut policies = self.policies.write().unwrap();
        if policies.remove(&policy_id).is_some() {
            Ok(())
        } else {
            Err(PolicyError::PolicyNotFound { id: policy_id })
        }
    }

    async fn list_policies(&self) -> Result<Vec<PolicyInfo>, PolicyError> {
        let policies = self.policies.read().unwrap();
        let policy_infos = policies.values().map(|policy| PolicyInfo {
            id: policy.id.unwrap(),
            name: policy.name.clone(),
            description: policy.description.clone(),
            version: policy.version.clone(),
            priority: policy.priority,
            enabled: policy.enabled,
            rule_count: policy.rules.len() as u32,
            created_at: policy.created_at,
            updated_at: policy.updated_at,
        }).collect();

        Ok(policy_infos)
    }

    async fn get_policy(&self, policy_id: PolicyId) -> Result<Policy, PolicyError> {
        let policies = self.policies.read().unwrap();
        policies.get(&policy_id)
            .cloned()
            .ok_or(PolicyError::PolicyNotFound { id: policy_id })
    }

    async fn validate_policy(&self, _policy: &Policy) -> Result<ValidationResult, PolicyError> {
        // Mock validation - always valid
        Ok(ValidationResult {
            valid: true,
            errors: vec![],
            warnings: vec![],
        })
    }

    async fn get_policy_stats(&self) -> Result<PolicyStatistics, PolicyError> {
        Ok(self.stats.read().unwrap().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_policy_engine() {
        let engine = MockPolicyEngine::new();
        
        // Test policy registration
        let policy = MockPolicyEngine::create_default_policy();
        let policy_id = engine.register_policy(policy).await.unwrap();
        
        // Test policy retrieval
        let retrieved_policy = engine.get_policy(policy_id).await.unwrap();
        assert_eq!(retrieved_policy.name, "Default Allow Policy");
        
        // Test policy evaluation
        let request = PolicyRequest {
            agent_id: AgentId::new(),
            action: AgentAction::Execute {
                command: "ls".to_string(),
                args: vec!["-la".to_string()],
            },
            context: PolicyContext {
                agent_metadata: AgentMetadata {
                    version: "1.0.0".to_string(),
                    author: "test".to_string(),
                    description: "Test agent".to_string(),
                    capabilities: vec![],
                    dependencies: vec![],
                    resource_requirements: crate::types::agent::ResourceRequirements::default(),
                    security_requirements: crate::types::agent::SecurityRequirements::default(),
                    custom_fields: std::collections::HashMap::new(),
                },
                resource_usage: ResourceUsage {
                    memory_used: 1024 * 1024,
                    cpu_utilization: 1.0,
                    disk_io_rate: 0,
                    network_io_rate: 0,
                    uptime: std::time::Duration::from_secs(60),
                },
                security_level: SecurityTier::Tier2,
                environment: HashMap::new(),
                previous_actions: vec![],
            },
            timestamp: SystemTime::now(),
        };
        
        let decision = engine.evaluate_policy(request).await.unwrap();
        assert_eq!(decision.decision, Decision::Allow);
    }

    #[tokio::test]
    async fn test_policy_validation() {
        let engine = MockPolicyEngine::new();
        let policy = MockPolicyEngine::create_default_policy();
        
        let result = engine.validate_policy(&policy).await.unwrap();
        assert!(result.valid);
        assert!(result.errors.is_empty());
    }
}