//! Configuration management module for Symbiont runtime
//!
//! Provides centralized configuration handling with validation, environment
//! variable abstraction, and secure defaults.

use serde::{Deserialize, Serialize};
use std::env;
use std::path::PathBuf;
use thiserror::Error;

/// Configuration errors
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Missing required configuration: {key}")]
    MissingRequired { key: String },
    
    #[error("Invalid configuration value for {key}: {reason}")]
    InvalidValue { key: String, reason: String },
    
    #[error("Environment variable error: {message}")]
    EnvError { message: String },
    
    #[error("IO error reading config file: {message}")]
    IoError { message: String },
    
    #[error("Configuration parsing error: {message}")]
    ParseError { message: String },
}

/// Main application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// API configuration
    pub api: ApiConfig,
    /// Database configuration
    pub database: DatabaseConfig,
    /// Logging configuration
    pub logging: LoggingConfig,
    /// Security configuration
    pub security: SecurityConfig,
    /// Storage configuration
    pub storage: StorageConfig,
}

/// API configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    /// API server port
    pub port: u16,
    /// API server host
    pub host: String,
    /// API authentication token (securely handled)
    #[serde(skip_serializing)]
    pub auth_token: Option<String>,
    /// Request timeout in seconds
    pub timeout_seconds: u64,
    /// Maximum request body size in bytes
    pub max_body_size: usize,
}

/// Database configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// PostgreSQL connection URL
    #[serde(skip_serializing)]
    pub url: Option<String>,
    /// Redis connection URL
    #[serde(skip_serializing)]
    pub redis_url: Option<String>,
    /// Qdrant vector database URL
    pub qdrant_url: String,
    /// Qdrant collection name
    pub qdrant_collection: String,
    /// Vector dimension
    pub vector_dimension: usize,
}

/// Logging configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Log level
    pub level: String,
    /// Log format
    pub format: LogFormat,
    /// Enable structured logging
    pub structured: bool,
}

/// Log format options
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LogFormat {
    Json,
    Pretty,
    Compact,
}

/// Security configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    /// Encryption key provider
    pub key_provider: KeyProvider,
    /// Enable/disable features
    pub enable_compression: bool,
    pub enable_backups: bool,
    pub enable_safety_checks: bool,
}

/// Key provider configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KeyProvider {
    Environment { var_name: String },
    File { path: PathBuf },
    Keychain { service: String, account: String },
}

/// Storage configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Context storage path
    pub context_path: PathBuf,
    /// Git clone base path
    pub git_clone_path: PathBuf,
    /// Backup directory
    pub backup_path: PathBuf,
    /// Maximum context size in MB
    pub max_context_size_mb: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api: ApiConfig::default(),
            database: DatabaseConfig::default(),
            logging: LoggingConfig::default(),
            security: SecurityConfig::default(),
            storage: StorageConfig::default(),
        }
    }
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            port: 8080,
            host: "127.0.0.1".to_string(),
            auth_token: None,
            timeout_seconds: 60,
            max_body_size: 16 * 1024 * 1024, // 16MB
        }
    }
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: None,
            redis_url: None,
            qdrant_url: "http://localhost:6333".to_string(),
            qdrant_collection: "agent_knowledge".to_string(),
            vector_dimension: 1536,
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            format: LogFormat::Pretty,
            structured: false,
        }
    }
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            key_provider: KeyProvider::Environment {
                var_name: "SYMBIONT_SECRET_KEY".to_string(),
            },
            enable_compression: true,
            enable_backups: true,
            enable_safety_checks: true,
        }
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            context_path: PathBuf::from("./agent_storage"),
            git_clone_path: PathBuf::from("./temp_repos"),
            backup_path: PathBuf::from("./backups"),
            max_context_size_mb: 100,
        }
    }
}

impl Config {
    /// Load configuration from environment variables and defaults
    pub fn from_env() -> Result<Self, ConfigError> {
        let mut config = Self::default();
        
        // Load API configuration
        if let Ok(port) = env::var("API_PORT") {
            config.api.port = port.parse().map_err(|_| ConfigError::InvalidValue {
                key: "API_PORT".to_string(),
                reason: "Invalid port number".to_string(),
            })?;
        }
        
        if let Ok(host) = env::var("API_HOST") {
            config.api.host = host;
        }
        
        // Load auth token if present
        if let Ok(token) = env::var("API_AUTH_TOKEN") {
            if !token.is_empty() {
                config.api.auth_token = Some(token);
            }
        }
        
        // Load database configuration
        if let Ok(db_url) = env::var("DATABASE_URL") {
            config.database.url = Some(db_url);
        }
        
        if let Ok(redis_url) = env::var("REDIS_URL") {
            config.database.redis_url = Some(redis_url);
        }
        
        if let Ok(qdrant_url) = env::var("QDRANT_URL") {
            config.database.qdrant_url = qdrant_url;
        }
        
        // Load logging configuration
        if let Ok(log_level) = env::var("LOG_LEVEL") {
            config.logging.level = log_level;
        }
        
        // Load security configuration
        if let Ok(key_var) = env::var("SYMBIONT_SECRET_KEY_VAR") {
            config.security.key_provider = KeyProvider::Environment { var_name: key_var };
        }
        
        // Load storage configuration
        if let Ok(context_path) = env::var("CONTEXT_STORAGE_PATH") {
            config.storage.context_path = PathBuf::from(context_path);
        }
        
        if let Ok(git_path) = env::var("GIT_CLONE_BASE_PATH") {
            config.storage.git_clone_path = PathBuf::from(git_path);
        }
        
        if let Ok(backup_path) = env::var("BACKUP_DIRECTORY") {
            config.storage.backup_path = PathBuf::from(backup_path);
        }
        
        Ok(config)
    }
    
