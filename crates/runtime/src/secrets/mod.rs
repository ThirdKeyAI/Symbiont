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

/// A secret value retrieved from a secrets backend.
///
/// The `value` field holds sensitive plaintext. A `Drop` impl zeroises the
/// buffer when the `Secret` goes out of scope so the plaintext does not
/// linger in the heap past its useful life (protects against core dumps,
/// swap, and post-free memory scraping).
///
/// `Serialize` is implemented by hand and redacts `value` exactly the same
/// way the custom `Debug` impl does — so a structured logger calling
/// `serde_json::to_string(&secret)` cannot leak plaintext. `Deserialize`
/// is still derived because secrets must be read back from backends like
/// Vault and the file backend.
#[derive(Clone, Deserialize)]
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

impl Serialize for Secret {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("Secret", 5)?;
        state.serialize_field("key", &self.key)?;
        // SECURITY: never serialize the plaintext value. The Debug impl
        // redacts the same field; serde must match so structured loggers
        // (tracing JSON appenders, log forwarders) cannot leak the secret.
        state.serialize_field("value", "[REDACTED]")?;
        state.serialize_field("metadata", &self.metadata)?;
        state.serialize_field("created_at", &self.created_at)?;
        state.serialize_field("version", &self.version)?;
        state.end()
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.value.zeroize();
    }
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

/// Metadata for a stored secret, including TTL for rotation.
#[derive(Debug, Clone)]
pub struct SecretMetadata {
    /// When the secret was created.
    pub created_at: std::time::SystemTime,
    /// When the secret expires (None = never).
    pub expires_at: Option<std::time::SystemTime>,
    /// How long before expiry to start warning about rotation.
    pub rotation_hint: Option<std::time::Duration>,
}

impl SecretMetadata {
    /// Check if this secret has expired.
    pub fn is_expired(&self) -> bool {
        if let Some(expires) = self.expires_at {
            std::time::SystemTime::now() > expires
        } else {
            false
        }
    }

    /// Check if this secret should be rotated (within rotation hint window).
    pub fn needs_rotation(&self) -> bool {
        if let (Some(expires), Some(hint)) = (self.expires_at, self.rotation_hint) {
            if let Ok(remaining) = expires.duration_since(std::time::SystemTime::now()) {
                return remaining < hint;
            }
        }
        false
    }
}

/// Retrieve a secret with expiry checking.
/// Returns an error if the secret has expired; logs a warning if rotation is due.
pub async fn get_secret_checked(
    store: &dyn SecretStore,
    key: &str,
    metadata: Option<&SecretMetadata>,
) -> Result<Secret, SecretError> {
    if let Some(meta) = metadata {
        if meta.is_expired() {
            return Err(SecretError::BackendError {
                message: format!("Secret '{key}' has expired"),
            });
        }
        if meta.needs_rotation() {
            tracing::warn!(
                secret = key,
                "Secret is approaching expiry and should be rotated"
            );
        }
    }
    store.get_secret(key).await
}

/// Result type for secrets operations
pub type SecretResult<T> = Result<T, SecretError>;

