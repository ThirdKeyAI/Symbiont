//! Selected sandbox execution for AI CLI tools.
//!
//! No host executable, working directory, credential directory or ambient
//! environment is implicitly made available to a contained child. The operator
//! supplies its image and scoped mounts. Development host execution is explicit.

use std::{collections::HashMap, path::Path, process::Stdio};

use serde::{Deserialize, Serialize};
use tokio::{io::AsyncWriteExt, process::Command, time::Duration};

use super::adapter::{AiCliAdapter, CodeGenRequest, CodeGenResult};
use crate::sandbox::{
    command::{CommandBoundary, CommandTier},
    docker::StdioContainerGuard,
    ExecutionResult,
};

// These are explicit caller overrides only, never an inheritance allowlist.
const ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "SHELL",
    "TERM",
    "LANG",
    "LC_ALL",
    "TZ",
    "OPENROUTER_API_KEY",
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
];

/// How to handle stdin for the spawned process.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum StdinStrategy {
    #[default]
    CloseImmediately,
    /// Cooperative prompt responses; never an authorization mechanism.
    AutoYes,
    AutoNo,
    Scripted(Vec<String>),
    DevNull,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliExecutorConfig {
    /// Wall-clock timeout, including sandbox initialization.
    pub max_runtime: Duration,
    pub stdin_strategy: StdinStrategy,
    /// Maximum inactivity across both output streams.
    pub idle_timeout: Duration,
    /// Maximum output bytes per stream; overflow fails and removes the worker.
    pub max_output_bytes: usize,
}

impl Default for CliExecutorConfig {
    fn default() -> Self {
        Self {
            max_runtime: Duration::from_secs(600),
            stdin_strategy: StdinStrategy::CloseImmediately,
            idle_timeout: Duration::from_secs(120),
            max_output_bytes: 10 * 1024 * 1024,
        }
    }
}

pub struct CliExecutor {
    config: CliExecutorConfig,
    boundary: Result<CommandBoundary, String>,
    stdout_line_sink: Option<super::watchdog::LineSink>,
}

impl CliExecutor {
    /// Defaults to a Docker worker with no host mounts or network access.
    pub fn new(config: CliExecutorConfig) -> Self {
        Self {
            config,
            boundary: Ok(CommandBoundary::default()),
            stdout_line_sink: None,
        }
    }

    /// Freeze an operator-supplied boundary. Invalid or unavailable selections
    /// fail at execution without falling back to a host subprocess.
    pub fn with_command_boundary(mut self, boundary: CommandBoundary) -> Self {
        self.boundary = Ok(boundary);
        self
    }

    pub fn with_project_sandbox(mut self, project: &Path) -> Self {
        self.boundary = CommandBoundary::load(project);
        self
    }

    /// Observe untrusted child output. This is not an authoritative action
    /// journal. The callback must not block the runtime's output reader.
    pub fn with_stdout_line_sink(mut self, sink: super::watchdog::LineSink) -> Self {
        self.stdout_line_sink = Some(sink);
        self
    }

    /// Probe the executable in the same selected boundary used for work.
    /// A version check must not turn an image executable into a host process.
    pub async fn health_check(&self, adapter: &dyn AiCliAdapter) -> anyhow::Result<()> {
        let boundary = self
            .boundary
            .as_ref()
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        let directory = match boundary.tier {
            CommandTier::Docker => boundary.docker.working_dir.as_str(),
            CommandTier::GVisor => boundary.gvisor.docker.working_dir.as_str(),
            CommandTier::Firecracker => boundary
                .firecracker
                .as_ref()
                .map_or("/tmp", |config| config.working_dir.as_str()),
            _ => "/tmp",
        };
        let probe = Self {
            config: CliExecutorConfig {
                max_runtime: self.config.max_runtime.min(Duration::from_secs(10)),
                idle_timeout: self.config.idle_timeout.min(Duration::from_secs(10)),
                max_output_bytes: self.config.max_output_bytes.min(65536),
                stdin_strategy: StdinStrategy::CloseImmediately,
            },
            boundary: self.boundary.clone(),
            stdout_line_sink: None,
        };
        let result = probe
            .spawn_and_monitor(
                adapter.executable(),
                &["--version".into()],
                Path::new(directory),
                HashMap::new(),
                HashMap::new(),
                StdinStrategy::CloseImmediately,
            )
            .await?;
        if !result.success {
            anyhow::bail!(
                "CLI health check failed with exit {}: {}",
                result.exit_code,
                result.stderr
            );
        }
        Ok(())
    }

