//! Configuration structures for secrets backends
//!
//! This module defines the configuration structures that can be deserialized
//! from symbiont.toml for different secrets backend types.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// Configuration for secrets management
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretsConfig {
    /// The secrets backend configuration
    #[serde(flatten)]
    pub backend: SecretsBackend,
    /// Common configuration options
    #[serde(default)]
    pub common: CommonSecretsConfig,
}

/// Enumeration of supported secrets backends
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SecretsBackend {
    /// HashiCorp Vault backend
    Vault(VaultConfig),
    /// File-based secrets backend
    File(FileConfig),
}

/// Common configuration options for all secrets backends
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommonSecretsConfig {
    /// Timeout for secrets operations (in seconds)
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    /// Maximum number of retry attempts
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// Enable caching of secrets
    #[serde(default = "default_enable_cache")]
    pub enable_cache: bool,
    /// Cache TTL in seconds
    #[serde(default = "default_cache_ttl")]
    pub cache_ttl_seconds: u64,
    /// Audit configuration for secret operations
    pub audit: Option<super::auditing::AuditConfig>,
}

impl Default for CommonSecretsConfig {
    fn default() -> Self {
        Self {
            timeout_seconds: default_timeout(),
            max_retries: default_max_retries(),
            enable_cache: default_enable_cache(),
            cache_ttl_seconds: default_cache_ttl(),
            audit: None,
        }
    }
}

/// Configuration for HashiCorp Vault backend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultConfig {
    /// Vault server URL
    pub url: String,
    /// Authentication method configuration
    pub auth: VaultAuthConfig,
    /// Vault namespace (optional)
    pub namespace: Option<String>,
    /// Default mount path for KV secrets engine
    #[serde(default = "default_vault_mount")]
    pub mount_path: String,
    /// API version for KV secrets engine (v1 or v2)
    #[serde(default = "default_vault_api_version")]
    pub api_version: String,
    /// TLS configuration
    #[serde(default)]
    pub tls: VaultTlsConfig,
    /// Connection pool settings
    #[serde(default)]
    pub connection: VaultConnectionConfig,
}

/// Vault authentication configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "lowercase")]
pub enum VaultAuthConfig {
    /// Token-based authentication
    Token {
        /// Vault token (can be from environment variable)
        token: String,
    },
    /// AppRole authentication
    AppRole {
        /// Role ID
        role_id: String,
        /// Secret ID
        secret_id: String,
        /// Mount path for AppRole auth
        #[serde(default = "default_approle_mount")]
        mount_path: String,
    },
    /// Kubernetes authentication
    Kubernetes {
        /// Service account token path
        #[serde(default = "default_k8s_token_path")]
        token_path: String,
        /// Kubernetes role
        role: String,
        /// Mount path for Kubernetes auth
        #[serde(default = "default_k8s_mount")]
        mount_path: String,
    },
    /// AWS IAM authentication
    Aws {
        /// AWS region
        region: String,
        /// Vault role name
        role: String,
        /// Mount path for AWS auth
        #[serde(default = "default_aws_mount")]
        mount_path: String,
    },
}

/// Vault TLS configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VaultTlsConfig {
    /// Skip TLS certificate verification (insecure)
    #[serde(default)]
    pub skip_verify: bool,
    /// Path to CA certificate file
    pub ca_cert: Option<PathBuf>,
    /// Path to client certificate file
    pub client_cert: Option<PathBuf>,
    /// Path to client private key file
    pub client_key: Option<PathBuf>,
}

/// Vault connection configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultConnectionConfig {
    /// Maximum number of connections in the pool
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,
    /// Connection timeout in seconds
    #[serde(default = "default_connection_timeout")]
    pub connection_timeout_seconds: u64,
    /// Request timeout in seconds
    #[serde(default = "default_request_timeout")]
    pub request_timeout_seconds: u64,
}

impl Default for VaultConnectionConfig {
    fn default() -> Self {
        Self {
            max_connections: default_max_connections(),
            connection_timeout_seconds: default_connection_timeout(),
            request_timeout_seconds: default_request_timeout(),
        }
    }
}

/// Configuration for file-based secrets backend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileConfig {
    /// Path to the secrets file or directory
    pub path: PathBuf,
    /// File format for secrets storage
    #[serde(default = "default_file_format")]
    pub format: FileFormat,
    /// Encryption configuration
    #[serde(default)]
    pub encryption: FileEncryptionConfig,
    /// File permissions (Unix only)
    pub permissions: Option<u32>,
    /// Watch for file changes and reload
    #[serde(default)]
    pub watch_for_changes: bool,
    /// Backup configuration
    #[serde(default)]
    pub backup: FileBackupConfig,
}

/// Supported file formats for secrets storage
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileFormat {
    /// JSON format
    Json,
    /// YAML format
    Yaml,
    /// TOML format
    Toml,
    /// Plain text (key=value pairs)
    Env,
}

/// File encryption configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEncryptionConfig {
    /// Enable encryption of secrets file
    #[serde(default)]
    pub enabled: bool,
    /// Encryption algorithm
    #[serde(default = "default_encryption_algorithm")]
    pub algorithm: String,
    /// Key derivation function
    #[serde(default = "default_kdf")]
    pub kdf: String,
    /// Key provider configuration
    #[serde(default)]
    pub key: FileKeyConfig,
}