/// Resolve a secret value, preferring a `SecretStore` over an environment
/// variable when both are available.
///
/// Resolution order:
/// 1. If `secret_key` is `Some` and `store` is `Some`, attempt to fetch the
///    secret from the store. On success, return the value.
/// 2. On store error (or if either is `None`), fall back to
///    `std::env::var(env_var)`.
/// 3. Return `None` if neither source produces a value.
///
/// Store failures are logged at WARN level. Successful store reads are logged
/// at DEBUG. The secret value itself is never logged.
///
/// This is intended for plumbing third-party API keys (LLM providers,
/// webhooks, etc.) through the existing secrets infrastructure while keeping
/// env-var configuration as a graceful fallback for development and CI.
pub async fn resolve_secret_or_env(
    env_var: &str,
    secret_key: Option<&str>,
    store: Option<&(dyn SecretStore + Send + Sync)>,
) -> Option<String> {
    if let (Some(store), Some(key)) = (store, secret_key) {
        match store.get_secret(key).await {
            Ok(secret) => {
                tracing::debug!(secret_key = %key, "Resolved secret from secret store");
                return Some(secret.value().to_string());
            }
            Err(e) => {
                tracing::warn!(
                    secret_key = %key,
                    env_var = %env_var,
                    error = %e,
                    "Failed to fetch secret from store; falling back to env var"
                );
            }
        }
    }
    std::env::var(env_var).ok()
}

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
    fn test_secret_serialize_redacts_value() {
        // M7: Serialize must NOT emit the plaintext `value`. Even if a
        // structured logger calls `serde_json::to_string(&secret)`, the
        // output must contain "[REDACTED]" and never the real secret.
        let secret = Secret::new(
            "api_key".to_string(),
            "super-secret-plaintext-value".to_string(),
        );
        let json = serde_json::to_string(&secret).expect("serialize Secret");
        assert!(
            json.contains("[REDACTED]"),
            "Serialize output must contain [REDACTED]; got: {}",
            json
        );
        assert!(
            !json.contains("super-secret-plaintext-value"),
            "Serialize output must NOT contain the plaintext value; got: {}",
            json
        );
        // Sanity-check that other fields still serialize.
        assert!(json.contains("api_key"));
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

    #[test]
    fn test_secret_metadata_not_expired() {
        let meta = SecretMetadata {
            created_at: std::time::SystemTime::now(),
            expires_at: Some(std::time::SystemTime::now() + std::time::Duration::from_secs(3600)),
            rotation_hint: None,
        };
        assert!(!meta.is_expired());
    }

    #[test]
    fn test_secret_metadata_expired() {
        let meta = SecretMetadata {
            created_at: std::time::SystemTime::now() - std::time::Duration::from_secs(7200),
            expires_at: Some(std::time::SystemTime::now() - std::time::Duration::from_secs(1)),
            rotation_hint: None,
        };
        assert!(meta.is_expired());
    }

    #[test]
    fn test_secret_metadata_no_expiry() {
        let meta = SecretMetadata {
            created_at: std::time::SystemTime::now(),
            expires_at: None,
            rotation_hint: None,
        };
        assert!(!meta.is_expired());
        assert!(!meta.needs_rotation());
    }

    #[test]
    fn test_secret_metadata_needs_rotation() {
        // Expires in 5 minutes, rotation hint is 10 minutes => needs rotation
        let meta = SecretMetadata {
            created_at: std::time::SystemTime::now() - std::time::Duration::from_secs(3600),
            expires_at: Some(std::time::SystemTime::now() + std::time::Duration::from_secs(300)),
            rotation_hint: Some(std::time::Duration::from_secs(600)),
        };
        assert!(!meta.is_expired());
        assert!(meta.needs_rotation());
    }

    #[test]
    fn test_secret_metadata_no_rotation_needed() {
        // Expires in 2 hours, rotation hint is 10 minutes => no rotation needed
        let meta = SecretMetadata {
            created_at: std::time::SystemTime::now(),
            expires_at: Some(std::time::SystemTime::now() + std::time::Duration::from_secs(7200)),
            rotation_hint: Some(std::time::Duration::from_secs(600)),
        };
        assert!(!meta.is_expired());
        assert!(!meta.needs_rotation());
    }

    /// Minimal in-memory `SecretStore` for tests of `resolve_secret_or_env`.
    struct MapStore {
        data: HashMap<String, String>,
        fail: bool,
    }

    #[async_trait]
    impl SecretStore for MapStore {
        async fn get_secret(&self, key: &str) -> Result<Secret, SecretError> {
            if self.fail {
                return Err(SecretError::BackendError {
                    message: "synthetic failure".to_string(),
                });
            }
            self.data
                .get(key)
                .map(|v| Secret::new(key.to_string(), v.clone()))
                .ok_or_else(|| SecretError::NotFound {
                    key: key.to_string(),
                })
        }

        async fn list_secrets(&self) -> Result<Vec<String>, SecretError> {
            Ok(self.data.keys().cloned().collect())
        }
    }

    // Each test uses a unique env-var name so concurrent runs don't race on
    // shared process state. We make sure to remove the var at the end of each
    // test. We don't restore prior values because the test names are unique
    // and chosen to be unused outside the test module.

    #[tokio::test]
    async fn resolve_returns_store_value_when_present() {
        std::env::set_var("RESOLVE_TEST_KEY_A", "env-key");
        let store = MapStore {
            data: HashMap::from([("llm/anthropic".to_string(), "vault-key".to_string())]),
            fail: false,
        };
        let got =
            resolve_secret_or_env("RESOLVE_TEST_KEY_A", Some("llm/anthropic"), Some(&store)).await;
        std::env::remove_var("RESOLVE_TEST_KEY_A");
        assert_eq!(got.as_deref(), Some("vault-key"));
    }

    #[tokio::test]
    async fn resolve_falls_back_to_env_on_store_error() {
        std::env::set_var("RESOLVE_TEST_KEY_B", "env-key");
        let store = MapStore {
            data: HashMap::new(),
            fail: true,
        };
        let got =
            resolve_secret_or_env("RESOLVE_TEST_KEY_B", Some("llm/anthropic"), Some(&store)).await;
        std::env::remove_var("RESOLVE_TEST_KEY_B");
        assert_eq!(got.as_deref(), Some("env-key"));
    }

    #[tokio::test]
    async fn resolve_uses_env_when_no_secret_key() {
        std::env::set_var("RESOLVE_TEST_KEY_C", "env-only");
        let got = resolve_secret_or_env("RESOLVE_TEST_KEY_C", None, None).await;
        std::env::remove_var("RESOLVE_TEST_KEY_C");
        assert_eq!(got.as_deref(), Some("env-only"));
    }

    #[tokio::test]
    async fn resolve_returns_none_when_nothing_set() {
        std::env::remove_var("RESOLVE_TEST_KEY_D");
        let got = resolve_secret_or_env("RESOLVE_TEST_KEY_D", None, None).await;
        assert!(got.is_none());
    }
}