    pub async fn execute(
        &self,
        adapter: &dyn AiCliAdapter,
        request: &CodeGenRequest,
    ) -> Result<CodeGenResult, anyhow::Error> {
        let result = self
            .spawn_and_monitor(
                adapter.executable(),
                &adapter.build_args(request),
                &request.working_dir,
                adapter.non_interactive_env(),
                request.options.clone(),
                adapter.stdin_strategy(),
            )
            .await?;
        let observed = result.clone();
        let mut parsed = adapter.parse_output(request, result);
        // Child-reported JSON cannot replace the observed process outcome.
        parsed.success &= observed.success;
        parsed.execution = observed;
        Ok(parsed)
    }

    async fn spawn_and_monitor(
        &self,
        executable: &str,
        args: &[String],
        working_dir: &Path,
        adapter_env: HashMap<String, String>,
        caller_env: HashMap<String, String>,
        stdin_strategy: StdinStrategy,
    ) -> anyhow::Result<ExecutionResult> {
        let started = std::time::Instant::now();
        validate_limits(&self.config)?;
        validate_stdin(&stdin_strategy)?;
        let mut boundary = self.boundary.clone().map_err(anyhow::Error::msg)?;
        boundary.validate().map_err(anyhow::Error::msg)?;
        let env = worker_environment(adapter_env, caller_env)?;
        let mut argv = vec![executable.to_owned()];
        argv.extend_from_slice(args);
        crate::sandbox::command::literal_command(&argv).map_err(anyhow::Error::msg)?;
        let deadline = started + self.config.max_runtime;
        #[cfg(target_os = "linux")]
        let landlock_domain = if boundary.tier == CommandTier::Landlock {
            // The child works in its own directory, so that path is granted
            // alongside the operator's declared roots. Without it the child
            // could not even enter its workspace.
            let mut roots = boundary.roots.clone();
            roots.output_roots.push(working_dir.display().to_string());
            Some(
                crate::sandbox::landlock::prepare(&boundary.landlock, &roots)
                    .map_err(anyhow::Error::msg)?,
            )
        } else {
            None
        };
        let mut worker = if boundary.tier == CommandTier::DevelopmentHost {
            let mut command = Command::new(executable);
            command
                .args(args)
                .current_dir(working_dir)
                .env_clear()
                .envs(env)
                .kill_on_drop(true)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            #[cfg(unix)]
            command.process_group(0);
            let child = command
                .spawn()
                .map_err(|e| anyhow::anyhow!("failed to spawn development CLI: {e}"))?;
            Worker { child, guard: None }
        } else if cfg!(target_os = "linux") && boundary.tier == CommandTier::Landlock {
            #[cfg(target_os = "linux")]
            {
                let domain = landlock_domain.expect("domain prepared for the landlock tier");
                let mut command = Command::new(executable);
                command
                    .args(args)
                    .current_dir(working_dir)
                    .env_clear()
                    .envs(env)
                    .kill_on_drop(true)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                command.process_group(0);
                domain.apply_to(&mut command);
                let child = command
                    .spawn()
                    .map_err(|e| anyhow::anyhow!("failed to spawn contained CLI: {e}"))?;
                Worker { child, guard: None }
            }
            #[cfg(not(target_os = "linux"))]
            unreachable!("the landlock tier is refused during validation off Linux")
        } else {
            configure_boundary(&mut boundary, working_dir, self.config.max_output_bytes)
                .map_err(anyhow::Error::msg)?;
            if boundary.tier == CommandTier::Firecracker {
                #[cfg(unix)]
                return self
                    .run_vm(boundary, argv, env, stdin_strategy, started, deadline)
                    .await;
                #[cfg(not(unix))]
                anyhow::bail!("Firecracker CLI transport requires Linux");
            }
            let container = boundary
                .spawn_stdio(
                    &argv,
                    env,
                    deadline.saturating_duration_since(std::time::Instant::now()),
                )
                .await
                .map_err(anyhow::Error::msg)?;
            Worker {
                child: container.child,
                guard: Some(container.guard),
            }
        };
        let output_limit = worker
            .guard
            .as_ref()
            .map_or(self.config.max_output_bytes, |guard| {
                guard.output_limit.min(self.config.max_output_bytes)
            });
        let input = InputTask::start(worker.child.stdin.take(), stdin_strategy);
        let result = super::monitor::monitor(
            &mut worker.child,
            deadline,
            self.config.idle_timeout,
            output_limit,
            self.stdout_line_sink.as_ref(),
            started,
        )
        .await;
        drop(input);
        // An otherwise successful child is a failed execution if its removal
        // cannot be acknowledged. Drop also signals the independent owner.
        let cleanup = worker.finish().await;
        match (result, cleanup) {
            (Ok(result), Ok(())) => Ok(result),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Err(error), Err(cleanup)) => {
                Err(anyhow::anyhow!("{error}; cleanup failed: {cleanup}"))
            }
        }
    }

    #[cfg(unix)]
    async fn run_vm(
        &self,
        boundary: CommandBoundary,
        argv: Vec<String>,
        env: HashMap<String, String>,
        strategy: StdinStrategy,
        started: std::time::Instant,
        deadline: std::time::Instant,
    ) -> anyhow::Result<ExecutionResult> {
        let config = boundary
            .firecracker
            .ok_or_else(|| anyhow::anyhow!("missing Firecracker configuration"))?;
        let runner = crate::sandbox::FirecrackerRunner::new(config)?;
        let worker = runner
            .spawn_stdio(
                &argv,
                env,
                deadline.saturating_duration_since(std::time::Instant::now()),
            )
            .await?;
        let crate::sandbox::firecracker::FirecrackerStdio {
            stdin,
            mut stdout,
            mut stderr,
            mut guard,
        } = worker;
        let input = InputTask::start(Some(stdin), strategy);
        let result = super::monitor::Settings {
            deadline,
            idle: self.config.idle_timeout,
            limit: guard.output_limit.min(self.config.max_output_bytes),
            sink: self.stdout_line_sink.as_ref(),
            started,
        }
        .collect(&mut stdout, &mut stderr, guard.wait_for_exit())
        .await;
        drop(input);
        // The monitor requires a verified guest exit. Cleanup independently
        // retains removal even when the operation failed or its caller timed out.
        let cleanup = guard.finish_cleanup().await;
        match (result, cleanup) {
            (result, Ok(())) => result,
            (Ok(_), Err(error)) => Err(error),
            (Err(error), Err(cleanup)) => {
                Err(anyhow::anyhow!("{error}; cleanup failed: {cleanup}"))
            }
        }
    }
}

