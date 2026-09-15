//! Docker container sandbox runner
//!
//! Executes code inside Docker containers for isolated agent execution.
//! Uses the `docker` CLI rather than a Rust Docker client library to
//! minimize dependencies and match the pattern of the native runner.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Component, Path};
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::oneshot;
use tokio::time::{timeout, Duration};

use super::{
    supervisor::{self, SupervisorConfig},
    ExecutionResult, SandboxRunner,
};
use symbi_sandbox_supervisor::protocol;

/// Default maximum output size in bytes (10 MB)
const DEFAULT_MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024;

/// Configuration for Docker sandbox execution
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DockerConfig {
    /// Docker image to use (e.g. "python:3.12-slim", "node:20-alpine")
    pub image: String,
    /// Shell executable inside the container (default: "sh")
    pub shell: String,
    /// Maximum memory for the container (e.g. "512m", "2g")
    pub memory_limit: Option<String>,
    /// Maximum CPU quota (e.g. "1.0" = 1 CPU, "0.5" = half CPU)
    pub cpu_limit: Option<f64>,
    /// Maximum execution time before killing the container
    pub max_execution_time: Duration,
    /// Network mode: "none" (isolated), or explicit non-production "bridge".
    /// Production refuses bridge because it does not authorize destinations.
    pub network_mode: String,
    /// Existing absolute bind sources (host_path:container_path:ro|rw).
    /// Omitted mode is read-only. The operator must scope these grants.
    pub volumes: Vec<String>,
    /// Working directory inside the container
    pub working_dir: String,
    /// Must be true: containers are removed after all outcomes
    pub auto_remove: bool,
    /// Docker binary path (default: "docker")
    pub docker_binary: String,
    /// Maximum output bytes per stream before failure and container removal
    pub max_output_bytes: usize,
    /// Hard per-file size limit inside the worker, including writable bind files.
    pub max_file_bytes: u64,
    /// Maximum processes in the container; zero is forbidden.
    #[serde(default = "default_pids_limit")]
    pub pids_limit: u32,
    /// Numeric, non-root UID:GID inside the container.
    #[serde(default = "default_user")]
    pub user: String,
    /// Additional metadata flags. Only --label=value is accepted.
    pub extra_flags: Vec<String>,
    /// Independent creation, lifetime and durable cleanup owner.
    pub supervisor: SupervisorConfig,
    /// Runtime-owned snapshot references; project configuration cannot supply them.
    #[serde(skip)]
    pub staging: Vec<uuid::Uuid>,
}

fn default_pids_limit() -> u32 {
    128
}
fn default_user() -> String {
    "65534:65534".into()
}

impl Default for DockerConfig {
    fn default() -> Self {
        Self {
            image: "python:3.12-slim".to_string(),
            shell: "sh".to_string(),
            memory_limit: Some("512m".to_string()),
            cpu_limit: Some(1.0),
            max_execution_time: Duration::from_secs(300),
            network_mode: "none".to_string(),
            volumes: Vec::new(),
            working_dir: "/workspace".to_string(),
            auto_remove: true,
            docker_binary: "docker".to_string(),
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            max_file_bytes: 64 * 1024 * 1024,
            extra_flags: Vec::new(),
            supervisor: SupervisorConfig::default(),
            staging: Vec::new(),
            pids_limit: default_pids_limit(),
            user: default_user(),
        }
    }
}

impl DockerConfig {
    /// Create a config for a specific image with sensible defaults
    pub fn for_image(image: &str) -> Self {
        Self {
            image: image.to_string(),
            ..Default::default()
        }
    }

    /// Enable unrestricted bridge networking for non-production use.
    /// Validation refuses this configuration in production.
    pub fn with_network(mut self) -> Self {
        self.network_mode = "bridge".to_string();
        self
    }

    /// Set memory limit
    pub fn with_memory(mut self, limit: &str) -> Self {
        self.memory_limit = Some(limit.to_string());
        self
    }

    /// Set CPU limit
    pub fn with_cpu(mut self, limit: f64) -> Self {
        self.cpu_limit = Some(limit);
        self
    }

