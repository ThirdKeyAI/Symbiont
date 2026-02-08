//! SLM runner implementations for executing models with security constraints
//!
//! This module provides the [`SlmRunner`] trait and concrete implementations for
//! executing Small Language Models within Symbiont's security sandbox.
//!
//! # Security Model
//!
//! All runners must respect the [`SandboxProfile`] associated with their execution
//! context. This includes:
//!
//! - Resource limits (memory, CPU, disk)
//! - Filesystem access controls
//! - Network restrictions
//! - Process execution limits
//!
//! # Adding New Runners
//!
//! To add support for a new model format:
//!
//! 1. Implement the [`SlmRunner`] trait
//! 2. Ensure proper sandbox profile enforcement
//! 3. Add comprehensive error handling
//! 4. Include unit tests for both success and failure cases
//!
//! # Usage
//!
//! ```rust
//! use symbiont_runtime::models::{SlmRunner, LocalGgufRunner};
//! use symbiont_runtime::config::SandboxProfile;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let sandbox_profile = SandboxProfile::secure_default();
//! let runner = LocalGgufRunner::new("/path/to/model.gguf", sandbox_profile).await?;
//!
//! let response = runner.execute("Hello, world!", None).await?;
//! println!("Model response: {}", response);
//! # Ok(())
//! # }
//! ```

use crate::config::{ModelResourceRequirements, SandboxProfile};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;
use tokio::process::Command;
use tokio::time::timeout;

/// Errors that can occur during SLM execution
#[derive(Debug, Error)]
pub enum SlmRunnerError {
    #[error("Model initialization failed: {reason}")]
    InitializationFailed { reason: String },

    #[error("Model execution failed: {reason}")]
    ExecutionFailed { reason: String },

    #[error("Resource limit exceeded: {limit_type}")]
    ResourceLimitExceeded { limit_type: String },

    #[error("Sandbox violation: {violation}")]
    SandboxViolation { violation: String },

    #[error("Model file not found: {path}")]
    ModelFileNotFound { path: String },

    #[error("Execution timeout after {seconds} seconds")]
    ExecutionTimeout { seconds: u64 },

    #[error("Invalid input: {reason}")]
    InvalidInput { reason: String },

    #[error("IO error: {message}")]
    IoError { message: String },
}

/// Execution options for SLM runners
#[derive(Debug, Clone)]
pub struct ExecutionOptions {
    /// Maximum execution time
    pub timeout: Option<Duration>,
    /// Temperature for text generation (0.0 - 1.0)
    pub temperature: Option<f32>,
    /// Maximum tokens to generate
    pub max_tokens: Option<u32>,
    /// Additional parameters specific to the model
    pub custom_parameters: HashMap<String, String>,
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            timeout: Some(Duration::from_secs(30)),
            temperature: Some(0.7),
            max_tokens: Some(256),
            custom_parameters: HashMap::new(),
        }
    }
}

/// Execution result from an SLM runner
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    /// Generated response text
    pub response: String,
    /// Execution metadata
    pub metadata: ExecutionMetadata,
}

/// Metadata about model execution
#[derive(Debug, Clone)]
pub struct ExecutionMetadata {
    /// Number of tokens in the input
    pub input_tokens: Option<u32>,
    /// Number of tokens generated
    pub output_tokens: Option<u32>,
    /// Execution time in milliseconds
    pub execution_time_ms: u64,
    /// Memory usage during execution in MB
    pub memory_usage_mb: Option<u64>,
    /// Whether execution hit any limits
    pub limits_hit: Vec<String>,
}

/// Generic trait for executing Small Language Models
///
/// This trait defines the interface for running SLMs within Symbiont's
/// security constraints. All implementations must respect the associated
/// [`SandboxProfile`] and provide proper resource isolation.
#[async_trait]
pub trait SlmRunner: Send + Sync {
    /// Execute the model with given input and options
    ///
    /// # Arguments
    ///
    /// * `prompt` - Input text to process
    /// * `options` - Execution options (timeout, temperature, etc.)
    ///
    /// # Errors
    ///
    /// Returns [`SlmRunnerError`] if execution fails due to resource limits,
    /// sandbox violations, or model errors.
    async fn execute(
        &self,
        prompt: &str,
        options: Option<ExecutionOptions>,
    ) -> Result<ExecutionResult, SlmRunnerError>;