pub(crate) fn configure_boundary(
    boundary: &mut CommandBoundary,
    directory: &Path,
    output: usize,
) -> Result<(), String> {
    let directory = directory
        .to_str()
        .ok_or("CLI working directory must be UTF-8")?;
    match boundary.tier {
        CommandTier::Docker | CommandTier::GVisor => {
            let config = if boundary.tier == CommandTier::Docker {
                &mut boundary.docker
            } else {
                &mut boundary.gvisor.docker
            };
            config.working_dir = directory.into();
            config.max_output_bytes = config.max_output_bytes.min(output);
        }
        CommandTier::Firecracker => {
            let config = boundary
                .firecracker
                .as_mut()
                .ok_or("missing Firecracker configuration")?;
            config.working_dir = directory.into();
            config.max_output_bytes = config.max_output_bytes.min(output);
        }
        _ => return Err("selected CLI transport is unavailable; no host fallback".into()),
    }
    boundary.validate()
}

pub(crate) fn validate_stdin(stdin_strategy: &StdinStrategy) -> anyhow::Result<()> {
    if let StdinStrategy::Scripted(lines) = stdin_strategy {
        let bytes = lines.iter().try_fold(0usize, |total, line| {
            total.checked_add(line.len())?.checked_add(1)
        });
        if lines.len() > 1024 || bytes.is_none_or(|bytes| bytes > 65536) {
            anyhow::bail!("CLI scripted stdin exceeds its limit");
        }
    }
    Ok(())
}