    /// Add a volume mount.
    ///
    /// Validates the host-side path against an obvious-danger blocklist
    /// (docker socket, host root filesystem, kernel interfaces, path
    /// traversal). Returns `Err` rather than silently accepting a mount
    /// that would punch a hole through the sandbox.
    pub fn with_volume(mut self, mount: &str) -> Result<Self, anyhow::Error> {
        validate_volume_mount(mount)?;
        self.volumes.push(mount.to_string());
        Ok(self)
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<(), anyhow::Error> {
        if self.image.is_empty()
            || self.image.starts_with('-')
            || self.image.chars().any(char::is_whitespace)
        {
            anyhow::bail!("Docker image must be a nonempty image reference");
        }
        if self.shell.is_empty() || self.shell.contains(['\n', '\r', '\0']) {
            anyhow::bail!("Docker shell must be a valid executable");
        }
        if self.max_execution_time.is_zero() || self.max_execution_time > Duration::from_secs(86400)
        {
            anyhow::bail!("Docker execution budget must be between zero and one day");
        }
        if self.max_output_bytes == 0 || self.max_output_bytes > DEFAULT_MAX_OUTPUT_BYTES {
            anyhow::bail!("Docker output limit must be between 1 and 10 MiB per stream");
        }
        if self.max_file_bytes == 0 || self.max_file_bytes > 1024 * 1024 * 1024 {
            anyhow::bail!("Docker file size limit must be between 1 byte and 1 GiB");
        }
        let memory = self
            .memory_limit
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Docker requires a memory limit"))?;
        let digits = memory.trim_end_matches(['b', 'k', 'm', 'g', 'B', 'K', 'M', 'G']);
        if digits.is_empty()
            || memory.len() - digits.len() > 1
            || !digits.bytes().all(|b| b.is_ascii_digit())
            || digits.parse::<u64>().is_err()
            || digits.parse::<u64>()? == 0
        {
            anyhow::bail!(
                "Docker memory limit must be a positive integer with an optional b/k/m/g suffix"
            );
        }
        if !self
            .cpu_limit
            .is_some_and(|cpu| cpu.is_finite() && cpu > 0.0)
            || self.pids_limit == 0
        {
            anyhow::bail!("Docker requires positive CPU and process limits");
        }
        let ids: Vec<_> = self.user.split(':').collect();
        if ids.len() != 2 || ids.iter().any(|id| !id.parse::<u32>().is_ok_and(|n| n > 0)) {
            anyhow::bail!("Docker user must be a non-root numeric UID:GID");
        }
        validate_container_path(&self.working_dir)?;
        if !self.auto_remove {
            anyhow::bail!("Docker sandbox containers must be removed after execution");
        }
        if self.network_mode != "none" && self.network_mode != "bridge" {
            anyhow::bail!("Docker network must be none or explicitly enabled bridge; host and shared namespaces are forbidden");
        }
        if self.network_mode == "bridge" && crate::env::is_production()? {
            anyhow::bail!("Docker bridge networking is forbidden in production; select network_mode=\"none\" and use governed HTTP tools for outbound requests");
        }
        let supervisor_root = self.supervisor.resolved_state_dir()?;
        for flag in &self.extra_flags {
            if !flag.starts_with("--label=")
                || flag.contains(['\n', '\r', '\0'])
                || flag
                    .strip_prefix("--label=")
                    .and_then(|label| label.split('=').next())
                    == Some(protocol::LABEL)
            {
                anyhow::bail!("Docker extra_flags accepts only --label=value metadata; isolation overrides are forbidden");
            }
        }
        for vol in &self.volumes {
            validate_volume_mount(vol)?;
            let source = Path::new(vol.split(':').next().unwrap()).canonicalize()?;
            let staged = self.staging.iter().any(|id| {
                source.starts_with(
                    supervisor_root
                        .join("staging")
                        .join(id.to_string())
                        .join("data"),
                )
            });
            if !staged
                && (source.starts_with(&supervisor_root) || supervisor_root.starts_with(&source))
            {
                anyhow::bail!("bind mount exposes the protected sandbox supervisor state");
            }
        }
        Ok(())
    }
}

/// Host-side paths that must never be mounted into a sandbox container.
///
/// The list is intentionally conservative: anything rooted in these
/// directories would let a container read host secrets, escape via the
/// container runtime's control socket, or edit kernel state.
const DANGEROUS_HOST_PATHS: &[&str] = &[
    "/var/run/docker.sock",
    "/run/docker.sock",
    "/var/run/containerd",
    "/var/run/crio",
    "/dev",
    "/run",
    "/proc",
    "/sys",
    "/boot",
    "/etc",
    "/root",
    "/var/lib/docker",
    "/var/lib/kubelet",
    "/var/lib/rancher",
];

/// Require explicit bind mounts. Named volumes can hide arbitrary daemon-side
/// paths and drivers, so they cannot be authorized by a host-path check.
fn validate_volume_mount(mount: &str) -> Result<(), anyhow::Error> {
    canonical_mount(mount).map(|_| ())
}

fn validate_container_path(value: &str) -> Result<(), anyhow::Error> {
    let path = Path::new(value);
    if !path.is_absolute()
        || path == Path::new("/")
        || value.contains([',', ':', '\n', '\r', '\0'])
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        anyhow::bail!("container path must be an absolute non-root path without traversal");
    }
    for reserved in ["/proc", "/sys", "/dev", "/etc", "/run"] {
        if path.starts_with(reserved) {
            anyhow::bail!("container path overlaps a reserved system directory");
        }
    }
    Ok(())
}

pub(in crate::sandbox) fn canonical_mount(mount: &str) -> Result<String, anyhow::Error> {
    let parts: Vec<_> = mount.split(':').collect();
    if !(2..=3).contains(&parts.len()) {
        anyhow::bail!("bind mount must be host:container[:ro|rw]");
    }
    let host = Path::new(parts[0]);
    if !host.is_absolute() || host.components().any(|c| matches!(c, Component::ParentDir)) {
        anyhow::bail!("bind mount requires an existing absolute host path without traversal; named volumes are forbidden");
    }
    validate_container_path(parts[1])?;
    let mode = parts.get(2).copied().unwrap_or("ro");
    if !matches!(mode, "ro" | "rw") {
        anyhow::bail!("bind mount mode must be ro or rw");
    }
    let canonical = host
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("cannot resolve bind mount source: {e}"))?;
    if canonical == Path::new("/")
        || DANGEROUS_HOST_PATHS
            .iter()
            .any(|p| canonical.starts_with(p) || Path::new(p).starts_with(&canonical))
    {
        anyhow::bail!("bind mount exposes a protected host path");
    }
    let metadata = canonical.metadata()?;
    if !metadata.is_dir() && !metadata.is_file() {
        anyhow::bail!("bind mount source must be a regular file or directory");
    }
    let source = canonical
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("bind mount source must be UTF-8"))?;
    if source.contains([',', ':', '\n', '\r', '\0']) {
        anyhow::bail!("bind mount source contains delimiters");
    }
    // Pass the resolved path to Docker, not the original symlink spelling.
    Ok(format!(
        "type=bind,src={source},dst={},bind-propagation=rprivate{}",
        parts[1],
        if mode == "ro" { ",readonly" } else { "" }
    ))
}

/// Docker sandbox runner. The OCI runtime is selected internally; extra_flags
/// cannot replace gVisor with a weaker runtime.
#[derive(Clone)]
pub struct DockerRunner {
    config: DockerConfig,
    runtime: Option<String>,
}

