//! Error types and recovery strategies for the Agent Runtime System

use std::time::Duration;
use thiserror::Error;

use super::{AgentId, MessageId, PolicyId, RequestId};

/// Main runtime error type
#[derive(Error, Debug, Clone)]
pub enum RuntimeError {
    #[error("Configuration error: {0}")]
    Configuration(#[from] ConfigError),

    #[error("Resource error: {0}")]
    Resource(#[from] ResourceError),

    #[error("Security error: {0}")]
    Security(#[from] SecurityError),

    #[error("Communication error: {0}")]
    Communication(#[from] CommunicationError),

    #[error("Policy error: {0}")]
    Policy(#[from] PolicyError),

    #[error("Sandbox error: {0}")]
    Sandbox(#[from] SandboxError),

    #[error("Scheduler error: {0}")]
    Scheduler(#[from] SchedulerError),

    #[error("Lifecycle error: {0}")]
    Lifecycle(#[from] LifecycleError),

    #[error("Audit error: {0}")]
    Audit(#[from] AuditError),

    #[error("Error handler error: {0}")]
    ErrorHandler(#[from] ErrorHandlerError),

    #[error("Internal error: {0}")]
    Internal(String),
}

/// Configuration-related errors
#[derive(Error, Debug, Clone)]
pub enum ConfigError {
    #[error("Invalid configuration: {0}")]
    Invalid(String),

    #[error("Missing required field: {0}")]
    MissingField(String),

    #[error("Configuration file not found: {0}")]
    FileNotFound(String),

    #[error("Failed to parse configuration: {0}")]
    ParseError(String),
}

/// Resource management errors
#[derive(Error, Debug, Clone)]
pub enum ResourceError {
    #[error("Insufficient resources: {0}")]
    Insufficient(String),

    #[error("Resource allocation failed for agent {agent_id}: {reason}")]
    AllocationFailed { agent_id: AgentId, reason: String },

    #[error("Resource limit exceeded: {0}")]
    LimitExceeded(String),

    #[error("Resource not found: {0}")]
    NotFound(String),

    #[error("Resource monitoring failed: {0}")]
    MonitoringFailed(String),

    #[error("Agent not found: {agent_id}")]
    AgentNotFound { agent_id: AgentId },

    #[error("System is shutting down")]
    ShuttingDown,

    #[error("Allocation already exists for agent: {agent_id}")]
    AllocationExists { agent_id: AgentId },

    #[error("Insufficient resources for requirements: {requirements:?}")]
    InsufficientResources { requirements: String },

    #[error("Policy error: {0}")]
    PolicyError(String),

    #[error("Policy violation: {reason}")]
    PolicyViolation { reason: String },

    #[error("Resource allocation queued: {reason}")]
    AllocationQueued { reason: String },

    #[error("Escalation required: {reason}")]
    EscalationRequired { reason: String },
}

/// Security-related errors
#[derive(Error, Debug, Clone)]
pub enum SecurityError {
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    #[error("Authorization denied: {0}")]
    AuthorizationDenied(String),

    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),

    #[error("Signature verification failed: {0}")]
    SignatureVerificationFailed(String),

    #[error("Policy violation: {0}")]
    PolicyViolation(String),

    #[error("Sandbox breach detected: {0}")]
    SandboxBreach(String),

    #[error("Key management error: {0}")]
    KeyManagement(String),
}

/// Communication system errors
#[derive(Error, Debug, Clone)]
pub enum CommunicationError {
    #[error("Message delivery failed for message {message_id}: {reason}")]
    DeliveryFailed {
        message_id: MessageId,
        reason: String,
    },

    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Message timeout: {0}")]
    Timeout(String),

    #[error("Invalid message format: {0}")]
    InvalidFormat(String),

    #[error("Rate limit exceeded: {0}")]
    RateLimitExceeded(String),

    #[error("Channel not found: {0}")]
    ChannelNotFound(String),

    #[error("Message too large: {size} bytes, max allowed: {max_size} bytes")]
    MessageTooLarge { size: usize, max_size: usize },

    #[error("Communication system is shutting down")]
    ShuttingDown,

    #[error("Event processing failed: {reason}")]
    EventProcessingFailed { reason: String },

    #[error("Agent not registered: {agent_id}")]
    AgentNotRegistered { agent_id: AgentId },

    #[error("Message not found: {message_id}")]
    MessageNotFound { message_id: MessageId },

    #[error("Request timeout: request {request_id} timed out after {timeout:?}")]
    RequestTimeout { request_id: RequestId, timeout: Duration },

    #[error("Request cancelled: {request_id}")]
    RequestCancelled { request_id: RequestId },
}

/// Policy enforcement errors
#[derive(Error, Debug, Clone)]
pub enum PolicyError {
    #[error("Policy not found: {policy_id}")]
    NotFound { policy_id: PolicyId },

    #[error("Policy not found: {id}")]
    PolicyNotFound { id: PolicyId },

    #[error("Policy evaluation failed: {0}")]
    EvaluationFailed(String),

    #[error("Policy compilation failed: {0}")]
    CompilationFailed(String),

    #[error("Policy conflict detected: {0}")]
    Conflict(String),

    #[error("Policy engine unavailable: {0}")]
    EngineUnavailable(String),

    #[error("Invalid policy: {reason}")]
    InvalidPolicy { reason: String },
}

/// Sandbox orchestration errors
#[derive(Error, Debug, Clone)]
pub enum SandboxError {
    #[error("Sandbox creation failed: {0}")]
    CreationFailed(String),

    #[error("Sandbox execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Sandbox not found: {0}")]
    NotFound(String),

    #[error("Sandbox not found: {id}")]
    SandboxNotFound { id: String },

    #[error("Snapshot not found: {id}")]
    SnapshotNotFound { id: String },

    #[error("Sandbox termination failed: {0}")]
    TerminationFailed(String),

    #[error("Sandbox monitoring failed: {0}")]
    MonitoringFailed(String),

    #[error("Unsupported security tier: {0}")]
    UnsupportedTier(String),
}

/// Scheduler errors
#[derive(Error, Debug, Clone)]
pub enum SchedulerError {
    #[error("Agent scheduling failed for {agent_id}: {reason}")]
    SchedulingFailed { agent_id: AgentId, reason: String },

    #[error("Agent not found: {agent_id}")]
    AgentNotFound { agent_id: AgentId },

    #[error("Scheduler overloaded: {0}")]
    Overloaded(String),

    #[error("Invalid priority: {0}")]
    InvalidPriority(String),

    #[error("Scheduler shutdown in progress")]
    ShuttingDown,
}

/// Lifecycle management errors
#[derive(Error, Debug, Clone)]
pub enum LifecycleError {
    #[error("Agent initialization failed for {agent_id}: {reason}")]
    InitializationFailed { agent_id: AgentId, reason: String },

    #[error("Agent execution failed for {agent_id}: {reason}")]
    ExecutionFailed { agent_id: AgentId, reason: String },

    #[error("Agent termination failed for {agent_id}: {reason}")]
    TerminationFailed { agent_id: AgentId, reason: String },

    #[error("Invalid state transition from {from} to {to}")]
    InvalidStateTransition { from: String, to: String },

    #[error("DSL parsing failed: {0}")]
    DslParsingFailed(String),

    #[error("Agent not found: {agent_id}")]
    AgentNotFound { agent_id: AgentId },

    #[error("Event processing failed: {reason}")]
    EventProcessingFailed { reason: String },

    #[error("System is shutting down")]
    ShuttingDown,

    #[error("Resource exhausted: {reason}")]
    ResourceExhausted { reason: String },
}

/// Audit trail errors
#[derive(Error, Debug, Clone)]
pub enum AuditError {
    #[error("Audit logging failed: {0}")]
    LoggingFailed(String),

    #[error("Audit verification failed: {0}")]
    VerificationFailed(String),

    #[error("Audit query failed: {0}")]
    QueryFailed(String),

    #[error("Audit storage full: {0}")]
    StorageFull(String),

    #[error("Audit trail corrupted: {0}")]
    Corrupted(String),

    #[error("Record not found: {id}")]
    RecordNotFound { id: String },

    #[error("Export failed: {reason}")]
    ExportFailed { reason: String },

    #[error("Unsupported format: {format}")]
    UnsupportedFormat { format: String },
}

/// Error handler errors
#[derive(Error, Debug, Clone)]
pub enum ErrorHandlerError {
    #[error("Configuration error: {reason}")]
    ConfigurationError { reason: String },

    #[error("Event processing failed: {reason}")]
    EventProcessingFailed { reason: String },

    #[error("Error handler is shutting down")]
    ShuttingDown,
}

/// Recovery strategies for different types of errors
#[derive(Debug, Clone)]
pub enum RecoveryStrategy {
    /// Retry the operation with exponential backoff
    Retry {
        max_attempts: u32,
        backoff: Duration,
    },
    /// Restart the agent, optionally preserving state
    Restart { preserve_state: bool },
    /// Failover to a backup agent
    Failover { backup_agent: Option<AgentId> },
    /// Terminate the agent with cleanup
    Terminate { cleanup: bool },
    /// Manual intervention required
    Manual { reason: String },
    /// No recovery possible
    None,
}

impl Default for RecoveryStrategy {
    fn default() -> Self {
        RecoveryStrategy::Retry {
            max_attempts: 3,
            backoff: Duration::from_secs(1),
        }
    }
}

/// Error recovery configuration
#[derive(Debug, Clone)]
pub struct ErrorRecoveryConfig {
    pub default_strategy: RecoveryStrategy,
    pub max_recovery_attempts: u32,
    pub recovery_timeout: Duration,
    pub escalation_threshold: u32,
}

impl Default for ErrorRecoveryConfig {
    fn default() -> Self {
        Self {
            default_strategy: RecoveryStrategy::default(),
            max_recovery_attempts: 5,
            recovery_timeout: Duration::from_secs(300), // 5 minutes
            escalation_threshold: 10,
        }
    }
}

/// Error context for better debugging and recovery
#[derive(Debug, Clone)]
pub struct ErrorContext {
    pub agent_id: Option<AgentId>,
    pub operation: String,
    pub timestamp: std::time::SystemTime,
    pub recovery_attempts: u32,
    pub additional_info: std::collections::HashMap<String, String>,
}

impl ErrorContext {
    pub fn new(operation: String) -> Self {
        Self {
            agent_id: None,
            operation,
            timestamp: std::time::SystemTime::now(),
            recovery_attempts: 0,
            additional_info: std::collections::HashMap::new(),
        }
    }

    pub fn with_agent(mut self, agent_id: AgentId) -> Self {
        self.agent_id = Some(agent_id);
        self
    }

    pub fn with_info(mut self, key: String, value: String) -> Self {
        self.additional_info.insert(key, value);
        self
    }

    pub fn increment_attempts(&mut self) {
        self.recovery_attempts += 1;
    }
}

/// Result type with error context
pub type RuntimeResult<T> = Result<T, RuntimeError>;

/// Trait for error recovery
pub trait ErrorRecovery {
    fn get_recovery_strategy(&self, error: &RuntimeError) -> RecoveryStrategy;
    fn should_retry(&self, error: &RuntimeError, attempts: u32) -> bool;
    fn escalate_error(&self, error: &RuntimeError, context: &ErrorContext);
}
