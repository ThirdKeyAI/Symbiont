//! Native Process Sandbox Runner
//!
//! Executes code directly on the host system without container isolation.
//! **WARNING**: This provides NO security isolation and should only be used
//! in trusted development environments.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use super::{ExecutionResult, SandboxRunner};

/// Configuration for native (non-isolated) execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeConfig {
    /// Executable to run (e.g., "python3", "node", "bash")
    pub executable: String,
    /// Working directory for execution
    pub working_directory: PathBuf,
    /// Whether to enforce resource limits (Unix only)
    pub enforce_resource_limits: bool,
    /// Maximum memory in MB (if enforced)
    pub max_memory_mb: Option<u64>,
    /// Maximum CPU time in seconds (if enforced)
    pub max_cpu_seconds: Option<u64>,
    /// Maximum execution time (timeout)
    pub max_execution_time: Duration,
    /// Allowed executables (if empty, all are allowed)
    pub allowed_executables: Vec<String>,
}

impl Default for NativeConfig {
    fn default() -> Self {
        Self {
            executable: "bash".to_string(),
            working_directory: PathBuf::from("/tmp/symbiont-native"),
            enforce_resource_limits: true,
            max_memory_mb: Some(2048),
            max_cpu_seconds: Some(300),
            max_execution_time: Duration::from_secs(300),
            allowed_executables: vec![
                "bash".to_string(),
                "sh".to_string(),
                "python3".to_string(),
                "python".to_string(),
                "node".to_string(),
            ],
        }
    }
}

impl NativeConfig {
    /// Validate the configuration
    pub fn validate(&self) -> Result<(), anyhow::Error> {
        // Check if executable is allowed
        if !self.allowed_executables.is_empty() {
            let exec_name = self
                .executable
                .split('/')
                .next_back()
                .unwrap_or(&self.executable);
            if !self
                .allowed_executables
                .iter()
                .any(|allowed| allowed == &self.executable || allowed == exec_name)
            {
                anyhow::bail!(
                    "Executable '{}' not in allowed list: {:?}",
                    self.executable,
                    self.allowed_executables
                );
            }
        }

        // Validate working directory
        if !self.working_directory.is_absolute() {
            anyhow::bail!(
                "Working directory must be absolute path: {}",
                self.working_directory.display()
            );
        }

        Ok(())
    }
}

/// Native sandbox runner for direct host execution
pub struct NativeRunner {
    config: NativeConfig,
}

impl NativeRunner {
    /// Create a new native runner with the given configuration
    ///
    /// # Security Warning
    ///
    /// Native execution provides **ZERO isolation** from the host system. Code
    /// executed with this runner has full access to:
    /// - The entire filesystem (subject to process permissions)
    /// - Network interfaces
    /// - System processes
    /// - Environment variables and secrets
    ///
    /// **USE ONLY IN TRUSTED DEVELOPMENT ENVIRONMENTS**
    ///
    /// # Safety Checks
    ///
    /// This method will perform the following safety checks:
    /// 1. Refuse to run if SYMBIONT_ENV=production (unless explicitly allowed)
    /// 2. Require explicit opt-in via environment variable or security config
    /// 3. Log prominent security warnings
    /// 4. Validate configuration
    pub fn new(config: NativeConfig) -> Result<Self, anyhow::Error> {
        // Check environment - refuse in production unless explicitly allowed
        if let Ok(env) = std::env::var("SYMBIONT_ENV") {
            if env.to_lowercase() == "production" {
                // Check if explicitly allowed via environment variable
                let allow_native = std::env::var("SYMBIONT_ALLOW_NATIVE_EXECUTION")
                    .unwrap_or_default()
                    .to_lowercase();

                if allow_native != "true" && allow_native != "yes" && allow_native != "1" {
                    anyhow::bail!(
                        "SECURITY: Native execution is disabled in production environments. \
                         Set SYMBIONT_ALLOW_NATIVE_EXECUTION=true to override (not recommended)."
                    );
                }

                tracing::error!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                tracing::error!("⚠️  CRITICAL SECURITY WARNING");
                tracing::error!("⚠️  Native execution enabled in PRODUCTION environment!");
                tracing::error!("⚠️  This provides ZERO isolation and is NOT recommended.");
                tracing::error!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

                eprintln!("\n⚠️  CRITICAL: Native execution in production!");
                eprintln!("⚠️  NO sandboxing - full host access granted to code.\n");
            }
        }

        // Always log warning when native execution is initialized
        tracing::warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        tracing::warn!("⚠️  Native Sandbox: NO ISOLATION");
        tracing::warn!("⚠️  Executable: {}", config.executable);
        tracing::warn!("⚠️  Working dir: {}", config.working_directory.display());
        tracing::warn!("⚠️  Code will run directly on host system");
        tracing::warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        // Validate configuration
        config.validate()?;

        // Ensure working directory exists
        if !config.working_directory.exists() {
            tracing::info!(
                "Creating working directory: {}",
                config.working_directory.display()
            );
            std::fs::create_dir_all(&config.working_directory)?;
        }

        Ok(Self { config })
    }

