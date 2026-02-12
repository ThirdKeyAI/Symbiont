//! Symbiont Secure Secrets Integration
//!
//! This module provides secure secrets management functionality for the Symbiont runtime,
//! supporting multiple backend types including HashiCorp Vault and file-based storage.

pub mod auditing;
pub mod config;
pub mod file_backend;
pub mod vault_backend;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

/// Errors that can occur during secrets operations
#[derive(Debug, Error, Clone, Serialize, Deserialize)]
pub enum SecretError {
    /// Secret not found
    #[error("Secret not found: {key}")]
    NotFound { key: String },

    /// Authentication failed with the secrets backend
    #[error("Authentication failed: {message}")]
    AuthenticationFailed { message: String },

    /// Network or connection error
    #[error("Connection error: {message}")]
    ConnectionError { message: String },

    /// Permission denied accessing secret
    #[error("Permission denied accessing secret: {key}")]
    PermissionDenied { key: String },

    /// Backend-specific error
    #[error("Backend error: {message}")]
    BackendError { message: String },

    /// Configuration error
    #[error("Configuration error: {message}")]
    ConfigurationError { message: String },

    /// Parsing or deserialization error
    #[error("Parse error: {message}")]
    ParseError { message: String },

    /// Timeout error
    #[error("Operation timed out: {message}")]
    Timeout { message: String },

    /// Invalid secret key format
    #[error("Invalid secret key format: {key}")]
    InvalidKeyFormat { key: String },

    /// Backend is unavailable
    #[error("Backend unavailable: {backend}")]
    BackendUnavailable { backend: String },

    /// Rate limit exceeded
    #[error("Rate limit exceeded: {message}")]
    RateLimitExceeded { message: String },

    /// Secret value is invalid or corrupted
    #[error("Invalid secret value: {reason}")]
    InvalidSecretValue { reason: String },

    /// Encryption/decryption error
    #[error("Crypto error: {message}")]
    CryptoError { message: String },

    /// IO error during file operations
    #[error("IO error: {message}")]
    IoError { message: String },

    /// Operation not supported by this backend
    #[error("Operation not supported by backend: {operation}")]
    UnsupportedOperation { operation: String },

    /// Audit logging failed in strict mode — operation blocked
    #[error("Audit logging failed (strict mode): {message}")]
    AuditFailed { message: String },
}

/// A secret value retrieved from a secrets backend
#[derive(Clone, Serialize, Deserialize)]
pub struct Secret {
    /// The secret key/name
    pub key: String,
    /// The secret value (sensitive data)
    pub value: String,
    /// Optional metadata about the secret
    pub metadata: Option<HashMap<String, String>>,
    /// Timestamp when the secret was created/last modified
    pub created_at: Option<String>,
    /// Version of the secret (for versioned backends)
    pub version: Option<String>,
}

impl Secret {
    /// Create a new secret
    pub fn new(key: String, value: String) -> Self {
        Self {
            key,
            value,
            metadata: None,
            created_at: None,
            version: None,
        }
    }

    /// Create a new secret with metadata
    pub fn with_metadata(key: String, value: String, metadata: HashMap<String, String>) -> Self {
        Self {
            key,
            value,
            metadata: Some(metadata),
            created_at: None,
            version: None,
        }
    }

    /// Get the secret value
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Get metadata for a specific key
    pub fn get_metadata(&self, key: &str) -> Option<&String> {
        self.metadata.as_ref()?.get(key)
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Secret")
            .field("key", &self.key)
            .field("value", &"[REDACTED]")
            .field("metadata", &self.metadata)
            .field("created_at", &self.created_at)
            .field("version", &self.version)
            .finish()
    }
}

/// Trait for secrets backend implementations
#[async_trait]
pub trait SecretStore: Send + Sync {
    /// Retrieve a secret by key
    ///
    /// # Arguments
    /// * `key` - The secret key to retrieve
    ///
    /// # Returns
    /// * `Ok(Secret)` - The secret if found
    /// * `Err(SecretError)` - Error if secret not found or other failure
    async fn get_secret(&self, key: &str) -> Result<Secret, SecretError>;

    /// List all available secret keys
    ///
    /// # Returns
    /// * `Ok(Vec<String>)` - List of secret keys
    /// * `Err(SecretError)` - Error if operation fails
    async fn list_secrets(&self) -> Result<Vec<String>, SecretError>;
}

/// Result type for secrets operations
pub type SecretResult<T> = Result<T, SecretError>;

// Re-export config types, backends, and auditing
pub use auditing::*;
pub use config::*;
pub use file_backend::FileSecretStore;
pub use vault_backend::VaultSecretStore;

/// Create a new SecretStore instance based on configuration
///
/// # Arguments
/// * `config` - The secrets configuration specifying the backend type and settings
/// * `agent_id` - The agent ID for agent-specific secret namespacing (used by Vault backend)
///
/// # Returns
/// * `Ok(Box<dyn SecretStore + Send + Sync>)` - The configured secret store
/// * `Err(SecretError)` - Error if backend initialization fails
pub async fn new_secret_store(
    config: &SecretsConfig,
    agent_id: &str,
) -> Result<Box<dyn SecretStore + Send + Sync>, SecretError> {
    // Create audit sink from configuration
    let audit_sink = auditing::create_audit_sink(&config.common.audit);

    match &config.backend {
        SecretsBackend::File(file_config) => {
            let store = FileSecretStore::new(file_config.clone(), audit_sink, agent_id.to_string())
                .await
                .map_err(|e| SecretError::ConfigurationError {
                    message: format!("Failed to initialize file backend: {}", e),
                })?;
            Ok(Box::new(store))
        }
        SecretsBackend::Vault(vault_config) => {
            let store =
                VaultSecretStore::new(vault_config.clone(), agent_id.to_string(), audit_sink)
                    .await
                    .map_err(|e| SecretError::ConfigurationError {
                        message: format!("Failed to initialize vault backend: {}", e),
                    })?;
            Ok(Box::new(store))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secret_creation() {
        let secret = Secret::new("test_key".to_string(), "test_value".to_string());
        assert_eq!(secret.key, "test_key");
        assert_eq!(secret.value(), "test_value");
        assert!(secret.metadata.is_none());
    }

    #[test]
    fn test_secret_with_metadata() {
        let mut metadata = HashMap::new();
        metadata.insert("description".to_string(), "Test secret".to_string());

        let secret =
            Secret::with_metadata("test_key".to_string(), "test_value".to_string(), metadata);

        assert_eq!(secret.key, "test_key");
        assert_eq!(secret.value(), "test_value");
        assert_eq!(
            secret.get_metadata("description"),
            Some(&"Test secret".to_string())
        );
    }

    #[test]
    fn test_secret_error_display() {
        let error = SecretError::NotFound {
            key: "missing_key".to_string(),
        };
        assert!(error.to_string().contains("Secret not found: missing_key"));
    }
}