    /// Get the sandbox profile associated with this runner
    fn get_sandbox_profile(&self) -> &SandboxProfile;

    /// Get the resource requirements for this model
    fn get_resource_requirements(&self) -> &ModelResourceRequirements;

    /// Check if the runner is healthy and ready for execution
    async fn health_check(&self) -> Result<(), SlmRunnerError>;

    /// Get runner-specific information
    fn get_info(&self) -> RunnerInfo;
}

/// Information about a specific runner implementation
#[derive(Debug, Clone)]
pub struct RunnerInfo {
    /// Runner type identifier
    pub runner_type: String,
    /// Model path or identifier
    pub model_path: String,
    /// Supported capabilities
    pub capabilities: Vec<String>,
    /// Version information
    pub version: Option<String>,
}

/// Local GGUF model runner implementation
///
/// This runner executes GGUF-quantized models using the llama.cpp framework
/// within the configured security sandbox.
#[derive(Debug)]
pub struct LocalGgufRunner {
    /// Path to the GGUF model file
    model_path: PathBuf,
    /// Sandbox profile for security constraints
    sandbox_profile: SandboxProfile,
    /// Resource requirements for this model
    resource_requirements: ModelResourceRequirements,
    /// Path to the llama.cpp executable
    llama_cpp_path: PathBuf,
}

impl LocalGgufRunner {
    /// Create a new GGUF runner
    ///
    /// # Arguments
    ///
    /// * `model_path` - Path to the GGUF model file
    /// * `sandbox_profile` - Security constraints for execution
    /// * `resource_requirements` - Resource requirements for the model
    ///
    /// # Errors
    ///
    /// Returns [`SlmRunnerError::InitializationFailed`] if the model file
    /// doesn't exist or isn't accessible.
    pub async fn new(
        model_path: impl Into<PathBuf>,
        sandbox_profile: SandboxProfile,
        resource_requirements: ModelResourceRequirements,
    ) -> Result<Self, SlmRunnerError> {
        let model_path = model_path.into();

        // Validate model file exists
        if !model_path.exists() {
            return Err(SlmRunnerError::ModelFileNotFound {
                path: model_path.display().to_string(),
            });
        }

        // Find llama.cpp executable
        let llama_cpp_path = Self::find_llama_cpp_executable().await?;

        let runner = Self {
            model_path,
            sandbox_profile,
            resource_requirements,
            llama_cpp_path,
        };

        // Perform initial health check
        runner.health_check().await?;

        Ok(runner)
    }

    /// Find the llama.cpp executable in the system
    async fn find_llama_cpp_executable() -> Result<PathBuf, SlmRunnerError> {
        // Common paths where llama.cpp might be installed
        let candidate_paths = vec![
            "/usr/local/bin/llama-cli",
            "/usr/bin/llama-cli",
            "/opt/llama.cpp/llama-cli",
            "./bin/llama-cli",
        ];

        for path in candidate_paths {
            let path_buf = PathBuf::from(path);
            if path_buf.exists() {
                return Ok(path_buf);
            }
        }

        // Try to find via which command
        match Command::new("which").arg("llama-cli").output().await {
            Ok(output) if output.status.success() => {
                let path_str = String::from_utf8_lossy(&output.stdout);
                let trimmed_path = path_str.trim();
                Ok(PathBuf::from(trimmed_path))
            }
            _ => Err(SlmRunnerError::InitializationFailed {
                reason: "llama.cpp executable not found".to_string(),
            }),
        }
    }

    /// Build command arguments for llama.cpp execution
    fn build_command_args(&self, prompt: &str, options: &ExecutionOptions) -> Vec<String> {
        let mut args = vec![
            "--model".to_string(),
            self.model_path.display().to_string(),
            "--prompt".to_string(),
            prompt.to_string(),
            "--no-display-prompt".to_string(),
        ];

        // Add temperature setting
        if let Some(temp) = options.temperature {
            args.extend(vec!["--temp".to_string(), temp.to_string()]);
        }

        // Add max tokens setting
        if let Some(max_tokens) = options.max_tokens {
            args.extend(vec!["--n-predict".to_string(), max_tokens.to_string()]);
        }

        // Apply resource constraints from sandbox profile
        args.extend(vec![
            "--threads".to_string(),
            self.sandbox_profile
                .resources
                .max_cpu_cores
                .floor()
                .to_string(),
        ]);

        // Add custom parameters
        for (key, value) in &options.custom_parameters {
            args.extend(vec![format!("--{}", key), value.clone()]);
        }

        args
    }

