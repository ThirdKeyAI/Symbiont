//! Resource Access Management Types
//!
//! Data structures for policy-based resource access control

use crate::types::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, SystemTime};

/// Condition types for policy decisions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConditionType {
    ResourceLimit,
    TimeWindow,
    ApprovalRequired,
    AuditRequired,
    SecurityScan,
    RateLimited,
    SecretRequired,
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
    SecretMatch {
        secret_name: String,
        permissions: Option<Vec<String>>,
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

/// Audit levels for policy actions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditLevel {
    Info,
    Warning,
    Critical,
}

/// Resource access request from an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceAccessRequest {
    /// Type of resource being accessed
    pub resource_type: ResourceType,
    /// Specific resource identifier (e.g., file path, network endpoint)
    pub resource_id: String,
    /// Type of access being requested
    pub access_type: AccessType,
    /// Additional context for the request
    pub context: AccessContext,
    /// Timestamp of the request
    pub timestamp: SystemTime,
}

/// Types of resources that can be accessed
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResourceType {
    /// File system resources
    File,
    /// Network endpoints
    Network,
    /// System commands/executables
    Command,
    /// Database connections
    Database,
    /// Environment variables
    Environment,
    /// Inter-agent communication
    Agent,
    /// Custom resource type
    Custom(String),
}

/// Types of access operations
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AccessType {
    /// Read access
    Read,
    /// Write access
    Write,
    /// Execute access
    Execute,
    /// Delete access
    Delete,
    /// Create new resource
    Create,
    /// Modify existing resource
    Modify,
    /// List/enumerate resources
    List,
    /// Connect to resource
    Connect,
}

/// Context information for access requests
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessContext {
    /// Agent metadata
    pub agent_metadata: AgentMetadata,
    /// Current security level
    pub security_level: SecurityTier,
    /// Previous access history
    pub access_history: Vec<AccessHistoryEntry>,
    /// Current resource usage
    pub resource_usage: ResourceUsage,
    /// Environment variables relevant to access
    pub environment: HashMap<String, String>,
    /// Request source information
    pub source_info: SourceInfo,
}

/// Historical access entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessHistoryEntry {
    pub resource_type: ResourceType,
    pub resource_id: String,
    pub access_type: AccessType,
    pub timestamp: SystemTime,
    pub decision: AccessDecision,
}

/// Source information for the access request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceInfo {
    /// IP address if network-based
    pub ip_address: Option<String>,
    /// User agent or client identifier
    pub user_agent: Option<String>,
    /// Session identifier
    pub session_id: Option<String>,
    /// Request ID for tracing
    pub request_id: String,
}

/// Decision result for resource access requests
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessDecision {
    /// Final access decision
    pub decision: AccessResult,
    /// Human-readable reason for the decision
    pub reason: String,
    /// Policy rule that was applied
    pub applied_rule: Option<String>,
    /// Conditions that must be met for access
    pub conditions: Vec<AccessCondition>,
    /// Time until decision expires
    pub expires_at: Option<SystemTime>,
    /// Additional metadata
    pub metadata: HashMap<String, String>,
}

/// Access decision results
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AccessResult {
    /// Access granted unconditionally
    Allow,
    /// Access denied
    Deny,
    /// Access granted with conditions
    Conditional,
    /// Decision deferred to higher authority
    Escalate,
}

/// Conditions that must be met for conditional access
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessCondition {
    /// Type of condition
    pub condition_type: ConditionType,
    /// Parameters for the condition
    pub parameters: HashMap<String, String>,
    /// Timeout for condition validation
    pub timeout: Option<Duration>,
    /// Whether condition must be met before access
    pub blocking: bool,
}

/// Resource allocation request from an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceAllocationRequest {
    /// Agent requesting allocation
    pub agent_id: AgentId,
    /// Resource requirements
    pub requirements: ResourceRequirements,
    /// Requested priority level
    pub priority: Priority,
    /// Justification for the request
    pub justification: Option<String>,
    /// Maximum time to hold allocation
    pub max_duration: Option<Duration>,
    /// Request timestamp
    pub timestamp: SystemTime,
}