pub(crate) fn validate_limits(config: &CliExecutorConfig) -> anyhow::Result<()> {
    if config.max_runtime.is_zero()
        || config.max_runtime > Duration::from_secs(86400)
        || config.idle_timeout.is_zero()
        || config.max_output_bytes == 0
        || config.max_output_bytes > 10 * 1024 * 1024
    {
        anyhow::bail!("invalid CLI execution bounds");
    }
    Ok(())
}

pub(crate) fn worker_environment(
    adapter_env: HashMap<String, String>,
    caller_env: HashMap<String, String>,
) -> anyhow::Result<HashMap<String, String>> {
    let mut env = HashMap::from([
        (
            "PATH".into(),
            "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into(),
        ),
        ("HOME".into(), "/tmp".into()),
        ("TERM".into(), "dumb".into()),
        ("CI".into(), "true".into()),
        ("NON_INTERACTIVE".into(), "1".into()),
        ("NO_COLOR".into(), "1".into()),
    ]);
    env.extend(adapter_env);
    env.extend(
        caller_env
            .into_iter()
            .filter(|(key, _)| ENV_ALLOWLIST.contains(&key.as_str())),
    );
    if env
        .iter()
        .any(|(key, value)| key.is_empty() || key.contains(['=', '\0']) || value.contains('\0'))
    {
        anyhow::bail!("invalid CLI worker environment");
    }
    Ok(env)
}

struct Worker {
    child: tokio::process::Child,
    guard: Option<StdioContainerGuard>,
}
impl Worker {
    async fn finish(&mut self) -> anyhow::Result<()> {
        if self.guard.is_none() {
            #[cfg(unix)]
            if let Some(id) = self.child.id() {
                // SAFETY: this still-unreaped child leads the development-only
                // process group created above. No descendant-isolation claim.
                unsafe {
                    libc::killpg(id as i32, libc::SIGKILL);
                }
            }
        }
        let _ = self.child.start_kill();
        let cleanup = match &mut self.guard {
            Some(guard) => guard.finish().await,
            None => Ok(()),
        };
        let wait = tokio::time::timeout(Duration::from_secs(3), self.child.wait())
            .await
            .map_err(|_| anyhow::anyhow!("CLI attachment cleanup timed out"))?;
        cleanup?;
        wait?;
        Ok(())
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        #[cfg(unix)]
        if self.guard.is_none() {
            if let Some(id) = self.child.id() {
                // SAFETY: same unreaped development child as in finish.
                unsafe {
                    libc::killpg(id as i32, libc::SIGKILL);
                }
            }
        }
    }
}

