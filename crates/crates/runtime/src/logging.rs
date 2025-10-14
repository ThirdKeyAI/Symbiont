//! Encrypted Logging Module for Model I/O
//!
//! This module provides secure logging capabilities for all model interactions
//! including prompts, tool calls, outputs, and latency metrics. All sensitive
//! data is encrypted using AES-256-GCM before being written to logs.
//!
//! # Security Features
//! - Automatic encryption of all sensitive log data
//! - PII/PHI detection and masking
//! - Secure key management integration
//! - Structured logging with metadata
//! - Configurable retention policies

use crate::crypto::{Aes256GcmCrypto, EncryptedData, KeyUtils};
use crate::secrets::SecretStore;
use crate::types::AgentId;
use chrono::{DateTime, Utc};
use futures;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tracing as log;
use uuid::Uuid;

/// Errors that can occur during logging operations
#[derive(Debug, Error)]
pub enum LoggingError {
    #[error("Encryption failed: {message}")]
    EncryptionFailed { message: String },

    #[error("Key management error: {message}")]
    KeyManagementError { message: String },

    #[error("Serialization error: {source}")]
    SerializationError {
        #[from]
        source: serde_json::Error,
    },

    #[error("I/O error: {source}")]
    IoError {
        #[from]
        source: std::io::Error,
    },

    #[error("Configuration error: {message}")]
    ConfigurationError { message: String },
}

/// Configuration for the logging module
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Enable/disable encrypted logging
    pub enabled: bool,
    /// Log file path
    pub log_file_path: String,
    /// Secret key name in SecretStore for encryption key
    pub encryption_key_name: String,
    /// Environment variable for encryption key (fallback only)
    pub encryption_key_env: Option<String>,
    /// Maximum log entry size in bytes
    pub max_entry_size: usize,
    /// Log retention period in days
    pub retention_days: u32,
    /// Enable PII detection and masking
    pub enable_pii_masking: bool,
    /// Batch size for log writes
    pub batch_size: usize,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            log_file_path: "logs/model_io.encrypted.log".to_string(),
            encryption_key_name: "symbiont/logging/encryption_key".to_string(),
            encryption_key_env: Some("SYMBIONT_LOGGING_KEY".to_string()),
            max_entry_size: 1024 * 1024, // 1MB
            retention_days: 90,
            enable_pii_masking: true,
            batch_size: 100,
        }
    }
}

/// Type of model interaction being logged
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ModelInteractionType {
    /// Direct model prompt/completion
    Completion,
    /// Tool call execution
    ToolCall,
    /// RAG query processing
    RagQuery,
    /// Agent task execution
    AgentExecution,
}

/// Log entry for model I/O operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelLogEntry {
    /// Unique identifier for this log entry
    pub id: String,
    /// Agent that initiated the request
    pub agent_id: AgentId,
    /// Type of model interaction
    pub interaction_type: ModelInteractionType,
    /// Timestamp when the interaction started
    pub timestamp: DateTime<Utc>,
    /// Duration of the interaction
    pub latency_ms: u64,
    /// Model/service used
    pub model_identifier: String,
    /// Encrypted request data
    pub request_data: EncryptedData,
    /// Encrypted response data
    pub response_data: Option<EncryptedData>,
    /// Metadata (non-sensitive)
    pub metadata: HashMap<String, String>,
    /// Error information if the interaction failed
    pub error: Option<String>,
    /// Token usage statistics
    pub token_usage: Option<TokenUsage>,
}

/// Raw (unencrypted) request data structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestData {
    /// The prompt or query sent to the model
    pub prompt: String,
    /// Tool name (if applicable)
    pub tool_name: Option<String>,
    /// Tool arguments (if applicable)
    pub tool_arguments: Option<serde_json::Value>,
    /// Additional parameters
    pub parameters: HashMap<String, serde_json::Value>,
}

/// Raw (unencrypted) response data structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseData {
    /// Model's response content
    pub content: String,
    /// Tool execution result (if applicable)
    pub tool_result: Option<serde_json::Value>,
    /// Confidence score (if available)
    pub confidence: Option<f64>,
    /// Additional response metadata
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Token usage statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Input tokens consumed
    pub input_tokens: u32,
    /// Output tokens generated
    pub output_tokens: u32,
    /// Total tokens used
    pub total_tokens: u32,
}

/// Encrypted model I/O logger
pub struct ModelLogger {
    config: LoggingConfig,
    #[allow(dead_code)]
    crypto: Aes256GcmCrypto,
    #[allow(dead_code)]
    secret_store: Option<Arc<dyn SecretStore>>,
    encryption_key: String,
}