impl DockerRunner {
    /// Create a new Docker runner with the given configuration
    pub fn new(config: DockerConfig) -> Result<Self, anyhow::Error> {
        config.validate()?;

        super::command::check_binary(
            &config.docker_binary,
            "version",
            config.max_execution_time.min(Duration::from_secs(5)),
        )?;

        tracing::info!(
            "Docker sandbox initialized: image={}, network={}, memory={:?}, cpu={:?}",
            config.image,
            config.network_mode,
            config.memory_limit,
            config.cpu_limit
        );

        Ok(Self {
            config,
            runtime: None,
        })
    }

    pub(crate) fn with_runtime(
        config: DockerConfig,
        runtime: String,
    ) -> Result<Self, anyhow::Error> {
        if runtime.is_empty()
            || !runtime
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        {
            anyhow::bail!("invalid Docker OCI runtime name");
        }
        let mut runner = Self::new(config)?;
        runner.runtime = Some(runtime);
        Ok(runner)
    }

    /// Build literal stopped-container argv. The independent supervisor owns
    /// the environment file and Docker create process through completion.
    fn build_command(
        &self,
        name: &str,
        code: &str,
        env: &HashMap<String, String>,
        terminal: bool,
    ) -> Result<Command, anyhow::Error> {
        let mut cmd = Command::new(&self.config.docker_binary);
        self.config.validate()?;
        cmd.arg("create")
            .arg("--interactive")
            .arg("--name")
            .arg(name);
        if terminal {
            cmd.arg("--tty");
        }
        cmd.args([
            "--pull",
            "never",
            "--init",
            "--restart",
            "no",
            "--no-healthcheck",
            "--log-driver",
            "none",
            "--ipc",
            "private",
            "--cgroupns",
            "private",
        ]);
        cmd.arg("--user").arg(&self.config.user);
        cmd.arg("--pids-limit")
            .arg(self.config.pids_limit.to_string());
        cmd.args(["--ulimit", "nofile=1024:1024", "--ulimit", "core=0:0"]);
        cmd.arg("--ulimit")
            .arg(format!("fsize={0}:{0}", self.config.max_file_bytes));
        if let Some(runtime) = &self.runtime {
            cmd.arg("--runtime").arg(runtime);
        }

        // Resource limits
        if let Some(ref mem) = self.config.memory_limit {
            cmd.arg("--memory").arg(mem);
            // Also set memory-swap equal to memory to disable swap
            cmd.arg("--memory-swap").arg(mem);
        }
        if let Some(cpu) = self.config.cpu_limit {
            cmd.arg("--cpus").arg(cpu.to_string());
        }

        // Network isolation
        cmd.arg("--network").arg(&self.config.network_mode);

        // Working directory
        cmd.arg("--workdir").arg(&self.config.working_dir);

        // Read-only root filesystem for security
        cmd.arg("--read-only");
        // But allow /tmp for scratch space
        cmd.arg("--tmpfs")
            .arg("/tmp:rw,noexec,nosuid,nodev,size=100m");
        if !self
            .config
            .volumes
            .iter()
            .any(|mount| mount.split(':').nth(1) == Some(self.config.working_dir.as_str()))
        {
            cmd.arg("--tmpfs").arg(format!(
                "{}:rw,nosuid,nodev,size=100m,uid={},gid={}",
                self.config.working_dir,
                self.config.user.split(':').next().unwrap(),
                self.config.user.split(':').nth(1).unwrap()
            ));
        }

        // No new privileges
        cmd.arg("--security-opt").arg("no-new-privileges");

        // No Linux capabilities are granted to the payload.
        cmd.arg("--cap-drop").arg("ALL");

        protocol::validate_environment(env)?;

        // Volume mounts
        for vol in &self.config.volumes {
            cmd.arg("--mount").arg(canonical_mount(vol)?);
        }

        // Extra flags
        for flag in &self.config.extra_flags {
            cmd.arg(flag);
        }

        // Image and command
        cmd.arg("--entrypoint").arg(&self.config.shell);
        cmd.arg(&self.config.image);
        cmd.arg("-c");
        cmd.arg(code);

        // Stdio
        cmd.kill_on_drop(true);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        Ok(cmd)
    }

    /// An excess byte is an execution failure, so a child cannot keep running
    /// after filling a pipe that the parent has stopped draining.
    async fn read_limited_output<R: AsyncReadExt + Unpin>(
        reader: &mut R,
        max_bytes: usize,
    ) -> anyhow::Result<String> {
        let mut output = Vec::new();
        let mut buffer = [0; 8192];
        loop {
            let n = reader.read(&mut buffer).await?;
            if n == 0 {
                return Ok(String::from_utf8_lossy(&output).into_owned());
            }
            if n > max_bytes.saturating_sub(output.len()) {
                anyhow::bail!("Docker output exceeded {max_bytes} bytes");
            }
            output.extend_from_slice(&buffer[..n]);
        }
    }

