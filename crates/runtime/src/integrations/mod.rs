//! External Integration Interfaces
//!
//! Provides interfaces for integrating with external security and policy components

pub mod agentpin;
pub mod mcp;
pub mod policy_engine;
pub mod sandbox_orchestrator;
pub mod schemapin;
pub mod tool_invocation;

#[cfg(feature = "composio")]
pub mod composio;

// Re-export specific types to avoid naming conflicts
pub use agentpin::{
    AgentPinConfig, AgentPinError, AgentPinKeyStore, AgentPinVerifier, AgentVerificationResult,
    CachingResolver, DefaultAgentPinVerifier, DiscoveryMode, MockAgentPinVerifier,
};
pub use mcp::{
    McpClient, McpClientConfig, McpClientError, McpTool, MockMcpClient, SecureMcpClient,
    ToolDiscoveryEvent, ToolProvider, ToolVerificationRequest, ToolVerificationResponse,
    VerificationStatus,
};
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
pub use schemapin::{
    DefaultSchemaPinClient, KeyStoreConfig, LocalKeyStore, MockNativeSchemaPinClient,
    NativeSchemaPinClient, PinnedKey, SchemaPinClient, SchemaPinError, SignArgs, SignatureInfo,
    SigningResult, VerificationResult, VerifyArgs,
};
pub use tool_invocation::{
    mask_sensitive_arguments, DefaultToolInvocationEnforcer, EnforcementDecision,
    EnforcementPolicy, InvocationContext, InvocationEnforcementConfig, InvocationResult,
    ToolInvocationEnforcer, ToolInvocationError,
};

#[cfg(feature = "composio")]
pub use composio::{
    load_mcp_config, ComposioGlobalConfig, ComposioMcpSource, McpConfigFile, McpServerEntry,
    ServerPolicy, SseTransport,
};

// Re-export error types from the types module
pub use crate::types::{AuditError, PolicyError, SandboxError};
