//! Configuration management module for Symbiont runtime
//!
//! Provides centralized configuration handling with validation, environment
//! variable abstraction, and secure defaults.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
    /// SLM-first configuration
    pub slm: Option<Slm>,
    /// Routing configuration
    pub routing: Option<crate::routing::RoutingConfig>,
    /// Native execution configuration (optional)
    pub native_execution: Option<NativeExecutionConfig>,
    /// AgentPin integration configuration (optional)
    pub agentpin: Option<crate::integrations::agentpin::AgentPinConfig>,
    /// CLI executor configuration (optional, requires `cli-executor` feature)
    #[cfg(feature = "cli-executor")]
    pub cli_executor: Option<CliExecutorConfigToml>,
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

/// Native execution configuration (non-isolated host execution)
/// ⚠️ WARNING: Use only in trusted development environments
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeExecutionConfig {
    /// Allow native execution without Docker/isolation
    pub enabled: bool,
    /// Default executable for native execution
    pub default_executable: String,
    /// Working directory for native execution
    pub working_directory: PathBuf,
    /// Enforce resource limits even in native mode
    pub enforce_resource_limits: bool,
    /// Maximum memory in MB
    pub max_memory_mb: Option<u64>,
    /// Maximum CPU time in seconds
    pub max_cpu_seconds: Option<u64>,
    /// Maximum execution time (timeout) in seconds
    pub max_execution_time_seconds: u64,
    /// Allowed executables (empty = all allowed)
    pub allowed_executables: Vec<String>,
}

impl Default for NativeExecutionConfig {
    fn default() -> Self {
        Self {
            enabled: false, // Disabled by default for safety
            default_executable: "bash".to_string(),
            working_directory: PathBuf::from("/tmp/symbiont-native"),
            enforce_resource_limits: true,
            max_memory_mb: Some(2048),
            max_cpu_seconds: Some(300),
            max_execution_time_seconds: 300,
            allowed_executables: vec![], // Empty — must be explicitly configured
        }
    }
}

/// CLI executor TOML configuration for AI CLI tool orchestration.
/// Gated behind the `cli-executor` feature.
#[cfg(feature = "cli-executor")]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CliExecutorConfigToml {
    /// Wall-clock timeout in seconds (default: 600).
    pub max_runtime_seconds: Option<u64>,
    /// Idle timeout in seconds — kill if no output for this long (default: 120).
    pub idle_timeout_seconds: Option<u64>,
    /// Maximum output bytes per stream (default: 10MB).
    pub max_output_bytes: Option<usize>,
    /// Per-adapter configurations keyed by adapter name.
    pub adapters: Option<HashMap<String, AdapterConfigToml>>,
}

/// Per-adapter TOML configuration.
#[cfg(feature = "cli-executor")]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AdapterConfigToml {
    /// Path or name of the executable.
    pub executable: Option<String>,
    /// Model override for this adapter.
    pub model: Option<String>,
    /// Maximum agentic turns (adapter-specific).
    pub max_turns: Option<u32>,
    /// Allowed tools list (adapter-specific).
    pub allowed_tools: Option<Vec<String>>,
    /// Disallowed tools list (adapter-specific).
    pub disallowed_tools: Option<Vec<String>>,
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

/// SLM-first configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Slm {
    /// Enable SLM-first mode globally
    pub enabled: bool,
    /// Model allow list configuration
    pub model_allow_lists: ModelAllowListConfig,
    /// Named sandbox profiles for different security tiers
    pub sandbox_profiles: HashMap<String, SandboxProfile>,
    /// Default sandbox profile name
    pub default_sandbox_profile: String,
}

/// Model allow list configuration with hierarchical overrides
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelAllowListConfig {
    /// Global model definitions available system-wide
    pub global_models: Vec<Model>,
    /// Agent-specific model mappings (agent_id -> model_ids)
    pub agent_model_maps: HashMap<String, Vec<String>>,
    /// Allow runtime API-based overrides
    pub allow_runtime_overrides: bool,
}

/// Individual model definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    /// Unique model identifier
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Model provider/source
    pub provider: ModelProvider,
    /// Model capabilities
    pub capabilities: Vec<ModelCapability>,
    /// Resource requirements for this model
    pub resource_requirements: ModelResourceRequirements,
}

/// Model provider configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ModelProvider {
    HuggingFace { model_path: String },
    LocalFile { file_path: PathBuf },
    OpenAI { model_name: String },
    Anthropic { model_name: String },
    Custom { endpoint_url: String },
}

/// Model capability enumeration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ModelCapability {
    TextGeneration,
    CodeGeneration,
    Reasoning,
    ToolUse,
    FunctionCalling,
    Embeddings,
}