    async fn collect_with_input(
        command: &mut Command,
        limit: usize,
        input: Option<Vec<u8>>,
    ) -> anyhow::Result<(std::process::ExitStatus, String, String)> {
        command
            .kill_on_drop(true)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn()?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("missing Docker stdout"))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("missing Docker stderr"))?;
        let stdin = child.stdin.take();
        let write_input = async move {
            use tokio::io::AsyncWriteExt;
            if let (Some(mut stdin), Some(input)) = (stdin, input) {
                stdin.write_all(&input).await?;
                stdin.shutdown().await?;
            }
            Ok::<(), anyhow::Error>(())
        };
        let result = tokio::try_join!(
            write_input,
            Self::read_limited_output(&mut stdout, limit),
            Self::read_limited_output(&mut stderr, limit)
        );
        match result {
            Ok(((), out, err)) => Ok((child.wait().await?, out, err)),
            Err(e) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                Err(e)
            }
        }
    }

    async fn create_container(
        &self,
        code: &str,
        env: &HashMap<String, String>,
        deadline: std::time::Instant,
        lifetime: std::time::Instant,
        terminal: bool,
    ) -> anyhow::Result<ContainerLease> {
        let identity = uuid::Uuid::new_v4();
        let name = format!("symbi-{identity}");
        let create = self.build_command(&name, code, env, terminal)?;
        let arguments = create
            .as_std()
            .get_args()
            .map(|arg| {
                arg.to_str()
                    .map(str::to_owned)
                    .ok_or_else(|| anyhow::anyhow!("Docker arguments must be UTF-8"))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let now = std::time::Instant::now();
        let request = protocol::Create {
            staging: self.config.staging.clone(),
            version: protocol::VERSION,
            implementation: protocol::IMPLEMENTATION.into(),
            lease: identity,
            name,
            docker_binary: supervisor::resolve_binary(Path::new(&self.config.docker_binary))?,
            docker_environment: supervisor::daemon_environment(),
            arguments,
            environment: env.clone(),
            lifetime_ms: lifetime.saturating_duration_since(now).as_millis() as u64,
            startup_ms: deadline.saturating_duration_since(now).as_millis() as u64,
            resources: symbi_sandbox_supervisor::admission::WorkerResources::docker(
                self.config
                    .memory_limit
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("missing memory limit"))?,
                self.config
                    .cpu_limit
                    .ok_or_else(|| anyhow::anyhow!("missing CPU limit"))?,
            )?,
        };
        request.validate()?;
        let binary = request.docker_binary.clone();
        let environment = request.docker_environment.clone();
        let mut owner =
            supervisor::Lease::register(&self.config.supervisor, request, deadline).await?;
        match owner.created(deadline).await {
            Ok(id) => Ok(ContainerLease {
                id,
                owner,
                binary,
                environment,
            }),
            Err(error) => {
                if let Err(cleanup) = owner.finish().await {
                    anyhow::bail!("{error}; {cleanup}");
                }
                Err(error)
            }
        }
    }

    async fn supervise(
        self,
        code: String,
        env: HashMap<String, String>,
        mut cancelled: oneshot::Receiver<()>,
        registration: Option<super::command_cleanup::Registration>,
        input: Option<Vec<u8>>,
    ) -> anyhow::Result<ExecutionResult> {
        let started = std::time::Instant::now();
        let deadline = started + self.config.max_execution_time;
        let mut lease = match self
            .create_container(&code, &env, deadline, deadline, false)
            .await
        {
            Ok(lease) => lease,
            Err(error) => {
                if error
                    .downcast_ref::<supervisor::CreationRefused>()
                    .is_some()
                {
                    if let Some(owner) = registration {
                        owner.finish(Ok(()));
                    }
                }
                return Err(error);
            }
        };
        let stop = registration
            .as_ref()
            .map(|owner| owner.stop.clone())
            .unwrap_or_default();
        let result = async {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() { anyhow::bail!("Docker execution timed out before start"); }
            let mut start = lease.attach();
            let (status, stdout, stderr) = tokio::select! {
                biased;
                _ = &mut cancelled => anyhow::bail!("Docker execution cancelled"),
                _ = stop.cancelled() => anyhow::bail!("Docker execution run cancelled"),
                output = timeout(remaining, Self::collect_with_input(&mut start, self.config.max_output_bytes, input)) => {
                    output.map_err(|_| anyhow::anyhow!("Docker execution timed out"))??
                }
            };
            Ok(ExecutionResult {
                stdout, stderr, exit_code: status.code().unwrap_or(-1), success: status.success(),
                execution_time_ms: started.elapsed().as_millis() as u64,
                stdout_truncated: false, stderr_truncated: false,
            })
        }.await;
        let cleanup = lease.remove().await;
        if let Some(owner) = registration {
            owner.finish(
                cleanup
                    .as_ref()
                    .map(|_| ())
                    .map_err(|error| error.to_string()),
            );
        }
        cleanup?;
        result
    }

    /// Create and inspect before exposing a bidirectional transport. A detached
    /// initialization task owns cleanup even when the caller cancels mid-create.
    #[cfg(any(
        feature = "mcp-client",
        feature = "toolclad-session",
        feature = "cli-executor"
    ))]
    pub(crate) async fn spawn_stdio(
        &self,
        code: String,
        env: HashMap<String, String>,
        budget: Duration,
        startup: Duration,
        terminal: bool,
        cancellation: Option<tokio::sync::watch::Receiver<bool>>,
    ) -> anyhow::Result<StdioContainer> {
        self.config.validate()?;
        let registration = super::command_cleanup::register().map_err(anyhow::Error::msg)?;
        let run_stop = registration
            .as_ref()
            .map(|owner| owner.stop.clone())
            .unwrap_or_default();
        let now = std::time::Instant::now();
        let deadline = now
            .checked_add(budget.min(self.config.max_execution_time))
            .ok_or_else(|| anyhow::anyhow!("container lifetime exceeds supported range"))?;
        let startup_deadline = now
            .checked_add(startup)
            .ok_or_else(|| anyhow::anyhow!("container startup deadline exceeds supported range"))?
            .min(deadline);
        let runner = self.clone();
        let (keep_alive, mut cancelled) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let mut lease = runner
                .create_container(&code, &env, startup_deadline, deadline, terminal)
                .await?;
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero()
                || run_stop.is_cancelled()
                || cancellation
                    .as_ref()
                    .is_some_and(|cancelled| *cancelled.borrow())
                || !matches!(
                    cancelled.try_recv(),
                    Err(oneshot::error::TryRecvError::Empty)
                )
            {
                let cleanup = lease.remove().await;
                if let Some(owner) = &registration {
                    owner.finish(cleanup.as_ref().map(|_| ()).map_err(ToString::to_string));
                }
                cleanup?;
                anyhow::bail!("Docker stdio initialization cancelled or timed out");
            }
            let child = match lease.attach().spawn() {
                Ok(child) => child,
                Err(error) => {
                    let cleanup = lease.remove().await;
                    if let Some(owner) = &registration {
                        owner.finish(cleanup.as_ref().map(|_| ()).map_err(ToString::to_string));
                    }
                    cleanup?;
                    return Err(error.into());
                }
            };
            let (stop, stopped) = oneshot::channel::<()>();
            let cleanup = tokio::spawn(async move {
                tokio::select! {
                    _ = stopped => {},
                    _ = run_stop.cancelled() => {},
                    _ = wait_for_stdio_cancellation(cancellation) => {},
                    _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {},
                }
                let result = lease.remove().await;
                if let Some(owner) = &registration {
                    owner.finish(result.as_ref().map(|_| ()).map_err(ToString::to_string));
                }
                result
            });
            Ok(StdioContainer {
                child,
                guard: StdioContainerGuard {
                    stop: Some(stop),
                    cleanup: Some(cleanup),
                    output_limit: runner.config.max_output_bytes,
                },
            })
        });
        let result = task
            .await
            .map_err(|e| anyhow::anyhow!("Docker stdio initialization failed: {e}"))?;
        drop(keep_alive);
        result
    }
}