/// Decision result for resource allocation requests
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllocationDecision {
    /// Final allocation decision
    pub decision: AllocationResult,
    /// Reason for the decision
    pub reason: String,
    /// Modified resource limits (if approved with modifications)
    pub modified_requirements: Option<ResourceRequirements>,
    /// Conditions for allocation
    pub conditions: Vec<AllocationCondition>,
    /// Time until decision expires
    pub expires_at: Option<SystemTime>,
    /// Policy metadata
    pub metadata: HashMap<String, String>,
}

/// Allocation decision results
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AllocationResult {
    /// Allocation approved as requested
    Approve,
    /// Allocation denied
    Deny,
    /// Allocation approved with modifications
    Modified,
    /// Allocation queued for later
    Queued,
    /// Decision escalated for manual review
    Escalate,
}

/// Conditions for resource allocations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllocationCondition {
    /// Type of condition
    pub condition_type: AllocationConditionType,
    /// Parameters for the condition
    pub parameters: HashMap<String, String>,
    /// Whether this condition blocks allocation
    pub blocking: bool,
}

/// Types of allocation conditions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AllocationConditionType {
    /// Must be within resource quotas
    QuotaCheck,
    /// Requires approval
    ApprovalRequired,
    /// Time-based restrictions
    TimeRestriction,
    /// Priority-based queuing
    PriorityQueue,
    /// Security scan required
    SecurityScan,
}

/// Resource access policy definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceAccessPolicy {
    /// Policy identifier
    pub id: String,
    /// Policy name
    pub name: String,
    /// Policy description
    pub description: String,
    /// Policy version
    pub version: String,
    /// Whether policy is enabled
    pub enabled: bool,
    /// Policy priority (higher number = higher priority)
    pub priority: u32,
    /// Rules in this policy
    pub rules: Vec<ResourceAccessRule>,
    /// Policy metadata
    pub metadata: HashMap<String, String>,
    /// Creation timestamp
    pub created_at: SystemTime,
    /// Last update timestamp
    pub updated_at: SystemTime,
}

/// Individual rule within a resource access policy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceAccessRule {
    /// Rule identifier
    pub id: String,
    /// Rule name
    pub name: String,
    /// Rule description
    pub description: String,
    /// Conditions that must match for rule to apply
    pub conditions: Vec<RuleCondition>,
    /// Effect of the rule when conditions match
    pub effect: RuleEffect,
    /// Priority within the policy
    pub priority: u32,
    /// Rule metadata
    pub metadata: HashMap<String, String>,
}

/// Rule effects when conditions are met
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RuleEffect {
    /// Allow the access
    Allow { conditions: Vec<AccessCondition> },
    /// Deny the access
    Deny { reason: String },
    /// Apply specific limits
    Limit { limits: ResourceConstraints },
    /// Require audit logging
    Audit { level: AuditLevel },
    /// Escalate to manual review
    Escalate { to: String, reason: String },
}

/// Resource constraints for limiting access
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceConstraints {
    /// Maximum number of simultaneous accesses
    pub max_concurrent_access: Option<u32>,
    /// Rate limiting (accesses per time period)
    pub rate_limit: Option<RateLimit>,
    /// Data transfer limits
    pub transfer_limits: Option<TransferLimits>,
    /// Time-based restrictions
    pub time_restrictions: Option<Vec<TimeWindow>>,
}

/// Rate limiting configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimit {
    /// Number of requests allowed
    pub requests: u32,
    /// Time window for the limit
    pub window: Duration,
    /// Burst allowance
    pub burst: Option<u32>,
}

/// Data transfer limits
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferLimits {
    /// Maximum bytes per request
    pub max_bytes_per_request: Option<u64>,
    /// Maximum total bytes per time window
    pub max_bytes_per_window: Option<u64>,
    /// Time window for transfer limits
    pub window: Duration,
}