struct InputTask(Option<tokio::task::JoinHandle<()>>);
impl InputTask {
    fn start<W: tokio::io::AsyncWrite + Unpin + Send + 'static>(
        stdin: Option<W>,
        strategy: StdinStrategy,
    ) -> Self {
        let Some(mut stdin) = stdin else {
            return Self(None);
        };
        if matches!(
            strategy,
            StdinStrategy::CloseImmediately | StdinStrategy::DevNull
        ) {
            return Self(None);
        }
        Self(Some(tokio::spawn(async move {
            match strategy {
                StdinStrategy::Scripted(lines) => {
                    for line in lines {
                        if stdin.write_all(line.as_bytes()).await.is_err()
                            || stdin.write_all(b"\n").await.is_err()
                        {
                            break;
                        }
                    }
                }
                StdinStrategy::AutoYes | StdinStrategy::AutoNo => {
                    let response = if matches!(strategy, StdinStrategy::AutoYes) {
                        b"y\n"
                    } else {
                        b"n\n"
                    };
                    loop {
                        if stdin.write_all(response).await.is_err() {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
                _ => {}
            }
        })))
    }
}
impl Drop for InputTask {
    fn drop(&mut self) {
        if let Some(task) = &self.0 {
            task.abort();
        }
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
        let _executor =
            CliExecutor::new(config).with_command_boundary(CommandBoundary::development_host());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn the_managed_cli_child_cannot_read_outside_its_grants() {
        if crate::sandbox::landlock::detect_abi() < 4 {
            eprintln!("skipped: kernel Landlock ABI below 4");
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        let secret_dir = tempfile::tempdir().unwrap();
        let secret = secret_dir.path().join("secret");
        std::fs::write(&secret, b"leaked").unwrap();

        let mut boundary = CommandBoundary::default();
        boundary.tier = CommandTier::Landlock;
        boundary.roots.source_roots = vec![format!("{}:/workspace:ro", workspace.path().display())];

        let executor = CliExecutor::new(Default::default()).with_command_boundary(boundary);
        let result = executor
            .spawn_and_monitor(
                "/bin/cat",
                &[secret.display().to_string()],
                workspace.path(),
                HashMap::new(),
                HashMap::new(),
                StdinStrategy::CloseImmediately,
            )
            .await;

        // Demand a real, contained run. Tolerating any Err here would pass
        // vacuously whenever the tier fell through to a backend that simply is
        // not available, which is how this test first passed before the
        // landlock branch existed.
        let execution = result.expect("the landlock tier must run the child, not fail to launch");
        assert!(
            !execution.success,
            "reading outside every grant must not succeed"
        );
        assert!(
            !execution.stdout.contains("leaked"),
            "the denied file's content must not reach the caller"
        );
    }

    #[tokio::test]
    async fn invalid_boundary_cannot_execute_host_command_or_health_probe() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("forbidden");
        let mut boundary = CommandBoundary::default();
        boundary.docker.pids_limit = 0;
        let executor = CliExecutor::new(Default::default()).with_command_boundary(boundary);
        let error = executor
            .spawn_and_monitor(
                "/usr/bin/touch",
                &[marker.display().to_string()],
                root.path(),
                HashMap::new(),
                HashMap::new(),
                StdinStrategy::CloseImmediately,
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("process limits"));
        let adapter = crate::cli_executor::ClaudeCodeAdapter {
            executable_path: "/usr/bin/true".into(),
            ..Default::default()
        };
        assert!(executor.health_check(&adapter).await.is_err());
        assert!(!marker.exists());
    }

    #[tokio::test]
    async fn test_non_interactive_env_vars() {
        let config = CliExecutorConfig::default();
        let executor =
            CliExecutor::new(config).with_command_boundary(CommandBoundary::development_host());

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

    #[cfg(unix)]
    #[tokio::test]
    #[serial_test::serial]
    async fn inherited_secrets_are_cleared_but_explicit_configuration_survives() {
        let key = "SYMBI_CLI_PROCESS_TEST_SECRET";
        std::env::set_var(key, "synthetic-canary");
        let result = CliExecutor::new(CliExecutorConfig::default())
            .with_command_boundary(CommandBoundary::development_host())
            .spawn_and_monitor(
                "/bin/sh",
                &["-c".into(), "printf '%s:%s:%s' \"${SYMBI_CLI_PROCESS_TEST_SECRET-unset}\" \"$SYMBI_TEST_HANDSHAKE\" \"$PATH\"".into()],
                std::path::Path::new("/tmp"),
                HashMap::from([("SYMBI_TEST_HANDSHAKE".into(), "managed".into())]),
                HashMap::from([("PATH".into(), "/explicit/bin".into()), (key.into(), "caller-canary".into())]),
                StdinStrategy::DevNull,
            ).await;
        std::env::remove_var(key);
        assert_eq!(result.unwrap().stdout, "unset:managed:/explicit/bin");
    }

    #[tokio::test]
    async fn test_stdin_close_immediately() {
        let config = CliExecutorConfig::default();
        let executor =
            CliExecutor::new(config).with_command_boundary(CommandBoundary::development_host());

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
        let executor =
            CliExecutor::new(config).with_command_boundary(CommandBoundary::development_host());

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
        let executor =
            CliExecutor::new(config).with_command_boundary(CommandBoundary::development_host());

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
        let executor =
            CliExecutor::new(config).with_command_boundary(CommandBoundary::development_host());

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
            .await;

        assert!(result.unwrap_err().to_string().contains("output limit"));
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
        let executor =
            CliExecutor::new(config).with_command_boundary(CommandBoundary::development_host());

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