    /// Load configuration from file
    pub fn from_file<P: AsRef<std::path::Path>>(path: P) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| ConfigError::IoError { message: e.to_string() })?;
        
        let config: Self = toml::from_str(&content)
            .map_err(|e| ConfigError::ParseError { message: e.to_string() })?;
        
        Ok(config)
    }
    
    /// Validate configuration
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Validate port range
        if self.api.port == 0 {
            return Err(ConfigError::InvalidValue {
                key: "api.port".to_string(),
                reason: "Port cannot be 0".to_string(),
            });
        }
        
        // Validate log level
        let valid_levels = ["error", "warn", "info", "debug", "trace"];
        if !valid_levels.contains(&self.logging.level.as_str()) {
            return Err(ConfigError::InvalidValue {
                key: "logging.level".to_string(),
                reason: format!("Must be one of: {}", valid_levels.join(", ")),
            });
        }
        
        // Validate vector dimension
        if self.database.vector_dimension == 0 {
            return Err(ConfigError::InvalidValue {
                key: "database.vector_dimension".to_string(),
                reason: "Vector dimension must be > 0".to_string(),
            });
        }
        
        Ok(())
    }
    
    /// Get API auth token securely
    pub fn get_api_auth_token(&self) -> Result<String, ConfigError> {
        match &self.api.auth_token {
            Some(token) => Ok(token.clone()),
            None => Err(ConfigError::MissingRequired {
                key: "API_AUTH_TOKEN".to_string(),
            }),
        }
    }
    
    /// Get database URL securely
    pub fn get_database_url(&self) -> Result<String, ConfigError> {
        match &self.database.url {
            Some(url) => Ok(url.clone()),
            None => Err(ConfigError::MissingRequired {
                key: "DATABASE_URL".to_string(),
            }),
        }
    }
    
    /// Get secret key based on provider configuration
    pub fn get_secret_key(&self) -> Result<String, ConfigError> {
        match &self.security.key_provider {
            KeyProvider::Environment { var_name } => {
                env::var(var_name).map_err(|_| ConfigError::MissingRequired {
                    key: var_name.clone(),
                })
            }
            KeyProvider::File { path } => {
                std::fs::read_to_string(path)
                    .map(|s| s.trim().to_string())
                    .map_err(|e| ConfigError::IoError { message: e.to_string() })
            }
            KeyProvider::Keychain { service, account } => {
                #[cfg(feature = "keychain")]
                {
                    use keyring::Entry;
                    let entry = Entry::new(service, account)
                        .map_err(|e| ConfigError::EnvError { message: e.to_string() })?;
                    entry.get_password()
                        .map_err(|e| ConfigError::EnvError { message: e.to_string() })
                }
                #[cfg(not(feature = "keychain"))]
                {
                    Err(ConfigError::EnvError {
                        message: "Keychain support not enabled".to_string(),
                    })
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    
    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.api.port, 8080);
        assert_eq!(config.api.host, "127.0.0.1");
        assert!(config.validate().is_ok());
    }
    
    #[test]
    fn test_config_from_env() {
        env::set_var("API_PORT", "9090");
        env::set_var("API_HOST", "0.0.0.0");
        env::set_var("LOG_LEVEL", "debug");
        
        let config = Config::from_env().unwrap();
        assert_eq!(config.api.port, 9090);
        assert_eq!(config.api.host, "0.0.0.0");
        assert_eq!(config.logging.level, "debug");
        
        // Cleanup
        env::remove_var("API_PORT");
        env::remove_var("API_HOST");
        env::remove_var("LOG_LEVEL");
    }
    
    #[test]
    fn test_invalid_port() {
        let mut config = Config::default();
        config.api.port = 0;
        assert!(config.validate().is_err());
    }
    
    #[test]
    fn test_invalid_log_level() {
        let mut config = Config::default();
        config.logging.level = "invalid".to_string();
        assert!(config.validate().is_err());
    }
}