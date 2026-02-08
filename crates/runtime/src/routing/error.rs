//! Error types for the routing module

use crate::config::ModelCapability;
use thiserror::Error;

/// Errors that can occur during routing operations
#[derive(Debug, Error)]
pub enum RoutingError {
    #[error("Policy evaluation failed: {reason}")]
    PolicyEvaluationFailed { reason: String },

    #[error("No suitable model found for task: {task_type:?}")]
    NoSuitableModel { task_type: TaskType },

    #[error("Model execution failed: {model_id} - {reason}")]
    ModelExecutionFailed { model_id: String, reason: String },

    #[error("LLM fallback failed: {provider} - {reason}")]
    LLMFallbackFailed { provider: String, reason: String },

    #[error("Routing denied by policy: {policy} - {reason}")]
    RoutingDenied { policy: String, reason: String },

    #[error("Task classification failed: {reason}")]
    ClassificationFailed { reason: String },

    #[error("Configuration error: {key} - {reason}")]
    ConfigurationError { key: String, reason: String },

    #[error("Resource constraint violation: {constraint}")]
    ResourceConstraintViolation { constraint: String },

    #[error("Confidence evaluation failed: {reason}")]
    ConfidenceEvaluationFailed { reason: String },

    #[error("Invalid routing context: {reason}")]
    InvalidContext { reason: String },

    #[error("Model catalog error: {reason}")]
    ModelCatalogError { reason: String },

    #[error("Timeout during routing: {operation}")]
    Timeout { operation: String },

    #[error("Invalid model selection: {model_id} - {reason}")]
    InvalidModelSelection { model_id: String, reason: String },

    #[error("Policy rule parsing failed: {rule} - {reason}")]
    PolicyRuleParsingFailed { rule: String, reason: String },

    #[error("Confidence threshold not met: current={current}, required={required}")]
    ConfidenceThresholdNotMet { current: f64, required: f64 },

    #[error("Routing context validation failed: {field} - {reason}")]
    ContextValidationFailed { field: String, reason: String },

    #[error("Model capability mismatch: required={required:?}, available={available:?}")]
    CapabilityMismatch {
        required: Vec<crate::config::ModelCapability>,
        available: Vec<crate::config::ModelCapability>,
    },

    #[error("External provider error: {provider} - {error}")]
    ExternalProviderError { provider: String, error: String },

    #[error("Concurrent access error: {resource}")]
    ConcurrentAccessError { resource: String },

    #[error("Resource exhaustion: {resource} - {details}")]
    ResourceExhaustion { resource: String, details: String },

    #[error("Serialization error: {context} - {source}")]
    SerializationError {
        context: String,
        #[source]
        source: serde_json::Error,
    },
}

/// Result type for routing operations
pub type RoutingResult<T> = Result<T, RoutingError>;

impl RoutingError {
    /// Check if the error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            RoutingError::Timeout { .. }
                | RoutingError::ExternalProviderError { .. }
                | RoutingError::ConcurrentAccessError { .. }
                | RoutingError::ResourceExhaustion { .. }
                | RoutingError::ModelExecutionFailed { .. }
        )
    }

    /// Get error severity level
    pub fn severity(&self) -> ErrorSeverity {
        match self {
            RoutingError::PolicyEvaluationFailed { .. } => ErrorSeverity::High,
            RoutingError::NoSuitableModel { .. } => ErrorSeverity::High,
            RoutingError::LLMFallbackFailed { .. } => ErrorSeverity::Critical,
            RoutingError::RoutingDenied { .. } => ErrorSeverity::Medium,
            RoutingError::ConfigurationError { .. } => ErrorSeverity::High,
            RoutingError::InvalidContext { .. } => ErrorSeverity::Medium,
            RoutingError::CapabilityMismatch { .. } => ErrorSeverity::Medium,
            RoutingError::ExternalProviderError { .. } => ErrorSeverity::Medium,
            RoutingError::ResourceExhaustion { .. } => ErrorSeverity::High,
            _ => ErrorSeverity::Low,
        }
    }
}

/// Error severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorSeverity {
    Low,
    Medium,
    High,
    Critical,
}

/// Task type enumeration for routing decisions
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum TaskType {
    Intent,
    Extract,
    Template,
    BoilerplateCode,
    CodeGeneration,
    Reasoning,
    Analysis,
    Summarization,
    Translation,
    QA,
    Custom(String),
}

impl TaskType {
    /// Convert task type to required model capabilities
    pub fn to_capabilities(&self) -> Vec<ModelCapability> {
        match self {
            TaskType::CodeGeneration | TaskType::BoilerplateCode => {
                vec![
                    ModelCapability::CodeGeneration,
                    ModelCapability::TextGeneration,
                ]
            }
            TaskType::Reasoning | TaskType::Analysis => {
                vec![ModelCapability::Reasoning, ModelCapability::TextGeneration]
            }
            TaskType::Intent | TaskType::Extract | TaskType::Template => {
                vec![ModelCapability::TextGeneration]
            }
            TaskType::Summarization | TaskType::Translation | TaskType::QA => {
                vec![ModelCapability::TextGeneration]
            }
            TaskType::Custom(_) => {
                vec![ModelCapability::TextGeneration]
            }
        }
    }

    /// Get the complexity level of the task type
    pub fn complexity_level(&self) -> u8 {
        match self {
            TaskType::Intent | TaskType::Extract | TaskType::Template => 1,
            TaskType::BoilerplateCode | TaskType::Summarization | TaskType::Translation => 2,
            TaskType::QA | TaskType::CodeGeneration => 3,
            TaskType::Reasoning | TaskType::Analysis => 4,
            TaskType::Custom(_) => 2, // Default to medium complexity
        }
    }
}

impl std::fmt::Display for TaskType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskType::Intent => write!(f, "Intent"),
            TaskType::Extract => write!(f, "Extract"),
            TaskType::Template => write!(f, "Template"),
            TaskType::BoilerplateCode => write!(f, "BoilerplateCode"),
            TaskType::CodeGeneration => write!(f, "CodeGeneration"),
            TaskType::Reasoning => write!(f, "Reasoning"),
            TaskType::Analysis => write!(f, "Analysis"),
            TaskType::Summarization => write!(f, "Summarization"),
            TaskType::Translation => write!(f, "Translation"),
            TaskType::QA => write!(f, "QA"),
            TaskType::Custom(name) => write!(f, "Custom({})", name),
        }
    }
}

impl From<crate::models::ModelCatalogError> for RoutingError {
    fn from(err: crate::models::ModelCatalogError) -> Self {
        RoutingError::ModelCatalogError {
            reason: err.to_string(),
        }
    }
}

impl From<super::confidence::ConfidenceError> for RoutingError {
    fn from(err: super::confidence::ConfidenceError) -> Self {
        RoutingError::ConfidenceEvaluationFailed {
            reason: err.to_string(),
        }
    }
}
