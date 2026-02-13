//! File-based secrets backend implementation
//!
//! This module provides a file-based secrets store that supports encrypted storage
//! using AES-256-GCM with various key providers (environment variables, OS keychain).

use super::{BoxedAuditSink, Secret, SecretAuditEvent, SecretError, SecretStore};
use crate::crypto::{Aes256GcmCrypto, CryptoError, EncryptedData, KeyUtils};
use crate::secrets::config::{FileConfig, FileFormat};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::time::SystemTime;
use tokio::sync::RwLock;

/// File-based secrets store implementation
pub struct FileSecretStore {
    config: FileConfig,
    audit_sink: Option<BoxedAuditSink>,
    agent_id: String,
    cache: RwLock<Option<(SystemTime, HashMap<String, String>)>>,
}

impl FileSecretStore {
    /// Create a new FileSecretStore with the given configuration
    pub async fn new(
        config: FileConfig,
        audit_sink: Option<BoxedAuditSink>,
        agent_id: String,
    ) -> Result<Self, SecretError> {
        Ok(Self {
            config,
            audit_sink,
            agent_id,
            cache: RwLock::new(None),
        })
    }

    /// Log an audit event if an audit sink is configured.
    /// In strict mode, returns an error if audit logging fails.
    /// In permissive mode, logs a warning and continues.
    async fn log_audit_event(&self, event: SecretAuditEvent) -> Result<(), SecretError> {
        if let Some(audit_sink) = &self.audit_sink {
            if let Err(e) = audit_sink.log_event(event).await {
                match audit_sink.failure_mode() {
                    crate::secrets::auditing::AuditFailureMode::Strict => {
                        return Err(SecretError::AuditFailed {
                            message: format!("Audit logging failed (strict mode): {}", e),
                        });
                    }
                    crate::secrets::auditing::AuditFailureMode::Permissive => {
                        tracing::warn!("Audit logging failed (permissive mode): {}", e);
                    }
                }
            }
        }
        Ok(())
    }

    /// Load secrets with mtime-based caching.
    ///
    /// Returns cached data if the file's mtime hasn't changed, avoiding
    /// expensive re-decryption (Argon2 KDF) on every call.
    async fn load_secrets_cached(&self) -> Result<HashMap<String, String>, SecretError> {
        let mtime = fs::metadata(&self.config.path)
            .and_then(|m| m.modified())
            .map_err(|e| SecretError::IoError {
                message: format!("Failed to stat secrets file: {}", e),
            })?;

        // Fast path: return cached data if mtime matches
        {
            let guard = self.cache.read().await;
            if let Some((cached_mtime, ref secrets)) = *guard {
                if cached_mtime == mtime {
                    return Ok(secrets.clone());
                }
            }
        }

        // Slow path: reload from disk
        let secrets = self.load_secrets().await?;

        // Update cache
        {
            let mut guard = self.cache.write().await;
            *guard = Some((mtime, secrets.clone()));
        }

        Ok(secrets)
    }

    /// Load and decrypt secrets from the file.
    ///
    /// Acquires a shared (read) file lock via `fd_lock::RwLock` so that
    /// concurrent readers never observe a partially-written file.
    /// The lock is released automatically when the guard drops.
    async fn load_secrets(&self) -> Result<HashMap<String, String>, SecretError> {
        let path = self.config.path.clone();
        let encryption_enabled = self.config.encryption.enabled;

        // Blocking I/O with file lock — run off the async executor
        let file_content = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, SecretError> {
            let file = std::fs::File::open(&path).map_err(|e| SecretError::IoError {
                message: format!("Failed to open secrets file: {}", e),
            })?;

            let lock = fd_lock::RwLock::new(file);
            let guard = lock.read().map_err(|e| SecretError::IoError {
                message: format!("Failed to acquire read lock on secrets file: {}", e),
            })?;

            // Read via &File (std::io::Read is impl'd for &File)
            use std::io::Read;
            let mut buf = Vec::new();
            (&*guard)
                .read_to_end(&mut buf)
                .map_err(|e| SecretError::IoError {
                    message: format!("Failed to read secrets file: {}", e),
                })?;
            // Lock released when `guard` drops here
            Ok(buf)
        })
        .await
        .map_err(|e| SecretError::IoError {
            message: format!("Blocking task panicked: {}", e),
        })??;