/// Model resource requirements
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelResourceRequirements {
    /// Minimum memory required in MB
    pub min_memory_mb: u64,
    /// Preferred CPU cores
    pub preferred_cpu_cores: f32,
    /// GPU requirements
    pub gpu_requirements: Option<GpuRequirements>,
}

/// GPU requirements specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuRequirements {
    /// Minimum VRAM in MB
    pub min_vram_mb: u64,
    /// Required compute capability
    pub compute_capability: String,
}

/// Sandbox profile for SLM runners with comprehensive controls
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxProfile {
    /// Resource allocation and limits
    pub resources: ResourceConstraints,
    /// Filesystem access controls
    pub filesystem: FilesystemControls,
    /// Process execution limits
    pub process_limits: ProcessLimits,
    /// Network access policies
    pub network: NetworkPolicy,
    /// Security settings
    pub security: SecuritySettings,
}

/// Resource constraints for sandbox
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceConstraints {
    /// Maximum memory allocation in MB
    pub max_memory_mb: u64,
    /// Maximum CPU cores (fractional allowed, e.g., 1.5)
    pub max_cpu_cores: f32,
    /// Maximum disk space in MB
    pub max_disk_mb: u64,
    /// GPU access configuration
    pub gpu_access: GpuAccess,
    /// I/O bandwidth limits
    pub max_io_bandwidth_mbps: Option<u64>,
}

/// Filesystem access controls
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilesystemControls {
    /// Allowed read paths (glob patterns supported)
    pub read_paths: Vec<String>,
    /// Allowed write paths (glob patterns supported)
    pub write_paths: Vec<String>,
    /// Explicitly denied paths (takes precedence)
    pub denied_paths: Vec<String>,
    /// Allow temporary file creation
    pub allow_temp_files: bool,
    /// Maximum file size in MB
    pub max_file_size_mb: u64,
}

/// Process execution limits
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessLimits {
    /// Maximum number of child processes
    pub max_child_processes: u32,
    /// Maximum execution time in seconds
    pub max_execution_time_seconds: u64,
    /// Allowed system calls (seccomp filter)
    pub allowed_syscalls: Vec<String>,
    /// Process priority (nice value)
    pub process_priority: i8,
}

/// Network access policy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkPolicy {
    /// Network access mode
    pub access_mode: NetworkAccessMode,
    /// Allowed destinations (when mode is Restricted)
    pub allowed_destinations: Vec<NetworkDestination>,
    /// Maximum bandwidth in Mbps
    pub max_bandwidth_mbps: Option<u64>,
}

/// Network access mode enumeration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetworkAccessMode {
    /// No network access
    None,
    /// Restricted to specific destinations
    Restricted,
    /// Full network access
    Full,
}

/// Network destination specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkDestination {
    /// Host (can be IP or domain)
    pub host: String,
    /// Port (optional, defaults to any)
    pub port: Option<u16>,
    /// Protocol restriction
    pub protocol: Option<NetworkProtocol>,
}

/// Network protocol enumeration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetworkProtocol {
    TCP,
    UDP,
    HTTP,
    HTTPS,
}

/// GPU access configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GpuAccess {
    /// No GPU access
    None,
    /// Shared GPU access with memory limit
    Shared { max_memory_mb: u64 },
    /// Exclusive GPU access
    Exclusive,
}

