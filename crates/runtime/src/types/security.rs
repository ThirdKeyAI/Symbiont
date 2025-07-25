//! Security-related types and data structures

use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};

use super::{AgentId, PolicyId};

/// Security tiers for sandboxing
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum SecurityTier {
    /// Docker-based isolation
    #[default]
    Tier1,
    /// gVisor-based isolation
    Tier2,
    /// Firecracker-based isolation
    Tier3,
}

impl std::fmt::Display for SecurityTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SecurityTier::Tier1 => write!(f, "Tier1 (Docker)"),
            SecurityTier::Tier2 => write!(f, "Tier2 (gVisor)"),
            SecurityTier::Tier3 => write!(f, "Tier3 (Firecracker)"),
        }
    }
}

/// Risk assessment levels
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum RiskLevel {
    Low,
    #[default]
    Medium,
    High,
    Critical,
}

/// Security configuration for the runtime
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    pub default_security_tier: SecurityTier,
    pub encryption_enabled: bool,
    pub signature_required: bool,
    pub policy_enforcement_strict: bool,
    pub sandbox_isolation_level: IsolationLevel,
    pub audit_all_operations: bool,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            default_security_tier: SecurityTier::Tier1,
            encryption_enabled: true,
            signature_required: true,
            policy_enforcement_strict: true,
            sandbox_isolation_level: IsolationLevel::High,
            audit_all_operations: true,
        }
    }
}

/// Isolation levels for sandboxing
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum IsolationLevel {
    None,
    Low,
    Medium,
    #[default]
    High,
    Maximum,
}

/// Policy context for enforcement decisions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyContext {
    pub agent_id: AgentId,
    pub operation: String,
    pub resource: Option<String>,
    pub timestamp: SystemTime,
    pub security_tier: SecurityTier,
    pub risk_level: RiskLevel,
}

/// Policy decision result
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyDecision {
    Allow,
    Deny(String),
    RequireApproval(String),
}

/// Policy enforcement result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyResult {
    pub decision: PolicyDecision,
    pub policy_id: Option<PolicyId>,
    pub reason: Option<String>,
    pub timestamp: SystemTime,
}

impl PolicyResult {
    pub fn allow() -> Self {
        Self {
            decision: PolicyDecision::Allow,
            policy_id: None,
            reason: None,
            timestamp: SystemTime::now(),
        }
    }

    pub fn deny(reason: String) -> Self {
        Self {
            decision: PolicyDecision::Deny(reason.clone()),
            policy_id: None,
            reason: Some(reason),
            timestamp: SystemTime::now(),
        }
    }

    pub fn require_approval(reason: String) -> Self {
        Self {
            decision: PolicyDecision::RequireApproval(reason.clone()),
            policy_id: None,
            reason: Some(reason),
            timestamp: SystemTime::now(),
        }
    }
}

/// Types of security events
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecurityEventType {
    PolicyViolation,
    UnauthorizedAccess,
    EncryptionFailure,
    SignatureVerificationFailure,
    SandboxBreach,
    ResourceExhaustion,
    SuspiciousActivity,
}

/// Policy violation details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyViolation {
    pub policy_id: PolicyId,
    pub violation_type: String,
    pub description: String,
    pub severity: ViolationSeverity,
    pub timestamp: SystemTime,
}

/// Severity levels for policy violations
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum ViolationSeverity {
    Info,
    #[default]
    Warning,
    Error,
    Critical,
}

/// Audit event types for the cryptographic audit trail
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditEvent {
    AgentCreated {
        agent_id: AgentId,
        config_hash: String,
    },
    AgentStarted {
        agent_id: AgentId,
        timestamp: SystemTime,
    },
    AgentTerminated {
        agent_id: AgentId,
        reason: super::agent::TerminationReason,
    },
    MessageSent {
        from: AgentId,
        to: Option<AgentId>,
        message_id: super::MessageId,
    },
    PolicyViolation {
        agent_id: AgentId,
        violation: PolicyViolation,
    },
    ResourceAllocation {
        agent_id: AgentId,
        resources: super::resource::ResourceAllocation,
    },
    SecurityEvent {
        event_type: SecurityEventType,
        details: String,
    },
}

/// Audit query for searching events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditQuery {
    pub agent_id: Option<AgentId>,
    pub event_types: Vec<String>,
    pub start_time: Option<SystemTime>,
    pub end_time: Option<SystemTime>,
    pub limit: Option<usize>,
}

/// Audit configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditConfig {
    pub enabled: bool,
    pub sign_events: bool,
    pub encrypt_events: bool,
    pub retention_duration: Duration,
    pub max_events_per_agent: usize,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            sign_events: true,
            encrypt_events: true,
            retention_duration: Duration::from_secs(86400 * 365), // 1 year
            max_events_per_agent: 10000,
        }
    }
}

/// Sandbox configuration for different security tiers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    pub security_tier: SecurityTier,
    pub isolation_level: IsolationLevel,
    pub network_isolation: bool,
    pub filesystem_isolation: bool,
    pub resource_limits: super::resource::ResourceLimits,
    pub allowed_syscalls: Vec<String>,
    pub environment_variables: std::collections::HashMap<String, String>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            security_tier: SecurityTier::Tier1,
            isolation_level: IsolationLevel::High,
            network_isolation: true,
            filesystem_isolation: true,
            resource_limits: super::resource::ResourceLimits::default(),
            allowed_syscalls: vec![
                "read".to_string(),
                "write".to_string(),
                "open".to_string(),
                "close".to_string(),
            ],
            environment_variables: std::collections::HashMap::new(),
        }
    }
}

/// Sandbox status information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxStatus {
    pub id: String,
    pub state: SandboxState,
    pub security_tier: SecurityTier,
    pub resource_usage: super::resource::ResourceUsage,
    pub uptime: Duration,
    pub last_activity: SystemTime,
}

/// Sandbox state
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SandboxState {
    #[default]
    Creating,
    Ready,
    Running,
    Suspended,
    Terminating,
    Terminated,
    Failed,
}
