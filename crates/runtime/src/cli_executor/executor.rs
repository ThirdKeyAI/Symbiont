//! CLI Executor — stdin-protected process runner for AI CLI tools
//!
//! Spawns AI CLI tools (Claude Code, Gemini, Aider, Codex) with proper
//! non-interactive handling: stdin protection, idle-timeout watchdogs,
//! wall-clock timeouts, and process-group cleanup.

use std::collections::HashMap;
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::Duration;

use crate::sandbox::ExecutionResult;

use super::adapter::{AiCliAdapter, CodeGenRequest, CodeGenResult};
use super::watchdog::OutputWatchdog;

/// How to handle stdin for the spawned process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StdinStrategy {
    /// Close stdin immediately after spawn (default for most tools).
    CloseImmediately,
    /// Continuously write `"y\n"` to auto-accept prompts.
    AutoYes,
    /// Continuously write `"n\n"` to auto-decline prompts.
    AutoNo,
    /// Write each line in order, then close stdin.
    Scripted(Vec<String>),
    /// Redirect stdin to /dev/null.
    DevNull,
}

impl Default for StdinStrategy {
    fn default() -> Self {
        Self::CloseImmediately
    }
}

/// Configuration for the CLI executor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliExecutorConfig {
    /// Wall-clock timeout — kill process if it exceeds this.
    pub max_runtime: Duration,
    /// Default stdin strategy.
    pub stdin_strategy: StdinStrategy,
    /// Kill if no output for this long.
    pub idle_timeout: Duration,
    /// Maximum output bytes per stream before truncation.
    pub max_output_bytes: usize,
}

impl Default for CliExecutorConfig {
    fn default() -> Self {
        Self {
            max_runtime: Duration::from_secs(600),
            stdin_strategy: StdinStrategy::CloseImmediately,
            idle_timeout: Duration::from_secs(120),
            max_output_bytes: 10 * 1024 * 1024, // 10 MB
        }
    }
}

/// CLI executor that spawns AI CLI tools with full process management.
pub struct CliExecutor {
    config: CliExecutorConfig,
}

impl CliExecutor {
    /// Create a new executor with the given configuration.
    pub fn new(config: CliExecutorConfig) -> Self {
        Self { config }
    }

    /// Execute an AI CLI tool via the given adapter and request.
    ///
    /// 1. Builds args/env from the adapter
    /// 2. Spawns the process with stdin protection
    /// 3. Monitors output via `OutputWatchdog`
    /// 4. Races process completion, wall-clock timeout, and idle timeout
    /// 5. Cleans up via process group kill on timeout
    pub async fn execute(
        &self,
        adapter: &dyn AiCliAdapter,
        request: &CodeGenRequest,
    ) -> Result<CodeGenResult, anyhow::Error> {
        let args = adapter.build_args(request);
        let adapter_env = adapter.non_interactive_env();
        let stdin_strategy = adapter.stdin_strategy();

        let result = self
            .spawn_and_monitor(
                adapter.executable(),
                &args,
                &request.working_dir,
                adapter_env,
                request.options.clone(),
                stdin_strategy,
            )
            .await?;

        Ok(adapter.parse_output(request, result))
    }

    /// Low-level spawn with monitoring — reusable without an adapter.
    async fn spawn_and_monitor(
        &self,
        executable: &str,
        args: &[String],
        working_dir: &std::path::Path,
        adapter_env: HashMap<String, String>,
        caller_env: HashMap<String, String>,
        stdin_strategy: StdinStrategy,
    ) -> Result<ExecutionResult, anyhow::Error> {
        let mut command = Command::new(executable);
        command.args(args);
        command.current_dir(working_dir);

        // Merge environment: base non-interactive → adapter → caller
        let mut env = HashMap::new();
        env.insert("TERM".to_string(), "dumb".to_string());
        env.insert("CI".to_string(), "true".to_string());
        env.insert("NON_INTERACTIVE".to_string(), "1".to_string());
        env.insert("NO_COLOR".to_string(), "1".to_string());
        env.extend(adapter_env);
        env.extend(caller_env);
        command.envs(&env);

        // Configure stdin
        match &stdin_strategy {
            StdinStrategy::DevNull => {
                command.stdin(Stdio::null());
            }
            _ => {
                command.stdin(Stdio::piped());
            }
        }
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());

        // Unix: put child in its own process group for clean kill
        #[cfg(unix)]
        {
            unsafe {
                command.pre_exec(|| {
                    libc::setpgid(0, 0);
                    Ok(())
                });
            }
        }