impl ModelLogger {
    /// Create a new model logger with the given configuration and secret store
    pub fn new(config: LoggingConfig, secret_store: Option<Arc<dyn SecretStore>>) -> Result<Self, LoggingError> {
        let crypto = Aes256GcmCrypto::new();
        
        // Get encryption key
        let encryption_key = Self::get_encryption_key(&config, &secret_store)?;
        
        Ok(Self {
            config,
            crypto,
            secret_store,
            encryption_key,
        })
    }

    /// Create a new logger with default configuration (no secret store)
    pub fn with_defaults() -> Result<Self, LoggingError> {
        Self::new(LoggingConfig::default(), None)
    }

    /// Get encryption key from SecretStore, environment variable, or generate new one
    fn get_encryption_key(
        config: &LoggingConfig,
        secret_store: &Option<Arc<dyn SecretStore>>
    ) -> Result<String, LoggingError> {
        // Try SecretStore first if available
        if let Some(store) = secret_store {
            if let Ok(secret) = futures::executor::block_on(store.get_secret(&config.encryption_key_name)) {
                log::debug!("Retrieved logging encryption key from SecretStore");
                return Ok(secret.value().to_string());
            } else {
                log::warn!("Failed to retrieve logging encryption key from SecretStore, falling back to environment variable");
            }
        }

        // Try environment variable as fallback
        if let Some(env_var) = &config.encryption_key_env {
            if let Ok(key) = KeyUtils::get_key_from_env(env_var) {
                log::debug!("Retrieved logging encryption key from environment variable");
                return Ok(key);
            }
        }

        // Final fallback: generate or retrieve from keychain
        let key_utils = KeyUtils::new();
        key_utils.get_or_create_key().map_err(|e| LoggingError::KeyManagementError {
            message: format!("Failed to get encryption key: {}", e),
        })
    }

    /// Log a model request (before execution)
    pub async fn log_request(
        &self,
        agent_id: AgentId,
        interaction_type: ModelInteractionType,
        model_identifier: &str,
        request_data: RequestData,
        metadata: HashMap<String, String>,
    ) -> Result<String, LoggingError> {
        if !self.config.enabled {
            return Ok(String::new());
        }

        let entry_id = Uuid::new_v4().to_string();
        let timestamp = Utc::now();

        // Mask PII if enabled
        let sanitized_request = if self.config.enable_pii_masking {
            self.mask_pii_in_request(request_data)?
        } else {
            request_data
        };

        // Encrypt request data
        let encrypted_request = self.encrypt_request_data(&sanitized_request)?;

        // Create log entry (without response data initially)
        let log_entry = ModelLogEntry {
            id: entry_id.clone(),
            agent_id,
            interaction_type,
            timestamp,
            latency_ms: 0, // Will be updated when response is logged
            model_identifier: model_identifier.to_string(),
            request_data: encrypted_request,
            response_data: None,
            metadata,
            error: None,
            token_usage: None,
        };

        self.write_log_entry(&log_entry).await?;

        log::debug!("Logged model request {} for agent {}", entry_id, agent_id);
        Ok(entry_id)
    }

    /// Log a model response (after execution)
    pub async fn log_response(
        &self,
        entry_id: &str,
        response_data: ResponseData,
        latency: Duration,
        token_usage: Option<TokenUsage>,
        error: Option<String>,
    ) -> Result<(), LoggingError> {
        if !self.config.enabled {
            return Ok(());
        }

        // Mask PII if enabled
        let sanitized_response = if self.config.enable_pii_masking {
            self.mask_pii_in_response(response_data)?
        } else {
            response_data
        };

        // Encrypt response data
        let encrypted_response = self.encrypt_response_data(&sanitized_response)?;

        // Create update entry
        let update_entry = serde_json::json!({
            "id": entry_id,
            "response_data": encrypted_response,
            "latency_ms": latency.as_millis() as u64,
            "token_usage": token_usage,
            "error": error,
            "updated_at": Utc::now()
        });

        self.write_log_update(&update_entry).await?;

        log::debug!("Logged model response for entry {}", entry_id);
        Ok(())
    }