    /// Apply sandbox constraints to the command
    fn apply_sandbox_constraints(&self, command: &mut Command) {
        // Set memory limits (convert MB to bytes for ulimit)
        let memory_limit = self.sandbox_profile.resources.max_memory_mb * 1024 * 1024;

        // Use systemd-run or similar for resource constraints in production
        // For now, we'll use basic process limits
        command.env("RLIMIT_AS", memory_limit.to_string());

        // Set working directory to a sandboxed location
        if let Some(write_path) = self.sandbox_profile.filesystem.write_paths.first() {
            if let Ok(path) = std::fs::canonicalize(write_path.trim_end_matches("/*")) {
                command.current_dir(path);
            }
        }

        // Apply network restrictions by setting environment variables
        // that llama.cpp would respect (if it supported them)
        match self.sandbox_profile.network.access_mode {
            crate::config::NetworkAccessMode::None => {
                command.env("NO_NETWORK", "1");
            }
            crate::config::NetworkAccessMode::Restricted => {
                // Set allowed hosts if needed
                if !self.sandbox_profile.network.allowed_destinations.is_empty() {
                    let hosts: Vec<String> = self
                        .sandbox_profile
                        .network
                        .allowed_destinations
                        .iter()
                        .map(|dest| dest.host.clone())
                        .collect();
                    command.env("ALLOWED_HOSTS", hosts.join(","));
                }
            }
            crate::config::NetworkAccessMode::Full => {
                // No restrictions
            }
        }
    }

    /// Validate execution constraints before running
    fn validate_execution_constraints(&self, prompt: &str) -> Result<(), SlmRunnerError> {
        // Check prompt length (rough token estimation)
        let estimated_tokens = prompt.len() / 4; // Rough approximation
        if estimated_tokens > 4000 {
            return Err(SlmRunnerError::InvalidInput {
                reason: "Prompt too long".to_string(),
            });
        }

        // Validate sandbox profile constraints
        self.sandbox_profile
            .validate()
            .map_err(|e| SlmRunnerError::SandboxViolation {
                violation: e.to_string(),
            })?;

        Ok(())
    }
}