        let start = std::time::Instant::now();

        let mut child = command
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to spawn '{}': {}", executable, e))?;

        // Handle stdin in a background task
        if let Some(mut stdin) = child.stdin.take() {
            match stdin_strategy {
                StdinStrategy::CloseImmediately => {
                    drop(stdin);
                }
                StdinStrategy::DevNull => {
                    // Already set to Stdio::null(), stdin won't be Some
                    drop(stdin);
                }
                StdinStrategy::AutoYes => {
                    tokio::spawn(async move {
                        loop {
                            if stdin.write_all(b"y\n").await.is_err() {
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    });
                }
                StdinStrategy::AutoNo => {
                    tokio::spawn(async move {
                        loop {
                            if stdin.write_all(b"n\n").await.is_err() {
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    });
                }
                StdinStrategy::Scripted(lines) => {
                    tokio::spawn(async move {
                        for line in lines {
                            if stdin
                                .write_all(format!("{}\n", line).as_bytes())
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        drop(stdin);
                    });
                }
            }
        }

        // Read stdout and stderr via watchdog
        let mut child_stdout = child.stdout.take();
        let mut child_stderr = child.stderr.take();

        let idle_timeout = self.config.idle_timeout;
        let max_output = self.config.max_output_bytes;

        let output_result = tokio::time::timeout(self.config.max_runtime, async {
            let stdout_watchdog = OutputWatchdog::new(idle_timeout, max_output);
            let stderr_watchdog = OutputWatchdog::new(idle_timeout, max_output);

            let stdout_future = async {
                match child_stdout.as_mut() {
                    Some(out) => stdout_watchdog.read_with_idle_detection(out).await,
                    None => super::watchdog::WatchdogOutput {
                        data: String::new(),
                        truncated: false,
                        idle_timeout_triggered: false,
                        bytes_read: 0,
                    },
                }
            };

            let stderr_future = async {
                match child_stderr.as_mut() {
                    Some(err) => stderr_watchdog.read_with_idle_detection(err).await,
                    None => super::watchdog::WatchdogOutput {
                        data: String::new(),
                        truncated: false,
                        idle_timeout_triggered: false,
                        bytes_read: 0,
                    },
                }
            };

            let (stdout_out, stderr_out) = tokio::join!(stdout_future, stderr_future);

            // If either stream triggered idle timeout, kill the process
            if stdout_out.idle_timeout_triggered || stderr_out.idle_timeout_triggered {
                tracing::warn!(
                    "Idle timeout triggered for '{}' — killing process",
                    executable
                );
                Self::kill_process(&mut child).await;
                return (stdout_out, stderr_out, true);
            }

            let status = child.wait().await;
            match status {
                Ok(_) => (stdout_out, stderr_out, false),
                Err(e) => {
                    tracing::error!("Failed to wait on child process: {}", e);
                    (stdout_out, stderr_out, false)
                }
            }
        })
        .await;

        let elapsed = start.elapsed();

        match output_result {
            Ok((stdout_out, stderr_out, idle_killed)) => {
                if idle_killed {
                    return Ok(ExecutionResult {
                        exit_code: -1,
                        stdout: stdout_out.data,
                        stderr: format!(
                            "{}\n[killed: idle timeout after {:?}]",
                            stderr_out.data, idle_timeout
                        ),
                        execution_time_ms: elapsed.as_millis() as u64,
                        success: false,
                        stdout_truncated: stdout_out.truncated,
                        stderr_truncated: stderr_out.truncated,
                    });
                }

                // Normal completion — get exit code from the already-waited child
                // The child.wait() already happened above, so we read the stored status.
                // Since we called child.wait() inside the future, the child is done.
                let exit_code = child
                    .try_wait()
                    .ok()
                    .flatten()
                    .map(|s| s.code().unwrap_or(-1))
                    .unwrap_or(0);

                let success = exit_code == 0;

                if stdout_out.truncated {
                    tracing::warn!(
                        "stdout truncated at {} bytes for '{}'",
                        max_output,
                        executable
                    );
                }

                Ok(ExecutionResult {
                    exit_code,
                    stdout: stdout_out.data,
                    stderr: stderr_out.data,
                    execution_time_ms: elapsed.as_millis() as u64,
                    success,
                    stdout_truncated: stdout_out.truncated,
                    stderr_truncated: stderr_out.truncated,
                })
            }
            Err(_) => {
                // Wall-clock timeout
                tracing::error!(
                    "Wall-clock timeout ({:?}) for '{}'",
                    self.config.max_runtime,
                    executable
                );
                Self::kill_process(&mut child).await;
                Err(anyhow::anyhow!(
                    "Execution timed out after {:?}",
                    self.config.max_runtime
                ))
            }
        }
    }

    /// Kill a child process and its entire process group (Unix).
    async fn kill_process(child: &mut tokio::process::Child) {
        #[cfg(unix)]
        {
            if let Some(id) = child.id() {
                unsafe {
                    libc::killpg(id as i32, libc::SIGKILL);
                }
            }
        }
        let _ = child.kill().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = CliExecutorConfig::default();
        assert_eq!(config.max_runtime, Duration::from_secs(600));
        assert_eq!(config.idle_timeout, Duration::from_secs(120));
        assert_eq!(config.max_output_bytes, 10 * 1024 * 1024);
        assert!(matches!(
            config.stdin_strategy,
            StdinStrategy::CloseImmediately
        ));
    }

    #[test]
    fn test_constructor() {
        let config = CliExecutorConfig::default();
        let _executor = CliExecutor::new(config);
    }

    #[tokio::test]
    async fn test_non_interactive_env_vars() {
        let config = CliExecutorConfig::default();
        let executor = CliExecutor::new(config);

        // Spawn a simple process that prints its env vars
        let result = executor
            .spawn_and_monitor(
                "env",
                &[],
                std::path::Path::new("/tmp"),
                HashMap::new(),
                HashMap::new(),
                StdinStrategy::CloseImmediately,
            )
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.stdout.contains("TERM=dumb"));
        assert!(result.stdout.contains("CI=true"));
        assert!(result.stdout.contains("NON_INTERACTIVE=1"));
        assert!(result.stdout.contains("NO_COLOR=1"));
    }

    #[tokio::test]
    async fn test_stdin_close_immediately() {
        let config = CliExecutorConfig::default();
        let executor = CliExecutor::new(config);

        let result = executor
            .spawn_and_monitor(
                "echo",
                &["hello".to_string()],
                std::path::Path::new("/tmp"),
                HashMap::new(),
                HashMap::new(),
                StdinStrategy::CloseImmediately,
            )
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.stdout.contains("hello"));
    }

    #[tokio::test]
    async fn test_stdin_devnull() {
        let config = CliExecutorConfig::default();
        let executor = CliExecutor::new(config);

        let result = executor
            .spawn_and_monitor(
                "echo",
                &["hello".to_string()],
                std::path::Path::new("/tmp"),
                HashMap::new(),
                HashMap::new(),
                StdinStrategy::DevNull,
            )
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.stdout.contains("hello"));
    }

    #[tokio::test]
    async fn test_wall_clock_timeout() {
        let config = CliExecutorConfig {
            max_runtime: Duration::from_secs(1),
            idle_timeout: Duration::from_secs(30),
            ..Default::default()
        };
        let executor = CliExecutor::new(config);

        let result = executor
            .spawn_and_monitor(
                "sleep",
                &["10".to_string()],
                std::path::Path::new("/tmp"),
                HashMap::new(),
                HashMap::new(),
                StdinStrategy::CloseImmediately,
            )
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timed out"));
    }

    #[tokio::test]
    async fn test_output_truncation() {
        let config = CliExecutorConfig {
            max_output_bytes: 50,
            idle_timeout: Duration::from_secs(10),
            ..Default::default()
        };
        let executor = CliExecutor::new(config);

        let result = executor
            .spawn_and_monitor(
                "bash",
                &[
                    "-c".to_string(),
                    "for i in $(seq 1 100); do echo 'line'; done".to_string(),
                ],
                std::path::Path::new("/tmp"),
                HashMap::new(),
                HashMap::new(),
                StdinStrategy::CloseImmediately,
            )
            .await
            .unwrap();

        assert!(result.stdout_truncated);
        assert!(result.stdout.contains("[output truncated at 50 bytes]"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_process_group_cleanup() {
        // Spawn a process that forks children, then timeout
        let config = CliExecutorConfig {
            max_runtime: Duration::from_secs(1),
            idle_timeout: Duration::from_secs(30),
            ..Default::default()
        };
        let executor = CliExecutor::new(config);

        let result = executor
            .spawn_and_monitor(
                "bash",
                &["-c".to_string(), "sleep 100 & sleep 100 & wait".to_string()],
                std::path::Path::new("/tmp"),
                HashMap::new(),
                HashMap::new(),
                StdinStrategy::CloseImmediately,
            )
            .await;

        // Should timeout
        assert!(result.is_err());
    }
}