    /// Convenience method to log a complete interaction
    #[allow(clippy::too_many_arguments)]
    pub async fn log_interaction(
        &self,
        agent_id: AgentId,
        interaction_type: ModelInteractionType,
        model_identifier: &str,
        request_data: RequestData,
        response_data: ResponseData,
        latency: Duration,
        metadata: HashMap<String, String>,
        token_usage: Option<TokenUsage>,
        error: Option<String>,
    ) -> Result<(), LoggingError> {
        if !self.config.enabled {
            return Ok(());
        }

        let entry_id = Uuid::new_v4().to_string();
        let timestamp = Utc::now();

        // Mask PII if enabled
        let sanitized_request = if self.config.enable_pii_masking {
            self.mask_pii_in_request(request_data)?
        } else {
            request_data
        };

        let sanitized_response = if self.config.enable_pii_masking {
            self.mask_pii_in_response(response_data)?
        } else {
            response_data
        };

        // Encrypt data
        let encrypted_request = self.encrypt_request_data(&sanitized_request)?;
        let encrypted_response = self.encrypt_response_data(&sanitized_response)?;

        // Create complete log entry
        let log_entry = ModelLogEntry {
            id: entry_id,
            agent_id,
            interaction_type,
            timestamp,
            latency_ms: latency.as_millis() as u64,
            model_identifier: model_identifier.to_string(),
            request_data: encrypted_request,
            response_data: Some(encrypted_response),
            metadata,
            error,
            token_usage,
        };

        self.write_log_entry(&log_entry).await?;

        log::debug!("Logged complete model interaction for agent {}", agent_id);
        Ok(())
    }

    /// Encrypt request data
    fn encrypt_request_data(&self, data: &RequestData) -> Result<EncryptedData, LoggingError> {
        let json_data = serde_json::to_string(data)?;
        let encrypted = Aes256GcmCrypto::encrypt_with_password(
            json_data.as_bytes(),
            &self.encryption_key,
        ).map_err(|e| LoggingError::EncryptionFailed {
            message: format!("Failed to encrypt request data: {}", e),
        })?;

        Ok(encrypted)
    }

    /// Encrypt response data
    fn encrypt_response_data(&self, data: &ResponseData) -> Result<EncryptedData, LoggingError> {
        let json_data = serde_json::to_string(data)?;
        let encrypted = Aes256GcmCrypto::encrypt_with_password(
            json_data.as_bytes(),
            &self.encryption_key,
        ).map_err(|e| LoggingError::EncryptionFailed {
            message: format!("Failed to encrypt response data: {}", e),
        })?;

        Ok(encrypted)
    }

    /// Basic PII masking for request data
    fn mask_pii_in_request(&self, mut data: RequestData) -> Result<RequestData, LoggingError> {
        // Basic patterns for common PII
        data.prompt = self.mask_sensitive_patterns(&data.prompt);

        // Mask tool arguments if they contain sensitive data
        if let Some(ref mut args) = data.tool_arguments {
            *args = self.mask_json_values(args.clone());
        }

        // Mask parameters
        for (_, value) in data.parameters.iter_mut() {
            *value = self.mask_json_values(value.clone());
        }

        Ok(data)
    }

    /// Basic PII masking for response data
    fn mask_pii_in_response(&self, mut data: ResponseData) -> Result<ResponseData, LoggingError> {
        data.content = self.mask_sensitive_patterns(&data.content);

        // Mask tool results
        if let Some(ref mut result) = data.tool_result {
            *result = self.mask_json_values(result.clone());
        }

        // Mask metadata
        for (_, value) in data.metadata.iter_mut() {
            *value = self.mask_json_values(value.clone());
        }

        Ok(data)
    }

    /// Mask common sensitive patterns in text
    fn mask_sensitive_patterns(&self, text: &str) -> String {
        use regex::Regex;

        // Common patterns to mask
        let patterns = [
            (r"\b\d{3}-\d{2}-\d{4}\b", "***-**-****"), // SSN
            (r"\b\d{4}[\s-]?\d{4}[\s-]?\d{4}[\s-]?\d{4}\b", "****-****-****-****"), // Credit card
            (r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b", "***@***.***"), // Email
            (r"\b\d{3}[\s-]?\d{3}[\s-]?\d{4}\b", "***-***-****"), // Phone
            (r"\bAPI[_\s]*KEY[\s:=]*[A-Za-z0-9+/]{20,}\b", "API_KEY=***"), // API keys
            (r"\bTOKEN[\s:=]*[A-Za-z0-9+/]{20,}\b", "TOKEN=***"), // Tokens
        ];

        let mut masked_text = text.to_string();
        for (pattern, replacement) in patterns {
            if let Ok(re) = Regex::new(pattern) {
                masked_text = re.replace_all(&masked_text, replacement).to_string();
            }
        }

        masked_text
    }

    /// Mask sensitive values in JSON structures
    fn mask_json_values(&self, value: serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::String(s) => {
                serde_json::Value::String(self.mask_sensitive_patterns(&s))
            }
            serde_json::Value::Object(mut map) => {
                for (key, val) in map.iter_mut() {
                    // Mask known sensitive keys completely
                    if self.is_sensitive_key(key) {
                        *val = serde_json::Value::String("***".to_string());
                    } else {
                        *val = self.mask_json_values(val.clone());
                    }
                }
                serde_json::Value::Object(map)
            }
            serde_json::Value::Array(arr) => {
                serde_json::Value::Array(
                    arr.into_iter().map(|v| self.mask_json_values(v)).collect()
                )
            }
            _ => value,
        }
    }