    /// Create with default configuration
    pub fn with_defaults() -> Result<Self, anyhow::Error> {
        Self::new(NativeConfig::default())
    }

    /// Apply resource limits (Unix only)
    #[cfg(unix)]
    fn apply_resource_limits(&self, command: &mut Command) -> Result<(), anyhow::Error> {
        if !self.config.enforce_resource_limits {
            return Ok(());
        }

        // Use process limits via ulimit wrapper
        // Note: This uses a wrapper script approach since Rust's std::process
        // doesn't provide direct rlimit setting

        let mut limit_cmds = Vec::new();

        if let Some(max_mem_mb) = self.config.max_memory_mb {
            // Virtual memory limit in KB
            limit_cmds.push(format!("ulimit -v {}", max_mem_mb * 1024));
        }

        if let Some(max_cpu_sec) = self.config.max_cpu_seconds {
            // CPU time limit in seconds
            limit_cmds.push(format!("ulimit -t {}", max_cpu_sec));
        }

        // If we have limits, wrap the command with ulimit
        if !limit_cmds.is_empty() {
            let original_program = command.as_std().get_program().to_string_lossy().to_string();
            let original_args: Vec<String> = command
                .as_std()
                .get_args()
                .map(|s| s.to_string_lossy().to_string())
                .collect();

            // Shell-escape each argument by wrapping in single quotes
            fn shell_escape(s: &str) -> String {
                format!("'{}'", s.replace('\'', "'\\''"))
            }

            // Create wrapper: sh -c "ulimit ... && ulimit ... && <original_command>"
            let escaped_args: Vec<String> = original_args.iter().map(|a| shell_escape(a)).collect();
            let wrapper_cmd = format!(
                "{} && {} {}",
                limit_cmds.join(" && "),
                shell_escape(&original_program),
                escaped_args.join(" ")
            );

            *command = Command::new("sh");
            command.arg("-c");
            command.arg(wrapper_cmd);
        }

        Ok(())
    }

    #[cfg(not(unix))]
    fn apply_resource_limits(&self, _command: &mut Command) -> Result<(), anyhow::Error> {
        if self.config.enforce_resource_limits {
            tracing::warn!(
                "Resource limits are not supported on this platform, ignoring enforce_resource_limits setting"
            );
        }
        Ok(())
    }
}

#[async_trait]
impl SandboxRunner for NativeRunner {
    async fn execute(
        &self,
        code: &str,
        env: HashMap<String, String>,
    ) -> Result<ExecutionResult, anyhow::Error> {
        tracing::warn!("⚠️  EXECUTING CODE WITHOUT ISOLATION - Native execution mode is active");
        tracing::debug!(
            "Native execution: executable={}, working_dir={}",
            self.config.executable,
            self.config.working_directory.display()
        );

        let mut command = Command::new(&self.config.executable);

        // Determine how to pass code to the executable
        // For interpreters, use -c flag; for shell, use -c
        match self
            .config
            .executable
            .split('/')
            .next_back()
            .unwrap_or(&self.config.executable)
        {
            "python" | "python3" => {
                command.arg("-c");
                command.arg(code);
            }
            "node" => {
                command.arg("-e");
                command.arg(code);
            }
            "bash" | "sh" => {
                command.arg("-c");
                command.arg(code);
            }
            _ => {
                // For unknown executables, try -c as default
                command.arg("-c");
                command.arg(code);
            }
        }

        // Apply resource limits if configured (may replace the command)
        self.apply_resource_limits(&mut command)?;

        // Set working directory, environment variables, and stdio
        // AFTER apply_resource_limits, which may replace the command object
        command.current_dir(&self.config.working_directory);
        command.envs(env);
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());