/// Security settings for sandbox
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecuritySettings {
    /// Enable additional syscall filtering
    pub strict_syscall_filtering: bool,
    /// Disable debugging interfaces
    pub disable_debugging: bool,
    /// Enable audit logging
    pub enable_audit_logging: bool,
    /// Encryption requirements
    pub require_encryption: bool,
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

impl Default for Slm {
    fn default() -> Self {
        let mut profiles = HashMap::new();
        profiles.insert("secure".to_string(), SandboxProfile::secure_default());
        profiles.insert("standard".to_string(), SandboxProfile::standard_default());

        Self {
            enabled: false,
            model_allow_lists: ModelAllowListConfig::default(),
            sandbox_profiles: profiles,
            default_sandbox_profile: "secure".to_string(),
        }
    }
}

impl SandboxProfile {
    /// Create a secure default profile
    pub fn secure_default() -> Self {
        Self {
            resources: ResourceConstraints {
                max_memory_mb: 512,
                max_cpu_cores: 1.0,
                max_disk_mb: 100,
                gpu_access: GpuAccess::None,
                max_io_bandwidth_mbps: Some(10),
            },
            filesystem: FilesystemControls {
                read_paths: vec!["/tmp/sandbox/*".to_string()],
                write_paths: vec!["/tmp/sandbox/output/*".to_string()],
                denied_paths: vec!["/etc/*".to_string(), "/proc/*".to_string()],
                allow_temp_files: true,
                max_file_size_mb: 10,
            },
            process_limits: ProcessLimits {
                max_child_processes: 0,
                max_execution_time_seconds: 300,
                allowed_syscalls: vec!["read".to_string(), "write".to_string(), "open".to_string()],
                process_priority: 19,
            },
            network: NetworkPolicy {
                access_mode: NetworkAccessMode::None,
                allowed_destinations: vec![],
                max_bandwidth_mbps: None,
            },
            security: SecuritySettings {
                strict_syscall_filtering: true,
                disable_debugging: true,
                enable_audit_logging: true,
                require_encryption: true,
            },
        }
    }

    /// Create a standard default profile (less restrictive)
    pub fn standard_default() -> Self {
        Self {
            resources: ResourceConstraints {
                max_memory_mb: 1024,
                max_cpu_cores: 2.0,
                max_disk_mb: 500,
                gpu_access: GpuAccess::Shared {
                    max_memory_mb: 1024,
                },
                max_io_bandwidth_mbps: Some(50),
            },
            filesystem: FilesystemControls {
                read_paths: vec!["/tmp/*".to_string(), "/home/sandbox/*".to_string()],
                write_paths: vec!["/tmp/*".to_string(), "/home/sandbox/*".to_string()],
                denied_paths: vec!["/etc/passwd".to_string(), "/etc/shadow".to_string()],
                allow_temp_files: true,
                max_file_size_mb: 100,
            },
            process_limits: ProcessLimits {
                max_child_processes: 5,
                max_execution_time_seconds: 600,
                allowed_syscalls: vec![], // Empty means allow all
                process_priority: 0,
            },
            network: NetworkPolicy {
                access_mode: NetworkAccessMode::Restricted,
                allowed_destinations: vec![NetworkDestination {
                    host: "api.openai.com".to_string(),
                    port: Some(443),
                    protocol: Some(NetworkProtocol::HTTPS),
                }],
                max_bandwidth_mbps: Some(100),
            },
            security: SecuritySettings {
                strict_syscall_filtering: false,
                disable_debugging: false,
                enable_audit_logging: true,
                require_encryption: false,
            },
        }
    }

    /// Validate sandbox profile configuration
    pub fn validate(&self) -> Result<(), Box<dyn std::error::Error>> {
        // Validate resource constraints
        if self.resources.max_memory_mb == 0 {
            return Err("max_memory_mb must be > 0".into());
        }
        if self.resources.max_cpu_cores <= 0.0 {
            return Err("max_cpu_cores must be > 0".into());
        }

        // Validate filesystem paths
        for path in &self.filesystem.read_paths {
            if path.is_empty() {
                return Err("read_paths cannot contain empty strings".into());
            }
        }

        // Validate process limits
        if self.process_limits.max_execution_time_seconds == 0 {
            return Err("max_execution_time_seconds must be > 0".into());
        }

        Ok(())
    }
}

impl Slm {
    /// Validate the SLM configuration
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Validate default sandbox profile exists
        if !self
            .sandbox_profiles
            .contains_key(&self.default_sandbox_profile)
        {
            return Err(ConfigError::InvalidValue {
                key: "slm.default_sandbox_profile".to_string(),
                reason: format!(
                    "Profile '{}' not found in sandbox_profiles",
                    self.default_sandbox_profile
                ),
            });
        }

        // Validate model definitions have unique IDs
        let mut model_ids = std::collections::HashSet::new();
        for model in &self.model_allow_lists.global_models {
            if !model_ids.insert(&model.id) {
                return Err(ConfigError::InvalidValue {
                    key: "slm.model_allow_lists.global_models".to_string(),
                    reason: format!("Duplicate model ID: {}", model.id),
                });
            }
        }

        // Validate agent model mappings reference existing models
        for (agent_id, model_ids) in &self.model_allow_lists.agent_model_maps {
            for model_id in model_ids {
                if !self
                    .model_allow_lists
                    .global_models
                    .iter()
                    .any(|m| &m.id == model_id)
                {
                    return Err(ConfigError::InvalidValue {
                        key: format!("slm.model_allow_lists.agent_model_maps.{}", agent_id),
                        reason: format!("Model ID '{}' not found in global_models", model_id),
                    });
                }
            }
        }

        // Validate sandbox profiles
        for (profile_name, profile) in &self.sandbox_profiles {
            profile.validate().map_err(|e| ConfigError::InvalidValue {
                key: format!("slm.sandbox_profiles.{}", profile_name),
                reason: e.to_string(),
            })?;
        }

        Ok(())
    }

    /// Get allowed models for a specific agent
    pub fn get_allowed_models(&self, agent_id: &str) -> Vec<&Model> {
        // Check agent-specific mappings first
        if let Some(model_ids) = self.model_allow_lists.agent_model_maps.get(agent_id) {
            self.model_allow_lists
                .global_models
                .iter()
                .filter(|model| model_ids.contains(&model.id))
                .collect()
        } else {
            // Fall back to all global models if no specific mapping
            self.model_allow_lists.global_models.iter().collect()
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

        // Load and validate auth token if present
        if let Ok(token) = env::var("API_AUTH_TOKEN") {
            match Self::validate_auth_token(&token) {
                Ok(validated_token) => {
                    config.api.auth_token = Some(validated_token);
                }
                Err(e) => {
                    tracing::error!("Invalid API_AUTH_TOKEN: {}", e);
                    eprintln!("⚠️  ERROR: Invalid API_AUTH_TOKEN: {}", e);
                    // Don't set the token if it's invalid
                }
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
        let content = std::fs::read_to_string(path).map_err(|e| ConfigError::IoError {
            message: e.to_string(),
        })?;

        let config: Self = toml::from_str(&content).map_err(|e| ConfigError::ParseError {
            message: e.to_string(),
        })?;

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

        // Validate SLM configuration if enabled
        if let Some(slm) = &self.slm {
            if slm.enabled {
                slm.validate()?;
            }
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
            KeyProvider::File { path } => std::fs::read_to_string(path)
                .map(|s| s.trim().to_string())
                .map_err(|e| ConfigError::IoError {
                    message: e.to_string(),
                }),
            KeyProvider::Keychain { service, account } => {
                #[cfg(feature = "keychain")]
                {
                    use keyring::Entry;
                    let entry =
                        Entry::new(service, account).map_err(|e| ConfigError::EnvError {
                            message: e.to_string(),
                        })?;
                    entry.get_password().map_err(|e| ConfigError::EnvError {
                        message: e.to_string(),
                    })
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

    /// Validate an authentication token for security best practices
    ///
    /// Returns an error if the token:
    /// - Is empty or only whitespace
    /// - Is too short (< 8 characters)
    /// - Matches known weak/default tokens
    /// - Contains only whitespace
    ///
    /// Returns Ok(trimmed_token) if validation passes
    fn validate_auth_token(token: &str) -> Result<String, ConfigError> {
        // Trim whitespace
        let trimmed = token.trim();

        // Check if empty
        if trimmed.is_empty() {
            return Err(ConfigError::InvalidValue {
                key: "auth_token".to_string(),
                reason: "Token cannot be empty".to_string(),
            });
        }

        // Check for known weak/default tokens (case-insensitive) before length check
        // so that short weak tokens like "dev" get the correct error message
        let weak_tokens = [
            "dev",
            "test",
            "password",
            "secret",
            "token",
            "api_key",
            "12345678",
            "admin",
            "root",
            "default",
            "changeme",
            "letmein",
            "qwerty",
            "abc123",
            "password123",
        ];

        if weak_tokens.contains(&trimmed.to_lowercase().as_str()) {
            return Err(ConfigError::InvalidValue {
                key: "auth_token".to_string(),
                reason: format!(
                    "Token '{}' is a known weak/default token. Use a strong random token instead.",
                    trimmed
                ),
            });
        }

        // Check minimum length
        if trimmed.len() < 8 {
            return Err(ConfigError::InvalidValue {
                key: "auth_token".to_string(),
                reason: "Token must be at least 8 characters long".to_string(),
            });
        }

        // Warn if token appears to be weak (all same character, sequential, etc.)
        if trimmed
            .chars()
            .all(|c| c == trimmed.chars().next().unwrap())
        {
            tracing::warn!("⚠️  Auth token appears weak (all same character)");
        }

        // Check for potential secrets in token (bcrypt hashes, jwt tokens, etc. are OK)
        if trimmed.contains(' ') && !trimmed.starts_with("Bearer ") {
            return Err(ConfigError::InvalidValue {
                key: "auth_token".to_string(),
                reason: "Token should not contain spaces (unless it's a Bearer token)".to_string(),
            });
        }

        Ok(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::collections::HashMap;
    use std::env;
    use std::path::PathBuf;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.api.port, 8080);
        assert_eq!(config.api.host, "127.0.0.1");
        assert!(config.validate().is_ok());
    }

    #[test]
    #[serial]
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

    // SLM Configuration Tests
    #[test]
    fn test_slm_default_config() {
        let slm = Slm::default();
        assert!(!slm.enabled);
        assert_eq!(slm.default_sandbox_profile, "secure");
        assert!(slm.sandbox_profiles.contains_key("secure"));
        assert!(slm.sandbox_profiles.contains_key("standard"));
        assert!(slm.validate().is_ok());
    }

    #[test]
    fn test_slm_validation_invalid_default_profile() {
        let slm = Slm {
            default_sandbox_profile: "nonexistent".to_string(),
            ..Default::default()
        };

        let result = slm.validate();
        assert!(result.is_err());
        if let Err(ConfigError::InvalidValue { key, reason }) = result {
            assert_eq!(key, "slm.default_sandbox_profile");
            assert!(reason.contains("nonexistent"));
        }
    }

    #[test]
    fn test_slm_validation_duplicate_model_ids() {
        let model1 = Model {
            id: "duplicate".to_string(),
            name: "Model 1".to_string(),
            provider: ModelProvider::LocalFile {
                file_path: PathBuf::from("/tmp/model1.gguf"),
            },
            capabilities: vec![ModelCapability::TextGeneration],
            resource_requirements: ModelResourceRequirements {
                min_memory_mb: 512,
                preferred_cpu_cores: 1.0,
                gpu_requirements: None,
            },
        };

        let model2 = Model {
            id: "duplicate".to_string(), // Same ID
            name: "Model 2".to_string(),
            provider: ModelProvider::LocalFile {
                file_path: PathBuf::from("/tmp/model2.gguf"),
            },
            capabilities: vec![ModelCapability::CodeGeneration],
            resource_requirements: ModelResourceRequirements {
                min_memory_mb: 1024,
                preferred_cpu_cores: 2.0,
                gpu_requirements: None,
            },
        };

        let mut slm = Slm::default();
        slm.model_allow_lists.global_models = vec![model1, model2];

        let result = slm.validate();
        assert!(result.is_err());
        if let Err(ConfigError::InvalidValue { key, reason }) = result {
            assert_eq!(key, "slm.model_allow_lists.global_models");
            assert!(reason.contains("Duplicate model ID: duplicate"));
        }
    }

    #[test]
    fn test_slm_validation_invalid_agent_model_mapping() {
        let model = Model {
            id: "test_model".to_string(),
            name: "Test Model".to_string(),
            provider: ModelProvider::LocalFile {
                file_path: PathBuf::from("/tmp/test.gguf"),
            },
            capabilities: vec![ModelCapability::TextGeneration],
            resource_requirements: ModelResourceRequirements {
                min_memory_mb: 512,
                preferred_cpu_cores: 1.0,
                gpu_requirements: None,
            },
        };

        let mut slm = Slm::default();
        slm.model_allow_lists.global_models = vec![model];

        let mut agent_model_maps = HashMap::new();
        agent_model_maps.insert(
            "test_agent".to_string(),
            vec!["nonexistent_model".to_string()],
        );
        slm.model_allow_lists.agent_model_maps = agent_model_maps;

        let result = slm.validate();
        assert!(result.is_err());
        if let Err(ConfigError::InvalidValue { key, reason }) = result {
            assert_eq!(key, "slm.model_allow_lists.agent_model_maps.test_agent");
            assert!(reason.contains("Model ID 'nonexistent_model' not found"));
        }
    }

    #[test]
    fn test_slm_get_allowed_models_with_agent_mapping() {
        let model1 = Model {
            id: "model1".to_string(),
            name: "Model 1".to_string(),
            provider: ModelProvider::LocalFile {
                file_path: PathBuf::from("/tmp/model1.gguf"),
            },
            capabilities: vec![ModelCapability::TextGeneration],
            resource_requirements: ModelResourceRequirements {
                min_memory_mb: 512,
                preferred_cpu_cores: 1.0,
                gpu_requirements: None,
            },
        };

        let model2 = Model {
            id: "model2".to_string(),
            name: "Model 2".to_string(),
            provider: ModelProvider::LocalFile {
                file_path: PathBuf::from("/tmp/model2.gguf"),
            },
            capabilities: vec![ModelCapability::CodeGeneration],
            resource_requirements: ModelResourceRequirements {
                min_memory_mb: 1024,
                preferred_cpu_cores: 2.0,
                gpu_requirements: None,
            },
        };

        let mut slm = Slm::default();
        slm.model_allow_lists.global_models = vec![model1, model2];

        let mut agent_model_maps = HashMap::new();
        agent_model_maps.insert("agent1".to_string(), vec!["model1".to_string()]);
        slm.model_allow_lists.agent_model_maps = agent_model_maps;

        // Agent with specific mapping should only get their models
        let allowed_models = slm.get_allowed_models("agent1");
        assert_eq!(allowed_models.len(), 1);
        assert_eq!(allowed_models[0].id, "model1");

        // Agent without mapping should get all global models
        let allowed_models = slm.get_allowed_models("agent2");
        assert_eq!(allowed_models.len(), 2);
    }

    // Sandbox Profile Tests
    #[test]
    fn test_sandbox_profile_secure_default() {
        let profile = SandboxProfile::secure_default();
        assert_eq!(profile.resources.max_memory_mb, 512);
        assert_eq!(profile.resources.max_cpu_cores, 1.0);
        assert!(matches!(profile.resources.gpu_access, GpuAccess::None));
        assert!(matches!(
            profile.network.access_mode,
            NetworkAccessMode::None
        ));
        assert!(profile.security.strict_syscall_filtering);
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn test_sandbox_profile_standard_default() {
        let profile = SandboxProfile::standard_default();
        assert_eq!(profile.resources.max_memory_mb, 1024);
        assert_eq!(profile.resources.max_cpu_cores, 2.0);
        assert!(matches!(
            profile.resources.gpu_access,
            GpuAccess::Shared { .. }
        ));
        assert!(matches!(
            profile.network.access_mode,
            NetworkAccessMode::Restricted
        ));
        assert!(!profile.security.strict_syscall_filtering);
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn test_sandbox_profile_validation_zero_memory() {
        let mut profile = SandboxProfile::secure_default();
        profile.resources.max_memory_mb = 0;

        let result = profile.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("max_memory_mb must be > 0"));
    }

    #[test]
    fn test_sandbox_profile_validation_zero_cpu() {
        let mut profile = SandboxProfile::secure_default();
        profile.resources.max_cpu_cores = 0.0;

        let result = profile.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("max_cpu_cores must be > 0"));
    }

    #[test]
    fn test_sandbox_profile_validation_empty_read_path() {
        let mut profile = SandboxProfile::secure_default();
        profile.filesystem.read_paths.push("".to_string());

        let result = profile.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("read_paths cannot contain empty strings"));
    }

    #[test]
    fn test_sandbox_profile_validation_zero_execution_time() {
        let mut profile = SandboxProfile::secure_default();
        profile.process_limits.max_execution_time_seconds = 0;

        let result = profile.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("max_execution_time_seconds must be > 0"));
    }

    // Model Configuration Tests
    #[test]
    fn test_model_provider_variants() {
        let huggingface_model = Model {
            id: "hf_model".to_string(),
            name: "HuggingFace Model".to_string(),
            provider: ModelProvider::HuggingFace {
                model_path: "microsoft/DialoGPT-medium".to_string(),
            },
            capabilities: vec![ModelCapability::TextGeneration],
            resource_requirements: ModelResourceRequirements {
                min_memory_mb: 512,
                preferred_cpu_cores: 1.0,
                gpu_requirements: None,
            },
        };

        let openai_model = Model {
            id: "openai_model".to_string(),
            name: "OpenAI Model".to_string(),
            provider: ModelProvider::OpenAI {
                model_name: "gpt-3.5-turbo".to_string(),
            },
            capabilities: vec![ModelCapability::TextGeneration, ModelCapability::Reasoning],
            resource_requirements: ModelResourceRequirements {
                min_memory_mb: 0, // Cloud model
                preferred_cpu_cores: 0.0,
                gpu_requirements: None,
            },
        };

        assert_eq!(huggingface_model.id, "hf_model");
        assert_eq!(openai_model.id, "openai_model");
    }

    #[test]
    fn test_model_capabilities() {
        let all_capabilities = vec![
            ModelCapability::TextGeneration,
            ModelCapability::CodeGeneration,
            ModelCapability::Reasoning,
            ModelCapability::ToolUse,
            ModelCapability::FunctionCalling,
            ModelCapability::Embeddings,
        ];

        let model = Model {
            id: "full_model".to_string(),
            name: "Full Capability Model".to_string(),
            provider: ModelProvider::LocalFile {
                file_path: PathBuf::from("/tmp/full.gguf"),
            },
            capabilities: all_capabilities.clone(),
            resource_requirements: ModelResourceRequirements {
                min_memory_mb: 2048,
                preferred_cpu_cores: 4.0,
                gpu_requirements: Some(GpuRequirements {
                    min_vram_mb: 8192,
                    compute_capability: "7.5".to_string(),
                }),
            },
        };

        assert_eq!(model.capabilities.len(), 6);
        for capability in &all_capabilities {
            assert!(model.capabilities.contains(capability));
        }
    }

    // Configuration File Tests
    #[test]
    fn test_config_validation_vector_dimension() {
        let mut config = Config::default();
        config.database.vector_dimension = 0;

        let result = config.validate();
        assert!(result.is_err());
        if let Err(ConfigError::InvalidValue { key, reason }) = result {
            assert_eq!(key, "database.vector_dimension");
            assert!(reason.contains("Vector dimension must be > 0"));
        }
    }

    #[test]
    fn test_config_validation_with_slm() {
        let mut config = Config::default();
        let slm = Slm {
            enabled: true,
            default_sandbox_profile: "invalid".to_string(), // This should cause validation to fail
            ..Default::default()
        };
        config.slm = Some(slm);

        let result = config.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_config_secret_key_retrieval() {
        // Test environment variable key provider
        env::set_var("TEST_SECRET_KEY", "test_secret_123");

        let mut config = Config::default();
        config.security.key_provider = KeyProvider::Environment {
            var_name: "TEST_SECRET_KEY".to_string(),
        };

        let key = config.get_secret_key();
        assert!(key.is_ok());
        assert_eq!(key.unwrap(), "test_secret_123");

        env::remove_var("TEST_SECRET_KEY");
    }

    #[test]
    fn test_config_secret_key_missing() {
        let mut config = Config::default();
        config.security.key_provider = KeyProvider::Environment {
            var_name: "NONEXISTENT_KEY".to_string(),
        };

        let result = config.get_secret_key();
        assert!(result.is_err());
        if let Err(ConfigError::MissingRequired { key }) = result {
            assert_eq!(key, "NONEXISTENT_KEY");
        }
    }

    #[test]
    fn test_network_policy_configurations() {
        // Test restricted network access
        let destination = NetworkDestination {
            host: "api.openai.com".to_string(),
            port: Some(443),
            protocol: Some(NetworkProtocol::HTTPS),
        };

        let network_policy = NetworkPolicy {
            access_mode: NetworkAccessMode::Restricted,
            allowed_destinations: vec![destination],
            max_bandwidth_mbps: Some(100),
        };

        let profile = SandboxProfile {
            resources: ResourceConstraints {
                max_memory_mb: 1024,
                max_cpu_cores: 2.0,
                max_disk_mb: 500,
                gpu_access: GpuAccess::None,
                max_io_bandwidth_mbps: Some(50),
            },
            filesystem: FilesystemControls {
                read_paths: vec!["/tmp/*".to_string()],
                write_paths: vec!["/tmp/output/*".to_string()],
                denied_paths: vec!["/etc/*".to_string()],
                allow_temp_files: true,
                max_file_size_mb: 10,
            },
            process_limits: ProcessLimits {
                max_child_processes: 2,
                max_execution_time_seconds: 300,
                allowed_syscalls: vec!["read".to_string(), "write".to_string()],
                process_priority: 0,
            },
            network: network_policy,
            security: SecuritySettings {
                strict_syscall_filtering: true,
                disable_debugging: true,
                enable_audit_logging: true,
                require_encryption: false,
            },
        };

        assert!(profile.validate().is_ok());
        assert!(matches!(
            profile.network.access_mode,
            NetworkAccessMode::Restricted
        ));
        assert_eq!(profile.network.allowed_destinations.len(), 1);
        assert_eq!(
            profile.network.allowed_destinations[0].host,
            "api.openai.com"
        );
    }

    #[test]
    fn test_gpu_requirements_configurations() {
        let gpu_requirements = GpuRequirements {
            min_vram_mb: 4096,
            compute_capability: "8.0".to_string(),
        };

        let model = Model {
            id: "gpu_model".to_string(),
            name: "GPU Model".to_string(),
            provider: ModelProvider::LocalFile {
                file_path: PathBuf::from("/tmp/gpu.gguf"),
            },
            capabilities: vec![ModelCapability::TextGeneration],
            resource_requirements: ModelResourceRequirements {
                min_memory_mb: 1024,
                preferred_cpu_cores: 2.0,
                gpu_requirements: Some(gpu_requirements),
            },
        };

        assert!(model.resource_requirements.gpu_requirements.is_some());
        let gpu_req = model.resource_requirements.gpu_requirements.unwrap();
        assert_eq!(gpu_req.min_vram_mb, 4096);
        assert_eq!(gpu_req.compute_capability, "8.0");
    }

    #[test]
    #[serial]
    fn test_config_from_env_invalid_port() {
        env::set_var("API_PORT", "invalid");

        let result = Config::from_env();
        assert!(result.is_err());
        if let Err(ConfigError::InvalidValue { key, reason }) = result {
            assert_eq!(key, "API_PORT");
            assert!(reason.contains("Invalid port number"));
        }

        env::remove_var("API_PORT");
    }

    #[test]
    fn test_api_auth_token_missing() {
        let config = Config::default();

        let result = config.get_api_auth_token();
        assert!(result.is_err());
        if let Err(ConfigError::MissingRequired { key }) = result {
            assert_eq!(key, "API_AUTH_TOKEN");
        }
    }

    #[test]
    fn test_database_url_missing() {
        let config = Config::default();

        let result = config.get_database_url();
        assert!(result.is_err());
        if let Err(ConfigError::MissingRequired { key }) = result {
            assert_eq!(key, "DATABASE_URL");
        }
    }

    // ============================================================================
    // Security Tests for Token Validation
    // ============================================================================

    #[test]
    fn test_validate_auth_token_valid_strong_token() {
        let tokens = vec![
            "MySecureToken123",
            "a1b2c3d4e5f6g7h8",
            "production_token_2024",
            "Bearer_abc123def456",
        ];

        for token in tokens {
            let result = Config::validate_auth_token(token);
            assert!(result.is_ok(), "Token '{}' should be valid", token);
            assert_eq!(result.unwrap(), token.trim());
        }
    }

    #[test]
    fn test_validate_auth_token_empty() {
        assert!(Config::validate_auth_token("").is_err());
        assert!(Config::validate_auth_token("   ").is_err());
        assert!(Config::validate_auth_token("\t\n").is_err());
    }

    #[test]
    fn test_validate_auth_token_too_short() {
        let short_tokens = vec!["abc", "12345", "short", "1234567"];

        for token in short_tokens {
            let result = Config::validate_auth_token(token);
            assert!(
                result.is_err(),
                "Token '{}' should be rejected (too short)",
                token
            );

            if let Err(ConfigError::InvalidValue { reason, .. }) = result {
                assert!(reason.contains("at least 8 characters"));
            }
        }
    }

    #[test]
    fn test_validate_auth_token_weak_defaults() {
        let weak_tokens = vec![
            "dev", "test", "password", "secret", "token", "admin", "root", "default", "changeme",
            "12345678",
        ];

        for token in weak_tokens {
            let result = Config::validate_auth_token(token);
            assert!(result.is_err(), "Weak token '{}' should be rejected", token);

            if let Err(ConfigError::InvalidValue { reason, .. }) = result {
                assert!(
                    reason.contains("weak/default token"),
                    "Expected 'weak/default token' message for '{}', got: {}",
                    token,
                    reason
                );
            }
        }
    }

    #[test]
    fn test_validate_auth_token_case_insensitive_weak_check() {
        let tokens = vec!["DEV", "Test", "PASSWORD", "Admin", "ROOT"];

        for token in tokens {
            let result = Config::validate_auth_token(token);
            assert!(
                result.is_err(),
                "Token '{}' should be rejected (case-insensitive)",
                token
            );
        }
    }

    #[test]
    fn test_validate_auth_token_with_spaces() {
        // Spaces should be rejected unless it's a Bearer token
        let result = Config::validate_auth_token("my token here");
        assert!(result.is_err());

        if let Err(ConfigError::InvalidValue { reason, .. }) = result {
            assert!(reason.contains("should not contain spaces"));
        }
    }

    #[test]
    fn test_validate_auth_token_trims_whitespace() {
        let result = Config::validate_auth_token("  validtoken123  ");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "validtoken123");
    }

    #[test]
    fn test_validate_auth_token_minimum_length_boundary() {
        // Exactly 8 characters should pass
        assert!(Config::validate_auth_token("12345678").is_err()); // Weak token
        assert!(Config::validate_auth_token("abcdefgh").is_ok());

        // 7 characters should fail
        assert!(Config::validate_auth_token("abcdefg").is_err());
    }

    #[test]
    #[serial]
    fn test_validate_auth_token_integration_with_from_env() {
        // Test that validation is called when loading from environment

        // Set a weak token
        env::set_var("API_AUTH_TOKEN", "dev");
        let config = Config::from_env().unwrap();
        // Token should be rejected, so it shouldn't be set
        assert!(config.api.auth_token.is_none());
        env::remove_var("API_AUTH_TOKEN");

        // Set a strong token
        env::set_var("API_AUTH_TOKEN", "strong_secure_token_12345");
        let config = Config::from_env().unwrap();
        assert!(config.api.auth_token.is_some());
        assert_eq!(config.api.auth_token.unwrap(), "strong_secure_token_12345");
        env::remove_var("API_AUTH_TOKEN");
    }

    #[test]
    fn test_validate_auth_token_special_characters_allowed() {
        let tokens = vec![
            "token-with-dashes",
            "token_with_underscores",
            "token.with.dots",
            "token@with#special$chars",
        ];

        for token in tokens {
            let result = Config::validate_auth_token(token);
            assert!(
                result.is_ok(),
                "Token '{}' with special chars should be valid",
                token
            );
        }
    }
}
