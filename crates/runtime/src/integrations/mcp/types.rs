//! MCP Client Types
//!
//! Defines types for the Model Context Protocol client with schema verification

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

use crate::integrations::schemapin::types::KeyStoreError;
use crate::integrations::schemapin::{SchemaPinError, VerificationResult};

/// MCP tool definition with verification status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    /// Tool name
    pub name: String,
    /// Tool description
    pub description: String,
    /// Tool schema (JSON Schema)
    pub schema: serde_json::Value,
    /// Provider information
    pub provider: ToolProvider,
    /// Verification status
    pub verification_status: VerificationStatus,
    /// Additional metadata
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    /// Parameter names that contain sensitive data and should be redacted in logs
    #[serde(default)]
    pub sensitive_params: Vec<String>,
}

/// Tool provider information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolProvider {
    /// Provider identifier (e.g., domain name)
    pub identifier: String,
    /// Provider name
    pub name: String,
    /// Public key URL for verification
    pub public_key_url: String,
    /// Provider version
    pub version: Option<String>,
}

/// Verification status of a tool
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum VerificationStatus {
    /// Tool has been verified successfully
    Verified {
        /// Verification result details
        result: Box<VerificationResult>,
        /// Timestamp of verification
        verified_at: String,
    },
    /// Tool verification failed
    Failed {
        /// Reason for failure
        reason: String,
        /// Timestamp of failed verification
        failed_at: String,
    },
    /// Tool has not been verified yet
    Pending,
    /// Tool verification was skipped
    Skipped {
        /// Reason for skipping
        reason: String,
    },
}

impl VerificationStatus {
    /// Check if the tool is verified
    pub fn is_verified(&self) -> bool {
        matches!(self, VerificationStatus::Verified { .. })
    }

    /// Check if verification failed
    pub fn is_failed(&self) -> bool {
        matches!(self, VerificationStatus::Failed { .. })
    }

    /// Check if verification is pending
    pub fn is_pending(&self) -> bool {
        matches!(self, VerificationStatus::Pending)
    }

    /// Get the verification result if available
    pub fn verification_result(&self) -> Option<&VerificationResult> {
        match self {
            VerificationStatus::Verified { result, .. } => Some(result),
            _ => None,
        }
    }
}

/// MCP client configuration
#[derive(Debug, Clone)]
pub struct McpClientConfig {
    /// Whether to enforce schema verification
    pub enforce_verification: bool,
    /// Whether to allow unverified tools in development mode
    pub allow_unverified_in_dev: bool,
    /// Timeout for verification operations in seconds
    pub verification_timeout_seconds: u64,
    /// Maximum number of concurrent verifications
    pub max_concurrent_verifications: usize,
}

impl Default for McpClientConfig {
    fn default() -> Self {
        Self {
            enforce_verification: true,
            allow_unverified_in_dev: false,
            verification_timeout_seconds: 30,
            max_concurrent_verifications: 5,
        }
    }
}

/// MCP client errors
#[derive(Error, Debug)]
pub enum McpClientError {
    #[error("Schema verification failed: {reason}")]
    VerificationFailed { reason: String },

    #[error("Tool not found: {name}")]
    ToolNotFound { name: String },

    #[error("Tool verification required but not verified: {name}")]
    ToolNotVerified { name: String },

    #[error("Invalid tool schema: {reason}")]
    InvalidSchema { reason: String },

    #[error("Provider key retrieval failed: {reason}")]
    KeyRetrievalFailed { reason: String },

    #[error("Communication error: {reason}")]
    CommunicationError { reason: String },

    #[error("Configuration error: {reason}")]
    ConfigurationError { reason: String },

    #[error("SchemaPin error: {source}")]
    SchemaPinError {
        #[from]
        source: SchemaPinError,
    },

    #[error("Key store error: {source}")]
    KeyStoreError {
        #[from]
        source: KeyStoreError,
    },

    #[error("Serialization error: {reason}")]
    SerializationError { reason: String },

    #[error("Timeout occurred during operation")]
    Timeout,
}

/// Tool discovery event
#[derive(Debug, Clone)]
pub struct ToolDiscoveryEvent {
    /// The discovered tool
    pub tool: McpTool,
    /// Source of the discovery
    pub source: String,
    /// Timestamp of discovery
    pub discovered_at: String,
}

/// Tool verification request
#[derive(Debug, Clone)]
pub struct ToolVerificationRequest {
    /// Tool to verify
    pub tool: McpTool,
    /// Whether to force re-verification
    pub force_reverify: bool,
}

/// Tool verification response
#[derive(Debug, Clone)]
pub struct ToolVerificationResponse {
    /// Tool name
    pub tool_name: String,
    /// Verification status
    pub status: VerificationStatus,
    /// Any warnings during verification
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::schemapin::VerificationResult;

    #[test]
    fn test_verification_status_is_verified() {
        let verified_status = VerificationStatus::Verified {
            result: Box::new(VerificationResult {
                success: true,
                message: "Test verification".to_string(),
                schema_hash: Some("hash123".to_string()),
                public_key_url: Some("https://example.com/key".to_string()),
                signature: None,
                metadata: None,
                timestamp: Some("2024-01-01T00:00:00Z".to_string()),
            }),
            verified_at: "2024-01-01T00:00:00Z".to_string(),
        };

        assert!(verified_status.is_verified());
        assert!(!verified_status.is_failed());
        assert!(!verified_status.is_pending());
    }

    #[test]
    fn test_verification_status_is_failed() {
        let failed_status = VerificationStatus::Failed {
            reason: "Invalid signature".to_string(),
            failed_at: "2024-01-01T00:00:00Z".to_string(),
        };

        assert!(!failed_status.is_verified());
        assert!(failed_status.is_failed());
        assert!(!failed_status.is_pending());
    }

    #[test]
    fn test_verification_status_is_pending() {
        let pending_status = VerificationStatus::Pending;

        assert!(!pending_status.is_verified());
        assert!(!pending_status.is_failed());
        assert!(pending_status.is_pending());
    }

    #[test]
    fn test_verification_result_extraction() {
        let result = VerificationResult {
            success: true,
            message: "Test verification".to_string(),
            schema_hash: Some("hash123".to_string()),
            public_key_url: Some("https://example.com/key".to_string()),
            signature: None,
            metadata: None,
            timestamp: Some("2024-01-01T00:00:00Z".to_string()),
        };

        let verified_status = VerificationStatus::Verified {
            result: Box::new(result.clone()),
            verified_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let extracted_result = verified_status.verification_result().unwrap();
        assert_eq!(extracted_result.success, result.success);
        assert_eq!(extracted_result.message, result.message);
    }

    #[test]
    fn test_default_config() {
        let config = McpClientConfig::default();
        assert!(config.enforce_verification);
        assert!(!config.allow_unverified_in_dev);
        assert_eq!(config.verification_timeout_seconds, 30);
        assert_eq!(config.max_concurrent_verifications, 5);
    }
}
