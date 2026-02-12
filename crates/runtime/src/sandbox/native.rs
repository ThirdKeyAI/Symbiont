//! Native Process Sandbox Runner
//!
//! Executes code directly on the host system with resource limits enforced
//! via direct `rlimit` syscalls (no shell wrapping).
//!
//! **WARNING**: This provides minimal security isolation and should only be used
//! in trusted development environments. Gated behind the `native-sandbox` feature.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use super::{ExecutionResult, SandboxRunner};

/// Default maximum output size in bytes (10 MB)
const DEFAULT_MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024;

/// Default file size limit in bytes (100 MB)
const DEFAULT_MAX_FSIZE_BYTES: u64 = 100 * 1024 * 1024;

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
    /// Maximum output bytes per stream before truncation (default: 10MB)
    pub max_output_bytes: usize,
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
            allowed_executables: vec![],
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }
}

/// Shell executable names that warrant a security warning
const SHELL_EXECUTABLES: &[&str] = &["bash", "sh", "zsh", "dash", "fish", "csh", "tcsh", "ksh"];

impl NativeConfig {
    /// Validate the configuration
    pub fn validate(&self) -> Result<(), anyhow::Error> {
        // Reject empty allowed_executables — force explicit configuration
        if self.allowed_executables.is_empty() {
            anyhow::bail!(
                "allowed_executables must not be empty — explicitly list the executables this runner may invoke"
            );
        }

        // Check if executable is allowed
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

        // Validate working directory
        if !self.working_directory.is_absolute() {
            anyhow::bail!(
                "Working directory must be absolute path: {}",
                self.working_directory.display()
            );
        }

        Ok(())
    }

    /// Log warnings for any shell executables in the allowed list
    pub fn warn_on_shell_executables(&self) {
        for allowed in &self.allowed_executables {
            let base_name = allowed.split('/').next_back().unwrap_or(allowed);
            if SHELL_EXECUTABLES.contains(&base_name) {
                tracing::warn!(
                    "SECURITY: shell executable '{}' is in allowed_executables — \
                     consider removing it unless explicitly required",
                    allowed
                );
            }
        }
    }
}

