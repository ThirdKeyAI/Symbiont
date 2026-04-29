//! Security-related types and data structures

use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};

use super::{AgentId, PolicyId};

/// Security tiers for sandboxing.
///
/// `Tier1` → `Tier3` form a monotonically increasing host-isolation ladder
/// (Docker → gVisor → Firecracker). `Hosted` is **not** a peer on that
/// ladder — it represents execution on third-party infrastructure (e.g.
/// E2B) where the operator does not run their own sandbox host. It carries
/// no on-host isolation guarantees but is not outright `None`, so it sits
/// between `None` and `Tier1` for ordering purposes.
///
/// Use `tier >= SecurityTier::Tier1` when policies require host isolation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum SecurityTier {
    /// No isolation — native execution (⚠️ DEVELOPMENT ONLY)
    None,
    /// Hosted cloud execution (e.g. E2B) — code runs on third-party
    /// infrastructure. No on-host isolation guarantees; trust assumption
    /// is "you trust the hosting provider."
    Hosted,
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
            SecurityTier::None => write!(f, "None (Native - No Isolation ⚠️)"),
            SecurityTier::Hosted => write!(f, "Hosted (third-party cloud sandbox)"),
            SecurityTier::Tier1 => write!(f, "Tier1 (Docker)"),
            SecurityTier::Tier2 => write!(f, "Tier2 (gVisor)"),
            SecurityTier::Tier3 => write!(f, "Tier3 (Firecracker)"),
        }
    }
}

impl SecurityTier {
    /// Map a DSL `sandbox_tier` value to the runtime's `SecurityTier`.
    ///
    /// `dsl::SandboxTier::E2B` maps to `Hosted` — it is not a host-
    /// isolation tier. The runner factory still picks the actual E2B
    /// backend; this mapping only affects how the agent's security
    /// requirements are categorised in policy decisions.
    pub fn from_dsl_sandbox(tier: &dsl::SandboxTier) -> Self {
        match tier {
            dsl::SandboxTier::Docker => SecurityTier::Tier1,
            dsl::SandboxTier::GVisor => SecurityTier::Tier2,
            dsl::SandboxTier::Firecracker => SecurityTier::Tier3,
            dsl::SandboxTier::E2B => SecurityTier::Hosted,
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
    pub e2b_api_key: Option<String>,
    /// Allow native execution without isolation (⚠️ DEVELOPMENT ONLY)
    pub allow_native_execution: bool,
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
            e2b_api_key: None,
            allow_native_execution: false,
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
    CronJobDeadLettered,
    AgentPinVerificationFailed,
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

/// Represents different types of capabilities that agents can request
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Capability {
    /// File system operations
    FileRead(String),
    FileWrite(String),
    FileDelete(String),

    /// Network operations
    NetworkRequest(String),
    NetworkListen(u16),

    /// System operations
    Execute(String),
    EnvironmentRead(String),
    EnvironmentWrite(String),

    /// Agent operations
    AgentCreate,
    AgentDelete,
    AgentModify,

    /// Data operations
    DataRead(String),
    DataWrite(String),
    DataDelete(String),
}

#[cfg(test)]
mod from_dsl_sandbox_tests {
    use super::*;

    #[test]
    fn dsl_sandbox_maps_to_runtime_tier() {
        assert_eq!(
            SecurityTier::from_dsl_sandbox(&dsl::SandboxTier::Docker),
            SecurityTier::Tier1
        );
        assert_eq!(
            SecurityTier::from_dsl_sandbox(&dsl::SandboxTier::GVisor),
            SecurityTier::Tier2
        );
        assert_eq!(
            SecurityTier::from_dsl_sandbox(&dsl::SandboxTier::Firecracker),
            SecurityTier::Tier3
        );
        // E2B is hosted (third-party cloud), not a host-isolation tier.
        assert_eq!(
            SecurityTier::from_dsl_sandbox(&dsl::SandboxTier::E2B),
            SecurityTier::Hosted
        );
    }

    #[test]
    fn hosted_orders_below_tier1() {
        // Policies that require host isolation use `tier >= Tier1`. Hosted
        // must sort below Tier1 so those checks reject it.
        assert!(SecurityTier::Hosted < SecurityTier::Tier1);
        assert!(SecurityTier::Hosted < SecurityTier::Tier2);
        assert!(SecurityTier::Hosted < SecurityTier::Tier3);
        assert!(SecurityTier::None < SecurityTier::Hosted);
    }
}
