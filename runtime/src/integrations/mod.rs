//! External Integration Interfaces
//! 
//! Provides interfaces for integrating with external security and policy components

pub mod policy_engine;
pub mod sandbox_orchestrator;
pub mod audit_trail;

// Re-export specific types to avoid naming conflicts
pub use policy_engine::{PolicyEngine, MockPolicyEngine, PolicyRequest, PolicyDecision};
pub use sandbox_orchestrator::{
    SandboxOrchestrator, MockSandboxOrchestrator, SandboxRequest, SandboxInfo,
    SandboxType, SandboxConfig, SandboxStatus, SandboxCommand, CommandResult,
    SnapshotId as SandboxSnapshotId
};
pub use audit_trail::{
    AuditTrail, MockAuditTrail, AuditEvent, AuditRecord, AuditQuery,
    AuditEventType, AuditSeverity, AuditCategory, AuditOutcome,
    SnapshotId as AuditSnapshotId
};

// Re-export error types from the types module
pub use crate::types::{PolicyError, SandboxError, AuditError};