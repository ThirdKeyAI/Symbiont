//! Secrets auditing infrastructure
//!
//! This module provides structured auditing for all secret operations,
//! allowing tracking of who accessed what secrets and when.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

/// Errors that can occur during audit operations
#[derive(Debug, Error, Clone, Serialize, Deserialize)]
pub enum AuditError {
    /// IO error during audit logging
    #[error("Audit IO error: {message}")]
    IoError { message: String },

    /// Serialization error when converting audit events to JSON
    #[error("Audit serialization error: {message}")]
    SerializationError { message: String },

    /// Configuration error for audit sink
    #[error("Audit configuration error: {message}")]
    ConfigurationError { message: String },

    /// Permission error when writing audit logs
    #[error("Audit permission error: {message}")]
    PermissionError { message: String },
}

/// A structured audit event for secret operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretAuditEvent {
    /// Timestamp when the event occurred
    pub timestamp: DateTime<Utc>,
    /// ID of the agent performing the action
    pub agent_id: String,
    /// The type of operation performed
    pub operation: String,
    /// The key of the secret being accessed (if applicable)
    pub secret_key: Option<String>,
    /// The result of the operation
    pub outcome: AuditOutcome,
    /// Error details if the operation failed
    pub error_message: Option<String>,
    /// Additional context or metadata
    pub metadata: Option<serde_json::Value>,
}

/// Outcome of a secret operation for auditing
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuditOutcome {
    /// Operation completed successfully
    Success,
    /// Operation failed
    Failure,
}

impl SecretAuditEvent {
    /// Create a new audit event for a successful operation
    pub fn success(agent_id: String, operation: String, secret_key: Option<String>) -> Self {
        Self {
            timestamp: Utc::now(),
            agent_id,
            operation,
            secret_key,
            outcome: AuditOutcome::Success,
            error_message: None,
            metadata: None,
        }
    }

    /// Create a new audit event for a failed operation
    pub fn failure(
        agent_id: String,
        operation: String,
        secret_key: Option<String>,
        error_message: String,
    ) -> Self {
        Self {
            timestamp: Utc::now(),
            agent_id,
            operation,
            secret_key,
            outcome: AuditOutcome::Failure,
            error_message: Some(error_message),
            metadata: None,
        }
    }

    /// Add metadata to the audit event
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

/// Trait for audit sink implementations that can log secret operations
#[async_trait]
pub trait SecretAuditSink: Send + Sync {
    /// Log an audit event
    ///
    /// # Arguments
    /// * `event` - The audit event to log
    ///
    /// # Returns
    /// * `Ok(())` - If the event was successfully logged
    /// * `Err(AuditError)` - If there was an error logging the event
    async fn log_event(&self, event: SecretAuditEvent) -> Result<(), AuditError>;
}

/// JSON file-based audit sink that appends audit events as JSON lines
pub struct JsonFileAuditSink {
    /// Path to the audit log file
    file_path: PathBuf,
}

impl JsonFileAuditSink {
    /// Create a new JSON file audit sink
    ///
    /// # Arguments
    /// * `file_path` - Path to the audit log file
    ///
    /// # Returns
    /// * New JsonFileAuditSink instance
    pub fn new(file_path: PathBuf) -> Self {
        Self { file_path }
    }

    /// Ensure the audit log directory exists
    async fn ensure_directory_exists(&self) -> Result<(), AuditError> {
        if let Some(parent) = self.file_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                AuditError::IoError {
                    message: format!("Failed to create audit log directory: {}", e),
                }
            })?;
        }
        Ok(())
    }
}

#[async_trait]
impl SecretAuditSink for JsonFileAuditSink {
    async fn log_event(&self, event: SecretAuditEvent) -> Result<(), AuditError> {
        // Ensure the directory exists
        self.ensure_directory_exists().await?;

        // Serialize the event to JSON
        let json_line = serde_json::to_string(&event).map_err(|e| {
            AuditError::SerializationError {
                message: format!("Failed to serialize audit event: {}", e),
            }
        })?;

        // Open the file in append mode (create if it doesn't exist)
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file_path)
            .await
            .map_err(|e| AuditError::IoError {
                message: format!("Failed to open audit log file: {}", e),
            })?;