#[cfg(any(
    feature = "mcp-client",
    feature = "toolclad-session",
    feature = "cli-executor"
))]
async fn wait_for_stdio_cancellation(cancellation: Option<tokio::sync::watch::Receiver<bool>>) {
    if let Some(mut cancelled) = cancellation {
        // A startup-only sender is dropped on successful attachment. Only its
        // explicit true signal cancels; the run owner and guard retain cleanup.
        let signalled = cancelled.wait_for(|cancelled| *cancelled).await.is_ok();
        if signalled {
            return;
        }
    }
    std::future::pending::<()>().await;
}

/// The child owns the Docker attachment; the guard owns the cleanup request.
/// Dropping the guard signals an independent owner which retains the lease.
/// The external supervisor retains ownership after runtime process loss.
#[cfg(any(
    feature = "mcp-client",
    feature = "toolclad-session",
    feature = "cli-executor"
))]
pub(crate) struct StdioContainer {
    pub child: tokio::process::Child,
    pub guard: StdioContainerGuard,
}

#[cfg(any(
    feature = "mcp-client",
    feature = "toolclad-session",
    feature = "cli-executor"
))]
pub(crate) struct StdioContainerGuard {
    stop: Option<oneshot::Sender<()>>,
    cleanup: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
    pub output_limit: usize,
}

#[cfg(any(
    feature = "mcp-client",
    feature = "toolclad-session",
    feature = "cli-executor"
))]
impl StdioContainerGuard {
    pub async fn finish(&mut self) -> anyhow::Result<()> {
        self.stop.take();
        if let Some(cleanup) = self.cleanup.take() {
            cleanup
                .await
                .map_err(|e| anyhow::anyhow!("Docker stdio cleanup supervisor failed: {e}"))??;
        }
        Ok(())
    }
}

struct ContainerLease {
    id: String,
    owner: supervisor::Lease,
    binary: std::path::PathBuf,
    environment: HashMap<String, String>,
}