        let secrets_data = if encryption_enabled {
            // Decrypt the content
            self.decrypt_content(&file_content).await?
        } else {
            // Use content as-is
            String::from_utf8(file_content).map_err(|e| SecretError::ParseError {
                message: format!("Invalid UTF-8 in secrets file: {}", e),
            })?
        };

        // Parse the content based on format
        self.parse_secrets_data(&secrets_data)
    }

    /// Decrypt file content using the configured key provider
    async fn decrypt_content(&self, encrypted_content: &[u8]) -> Result<String, SecretError> {
        // Get the decryption key
        let key = self.get_decryption_key().await?;

        // Parse the encrypted content as JSON to get the EncryptedData structure
        let encrypted_data: EncryptedData =
            serde_json::from_slice(encrypted_content).map_err(|e| SecretError::ParseError {
                message: format!("Failed to parse encrypted data: {}", e),
            })?;

        // Verify the algorithm matches our configuration
        if encrypted_data.algorithm != self.config.encryption.algorithm {
            return Err(SecretError::CryptoError {
                message: format!(
                    "Algorithm mismatch: expected {}, found {}",
                    self.config.encryption.algorithm, encrypted_data.algorithm
                ),
            });
        }

        // Decrypt the content
        let decrypted_bytes = Aes256GcmCrypto::decrypt_with_password(&encrypted_data, &key)
            .map_err(|e| self.map_crypto_error(e))?;

        String::from_utf8(decrypted_bytes).map_err(|e| SecretError::ParseError {
            message: format!("Decrypted content is not valid UTF-8: {}", e),
        })
    }

    /// Get the decryption key from the configured provider
    async fn get_decryption_key(&self) -> Result<String, SecretError> {
        match self.config.encryption.key.provider.as_str() {
            "env" => {
                let env_var = self.config.encryption.key.env_var.as_ref().ok_or_else(|| {
                    SecretError::ConfigurationError {
                        message: "Environment variable name not specified for 'env' key provider"
                            .to_string(),
                    }
                })?;

                KeyUtils::get_key_from_env(env_var).map_err(|e| self.map_crypto_error(e))
            }
            "os_keychain" => {
                let service = self.config.encryption.key.service.as_ref().ok_or_else(|| {
                    SecretError::ConfigurationError {
                        message: "Service name not specified for 'os_keychain' key provider"
                            .to_string(),
                    }
                })?;

                let account = self.config.encryption.key.account.as_ref().ok_or_else(|| {
                    SecretError::ConfigurationError {
                        message: "Account name not specified for 'os_keychain' key provider"
                            .to_string(),
                    }
                })?;

                let key_utils = KeyUtils::new();
                key_utils
                    .get_key_from_keychain(service, account)
                    .map_err(|e| self.map_crypto_error(e))
            }
            "file" => {
                let file_path = self
                    .config
                    .encryption
                    .key
                    .file_path
                    .as_ref()
                    .ok_or_else(|| SecretError::ConfigurationError {
                        message: "File path not specified for 'file' key provider".to_string(),
                    })?;

                fs::read_to_string(file_path)
                    .map(|content| content.trim().to_string())
                    .map_err(|e| SecretError::IoError {
                        message: format!("Failed to read key file: {}", e),
                    })
            }
            _ => Err(SecretError::ConfigurationError {
                message: format!(
                    "Unsupported key provider: {}",
                    self.config.encryption.key.provider
                ),
            }),
        }
    }

    /// Parse secrets data based on the configured format
    fn parse_secrets_data(&self, data: &str) -> Result<HashMap<String, String>, SecretError> {
        match self.config.format {
            FileFormat::Json => self.parse_json_secrets(data),
            FileFormat::Yaml => self.parse_yaml_secrets(data),
            FileFormat::Toml => self.parse_toml_secrets(data),
            FileFormat::Env => self.parse_env_secrets(data),
        }
    }

    /// Parse JSON format secrets
    fn parse_json_secrets(&self, data: &str) -> Result<HashMap<String, String>, SecretError> {
        let value: Value = serde_json::from_str(data).map_err(|e| SecretError::ParseError {
            message: format!("Failed to parse JSON: {}", e),
        })?;

        let mut secrets = HashMap::new();
        if let Value::Object(map) = value {
            for (key, value) in map {
                let secret_value = match value {
                    Value::String(s) => s,
                    _ => value.to_string(),
                };
                secrets.insert(key, secret_value);
            }
        } else {
            return Err(SecretError::ParseError {
                message: "JSON root must be an object".to_string(),
            });
        }

        Ok(secrets)
    }

    /// Parse YAML format secrets
    fn parse_yaml_secrets(&self, data: &str) -> Result<HashMap<String, String>, SecretError> {
        let value: serde_yaml::Value =
            serde_yaml::from_str(data).map_err(|e| SecretError::ParseError {
                message: format!("Failed to parse YAML: {}", e),
            })?;

        let mut secrets = HashMap::new();
        if let serde_yaml::Value::Mapping(map) = value {
            for (key, value) in map {
                if let serde_yaml::Value::String(key_str) = key {
                    let secret_value = match value {
                        serde_yaml::Value::String(s) => s,
                        _ => {
                            serde_yaml::to_string(&value).map_err(|e| SecretError::ParseError {
                                message: format!("Failed to serialize YAML value: {}", e),
                            })?
                        }
                    };
                    secrets.insert(key_str, secret_value);
                }
            }
        } else {
            return Err(SecretError::ParseError {
                message: "YAML root must be a mapping".to_string(),
            });
        }

        Ok(secrets)
    }

    /// Parse TOML format secrets
    fn parse_toml_secrets(&self, data: &str) -> Result<HashMap<String, String>, SecretError> {
        let value: toml::Value = toml::from_str(data).map_err(|e| SecretError::ParseError {
            message: format!("Failed to parse TOML: {}", e),
        })?;

        let mut secrets = HashMap::new();
        if let toml::Value::Table(table) = value {
            for (key, value) in table {
                let secret_value = match value {
                    toml::Value::String(s) => s,
                    _ => value.to_string(),
                };
                secrets.insert(key, secret_value);
            }
        } else {
            return Err(SecretError::ParseError {
                message: "TOML root must be a table".to_string(),
            });
        }

        Ok(secrets)
    }

    /// Parse environment file format secrets (key=value pairs) using dotenvy
    /// for robust handling of multiline values, escape sequences, export prefix, etc.
    fn parse_env_secrets(&self, data: &str) -> Result<HashMap<String, String>, SecretError> {
        let mut secrets = HashMap::new();
        for item in dotenvy::from_read_iter(data.as_bytes()) {
            match item {
                Ok((key, value)) => {
                    secrets.insert(key, value);
                }
                Err(e) => {
                    return Err(SecretError::ParseError {
                        message: format!("Failed to parse env file: {}", e),
                    });
                }
            }
        }
        Ok(secrets)
    }

    /// Map crypto errors to secret errors
    fn map_crypto_error(&self, error: CryptoError) -> SecretError {
        SecretError::CryptoError {
            message: error.to_string(),
        }
    }
}