/// Configuration for encryption key retrieval
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileKeyConfig {
    /// Key provider type
    #[serde(default = "default_key_provider")]
    pub provider: String,
    /// Environment variable containing encryption key (for 'env' provider)
    pub env_var: Option<String>,
    /// Keychain service name (for 'os_keychain' provider)
    pub service: Option<String>,
    /// Keychain account name (for 'os_keychain' provider)
    pub account: Option<String>,
    /// Path to key file (for 'file' provider)
    pub file_path: Option<PathBuf>,
}

impl Default for FileKeyConfig {
    fn default() -> Self {
        Self {
            provider: default_key_provider(),
            env_var: None,
            service: None,
            account: None,
            file_path: None,
        }
    }
}

impl Default for FileEncryptionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            algorithm: default_encryption_algorithm(),
            kdf: default_kdf(),
            key: FileKeyConfig::default(),
        }
    }
}

/// File backup configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileBackupConfig {
    /// Enable automatic backups
    #[serde(default)]
    pub enabled: bool,
    /// Directory for backup files
    pub backup_dir: Option<PathBuf>,
    /// Maximum number of backup files to keep
    #[serde(default = "default_max_backups")]
    pub max_backups: usize,
    /// Create backup before modifications
    #[serde(default = "default_backup_before_write")]
    pub backup_before_write: bool,
}

impl Default for FileBackupConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            backup_dir: None,
            max_backups: default_max_backups(),
            backup_before_write: default_backup_before_write(),
        }
    }
}

// Default value functions
fn default_timeout() -> u64 { 30 }
fn default_max_retries() -> u32 { 3 }
fn default_enable_cache() -> bool { true }
fn default_cache_ttl() -> u64 { 300 }
fn default_vault_mount() -> String { "secret".to_string() }
fn default_vault_api_version() -> String { "v2".to_string() }
fn default_approle_mount() -> String { "approle".to_string() }
fn default_k8s_token_path() -> String { "/var/run/secrets/kubernetes.io/serviceaccount/token".to_string() }
fn default_k8s_mount() -> String { "kubernetes".to_string() }
fn default_aws_mount() -> String { "aws".to_string() }
fn default_max_connections() -> usize { 10 }
fn default_connection_timeout() -> u64 { 10 }
fn default_request_timeout() -> u64 { 30 }
fn default_file_format() -> FileFormat { FileFormat::Json }
fn default_encryption_algorithm() -> String { "AES-256-GCM".to_string() }
fn default_kdf() -> String { "PBKDF2".to_string() }
fn default_key_provider() -> String { "env".to_string() }
fn default_max_backups() -> usize { 5 }
fn default_backup_before_write() -> bool { true }

impl SecretsConfig {
    /// Create a Vault configuration with token authentication
    pub fn vault_with_token(url: String, token: String) -> Self {
        Self {
            backend: SecretsBackend::Vault(VaultConfig {
                url,
                auth: VaultAuthConfig::Token { token },
                namespace: None,
                mount_path: default_vault_mount(),
                api_version: default_vault_api_version(),
                tls: VaultTlsConfig::default(),
                connection: VaultConnectionConfig::default(),
            }),
            common: CommonSecretsConfig::default(),
        }
    }

    /// Create a file-based configuration with JSON format
    pub fn file_json(path: PathBuf) -> Self {
        Self {
            backend: SecretsBackend::File(FileConfig {
                path,
                format: FileFormat::Json,
                encryption: FileEncryptionConfig::default(),
                permissions: Some(0o600),
                watch_for_changes: false,
                backup: FileBackupConfig::default(),
            }),
            common: CommonSecretsConfig::default(),
        }
    }

    /// Get the backend type as a string
    pub fn backend_type(&self) -> &'static str {
        match &self.backend {
            SecretsBackend::Vault(_) => "vault",
            SecretsBackend::File(_) => "file",
        }
    }

    /// Get timeout as Duration
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.common.timeout_seconds)
    }

    /// Get cache TTL as Duration
    pub fn cache_ttl(&self) -> Duration {
        Duration::from_secs(self.common.cache_ttl_seconds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vault_config_creation() {
        let config = SecretsConfig::vault_with_token(
            "https://vault.example.com".to_string(),
            "hvs.token123".to_string(),
        );

        assert_eq!(config.backend_type(), "vault");
        if let SecretsBackend::Vault(vault_config) = &config.backend {
            assert_eq!(vault_config.url, "https://vault.example.com");
            if let VaultAuthConfig::Token { token } = &vault_config.auth {
                assert_eq!(token, "hvs.token123");
            } else {
                panic!("Expected token auth");
            }
        } else {
            panic!("Expected vault backend");
        }
    }

    #[test]
    fn test_file_config_creation() {
        let path = PathBuf::from("/etc/secrets/app.json");
        let config = SecretsConfig::file_json(path.clone());

        assert_eq!(config.backend_type(), "file");
        if let SecretsBackend::File(file_config) = &config.backend {
            assert_eq!(file_config.path, path);
            assert!(matches!(file_config.format, FileFormat::Json));
        } else {
            panic!("Expected file backend");
        }
    }

    #[test]
    fn test_common_config_defaults() {
        let config = CommonSecretsConfig::default();
        assert_eq!(config.timeout_seconds, 30);
        assert_eq!(config.max_retries, 3);
        assert!(config.enable_cache);
        assert_eq!(config.cache_ttl_seconds, 300);
    }

    #[test]
    fn test_timeout_conversion() {
        let config = SecretsConfig::file_json(PathBuf::from("/test"));
        assert_eq!(config.timeout(), Duration::from_secs(30));
        assert_eq!(config.cache_ttl(), Duration::from_secs(300));
    }
}