impl ContainerLease {
    fn attach(&self) -> Command {
        let mut command = Command::new(&self.binary);
        command
            .env_clear()
            .envs(&self.environment)
            .args(["start", "--attach", "--interactive", &self.id])
            .kill_on_drop(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    async fn remove(&mut self) -> anyhow::Result<()> {
        self.owner.finish().await
    }
}

impl DockerRunner {
    pub(crate) async fn execute_with_input(
        &self,
        code: &str,
        env: HashMap<String, String>,
        input: Option<Vec<u8>>,
    ) -> anyhow::Result<ExecutionResult> {
        self.config.validate()?;
        let registration = super::command_cleanup::register().map_err(anyhow::Error::msg)?;
        let (keep_alive, cancelled) = oneshot::channel();
        let runner = self.clone();
        let code = code.to_owned();
        // Dropping the caller closes keep_alive. The supervisor remains alive
        // to stop/remove the container and reap the Docker CLI before exiting.
        let task = tokio::spawn(async move {
            runner
                .supervise(code, env, cancelled, registration, input)
                .await
        });
        let result = task
            .await
            .map_err(|e| anyhow::anyhow!("Docker supervisor failed: {e}"))?;
        drop(keep_alive);
        result
    }
}

#[async_trait]
impl SandboxRunner for DockerRunner {
    async fn execute(
        &self,
        code: &str,
        env: HashMap<String, String>,
    ) -> anyhow::Result<ExecutionResult> {
        self.execute_with_input(code, env, None).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(
        feature = "mcp-client",
        feature = "toolclad-session",
        feature = "cli-executor"
    ))]
    #[tokio::test]
    async fn stdio_startup_sender_closure_is_distinct_from_explicit_cancellation() {
        let (sender, receiver) = tokio::sync::watch::channel(false);
        drop(sender);
        assert!(tokio::time::timeout(
            Duration::from_millis(5),
            wait_for_stdio_cancellation(Some(receiver))
        )
        .await
        .is_err());
        let (sender, receiver) = tokio::sync::watch::channel(true);
        drop(sender);
        tokio::time::timeout(
            Duration::from_secs(1),
            wait_for_stdio_cancellation(Some(receiver)),
        )
        .await
        .unwrap();
        let (sender, receiver) = tokio::sync::watch::channel(false);
        let waiting = tokio::spawn(wait_for_stdio_cancellation(Some(receiver)));
        sender.send_replace(true);
        tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .unwrap()
            .unwrap();
    }

    #[test]
    fn test_default_config() {
        let config = DockerConfig::default();
        assert_eq!(config.image, "python:3.12-slim");
        assert_eq!(config.network_mode, "none");
        assert_eq!(config.memory_limit, Some("512m".to_string()));
        assert!(config.auto_remove);
    }

    #[test]
    fn production_network_contract() {
        const CHILD: &str = "SYMBI_NETWORK_CONTRACT_CHILD";
        if let Ok(expected) = std::env::var(CHILD) {
            let allowed = expected == "allowed";
            let config = DockerConfig::default().with_network();
            assert_eq!(config.validate().is_ok(), allowed);
            assert!(DockerConfig::default().validate().is_ok());

            // Selected profiles and deserialized configurations share validation.
            for tier in ["docker", "gvisor"] {
                let root = tempfile::tempdir().unwrap();
                let table = if tier == "docker" {
                    "sandbox.docker"
                } else {
                    "sandbox.gvisor.docker"
                };
                std::fs::write(
                    root.path().join("symbiont.toml"),
                    format!("[sandbox]\ntier='{tier}'\n[{table}]\nnetwork_mode='bridge'\n"),
                )
                .unwrap();
                assert_eq!(
                    super::super::command::CommandBoundary::load(root.path()).is_ok(),
                    allowed,
                    "{tier}"
                );
            }

            // A retained runner must revalidate before constructing create argv.
            let runner = DockerRunner {
                config,
                runtime: None,
            };
            assert_eq!(
                runner
                    .build_command("network-contract", "true", &HashMap::new(), false)
                    .is_ok(),
                allowed
            );
            return;
        }
        let environments = [
            ("production", "denied"),
            ("prod", "denied"),
            (" PrOdUcTiOn ", "denied"),
            ("production-like", "denied"),
            ("development", "allowed"),
            ("dev", "allowed"),
            ("staging", "allowed"),
            ("test", "allowed"),
        ]
        .map(|(environment, expected)| (std::ffi::OsString::from(environment), expected));
        #[cfg(unix)]
        let environments = {
            use std::os::unix::ffi::OsStringExt;
            let mut values = environments.to_vec();
            values.push((std::ffi::OsString::from_vec(vec![0xff]), "denied"));
            values
        };
        for (environment, expected) in environments {
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "sandbox::docker::tests::production_network_contract",
                    "--nocapture",
                ])
                .env(CHILD, expected)
                .env("SYMBIONT_ENV", &environment)
                .output()
                .unwrap();
            assert!(
                child.status.success(),
                "{environment:?}: {} {}",
                String::from_utf8_lossy(&child.stdout),
                String::from_utf8_lossy(&child.stderr)
            );
        }
    }

    #[test]
    fn test_config_builder() {
        let data = tempfile::tempdir().unwrap();
        let mount = format!("{}:/data:ro", data.path().display());
        let config = DockerConfig::for_image("node:20-alpine")
            .with_network()
            .with_memory("1g")
            .with_cpu(2.0)
            .with_volume(&mount)
            .expect("safe volume");

        assert_eq!(config.image, "node:20-alpine");
        assert_eq!(config.network_mode, "bridge");
        assert_eq!(config.memory_limit, Some("1g".to_string()));
        assert_eq!(config.cpu_limit, Some(2.0));
        assert_eq!(config.volumes, vec![mount]);
    }

    #[test]
    fn test_with_volume_refuses_docker_socket() {
        let cfg = DockerConfig::for_image("x").with_volume("/var/run/docker.sock:/sock");
        assert!(cfg.is_err());
    }

    #[test]
    fn test_with_volume_refuses_host_etc() {
        let cfg = DockerConfig::for_image("x").with_volume("/etc:/data:ro");
        assert!(cfg.is_err());
    }

    #[test]
    fn test_with_volume_refuses_traversal() {
        let cfg = DockerConfig::for_image("x").with_volume("/home/../etc:/mnt");
        assert!(cfg.is_err());
    }

    #[test]
    fn test_with_volume_refuses_proc() {
        let cfg = DockerConfig::for_image("x").with_volume("/proc:/proc");
        assert!(cfg.is_err());
    }

    #[test]
    fn test_with_volume_refuses_named_volume() {
        let cfg = DockerConfig::for_image("x").with_volume("myvol:/data");
        assert!(cfg.is_err());
    }

    #[test]
    fn test_with_volume_refuses_empty_container() {
        let cfg = DockerConfig::for_image("x").with_volume("/data");
        assert!(cfg.is_err());
    }

    #[test]
    fn test_validate_refuses_injected_dangerous_volume() {
        // Volumes pushed directly around the builder must still be caught
        // by validate().
        let mut config = DockerConfig::for_image("x");
        config
            .volumes
            .push("/var/run/docker.sock:/sock".to_string());
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_empty_image_rejected() {
        let config = DockerConfig {
            image: String::new(),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_command_building() {
        let config = DockerConfig::default();
        let runner = DockerRunner {
            config,
            runtime: None,
        };

        let mut env = HashMap::new();
        env.insert("FOO".to_string(), "bar".to_string());

        let cmd = runner
            .build_command("symbi-test", "echo hello", &env, false)
            .expect("build_command must succeed for valid env");
        // Verify the command is constructed (we can't easily inspect it,
        // but it shouldn't panic)
        let _ = cmd;
    }

    #[test]
    fn rejects_isolation_overrides_and_missing_limits() {
        for flag in [
            "--privileged",
            "--runtime=runc",
            "--network=host",
            "--pid=host",
            "--mount=type=bind,src=/,dst=/host",
            "--entrypoint=other",
            "--env-file=/tmp/secrets",
            "--user=0",
            "--label=ai.symbiont.lease=another-owner",
            "--label=ai.symbiont.lease",
        ] {
            let config = DockerConfig {
                extra_flags: vec![flag.into()],
                ..Default::default()
            };
            assert!(config.validate().is_err(), "{flag}");
        }
        assert!(DockerConfig {
            memory_limit: None,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(DockerConfig {
            cpu_limit: Some(f64::NAN),
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(DockerConfig {
            pids_limit: 0,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(DockerConfig {
            user: "0:0".into(),
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(DockerConfig {
            max_execution_time: Duration::ZERO,
            ..Default::default()
        }
        .validate()
        .is_err());
    }

    #[test]
    fn supervisor_state_and_ancestors_cannot_be_mounted() {
        assert!(serde_json::from_value::<DockerConfig>(serde_json::json!({
            "staging": [uuid::Uuid::new_v4()]
        }))
        .is_err());
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("private-state");
        std::fs::create_dir(&state).unwrap();
        let child = state.join("nested");
        std::fs::create_dir(&child).unwrap();
        for source in [dir.path(), state.as_path(), child.as_path()] {
            let mut config = DockerConfig::default();
            config.supervisor.state_dir = state.clone();
            config.volumes = vec![format!("{}:/workspace:ro", source.display())];
            assert!(config
                .validate()
                .unwrap_err()
                .to_string()
                .contains("supervisor"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_mount_symlink_to_protected_path_and_special_files() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("alias");
        std::os::unix::fs::symlink("/etc", &link).unwrap();
        assert!(DockerConfig::default()
            .with_volume(&format!("{}:/data:ro", link.display()))
            .is_err());
        assert!(DockerConfig::default()
            .with_volume("/dev/null:/data")
            .is_err());
        assert!(DockerConfig::default()
            .with_volume("/tmp:/data:rshared")
            .is_err());
    }

    // Run explicitly with --ignored. An unavailable Docker daemon/image fails
    // these tests; it is never counted as successful containment evidence.
    #[cfg(target_os = "linux")]
    mod e2e {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        fn fixture() -> (DockerRunner, tempfile::TempDir, String) {
            let dir = tempfile::tempdir().unwrap();
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
            let label = format!("symbi.e2e={}", uuid::Uuid::new_v4());
            let config = DockerConfig {
                image: "python:3.12-slim".into(),
                max_execution_time: Duration::from_secs(20),
                memory_limit: Some("64m".into()),
                cpu_limit: Some(0.5),
                pids_limit: 16,
                extra_flags: vec![format!("--label={label}")],
                ..Default::default()
            }
            .with_volume(&format!("{}:/workspace:rw", dir.path().display()))
            .unwrap();
            (
                DockerRunner::new(config).expect("Docker and cached python:3.12-slim are required"),
                dir,
                label,
            )
        }

        async fn containers(label: &str) -> Vec<String> {
            let output = Command::new("docker")
                .args([
                    "ps",
                    "--all",
                    "--quiet",
                    "--filter",
                    &format!("label={label}"),
                ])
                .output()
                .await
                .unwrap();
            assert!(
                output.status.success(),
                "Docker inspection failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout)
                .unwrap()
                .lines()
                .map(str::to_owned)
                .collect()
        }

        async fn await_cleanup(label: &str) {
            timeout(Duration::from_secs(15), async {
                loop {
                    if containers(label).await.is_empty() {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
            .await
            .expect("container survived completion/cancellation");
        }

        async fn await_marker(path: &Path) {
            timeout(Duration::from_secs(10), async {
                while !path.exists() {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("real workload did not start");
        }

        #[tokio::test]
        #[ignore = "requires local Docker and cached python:3.12-slim"]
        async fn allowed_io_and_host_file_credential_network_denial() {
            let (mut runner, dir, label) = fixture();
            let input = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(input.path(), b"allowed-input").unwrap();
            std::fs::set_permissions(input.path(), std::fs::Permissions::from_mode(0o666)).unwrap();
            runner
                .config
                .volumes
                .push(format!("{}:/input", input.path().display()));
            let canary = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(canary.path(), b"synthetic-host-only-canary").unwrap();
            let listener = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let network = Command::new("docker")
                .args([
                    "network",
                    "inspect",
                    "bridge",
                    "--format",
                    "{{(index .IPAM.Config 0).Gateway}}",
                ])
                .output()
                .await
                .unwrap();
            assert!(network.status.success());
            let gateway = String::from_utf8(network.stdout).unwrap().trim().to_owned();
            let script = r#"python - <<'PY'
import json, os, pathlib, socket
assert os.getuid() != 0
assert pathlib.Path('/input').read_text() == 'allowed-input'
try:
    pathlib.Path('/input').write_text('forbidden-change')
    raise AssertionError('input mount is writable')
except OSError:
    pass
assert os.environ['EXPLICIT_CANARY'] == 'synthetic-allowed-value'
assert 'SYMBI_DOCKER_AMBIENT_CANARY' not in os.environ
assert not pathlib.Path(os.environ['HOST_CANARY_PATH']).exists()
assert not pathlib.Path('/var/run/docker.sock').exists()
pathlib.Path('/workspace/result').write_text('allowed-work-complete')
pathlib.Path('/tmp/scratch').write_text('scratch')
try:
    pathlib.Path('/rootfs-write').write_text('forbidden')
    raise AssertionError('writable root filesystem')
except OSError:
    pass
sock = socket.socket()
sock.settimeout(0.5)
try:
    sock.connect((os.environ['SINK_HOST'], int(os.environ['SINK_PORT'])))
    sock.sendall(b'synthetic-escape')
    raise AssertionError('egress reached host')
except OSError:
    pass
finally:
    sock.close()
print(json.dumps({'allowed': True, 'host_file': False, 'network': False}))
PY"#;
            // The test process receives the ambient canary from the test driver,
            // avoiding mutations of the process-wide environment during tests.
            assert_eq!(
                std::env::var("SYMBI_DOCKER_AMBIENT_CANARY").unwrap(),
                "synthetic-ambient-value"
            );
            let env = HashMap::from([
                ("EXPLICIT_CANARY".into(), "synthetic-allowed-value".into()),
                (
                    "HOST_CANARY_PATH".into(),
                    canary.path().to_str().unwrap().into(),
                ),
                ("SINK_HOST".into(), gateway),
                (
                    "SINK_PORT".into(),
                    listener.local_addr().unwrap().port().to_string(),
                ),
            ]);
            let result = runner.execute(script, env).await.unwrap();
            assert!(result.success, "{result:?}");
            assert_eq!(
                std::fs::read_to_string(dir.path().join("result")).unwrap(),
                "allowed-work-complete"
            );
            assert_eq!(
                std::fs::read(canary.path()).unwrap(),
                b"synthetic-host-only-canary"
            );
            assert_eq!(
                listener.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
            assert!(
                serde_json::from_str::<serde_json::Value>(&result.stdout).unwrap()["allowed"]
                    .as_bool()
                    .unwrap()
            );
            assert!(containers(&label).await.is_empty());
        }

        #[tokio::test]
        #[ignore = "requires local Docker and cached python:3.12-slim"]
        async fn image_volume_is_rejected_before_payload_execution() {
            let (mut runner, dir, label) = fixture();
            let context = tempfile::tempdir().unwrap();
            std::fs::write(
                context.path().join("Dockerfile"),
                "FROM python:3.12-slim\nVOLUME /unbounded\n",
            )
            .unwrap();
            let tag = format!("symbi-e2e-volume:{}", uuid::Uuid::new_v4());
            let build = Command::new("docker")
                .args(["build", "--network=none", "--pull=false", "--tag", &tag])
                .arg(context.path())
                .output()
                .await
                .unwrap();
            assert!(
                build.status.success(),
                "{}",
                String::from_utf8_lossy(&build.stderr)
            );
            runner.config.image = tag.clone();
            let result = runner
                .execute("touch /workspace/unexpected", HashMap::new())
                .await;
            let removed = Command::new("docker")
                .args(["image", "rm", &tag])
                .output()
                .await
                .unwrap();
            assert!(removed.status.success(), "test image cleanup failed");
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("unapproved volume"));
            assert!(!dir.path().join("unexpected").exists());
            assert!(containers(&label).await.is_empty());
        }

        const DETACHED_WORKER: &str = r#"python - <<'PY'
import os, pathlib, time
pid = os.fork()
if pid == 0:
    os.setsid()
    pathlib.Path('/workspace/ready').write_text('started')
    for n in range(200):
        pathlib.Path('/workspace/ticks').write_text(str(n))
        time.sleep(0.1)
    os._exit(0)
os.waitpid(pid, 0)
PY"#;

        #[tokio::test]
        #[ignore = "requires local Docker and cached python:3.12-slim"]
        async fn timeout_and_cancellation_remove_detached_descendants() {
            for cancel in [false, true] {
                let (mut runner, dir, label) = fixture();
                runner.config.max_execution_time = if cancel {
                    Duration::from_secs(20)
                } else {
                    Duration::from_secs(5)
                };
                let task =
                    tokio::spawn(
                        async move { runner.execute(DETACHED_WORKER, HashMap::new()).await },
                    );
                await_marker(&dir.path().join("ready")).await;
                assert_eq!(containers(&label).await.len(), 1);
                if cancel {
                    task.abort();
                    assert!(task.await.unwrap_err().is_cancelled());
                } else {
                    assert!(task
                        .await
                        .unwrap()
                        .unwrap_err()
                        .to_string()
                        .contains("timed out"));
                }
                await_cleanup(&label).await;
                let ticks = std::fs::read(dir.path().join("ticks")).unwrap();
                tokio::time::sleep(Duration::from_millis(250)).await;
                assert_eq!(
                    std::fs::read(dir.path().join("ticks")).unwrap(),
                    ticks,
                    "descendant continued writing after cleanup"
                );
            }
        }

        #[tokio::test]
        #[ignore = "requires local Docker and cached python:3.12-slim"]
        async fn stream_limits_fail_and_remove_worker() {
            for (fd, count, allowed) in [(1, 4096, true), (1, 4097, false), (2, 4097, false)] {
                let (mut runner, _dir, label) = fixture();
                runner.config.max_output_bytes = 4096;
                let script = format!(
                    "python -c 'import os,time; os.write({fd}, b\"x\"*{count}); {}'",
                    if allowed { "pass" } else { "time.sleep(20)" }
                );
                let result = runner.execute(&script, HashMap::new()).await;
                if allowed {
                    let result = result.unwrap();
                    assert!(result.success, "{result:?}");
                    assert_eq!(result.stdout.len(), count);
                } else {
                    assert!(result.unwrap_err().to_string().contains("output exceeded"));
                }
                assert!(containers(&label).await.is_empty());
            }
        }

        #[tokio::test]
        #[ignore = "requires local Docker with cgroup v2 and cached python:3.12-slim"]
        async fn kernel_resource_limits_and_failed_exit_are_observed() {
            let (runner, _dir, label) = fixture();
            let script = r#"python - <<'PY'
import errno, os, pathlib, signal
cgroup = pathlib.Path('/sys/fs/cgroup')
assert int((cgroup / 'memory.max').read_text()) == 64 * 1024 * 1024
assert int((cgroup / 'pids.max').read_text()) == 16
quota, period = map(int, (cgroup / 'cpu.max').read_text().split())
assert quota / period == 0.5
children = []
try:
    for _ in range(32):
        try:
            pid = os.fork()
        except OSError as e:
            assert e.errno == errno.EAGAIN
            break
        if pid == 0:
            signal.pause()
            os._exit(0)
        children.append(pid)
    else:
        raise AssertionError('process quota was not enforced')
finally:
    for pid in children:
        os.kill(pid, signal.SIGKILL)
    for pid in children:
        os.waitpid(pid, 0)
print('resource-limits-enforced')
PY"#;
            let result = runner.execute(script, HashMap::new()).await.unwrap();
            assert!(result.success, "{result:?}");
            assert!(result.stdout.contains("resource-limits-enforced"));
            let failed = runner.execute("exit 17", HashMap::new()).await.unwrap();
            assert!(!failed.success);
            assert_eq!(failed.exit_code, 17);
            let oom = runner
                .execute("python -c 'a=bytearray(128*1024*1024)'", HashMap::new())
                .await
                .unwrap();
            assert!(
                !oom.success,
                "allocation exceeded container memory ceiling: {oom:?}"
            );
            assert!(containers(&label).await.is_empty());
        }
    }
}