#[async_trait]
impl SecretStore for FileSecretStore {
    /// Retrieve a secret by key
    async fn get_secret(&self, key: &str) -> Result<Secret, SecretError> {
        let result: Result<Secret, SecretError> = async {
            let secrets = self.load_secrets_cached().await?;

            match secrets.get(key) {
                Some(value) => Ok(Secret::new(key.to_string(), value.clone())),
                None => Err(SecretError::NotFound {
                    key: key.to_string(),
                }),
            }
        }
        .await;

        // Log audit event — in strict mode, audit failure blocks the operation
        let audit_event = match &result {
            Ok(_) => SecretAuditEvent::success(
                self.agent_id.clone(),
                "get_secret".to_string(),
                Some(key.to_string()),
            ),
            Err(e) => SecretAuditEvent::failure(
                self.agent_id.clone(),
                "get_secret".to_string(),
                Some(key.to_string()),
                e.to_string(),
            ),
        };
        self.log_audit_event(audit_event).await?;

        result
    }

    /// List all available secret keys, optionally filtered by prefix
    async fn list_secrets(&self) -> Result<Vec<String>, SecretError> {
        let result: Result<Vec<String>, SecretError> = async {
            let secrets = self.load_secrets_cached().await?;
            Ok(secrets.keys().cloned().collect())
        }
        .await;

        // Log audit event — in strict mode, audit failure blocks the operation
        let audit_event = match &result {
            Ok(keys) => {
                SecretAuditEvent::success(self.agent_id.clone(), "list_secrets".to_string(), None)
                    .with_metadata(serde_json::json!({
                        "secrets_count": keys.len()
                    }))
            }
            Err(e) => SecretAuditEvent::failure(
                self.agent_id.clone(),
                "list_secrets".to_string(),
                None,
                e.to_string(),
            ),
        };
        self.log_audit_event(audit_event).await?;

        result
    }
}