/// Policy enforcement statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnforcementStatistics {
    /// Total access requests processed
    pub total_requests: u64,
    /// Breakdown by decision type
    pub decisions: HashMap<AccessResult, u64>,
    /// Breakdown by resource type
    pub resource_types: HashMap<ResourceType, u64>,
    /// Policy evaluation performance
    pub performance: PerformanceMetrics,
    /// Last update time
    pub last_updated: SystemTime,
}

/// Performance metrics for policy evaluation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceMetrics {
    /// Average evaluation time in milliseconds
    pub avg_evaluation_time_ms: f64,
    /// 95th percentile evaluation time
    pub p95_evaluation_time_ms: f64,
    /// Cache hit rate (if caching enabled)
    pub cache_hit_rate: Option<f64>,
    /// Number of policy reloads
    pub policy_reloads: u64,
}

/// Default policy definitions in YAML format
pub const DEFAULT_POLICIES_YAML: &str = r#"
policies:
  - id: "default-file-access"
    name: "Default File Access Policy"
    description: "Default policy for file system access"
    version: "1.0.0"
    enabled: true
    priority: 1000
    rules:
      - id: "allow-read-temp"
        name: "Allow read access to temp directories"
        description: "Allow agents to read from temporary directories"
        conditions:
          - type: "resource_match"
            parameters:
              resource_type: "File"
              patterns: ["/tmp/*", "/var/tmp/*"]
          - type: "access_match"
            parameters:
              access_types: ["Read", "List"]
        effect:
          type: "Allow"
          conditions: []
        priority: 100
      
      - id: "deny-system-write"
        name: "Deny write access to system directories"
        description: "Prevent agents from writing to critical system directories"
        conditions:
          - type: "resource_match"
            parameters:
              resource_type: "File"
              patterns: ["/etc/*", "/sys/*", "/proc/*", "/boot/*"]
          - type: "access_match"
            parameters:
              access_types: ["Write", "Create", "Delete", "Modify"]
        effect:
          type: "Deny"
          reason: "System directory write access not permitted"
        priority: 500

  - id: "default-network-access"
    name: "Default Network Access Policy"
    description: "Default policy for network access"
    version: "1.0.0"
    enabled: true
    priority: 1000
    rules:
      - id: "allow-http-outbound"
        name: "Allow outbound HTTP/HTTPS"
        description: "Allow agents to make outbound HTTP/HTTPS requests"
        conditions:
          - type: "resource_match"
            parameters:
              resource_type: "Network"
              patterns: ["http://*", "https://*"]
          - type: "access_match"
            parameters:
              access_types: ["Connect"]
        effect:
          type: "Allow"
          conditions:
            - type: "rate_limit"
              parameters:
                requests: "100"
                window: "60s"
        priority: 100
      
      - id: "deny-local-network"
        name: "Deny local network access"
        description: "Prevent access to local network ranges"
        conditions:
          - type: "resource_match"
            parameters:
              resource_type: "Network"
              patterns: ["192.168.*", "10.*", "172.16.*", "127.*"]
        effect:
          type: "Deny"
          reason: "Local network access not permitted"
        priority: 500

  - id: "resource-allocation-limits"
    name: "Resource Allocation Limits"
    description: "Default limits for resource allocation"
    version: "1.0.0"
    enabled: true
    priority: 1000
    rules:
      - id: "basic-agent-limits"
        name: "Basic agent resource limits"
        description: "Standard resource limits for basic agents"
        conditions:
          - type: "security_level"
            parameters:
              levels: ["Tier1", "Tier2"]
        effect:
          type: "Limit"
          limits:
            max_memory_mb: 512
            max_cpu_cores: 1.0
            max_disk_io_mbps: 100
            max_network_io_mbps: 100
        priority: 100
      
      - id: "privileged-agent-limits"
        name: "Privileged agent resource limits"
        description: "Higher resource limits for privileged agents"
        conditions:
          - type: "security_level"
            parameters:
              levels: ["Tier3", "Tier4"]
        effect:
          type: "Limit"
          limits:
            max_memory_mb: 2048
            max_cpu_cores: 4.0
            max_disk_io_mbps: 500
            max_network_io_mbps: 500
        priority: 200
"#;