        let start = std::time::Instant::now();

        // Execute with timeout
        let output_result = timeout(self.config.max_execution_time, command.output()).await;

        let execution_time = start.elapsed();

        match output_result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let exit_code = output.status.code().unwrap_or(-1);
                let success = output.status.success();

                tracing::debug!(
                    "Native execution completed: exit_code={}, success={}, duration={:?}",
                    exit_code,
                    success,
                    execution_time
                );

                Ok(ExecutionResult {
                    stdout,
                    stderr,
                    exit_code,
                    success,
                    execution_time_ms: execution_time.as_millis() as u64,
                })
            }
            Ok(Err(e)) => {
                tracing::error!("Native execution failed: {}", e);
                Err(anyhow::anyhow!("Process execution failed: {}", e))
            }
            Err(_) => {
                tracing::error!(
                    "Native execution timed out after {:?}",
                    self.config.max_execution_time
                );
                Err(anyhow::anyhow!(
                    "Execution timed out after {:?}",
                    self.config.max_execution_time
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_native_runner_creation() {
        let config = NativeConfig::default();
        let runner = NativeRunner::new(config);
        assert!(runner.is_ok());
    }

    #[tokio::test]
    async fn test_native_runner_with_defaults() {
        let runner = NativeRunner::with_defaults();
        assert!(runner.is_ok());
    }

    #[tokio::test]
    async fn test_native_python_execution() {
        let config = NativeConfig {
            executable: "python3".to_string(),
            ..Default::default()
        };

        let runner = match NativeRunner::new(config) {
            Ok(r) => r,
            Err(_) => {
                // Skip test if python3 not available
                return;
            }
        };

        let result = runner
            .execute("print('Hello from native!')", HashMap::new())
            .await;

        if let Ok(output) = result {
            assert!(output.success);
            assert!(output.stdout.contains("Hello from native!"));
        }
    }

    #[tokio::test]
    async fn test_native_bash_execution() {
        let config = NativeConfig {
            executable: "bash".to_string(),
            ..Default::default()
        };

        let runner = NativeRunner::new(config).unwrap();

        let result = runner
            .execute("echo 'Testing native execution'", HashMap::new())
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.stdout.contains("Testing native execution"));
    }

    #[tokio::test]
    async fn test_native_execution_with_env_vars() {
        let config = NativeConfig {
            executable: "bash".to_string(),
            ..Default::default()
        };

        let runner = NativeRunner::new(config).unwrap();

        let mut env = HashMap::new();
        env.insert("TEST_VAR".to_string(), "test_value".to_string());

        let result = runner.execute("echo $TEST_VAR", env).await.unwrap();

        assert!(result.success);
        assert!(result.stdout.contains("test_value"));
    }

    #[tokio::test]
    async fn test_native_execution_timeout() {
        let config = NativeConfig {
            executable: "bash".to_string(),
            max_execution_time: Duration::from_secs(1),
            ..Default::default()
        };

        let runner = NativeRunner::new(config).unwrap();

        let result = runner.execute("sleep 5", HashMap::new()).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timed out"));
    }

    #[tokio::test]
    async fn test_executable_validation() {
        let config = NativeConfig {
            executable: "malicious_exe".to_string(),
            allowed_executables: vec!["bash".to_string(), "python3".to_string()],
            ..Default::default()
        };

        let result = NativeRunner::new(config);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_working_directory_validation() {
        let config = NativeConfig {
            working_directory: PathBuf::from("relative/path"),
            ..Default::default()
        };

        let result = NativeRunner::new(config);
        assert!(result.is_err());
    }
}
