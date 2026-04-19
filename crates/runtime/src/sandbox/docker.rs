//! Docker container sandbox runner
//!
//! Executes code inside Docker containers for isolated agent execution.
//! Uses the `docker` CLI rather than a Rust Docker client library to
//! minimize dependencies and match the pattern of the native runner.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use super::{ExecutionResult, SandboxRunner};

/// Default maximum output size in bytes (10 MB)
const DEFAULT_MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024;

/// Configuration for Docker sandbox execution
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Network mode: "none" (isolated), "bridge" (default Docker), "host"
    pub network_mode: String,
    /// Extra volumes to mount (host_path:container_path:mode)
    pub volumes: Vec<String>,
    /// Working directory inside the container
    pub working_dir: String,
    /// Whether to remove the container after execution
    pub auto_remove: bool,
    /// Docker binary path (default: "docker")
    pub docker_binary: String,
    /// Maximum output bytes per stream before truncation
    pub max_output_bytes: usize,
    /// Extra docker run flags
    pub extra_flags: Vec<String>,
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
            extra_flags: Vec::new(),
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

    /// Create a config with network access enabled
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
        if self.image.is_empty() {
            anyhow::bail!("Docker image must not be empty");
        }
        // Validate every volume mount in case it was pushed directly into
        // `volumes` (e.g. by deserializing a config file rather than going
        // through `with_volume`).
        for vol in &self.volumes {
            validate_volume_mount(vol)?;
        }
        // Block host network in production
        if self.network_mode == "host" {
            if let Ok(env) = std::env::var("SYMBIONT_ENV") {
                if env.eq_ignore_ascii_case("production") {
                    anyhow::bail!("SECURITY: host network mode is disabled in production");
                }
            }
            tracing::warn!("SECURITY: Docker host network mode provides no network isolation");
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
    "/proc",
    "/sys",
    "/boot",
    "/etc",
    "/root",
    "/var/lib/docker",
    "/var/lib/kubelet",
    "/var/lib/rancher",
];

/// Validate a docker `-v host:container[:mode]` string.
fn validate_volume_mount(mount: &str) -> Result<(), anyhow::Error> {
    let mut parts = mount.splitn(3, ':');
    let host = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("empty volume mount"))?;
    let _container = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("volume mount {:?} missing container path", mount))?;

    if host.is_empty() {
        anyhow::bail!("volume mount {:?} has empty host path", mount);
    }

    // Named volume references (no slash, not absolute) are left to docker;
    // we only police host-path mounts.
    if !host.starts_with('/') {
        return Ok(());
    }

    // Normalize lazily by rejecting `..` segments; canonicalize() would
    // require the path to exist, which isn't guaranteed at config time.
    if host.split('/').any(|seg| seg == "..") {
        anyhow::bail!(
            "volume mount {:?} contains '..' path segments; refusing",
            mount
        );
    }

    for dangerous in DANGEROUS_HOST_PATHS {
        if host == *dangerous || host.starts_with(&format!("{}/", dangerous)) {
            anyhow::bail!(
                "volume mount {:?} targets dangerous host path {:?}; refusing",
                mount,
                dangerous
            );
        }
    }

    if host == "/" {
        anyhow::bail!("volume mount {:?} targets host root filesystem", mount);
    }

    Ok(())
}

/// Docker sandbox runner
pub struct DockerRunner {
    config: DockerConfig,
}

impl DockerRunner {
    /// Create a new Docker runner with the given configuration
    pub fn new(config: DockerConfig) -> Result<Self, anyhow::Error> {
        config.validate()?;

        // Verify docker is available
        let check = std::process::Command::new(&config.docker_binary)
            .arg("version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        match check {
            Ok(status) if status.success() => {}
            _ => {
                anyhow::bail!(
                    "Docker is not available at '{}'. Install Docker or set docker_binary path.",
                    config.docker_binary
                );
            }
        }

        tracing::info!(
            "Docker sandbox initialized: image={}, network={}, memory={:?}, cpu={:?}",
            config.image,
            config.network_mode,
            config.memory_limit,
            config.cpu_limit
        );

        Ok(Self { config })
    }