/// Extension trait for prefix filtering
impl FileSecretStore {
    /// List secrets with prefix filtering
    pub async fn list_secrets_with_prefix(&self, prefix: &str) -> Result<Vec<String>, SecretError> {
        let secrets = self.load_secrets_cached().await?;
        Ok(secrets
            .keys()
            .filter(|key| key.starts_with(prefix))
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::NamedTempFile;

    fn create_test_config(path: PathBuf) -> FileConfig {
        FileConfig {
            path,
            format: FileFormat::Json,
            encryption: crate::secrets::config::FileEncryptionConfig {
                enabled: false,
                algorithm: "AES-256-GCM".to_string(),
                kdf: "Argon2".to_string(),
                key: crate::secrets::config::FileKeyConfig {
                    provider: "env".to_string(),
                    env_var: Some("TEST_KEY".to_string()),
                    service: None,
                    account: None,
                    file_path: None,
                },
            },
            permissions: Some(0o600),
            watch_for_changes: false,
            backup: crate::secrets::config::FileBackupConfig::default(),
        }
    }

    #[tokio::test]
    async fn test_parse_json_secrets() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, r#"{{"key1": "value1", "key2": "value2"}}"#).unwrap();

        let config = create_test_config(temp_file.path().to_path_buf());
        let store = FileSecretStore::new(config, None, "test-agent".to_string())
            .await
            .unwrap();

        let secret = store.get_secret("key1").await.unwrap();
        assert_eq!(secret.value(), "value1");

        let keys = store.list_secrets().await.unwrap();
        assert!(keys.contains(&"key1".to_string()));
        assert!(keys.contains(&"key2".to_string()));
    }

    #[tokio::test]
    async fn test_secret_not_found() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, r#"{{"key1": "value1"}}"#).unwrap();

        let config = create_test_config(temp_file.path().to_path_buf());
        let store = FileSecretStore::new(config, None, "test-agent".to_string())
            .await
            .unwrap();

        let result = store.get_secret("nonexistent").await;
        assert!(matches!(result, Err(SecretError::NotFound { .. })));
    }

    #[tokio::test]
    async fn test_list_secrets_with_prefix() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(
            temp_file,
            r#"{{"app_key1": "value1", "app_key2": "value2", "other_key": "value3"}}"#
        )
        .unwrap();

        let config = create_test_config(temp_file.path().to_path_buf());
        let store = FileSecretStore::new(config, None, "test-agent".to_string())
            .await
            .unwrap();

        let keys = store.list_secrets_with_prefix("app_").await.unwrap();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&"app_key1".to_string()));
        assert!(keys.contains(&"app_key2".to_string()));
        assert!(!keys.contains(&"other_key".to_string()));
    }

    #[tokio::test]
    async fn test_concurrent_reads_no_deadlock() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, r#"{{"secret_a": "val_a", "secret_b": "val_b"}}"#).unwrap();

        let config = create_test_config(temp_file.path().to_path_buf());
        let store = std::sync::Arc::new(
            FileSecretStore::new(config, None, "test-agent".to_string())
                .await
                .unwrap(),
        );

        // Spawn multiple concurrent readers — shared read locks must not deadlock
        let mut handles = Vec::new();
        for _ in 0..10 {
            let s = store.clone();
            handles.push(tokio::spawn(async move {
                let secret = s.get_secret("secret_a").await.unwrap();
                assert_eq!(secret.value(), "val_a");
            }));
        }

        for h in handles {
            h.await.unwrap();
        }
    }
}