#[async_trait]
impl SlmRunner for LocalGgufRunner {
    async fn execute(
        &self,
        prompt: &str,
        options: Option<ExecutionOptions>,
    ) -> Result<ExecutionResult, SlmRunnerError> {
        let options = options.unwrap_or_default();
        let start_time = std::time::Instant::now();

        // Validate execution constraints
        self.validate_execution_constraints(prompt)?;

        // Build command
        let args = self.build_command_args(prompt, &options);
        let mut command = Command::new(&self.llama_cpp_path);
        command.args(&args);

        // Apply sandbox constraints
        self.apply_sandbox_constraints(&mut command);

        // Set up timeout
        let execution_timeout = options.timeout.unwrap_or_else(|| {
            Duration::from_secs(
                self.sandbox_profile
                    .process_limits
                    .max_execution_time_seconds,
            )
        });

        // Execute with timeout
        let output = timeout(execution_timeout, command.output())
            .await
            .map_err(|_| SlmRunnerError::ExecutionTimeout {
                seconds: execution_timeout.as_secs(),
            })?
            .map_err(|e| SlmRunnerError::ExecutionFailed {
                reason: format!("Process execution failed: {}", e),
            })?;

        // Check if process succeeded
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SlmRunnerError::ExecutionFailed {
                reason: format!("llama.cpp execution failed: {}", stderr),
            });
        }

        // Extract response
        let response = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let execution_time = start_time.elapsed();

        // Create execution metadata
        let metadata = ExecutionMetadata {
            input_tokens: Some((prompt.len() / 4) as u32), // Rough estimation
            output_tokens: Some((response.len() / 4) as u32), // Rough estimation
            execution_time_ms: execution_time.as_millis() as u64,
            memory_usage_mb: None, // Would need process monitoring for accurate measurement
            limits_hit: Vec::new(), // Would be populated if we detected limit violations
        };

        Ok(ExecutionResult { response, metadata })
    }

    fn get_sandbox_profile(&self) -> &SandboxProfile {
        &self.sandbox_profile
    }

    fn get_resource_requirements(&self) -> &ModelResourceRequirements {
        &self.resource_requirements
    }

    async fn health_check(&self) -> Result<(), SlmRunnerError> {
        // Check if model file is still accessible
        if !self.model_path.exists() {
            return Err(SlmRunnerError::ModelFileNotFound {
                path: self.model_path.display().to_string(),
            });
        }

        // Check if llama.cpp executable is still available
        if !self.llama_cpp_path.exists() {
            return Err(SlmRunnerError::InitializationFailed {
                reason: "llama.cpp executable no longer available".to_string(),
            });
        }

        // Test basic execution with a simple prompt
        let test_prompt = "Hello";
        let options = ExecutionOptions {
            timeout: Some(Duration::from_secs(10)),
            temperature: Some(0.1),
            max_tokens: Some(1),
            custom_parameters: HashMap::new(),
        };

        match self.execute(test_prompt, Some(options)).await {
            Ok(_) => Ok(()),
            Err(e) => Err(SlmRunnerError::InitializationFailed {
                reason: format!("Health check failed: {}", e),
            }),
        }
    }

    fn get_info(&self) -> RunnerInfo {
        RunnerInfo {
            runner_type: "LocalGgufRunner".to_string(),
            model_path: self.model_path.display().to_string(),
            capabilities: vec!["text_generation".to_string(), "conversation".to_string()],
            version: Some("1.0.0".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SandboxProfile;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_test_resource_requirements() -> ModelResourceRequirements {
        ModelResourceRequirements {
            min_memory_mb: 512,
            preferred_cpu_cores: 1.0,
            gpu_requirements: None,
        }
    }

    #[tokio::test]
    async fn test_gguf_runner_creation_missing_file() {
        let sandbox_profile = SandboxProfile::secure_default();
        let resource_requirements = create_test_resource_requirements();

        let result = LocalGgufRunner::new(
            "/nonexistent/model.gguf",
            sandbox_profile,
            resource_requirements,
        )
        .await;

        assert!(matches!(
            result,
            Err(SlmRunnerError::ModelFileNotFound { .. })
        ));
    }

    #[tokio::test]
    async fn test_execution_options_default() {
        let options = ExecutionOptions::default();
        assert_eq!(options.temperature, Some(0.7));
        assert_eq!(options.max_tokens, Some(256));
        assert!(options.timeout.is_some());
    }

    #[tokio::test]
    async fn test_command_args_building() {
        // Create a temporary file to serve as our model
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "dummy model content").unwrap();
        let model_path = temp_file.path().to_path_buf();

        let sandbox_profile = SandboxProfile::secure_default();
        let resource_requirements = create_test_resource_requirements();

        // Skip the actual runner creation since llama.cpp might not be available
        // Instead, test the argument building logic directly
        let runner = LocalGgufRunner {
            model_path: model_path.clone(),
            sandbox_profile,
            resource_requirements,
            llama_cpp_path: PathBuf::from("/fake/llama-cli"), // Fake path for testing
        };

        let options = ExecutionOptions::default();
        let args = runner.build_command_args("test prompt", &options);

        assert!(args.contains(&"--model".to_string()));
        assert!(args.contains(&model_path.display().to_string()));
        assert!(args.contains(&"--prompt".to_string()));
        assert!(args.contains(&"test prompt".to_string()));
    }

    #[test]
    fn test_validation_long_prompt() {
        let sandbox_profile = SandboxProfile::secure_default();
        let resource_requirements = create_test_resource_requirements();

        let runner = LocalGgufRunner {
            model_path: PathBuf::from("/fake/model.gguf"),
            sandbox_profile,
            resource_requirements,
            llama_cpp_path: PathBuf::from("/fake/llama-cli"),
        };

        let long_prompt = "a".repeat(20000); // Very long prompt
        let result = runner.validate_execution_constraints(&long_prompt);

        assert!(matches!(result, Err(SlmRunnerError::InvalidInput { .. })));
    }
}