        // Write the JSON line followed by a newline
        file.write_all(json_line.as_bytes()).await.map_err(|e| {
            AuditError::IoError {
                message: format!("Failed to write to audit log: {}", e),
            }
        })?;

        file.write_all(b"\n").await.map_err(|e| AuditError::IoError {
            message: format!("Failed to write newline to audit log: {}", e),
        })?;

        // Ensure data is written to disk
        file.flush().await.map_err(|e| AuditError::IoError {
            message: format!("Failed to flush audit log: {}", e),
        })?;

        Ok(())
    }
}

/// Convenience type for boxed audit sink
pub type BoxedAuditSink = Arc<dyn SecretAuditSink + Send + Sync>;

/// Helper function to create an optional audit sink from configuration
pub fn create_audit_sink(audit_config: &Option<AuditConfig>) -> Option<BoxedAuditSink> {
    audit_config.as_ref().map(|config| match config {
        AuditConfig::JsonFile { file_path } => {
            Arc::new(JsonFileAuditSink::new(file_path.clone())) as BoxedAuditSink
        }
    })
}

/// Configuration for audit logging
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum AuditConfig {
    /// JSON file-based audit logging
    JsonFile {
        /// Path to the audit log file
        file_path: PathBuf,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use tokio::fs;

    #[tokio::test]
    async fn test_secret_audit_event_creation() {
        let event = SecretAuditEvent::success(
            "agent-123".to_string(),
            "get_secret".to_string(),
            Some("api-key".to_string()),
        );

        assert_eq!(event.agent_id, "agent-123");
        assert_eq!(event.operation, "get_secret");
        assert_eq!(event.secret_key, Some("api-key".to_string()));
        assert!(matches!(event.outcome, AuditOutcome::Success));
        assert!(event.error_message.is_none());
    }

    #[tokio::test]
    async fn test_failure_audit_event() {
        let event = SecretAuditEvent::failure(
            "agent-456".to_string(),
            "get_secret".to_string(),
            Some("missing-key".to_string()),
            "Secret not found".to_string(),
        );

        assert_eq!(event.agent_id, "agent-456");
        assert!(matches!(event.outcome, AuditOutcome::Failure));
        assert_eq!(event.error_message, Some("Secret not found".to_string()));
    }

    #[tokio::test]
    async fn test_json_file_audit_sink() {
        let temp_file = NamedTempFile::new().unwrap();
        let sink = JsonFileAuditSink::new(temp_file.path().to_path_buf());

        let event = SecretAuditEvent::success(
            "test-agent".to_string(),
            "get_secret".to_string(),
            Some("test-key".to_string()),
        );

        let result = sink.log_event(event.clone()).await;
        assert!(result.is_ok());

        // Verify the file was written correctly
        let content = fs::read_to_string(temp_file.path()).await.unwrap();
        let lines: Vec<&str> = content.trim().split('\n').collect();
        assert_eq!(lines.len(), 1);

        // Parse and verify the JSON
        let parsed_event: SecretAuditEvent = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(parsed_event.agent_id, "test-agent");
        assert_eq!(parsed_event.operation, "get_secret");
    }

    #[tokio::test]
    async fn test_multiple_audit_events() {
        let temp_file = NamedTempFile::new().unwrap();
        let sink = JsonFileAuditSink::new(temp_file.path().to_path_buf());

        // Log multiple events
        for i in 0..3 {
            let event = SecretAuditEvent::success(
                format!("agent-{}", i),
                "list_secrets".to_string(),
                None,
            );
            sink.log_event(event).await.unwrap();
        }

        // Verify all events were written
        let content = fs::read_to_string(temp_file.path()).await.unwrap();
        let lines: Vec<&str> = content.trim().split('\n').collect();
        assert_eq!(lines.len(), 3);

        // Verify each line is valid JSON
        for (i, line) in lines.iter().enumerate() {
            let parsed_event: SecretAuditEvent = serde_json::from_str(line).unwrap();
            assert_eq!(parsed_event.agent_id, format!("agent-{}", i));
            assert_eq!(parsed_event.operation, "list_secrets");
        }
    }
}