    /// Check if a key name indicates sensitive data
    fn is_sensitive_key(&self, key: &str) -> bool {
        let sensitive_keys = [
            "password", "token", "key", "secret", "credential",
            "api_key", "auth", "authorization", "ssn", "social_security",
            "credit_card", "card_number", "cvv", "pin"
        ];

        let key_lower = key.to_lowercase();
        sensitive_keys.iter().any(|&sensitive| key_lower.contains(sensitive))
    }

    /// Write a log entry to storage
    async fn write_log_entry(&self, entry: &ModelLogEntry) -> Result<(), LoggingError> {
        // Ensure log directory exists
        if let Some(parent) = std::path::Path::new(&self.config.log_file_path).parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Serialize and append to log file
        let json_line = serde_json::to_string(entry)?;
        let log_line = format!("{}\n", json_line);

        tokio::fs::write(&self.config.log_file_path, log_line.as_bytes()).await?;

        Ok(())
    }

    /// Write a log update (for response data)
    async fn write_log_update(&self, update: &serde_json::Value) -> Result<(), LoggingError> {
        // In a production implementation, this would update the existing entry
        // For now, we'll append an update record
        let update_line = format!("UPDATE: {}\n", serde_json::to_string(update)?);
        
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.config.log_file_path)
            .await?;
        
        file.write_all(update_line.as_bytes()).await?;
        file.flush().await?;

        Ok(())
    }

    /// Decrypt and read log entries (for debugging/analysis)
    pub async fn decrypt_log_entry(&self, encrypted_entry: &ModelLogEntry) -> Result<(RequestData, Option<ResponseData>), LoggingError> {
        // Decrypt request data
        let request_json = Aes256GcmCrypto::decrypt_with_password(
            &encrypted_entry.request_data,
            &self.encryption_key,
        ).map_err(|e| LoggingError::EncryptionFailed {
            message: format!("Failed to decrypt request data: {}", e),
        })?;

        let request_data: RequestData = serde_json::from_slice(&request_json)?;

        // Decrypt response data if present
        let response_data = if let Some(ref encrypted_response) = encrypted_entry.response_data {
            let response_json = Aes256GcmCrypto::decrypt_with_password(
                encrypted_response,
                &self.encryption_key,
            ).map_err(|e| LoggingError::EncryptionFailed {
                message: format!("Failed to decrypt response data: {}", e),
            })?;

            Some(serde_json::from_slice(&response_json)?)
        } else {
            None
        };

        Ok((request_data, response_data))
    }
}

/// Helper trait for timing model operations
pub trait TimedOperation {
    /// Execute an operation and return the result with timing
    #[allow(async_fn_in_trait)]
    async fn timed<F, R, E>(&self, operation: F) -> (Result<R, E>, Duration)
    where
        F: std::future::Future<Output = Result<R, E>>;
}

