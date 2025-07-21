//! External Integration Interfaces
//! 
//! Provides interfaces for integrating with external security and policy components

pub mod policy_engine;
pub mod sandbox_orchestrator;
pub mod schemapin;
pub mod mcp;
pub mod tool_invocation;

// Enterprise-only modules (gated behind feature flag)
#[cfg(feature = "enterprise")]
pub mod audit_trail;

#[cfg(feature = "enterprise")]
pub mod tool_review;

// Re-export specific types to avoid naming conflicts
pub use policy_engine::{
    PolicyEngine, MockPolicyEngine,
    PolicyEnforcementPoint, PolicyEnforcementFactory, ResourceAccessConfig,
    DefaultPolicyEnforcementPoint, MockPolicyEnforcementPoint,
    ResourceAccessRequest, ResourceType, AccessType, AccessContext, SourceInfo,
    AccessDecision, AllocationDecision, AllocationResult,
    ResourceAllocationRequest, EnforcementStatistics
};
pub use sandbox_orchestrator::{
    SandboxOrchestrator, MockSandboxOrchestrator, SandboxRequest, SandboxInfo,
    SandboxType, SandboxConfig, SandboxStatus, SandboxCommand, CommandResult,
    SnapshotId as SandboxSnapshotId
};
#[cfg(feature = "enterprise")]
pub use audit_trail::{
    AuditTrail, MockAuditTrail, AuditEvent, AuditRecord, AuditQuery,
    AuditEventType, AuditSeverity, AuditCategory, AuditOutcome,
    SnapshotId as AuditSnapshotId
};
pub use schemapin::{
    SchemaPinCli, SchemaPinCliWrapper, MockSchemaPinCli,
    SchemaPinClient, NativeSchemaPinClient, MockNativeSchemaPinClient, DefaultSchemaPinClient,
    SchemaPinConfig, SchemaPinError, VerificationResult, VerifyArgs, SignatureInfo,
    SigningResult, SignArgs, LocalKeyStore, KeyStoreConfig, PinnedKey
};
pub use mcp::{
    McpClient, SecureMcpClient, MockMcpClient,
    McpTool, McpClientConfig, McpClientError, ToolProvider, VerificationStatus,
    ToolDiscoveryEvent, ToolVerificationRequest, ToolVerificationResponse
};
#[cfg(feature = "enterprise")]
pub use tool_review::{
    ToolReviewOrchestrator, WorkflowEvent, WorkflowEventHandler, WorkflowStats,
    SecurityAnalyzer, AISecurityAnalyzer, SecurityAnalyzerConfig,
    HumanReviewInterface, StandardReviewInterface, ReviewPresentation,
    ReviewInterfaceConfig, RiskLevel, CriticalFinding, ReviewSummary,
    ToolReviewState, ToolReviewSession, SecurityAnalysis, SecurityFinding,
    SecuritySeverity, SecurityCategory, ReviewRecommendation, ToolReviewConfig,
    ToolReviewError, ToolReviewResult, ReviewId, AnalysisId
};
pub use tool_invocation::{
    ToolInvocationEnforcer, DefaultToolInvocationEnforcer, ToolInvocationError,
    EnforcementPolicy, InvocationEnforcementConfig, InvocationContext,
    InvocationResult, EnforcementDecision,
};

// Re-export error types from the types module
pub use crate::types::{PolicyError, SandboxError, AuditError};