/// Native sandbox runner for direct host execution
#[derive(Debug)]
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
    /// This runner is gated behind the `native-sandbox` compile-time feature.
    /// It will refuse to run in production environments (`SYMBIONT_ENV=production`).
    pub fn new(config: NativeConfig) -> Result<Self, anyhow::Error> {
        // Hard-block in production — no runtime override
        if let Ok(env) = std::env::var("SYMBIONT_ENV") {
            if env.eq_ignore_ascii_case("production") {
                anyhow::bail!(
                    "SECURITY: Native execution is unconditionally disabled in production. \
                     Use a proper sandbox (Docker, gVisor, Firecracker, or E2B) instead."
                );
            }
        }

        // Always log warning when native execution is initialized
        tracing::warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        tracing::warn!("Native Sandbox: NO ISOLATION");
        tracing::warn!("Executable: {}", config.executable);
        tracing::warn!("Working dir: {}", config.working_directory.display());
        tracing::warn!("Code will run directly on host system");
        tracing::warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        // Validate configuration
        config.validate()?;

        // Warn about shell executables in the allow list
        config.warn_on_shell_executables();

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

    /// Apply resource limits via direct rlimit syscalls in a pre_exec closure (Unix only).
    /// No shell wrapping — the command directly invokes the target executable.
    #[cfg(unix)]
    fn apply_resource_limits(&self, command: &mut Command) -> Result<(), anyhow::Error> {
        if !self.config.enforce_resource_limits {
            return Ok(());
        }

        let max_memory_mb = self.config.max_memory_mb;
        let max_cpu_seconds = self.config.max_cpu_seconds;

        // SAFETY: pre_exec runs between fork() and exec() in the child process.
        // We only call async-signal-safe functions (setrlimit is async-signal-safe).
        unsafe {
            command.pre_exec(move || {
                // Set virtual memory limit (RLIMIT_AS)
                if let Some(mem_mb) = max_memory_mb {
                    let mem_bytes = mem_mb * 1024 * 1024;
                    rlimit::setrlimit(rlimit::Resource::AS, mem_bytes, mem_bytes).map_err(|e| {
                        std::io::Error::other(format!("Failed to set RLIMIT_AS: {}", e))
                    })?;
                }

                // Set CPU time limit (RLIMIT_CPU)
                if let Some(cpu_sec) = max_cpu_seconds {
                    rlimit::setrlimit(rlimit::Resource::CPU, cpu_sec, cpu_sec).map_err(|e| {
                        std::io::Error::other(format!("Failed to set RLIMIT_CPU: {}", e))
                    })?;
                }

                // Set file size limit (RLIMIT_FSIZE) — 100MB default
                rlimit::setrlimit(
                    rlimit::Resource::FSIZE,
                    DEFAULT_MAX_FSIZE_BYTES,
                    DEFAULT_MAX_FSIZE_BYTES,
                )
                .map_err(|e| std::io::Error::other(format!("Failed to set RLIMIT_FSIZE: {}", e)))?;

                Ok(())
            });
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

    /// Read output from a child stream with a size limit, returning the string and
    /// whether it was truncated.
    async fn read_limited_output<R: AsyncReadExt + Unpin>(
        reader: &mut R,
        max_bytes: usize,
    ) -> (String, bool) {
        let mut buf = vec![0u8; max_bytes + 1];
        let mut total = 0usize;

        loop {
            match reader.read(&mut buf[total..]).await {
                Ok(0) => break,
                Ok(n) => {
                    total += n;
                    if total > max_bytes {
                        total = max_bytes;
                        break;
                    }
                }
                Err(_) => break,
            }
        }

        let truncated = total == max_bytes;
        let output = String::from_utf8_lossy(&buf[..total]).to_string();

        if truncated {
            let with_marker = format!("{}\n... [output truncated at {} bytes]", output, max_bytes);
            (with_marker, true)
        } else {
            (output, false)
        }
    }
}

#[async_trait]
impl SandboxRunner for NativeRunner {
    async fn execute(
        &self,
        code: &str,
        env: HashMap<String, String>,
    ) -> Result<ExecutionResult, anyhow::Error> {
        tracing::warn!("EXECUTING CODE WITHOUT ISOLATION - Native execution mode is active");
        tracing::debug!(
            "Native execution: executable={}, working_dir={}",
            self.config.executable,
            self.config.working_directory.display()
        );

        let mut command = Command::new(&self.config.executable);

        // Determine how to pass code to the executable — direct argv, no shell wrapping
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

        // Set working directory, environment variables, and stdio BEFORE apply_resource_limits
        command.current_dir(&self.config.working_directory);
        command.envs(env);
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());

        // Apply resource limits via pre_exec rlimit syscalls — no shell wrapping
        self.apply_resource_limits(&mut command)?;

        let start = std::time::Instant::now();
        let max_output = self.config.max_output_bytes;

        // Spawn the child process
        let mut child = command.spawn().map_err(|e| {
            anyhow::anyhow!(
                "Failed to spawn process '{}': {}",
                self.config.executable,
                e
            )
        })?;

        // Take ownership of stdout/stderr handles
        let mut child_stdout = child.stdout.take();
        let mut child_stderr = child.stderr.take();

        // Execute with timeout, reading output with limits
        let output_result = timeout(self.config.max_execution_time, async {
            let stdout_future = async {
                match child_stdout.as_mut() {
                    Some(stdout) => Self::read_limited_output(stdout, max_output).await,
                    None => (String::new(), false),
                }
            };

            let stderr_future = async {
                match child_stderr.as_mut() {
                    Some(stderr) => Self::read_limited_output(stderr, max_output).await,
                    None => (String::new(), false),
                }
            };

            let ((stdout, stdout_truncated), (stderr, stderr_truncated)) =
                tokio::join!(stdout_future, stderr_future);

            let status = child.wait().await;

            (stdout, stdout_truncated, stderr, stderr_truncated, status)
        })
        .await;

        let execution_time = start.elapsed();

        match output_result {
            Ok((stdout, stdout_truncated, stderr, stderr_truncated, Ok(status))) => {
                let exit_code = status.code().unwrap_or(-1);
                let success = status.success();

                if stdout_truncated {
                    tracing::warn!(
                        "stdout truncated at {} bytes for native execution",
                        max_output
                    );
                }
                if stderr_truncated {
                    tracing::warn!(
                        "stderr truncated at {} bytes for native execution",
                        max_output
                    );
                }

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
                    stdout_truncated,
                    stderr_truncated,
                })
            }
            Ok((_, _, _, _, Err(e))) => {
                tracing::error!("Native execution failed: {}", e);
                Err(anyhow::anyhow!("Process execution failed: {}", e))
            }
            Err(_) => {
                // Kill the child on timeout
                let _ = child.kill().await;
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

    /// Helper: create a NativeConfig with bash allowed (most tests need this)
    fn config_with_bash() -> NativeConfig {
        NativeConfig {
            executable: "bash".to_string(),
            allowed_executables: vec!["bash".to_string()],
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_native_runner_creation() {
        let config = config_with_bash();
        let runner = NativeRunner::new(config);
        assert!(runner.is_ok());
    }

    #[tokio::test]
    async fn test_default_config_rejected_without_executables() {
        // Default has empty allowed_executables — validate() should reject it
        let config = NativeConfig::default();
        assert!(config.validate().is_err());
    }

    #[tokio::test]
    async fn test_native_python_execution() {
        let config = NativeConfig {
            executable: "python3".to_string(),
            allowed_executables: vec!["python3".to_string()],
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
            assert!(!output.stdout_truncated);
        }
    }

    #[tokio::test]
    async fn test_native_bash_execution() {
        let config = config_with_bash();

        let runner = NativeRunner::new(config).unwrap();

        let result = runner
            .execute("echo 'Testing native execution'", HashMap::new())
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.stdout.contains("Testing native execution"));
        assert!(!result.stdout_truncated);
        assert!(!result.stderr_truncated);
    }

    #[tokio::test]
    async fn test_native_execution_with_env_vars() {
        let config = config_with_bash();

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
            allowed_executables: vec!["bash".to_string()],
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
            allowed_executables: vec!["bash".to_string()],
            ..Default::default()
        };

        let result = NativeRunner::new(config);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_production_environment_blocked() {
        // Save original value
        let original = std::env::var("SYMBIONT_ENV").ok();

        std::env::set_var("SYMBIONT_ENV", "production");
        let config = config_with_bash();
        let result = NativeRunner::new(config);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("unconditionally disabled"));

        // Restore original value
        match original {
            Some(val) => std::env::set_var("SYMBIONT_ENV", val),
            None => std::env::remove_var("SYMBIONT_ENV"),
        }
    }

    #[tokio::test]
    async fn test_output_truncation() {
        let config = NativeConfig {
            executable: "bash".to_string(),
            allowed_executables: vec!["bash".to_string()],
            max_output_bytes: 50,
            ..Default::default()
        };

        let runner = NativeRunner::new(config).unwrap();

        // Generate output larger than 50 bytes
        let result = runner
            .execute(
                "for i in $(seq 1 100); do echo 'line'; done",
                HashMap::new(),
            )
            .await
            .unwrap();

        assert!(result.stdout_truncated);
        assert!(result.stdout.contains("[output truncated at 50 bytes]"));
    }
}