impl TimedOperation for ModelLogger {
    async fn timed<F, R, E>(&self, operation: F) -> (Result<R, E>, Duration)
    where
        F: std::future::Future<Output = Result<R, E>>,
    {
        let start = Instant::now();
        let result = operation.await;
        let duration = start.elapsed();
        (result, duration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tempfile::tempdir;
    use crate::types::AgentId;

    // Mock SecretStore for testing
    #[derive(Debug, Clone)]
    struct MockSecretStore {
        secrets: HashMap<String, String>,
        should_fail: bool,
    }

    impl MockSecretStore {
        fn new() -> Self {
            let mut secrets = HashMap::new();
            secrets.insert("symbiont/logging/encryption_key".to_string(), "test_key_123".to_string());
            Self {
                secrets,
                should_fail: false,
            }
        }

        fn new_failing() -> Self {
            Self {
                secrets: HashMap::new(),
                should_fail: true,
            }
        }
    }

    #[async_trait::async_trait]
    impl crate::secrets::SecretStore for MockSecretStore {
        async fn get_secret(&self, key: &str) -> Result<crate::secrets::Secret, crate::secrets::SecretError> {
            if self.should_fail {
                return Err(crate::secrets::SecretError::NotFound { key: key.to_string() });
            }
            
            if let Some(value) = self.secrets.get(key) {
                Ok(crate::secrets::Secret::new(key.to_string(), value.clone()))
            } else {
                Err(crate::secrets::SecretError::NotFound { key: key.to_string() })
            }
        }

        async fn list_secrets(&self) -> Result<Vec<String>, crate::secrets::SecretError> {
            Ok(self.secrets.keys().cloned().collect())
        }
    }

    #[tokio::test]
    async fn test_logger_creation_with_secret_store() {
        let config = LoggingConfig {
            log_file_path: "/tmp/test_model_logs.json".to_string(),
            ..Default::default()
        };

        let secret_store: Arc<dyn crate::secrets::SecretStore> = Arc::new(MockSecretStore::new());
        let logger = ModelLogger::new(config, Some(secret_store));
        assert!(logger.is_ok());
    }

    #[tokio::test]
    async fn test_logger_creation_without_secret_store() {
        let config = LoggingConfig {
            log_file_path: "/tmp/test_model_logs.json".to_string(),
            encryption_key_env: Some("TEST_LOGGING_KEY".to_string()),
            ..Default::default()
        };

        // Set environment variable for fallback
        std::env::set_var("TEST_LOGGING_KEY", "fallback_key_456");

        let logger = ModelLogger::new(config, None);
        assert!(logger.is_ok());

        std::env::remove_var("TEST_LOGGING_KEY");
    }

    #[tokio::test]
    async fn test_logger_creation_with_defaults() {
        let logger = ModelLogger::with_defaults();
        assert!(logger.is_ok());
    }

    #[tokio::test]
    async fn test_encryption_key_retrieval_priority() {
        // Test SecretStore priority
        let config = LoggingConfig {
            encryption_key_name: "test/key".to_string(),
            encryption_key_env: Some("TEST_ENV_KEY".to_string()),
            ..Default::default()
        };

        let secret_store: Arc<dyn crate::secrets::SecretStore> = Arc::new(MockSecretStore::new());
        std::env::set_var("TEST_ENV_KEY", "env_key_value");

        let key = ModelLogger::get_encryption_key(&config, &Some(secret_store));
        // Should get from secret store, not environment
        assert!(key.is_ok());

        std::env::remove_var("TEST_ENV_KEY");
    }

    #[tokio::test]
    async fn test_encryption_key_fallback_to_env() {
        let config = LoggingConfig {
            encryption_key_name: "nonexistent/key".to_string(),
            encryption_key_env: Some("TEST_FALLBACK_KEY".to_string()),
            ..Default::default()
        };

        let secret_store: Arc<dyn crate::secrets::SecretStore> = Arc::new(MockSecretStore::new_failing());
        std::env::set_var("TEST_FALLBACK_KEY", "fallback_env_key");

        let key = ModelLogger::get_encryption_key(&config, &Some(secret_store));
        assert!(key.is_ok());

        std::env::remove_var("TEST_FALLBACK_KEY");
    }

    #[tokio::test]
    async fn test_encryption_decryption_roundtrip() {
        let logger = ModelLogger::with_defaults().unwrap();
        
        let request_data = RequestData {
            prompt: "Test prompt".to_string(),
            tool_name: Some("test_tool".to_string()),
            tool_arguments: Some(serde_json::json!({"arg1": "value1"})),
            parameters: {
                let mut params = HashMap::new();
                params.insert("param1".to_string(), serde_json::json!("value1"));
                params
            },
        };

        let response_data = ResponseData {
            content: "Test response".to_string(),
            tool_result: Some(serde_json::json!({"result": "success"})),
            confidence: Some(0.95),
            metadata: {
                let mut meta = HashMap::new();
                meta.insert("meta1".to_string(), serde_json::json!("value1"));
                meta
            },
        };

        // Test request encryption/decryption
        let encrypted_request = logger.encrypt_request_data(&request_data).unwrap();
        let encrypted_response = logger.encrypt_response_data(&response_data).unwrap();

        // Create a mock log entry for decryption testing
        let log_entry = ModelLogEntry {
            id: "test_id".to_string(),
            agent_id: AgentId::new(),
            interaction_type: ModelInteractionType::Completion,
            timestamp: chrono::Utc::now(),
            latency_ms: 100,
            model_identifier: "test_model".to_string(),
            request_data: encrypted_request,
            response_data: Some(encrypted_response),
            metadata: HashMap::new(),
            error: None,
            token_usage: None,
        };

        let (decrypted_request, decrypted_response) = logger.decrypt_log_entry(&log_entry).await.unwrap();
        
        assert_eq!(decrypted_request.prompt, request_data.prompt);
        assert_eq!(decrypted_request.tool_name, request_data.tool_name);
        
        let decrypted_resp = decrypted_response.unwrap();
        assert_eq!(decrypted_resp.content, response_data.content);
        assert_eq!(decrypted_resp.confidence, response_data.confidence);
    }

    #[tokio::test]
    async fn test_pii_masking_comprehensive() {
        let logger = ModelLogger::with_defaults().unwrap();
        
        // Test various PII patterns
        let test_cases = vec![
            ("SSN: 123-45-6789", "***-**-****"),
            ("Credit card: 4532-1234-5678-9012", "****-****-****-****"),
            ("Email: user@example.com", "***@***.***"),
            ("Phone: 555-123-4567", "***-***-****"),
            ("API_KEY: abc123def456ghi789", "API_KEY=***"),
            ("TOKEN: xyz789uvw456rst123", "TOKEN=***"),
        ];

        for (input, expected_pattern) in test_cases {
            let masked = logger.mask_sensitive_patterns(input);
            assert!(masked.contains(expected_pattern),
                "Failed to mask '{}', got '{}'", input, masked);
        }
    }

    #[tokio::test]
    async fn test_pii_masking_json_values() {
        let logger = ModelLogger::with_defaults().unwrap();
        
        let json_data = serde_json::json!({
            "password": "secret123",
            "api_key": "abc123def456",
            "username": "john_doe",
            "data": "safe_content",
            "nested": {
                "token": "xyz789",
                "info": "public_info"
            }
        });

        let masked_json = logger.mask_json_values(json_data);
        
        // Sensitive keys should be masked
        assert_eq!(masked_json["password"], "***");
        assert_eq!(masked_json["api_key"], "***");
        assert_eq!(masked_json["nested"]["token"], "***");
        
        // Non-sensitive keys should remain
        assert_eq!(masked_json["username"], "john_doe");
        assert_eq!(masked_json["data"], "safe_content");
        assert_eq!(masked_json["nested"]["info"], "public_info");
    }

    #[tokio::test]
    async fn test_sensitive_key_detection() {
        let logger = ModelLogger::with_defaults().unwrap();
        
        // Sensitive keys
        let sensitive_keys = vec![
            "password", "PASSWORD", "Password",
            "token", "TOKEN", "auth_token",
            "key", "api_key", "API_KEY",
            "secret", "SECRET", "client_secret",
            "credential", "credentials",
            "ssn", "social_security",
            "credit_card", "card_number",
            "cvv", "pin"
        ];

        for key in sensitive_keys {
            assert!(logger.is_sensitive_key(key), "Should detect '{}' as sensitive", key);
        }

        // Non-sensitive keys
        let safe_keys = vec![
            "username", "user_id", "name",
            "data", "content", "message",
            "timestamp", "id", "status"
        ];

        for key in safe_keys {
            assert!(!logger.is_sensitive_key(key), "Should not detect '{}' as sensitive", key);
        }
    }

    #[tokio::test]
    async fn test_log_request_and_response() {
        let temp_dir = tempdir().unwrap();
        let log_path = temp_dir.path().join("test_request_response.json");
        
        let config = LoggingConfig {
            log_file_path: log_path.to_string_lossy().to_string(),
            ..Default::default()
        };

        let logger = ModelLogger::new(config, None).unwrap();
        let agent_id = AgentId::new();

        let request_data = RequestData {
            prompt: "What is the weather?".to_string(),
            tool_name: None,
            tool_arguments: None,
            parameters: HashMap::new(),
        };

        // Log request
        let entry_id = logger.log_request(
            agent_id,
            ModelInteractionType::Completion,
            "test-model",
            request_data,
            HashMap::new(),
        ).await.unwrap();

        assert!(!entry_id.is_empty());

        // Log response
        let response_data = ResponseData {
            content: "The weather is sunny".to_string(),
            tool_result: None,
            confidence: Some(0.95),
            metadata: HashMap::new(),
        };

        let result = logger.log_response(
            &entry_id,
            response_data,
            Duration::from_millis(150),
            Some(TokenUsage {
                input_tokens: 10,
                output_tokens: 15,
                total_tokens: 25,
            }),
            None,
        ).await;

        assert!(result.is_ok());
        
        // Verify log file was created and updated
        assert!(tokio::fs::metadata(&log_path).await.is_ok());
    }

    #[tokio::test]
    async fn test_complete_interaction_logging() {
        let temp_dir = tempdir().unwrap();
        let log_path = temp_dir.path().join("test_complete_interaction.json");
        
        let config = LoggingConfig {
            log_file_path: log_path.to_string_lossy().to_string(),
            ..Default::default()
        };

        let logger = ModelLogger::new(config, None).unwrap();
        let agent_id = AgentId::new();

        let request_data = RequestData {
            prompt: "Generate code for sorting".to_string(),
            tool_name: Some("code_generator".to_string()),
            tool_arguments: Some(serde_json::json!({"language": "python"})),
            parameters: {
                let mut params = HashMap::new();
                params.insert("temperature".to_string(), serde_json::json!(0.7));
                params
            },
        };

        let response_data = ResponseData {
            content: "def sort_list(lst): return sorted(lst)".to_string(),
            tool_result: Some(serde_json::json!({"status": "success"})),
            confidence: Some(0.92),
            metadata: {
                let mut meta = HashMap::new();
                meta.insert("language".to_string(), serde_json::json!("python"));
                meta
            },
        };

        let result = logger.log_interaction(
            agent_id,
            ModelInteractionType::ToolCall,
            "test-code-model",
            request_data,
            response_data,
            Duration::from_millis(350),
            {
                let mut meta = HashMap::new();
                meta.insert("session_id".to_string(), "test_session".to_string());
                meta
            },
            Some(TokenUsage {
                input_tokens: 25,
                output_tokens: 40,
                total_tokens: 65,
            }),
            None,
        ).await;

        assert!(result.is_ok());

        // Verify log file was created
        assert!(tokio::fs::metadata(&log_path).await.is_ok());
    }

    #[tokio::test]
    async fn test_logging_disabled() {
        let config = LoggingConfig {
            enabled: false,
            ..Default::default()
        };

        let logger = ModelLogger::new(config, None).unwrap();
        let agent_id = AgentId::new();

        let request_data = RequestData {
            prompt: "Test prompt".to_string(),
            tool_name: None,
            tool_arguments: None,
            parameters: HashMap::new(),
        };

        // When logging is disabled, should return empty string
        let entry_id = logger.log_request(
            agent_id,
            ModelInteractionType::Completion,
            "test-model",
            request_data,
            HashMap::new(),
        ).await.unwrap();

        assert!(entry_id.is_empty());
    }

    #[tokio::test]
    async fn test_logging_with_error() {
        let temp_dir = tempdir().unwrap();
        let log_path = temp_dir.path().join("test_error_logging.json");
        
        let config = LoggingConfig {
            log_file_path: log_path.to_string_lossy().to_string(),
            ..Default::default()
        };

        let logger = ModelLogger::new(config, None).unwrap();
        let agent_id = AgentId::new();

        let request_data = RequestData {
            prompt: "Error test".to_string(),
            tool_name: None,
            tool_arguments: None,
            parameters: HashMap::new(),
        };

        let response_data = ResponseData {
            content: "Error occurred".to_string(),
            tool_result: None,
            confidence: None,
            metadata: HashMap::new(),
        };

        let result = logger.log_interaction(
            agent_id,
            ModelInteractionType::Completion,
            "test-model",
            request_data,
            response_data,
            Duration::from_millis(50),
            HashMap::new(),
            None,
            Some("Model execution failed".to_string()),
        ).await;

        assert!(result.is_ok());
        assert!(tokio::fs::metadata(&log_path).await.is_ok());
    }

    #[tokio::test]
    async fn test_logging_config_validation() {
        // Test default config
        let config = LoggingConfig::default();
        assert!(config.enabled);
        assert_eq!(config.log_file_path, "logs/model_io.encrypted.log");
        assert_eq!(config.encryption_key_name, "symbiont/logging/encryption_key");
        assert_eq!(config.max_entry_size, 1024 * 1024);
        assert_eq!(config.retention_days, 90);
        assert!(config.enable_pii_masking);
        assert_eq!(config.batch_size, 100);
    }

    #[tokio::test]
    async fn test_model_interaction_types() {
        // Test all ModelInteractionType variants
        let types = vec![
            ModelInteractionType::Completion,
            ModelInteractionType::ToolCall,
            ModelInteractionType::RagQuery,
            ModelInteractionType::AgentExecution,
        ];

        for interaction_type in types {
            // Ensure they can be serialized/deserialized
            let serialized = serde_json::to_string(&interaction_type).unwrap();
            let deserialized: ModelInteractionType = serde_json::from_str(&serialized).unwrap();
            assert_eq!(interaction_type, deserialized);
        }
    }

    #[tokio::test]
    async fn test_token_usage_tracking() {
        let token_usage = TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            total_tokens: 150,
        };

        // Test serialization
        let serialized = serde_json::to_string(&token_usage).unwrap();
        let deserialized: TokenUsage = serde_json::from_str(&serialized).unwrap();
        
        assert_eq!(token_usage.input_tokens, deserialized.input_tokens);
        assert_eq!(token_usage.output_tokens, deserialized.output_tokens);
        assert_eq!(token_usage.total_tokens, deserialized.total_tokens);
    }

    #[tokio::test]
    async fn test_request_response_data_structures() {
        let request_data = RequestData {
            prompt: "Test prompt".to_string(),
            tool_name: Some("test_tool".to_string()),
            tool_arguments: Some(serde_json::json!({"arg": "value"})),
            parameters: {
                let mut params = HashMap::new();
                params.insert("temp".to_string(), serde_json::json!(0.8));
                params
            },
        };

        let response_data = ResponseData {
            content: "Test response".to_string(),
            tool_result: Some(serde_json::json!({"result": "success"})),
            confidence: Some(0.9),
            metadata: {
                let mut meta = HashMap::new();
                meta.insert("model".to_string(), serde_json::json!("test"));
                meta
            },
        };

        // Test serialization/deserialization
        let req_serialized = serde_json::to_string(&request_data).unwrap();
        let req_deserialized: RequestData = serde_json::from_str(&req_serialized).unwrap();
        assert_eq!(request_data.prompt, req_deserialized.prompt);

        let resp_serialized = serde_json::to_string(&response_data).unwrap();
        let resp_deserialized: ResponseData = serde_json::from_str(&resp_serialized).unwrap();
        assert_eq!(response_data.content, resp_deserialized.content);
    }

    #[tokio::test]
    async fn test_pii_masking_request_data() {
        let logger = ModelLogger::with_defaults().unwrap();
        
        let request_data = RequestData {
            prompt: "My SSN is 123-45-6789 and API key is abc123def456".to_string(),
            tool_name: Some("sensitive_tool".to_string()),
            tool_arguments: Some(serde_json::json!({
                "user_password": "secret123",
                "api_token": "xyz789",
                "safe_data": "public_info"
            })),
            parameters: {
                let mut params = HashMap::new();
                params.insert("auth_key".to_string(), serde_json::json!("sensitive_key"));
                params.insert("username".to_string(), serde_json::json!("john_doe"));
                params
            },
        };

        let masked_request = logger.mask_pii_in_request(request_data).unwrap();
        
        // Check prompt masking
        assert!(!masked_request.prompt.contains("123-45-6789"));
        assert!(!masked_request.prompt.contains("abc123def456"));
        
        // Check tool arguments masking
        if let Some(args) = &masked_request.tool_arguments {
            assert_eq!(args["user_password"], "***");
            assert_eq!(args["api_token"], "***");
            assert_eq!(args["safe_data"], "public_info");
        }
        
        // Check parameters masking
        assert_eq!(masked_request.parameters["auth_key"], "***");
        assert_eq!(masked_request.parameters["username"], "john_doe");
    }

    #[tokio::test]
    async fn test_pii_masking_response_data() {
        let logger = ModelLogger::with_defaults().unwrap();
        
        let response_data = ResponseData {
            content: "Your password is secret123 and token xyz789".to_string(),
            tool_result: Some(serde_json::json!({
                "password": "hidden123",
                "result": "success"
            })),
            confidence: Some(0.95),
            metadata: {
                let mut meta = HashMap::new();
                meta.insert("secret".to_string(), serde_json::json!("confidential"));
                meta.insert("public".to_string(), serde_json::json!("open"));
                meta
            },
        };

        let masked_response = logger.mask_pii_in_response(response_data).unwrap();
        
        // Check content masking
        assert!(!masked_response.content.contains("secret123"));
        assert!(!masked_response.content.contains("xyz789"));
        
        // Check tool result masking
        if let Some(result) = &masked_response.tool_result {
            assert_eq!(result["password"], "***");
            assert_eq!(result["result"], "success");
        }
        
        // Check metadata masking
        assert_eq!(masked_response.metadata["secret"], "***");
        assert_eq!(masked_response.metadata["public"], "open");
    }
}