    /// Build the `docker run` command with all configuration applied
    fn build_command(&self, code: &str, env: &HashMap<String, String>) -> Command {
        let mut cmd = Command::new(&self.config.docker_binary);
        cmd.arg("run");

        // Auto-remove container after execution
        if self.config.auto_remove {
            cmd.arg("--rm");
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
        cmd.arg("--tmpfs").arg("/tmp:rw,noexec,nosuid,size=100m");

        // No new privileges
        cmd.arg("--security-opt").arg("no-new-privileges");

        // Drop all capabilities, add back only what's needed
        cmd.arg("--cap-drop").arg("ALL");

        // Environment variables
        for (key, value) in env {
            cmd.arg("-e").arg(format!("{}={}", key, value));
        }

        // Volume mounts
        for vol in &self.config.volumes {
            cmd.arg("-v").arg(vol);
        }

        // Extra flags
        for flag in &self.config.extra_flags {
            cmd.arg(flag);
        }

        // Image and command
        cmd.arg(&self.config.image);
        cmd.arg(&self.config.shell);
        cmd.arg("-c");
        cmd.arg(code);

        // Stdio
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        cmd
    }

    /// Read output with size limit (same pattern as NativeRunner)
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
impl SandboxRunner for DockerRunner {
    async fn execute(
        &self,
        code: &str,
        env: HashMap<String, String>,
    ) -> Result<ExecutionResult, anyhow::Error> {
        tracing::info!(
            "Docker sandbox executing: image={}, code_len={}, env_count={}",
            self.config.image,
            code.len(),
            env.len()
        );

        let mut command = self.build_command(code, &env);
        let start = std::time::Instant::now();
        let max_output = self.config.max_output_bytes;

        let mut child = command.spawn().map_err(|e| {
            anyhow::anyhow!("Failed to spawn docker process: {}. Is Docker running?", e)
        })?;

        let mut child_stdout = child.stdout.take();
        let mut child_stderr = child.stderr.take();

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

                tracing::info!(
                    "Docker execution completed: exit_code={}, success={}, duration={:?}",
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
                tracing::error!("Docker execution failed: {}", e);
                Err(anyhow::anyhow!("Docker execution failed: {}", e))
            }
            Err(_) => {
                // Timeout — kill the container
                let _ = child.kill().await;
                tracing::error!(
                    "Docker execution timed out after {:?}",
                    self.config.max_execution_time
                );
                Err(anyhow::anyhow!(
                    "Docker execution timed out after {:?}",
                    self.config.max_execution_time
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = DockerConfig::default();
        assert_eq!(config.image, "python:3.12-slim");
        assert_eq!(config.network_mode, "none");
        assert_eq!(config.memory_limit, Some("512m".to_string()));
        assert!(config.auto_remove);
    }

    #[test]
    fn test_config_builder() {
        let config = DockerConfig::for_image("node:20-alpine")
            .with_network()
            .with_memory("1g")
            .with_cpu(2.0)
            .with_volume("/data:/data:ro")
            .expect("safe volume");

        assert_eq!(config.image, "node:20-alpine");
        assert_eq!(config.network_mode, "bridge");
        assert_eq!(config.memory_limit, Some("1g".to_string()));
        assert_eq!(config.cpu_limit, Some(2.0));
        assert_eq!(config.volumes, vec!["/data:/data:ro"]);
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
    fn test_with_volume_allows_named_volume() {
        let cfg = DockerConfig::for_image("x").with_volume("myvol:/data");
        assert!(cfg.is_ok());
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
        config.volumes.push("/var/run/docker.sock:/sock".to_string());
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
        let runner = DockerRunner { config };

        let mut env = HashMap::new();
        env.insert("FOO".to_string(), "bar".to_string());

        let cmd = runner.build_command("echo hello", &env);
        // Verify the command is constructed (we can't easily inspect it,
        // but it shouldn't panic)
        let _ = cmd;
    }

    // Integration tests that require Docker are below.
    // They're ignored by default since CI may not have Docker.

    #[tokio::test]
    #[ignore] // Requires Docker
    async fn test_docker_runner_creation() {
        let config = DockerConfig::for_image("alpine:latest");
        let runner = DockerRunner::new(config);
        // Only passes if Docker is installed
        if runner.is_err() {
            eprintln!("Skipping: Docker not available");
            return;
        }
        assert!(runner.is_ok());
    }

    #[tokio::test]
    #[ignore] // Requires Docker
    async fn test_docker_execution() {
        let config = DockerConfig::for_image("alpine:latest");
        let runner = match DockerRunner::new(config) {
            Ok(r) => r,
            Err(_) => return, // Docker not available
        };

        let result = runner
            .execute("echo 'Hello from Docker!'", HashMap::new())
            .await;

        if let Ok(output) = result {
            assert!(output.success);
            assert!(output.stdout.contains("Hello from Docker!"));
        }
    }

    #[tokio::test]
    #[ignore] // Requires Docker
    async fn test_docker_env_vars() {
        let config = DockerConfig::for_image("alpine:latest");
        let runner = match DockerRunner::new(config) {
            Ok(r) => r,
            Err(_) => return,
        };

        let mut env = HashMap::new();
        env.insert("TEST_VAR".to_string(), "docker_test".to_string());

        let result = runner.execute("echo $TEST_VAR", env).await;

        if let Ok(output) = result {
            assert!(output.success);
            assert!(output.stdout.contains("docker_test"));
        }
    }

    #[tokio::test]
    #[ignore] // Requires Docker
    async fn test_docker_network_isolation() {
        let config = DockerConfig {
            image: "alpine:latest".to_string(),
            network_mode: "none".to_string(),
            ..Default::default()
        };
        let runner = match DockerRunner::new(config) {
            Ok(r) => r,
            Err(_) => return,
        };

        // This should fail since network is disabled
        let result = runner
            .execute("wget -q -O- http://example.com", HashMap::new())
            .await;

        if let Ok(output) = result {
            assert!(!output.success);
        }
    }

    #[tokio::test]
    #[ignore] // Requires Docker
    async fn test_docker_timeout() {
        let config = DockerConfig {
            image: "alpine:latest".to_string(),
            max_execution_time: Duration::from_secs(2),
            ..Default::default()
        };
        let runner = match DockerRunner::new(config) {
            Ok(r) => r,
            Err(_) => return,
        };

        let result = runner.execute("sleep 30", HashMap::new()).await;
        assert!(result.is_err());
    }
}
