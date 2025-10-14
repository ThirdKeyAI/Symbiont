//! External Integration Interfaces
//!
//! Provides interfaces for integrating with external security and policy components

pub mod mcp;
pub mod policy_engine;
pub mod sandbox_orchestrator;
pub mod schemapin;
pub mod tool_invocation;

// Enterprise-only modules (gated behind feature flag)
// Enterprise modules removed for OSS build
// #[cfg(feature = "enterprise")]
// pub mod audit_trail;

// #[cfg(feature = "enterprise")]
// pub mod tool_review;

// Re-export specific types to avoid naming conflicts
pub use policy_engine::{
    AccessContext, AccessDecision, AccessType, AllocationDecision, AllocationResult,
    DefaultPolicyEnforcementPoint, EnforcementStatistics, MockPolicyEnforcementPoint,
    MockPolicyEngine, PolicyEnforcementFactory, PolicyEnforcementPoint, PolicyEngine,
    ResourceAccessConfig, ResourceAccessRequest, ResourceAllocationRequest, ResourceType,
    SourceInfo,
};
pub use sandbox_orchestrator::{
    CommandResult, MockSandboxOrchestrator, SandboxCommand, SandboxConfig, SandboxInfo,
    SandboxOrchestrator, SandboxRequest, SandboxStatus, SandboxType,
    SnapshotId as SandboxSnapshotId,
};
// #[cfg(feature = "enterprise")]
// pub use audit_trail::{
//     AuditTrail, MockAuditTrail, AuditEvent, AuditRecord, AuditQuery,
//     AuditEventType, AuditSeverity, AuditCategory, AuditOutcome,
//     SnapshotId as AuditSnapshotId
// };
pub use mcp::{
    McpClient, McpClientConfig, McpClientError, McpTool, MockMcpClient, SecureMcpClient,
    ToolDiscoveryEvent, ToolProvider, ToolVerificationRequest, ToolVerificationResponse,
    VerificationStatus,
};
pub use schemapin::{
    DefaultSchemaPinClient, KeyStoreConfig, LocalKeyStore, MockNativeSchemaPinClient,
    MockSchemaPinCli, NativeSchemaPinClient, PinnedKey, SchemaPinCli, SchemaPinCliWrapper,
    SchemaPinClient, SchemaPinConfig, SchemaPinError, SignArgs, SignatureInfo, SigningResult,
    VerificationResult, VerifyArgs,
};
// #[cfg(feature = "enterprise")]
// pub use tool_review::{
//     ToolReviewOrchestrator, WorkflowEvent, WorkflowEventHandler, WorkflowStats,
//     SecurityAnalyzer, AISecurityAnalyzer, SecurityAnalyzerConfig,
//     HumanReviewInterface, StandardReviewInterface, ReviewPresentation,
//     ReviewInterfaceConfig, RiskLevel, CriticalFinding, ReviewSummary,
//     ToolReviewState, ToolReviewSession, SecurityAnalysis, SecurityFinding,
//     SecuritySeverity, SecurityCategory, ReviewRecommendation, ToolReviewConfig,
//     ToolReviewError, ToolReviewResult, ReviewId, AnalysisId
// };
pub use tool_invocation::{
    DefaultToolInvocationEnforcer, EnforcementDecision, EnforcementPolicy, InvocationContext,
    InvocationEnforcementConfig, InvocationResult, ToolInvocationEnforcer, ToolInvocationError,
};

// Re-export error types from the types module
pub use crate::types::{AuditError, PolicyError, SandboxError};
