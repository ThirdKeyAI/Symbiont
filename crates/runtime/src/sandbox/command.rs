//! Selected isolation boundary for executable ToolClad commands and parsers.

use super::{DockerConfig, DockerRunner, ExecutionResult, GVisorConfig, GVisorRunner};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum CommandTier {
    #[default]
    #[serde(rename = "docker", alias = "tier1")]
    Docker,
    #[serde(rename = "gvisor", alias = "tier2")]
    GVisor,
    #[serde(rename = "firecracker", alias = "tier3")]
    Firecracker,
    #[serde(rename = "landlock")]
    Landlock,
    #[serde(rename = "e2b")]
    E2B,
    #[serde(rename = "none", alias = "tier0")]
    DevelopmentHost,
}

/// Read-only and writable host ceilings, shared by every isolated backend.
/// Entries keep the `host:virtual:ro` form the file broker already parses.
///
/// Canonical location is `[sandbox.roots]`. The per-backend `source_roots` and
/// `output_roots` under `[sandbox.firecracker]` remain accepted and are lifted
/// here at load, because both structs deny unknown fields and removing them
/// would make existing configuration fail to parse rather than warn.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BoundaryRoots {
    /// Explicit host ceilings for source queries and declared inputs.
    pub source_roots: Vec<String>,
    /// Separate write-only host ceilings for new-file publication.
    pub output_roots: Vec<String>,
}

/// Operator-owned configuration, frozen before authorization. There is no
/// fallback from an unavailable isolated backend to host execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CommandBoundary {
    pub tier: CommandTier,
    pub roots: BoundaryRoots,
    #[cfg(target_os = "linux")]
    pub landlock: super::landlock::LandlockProfile,
    pub docker: DockerConfig,
    pub gvisor: GVisorConfig,
    pub firecracker: Option<super::FirecrackerConfig>,
    #[serde(skip_deserializing)]
    protected_paths: Vec<PathBuf>,
}

impl CommandBoundary {
    /// Remove all operator host mounts for a fresh operation or output parser.
    pub fn without_host_mounts(&self) -> Self {
        let mut boundary = self.clone();
        boundary.docker.volumes.clear();
        boundary.gvisor.docker.volumes.clear();
        boundary.docker.staging.clear();
        boundary.gvisor.docker.staging.clear();
        boundary.roots.source_roots.clear();
        boundary.roots.output_roots.clear();
        #[cfg(target_os = "linux")]
        {
            boundary.landlock.workspace = None;
            boundary.landlock.clear_executable();
        }
        if let Some(config) = &mut boundary.firecracker {
            config.source_roots.clear();
            config.output_roots.clear();
            #[cfg(unix)]
            {
                config.files = None;
                config.snapshot = None;
            }
        }
        boundary
    }
    /// Intersect the selected container's limits with the agent's limits.
    pub fn tighten_resources(
        &mut self,
        limits: &crate::types::ResourceLimits,
    ) -> Result<(), String> {
        if limits.memory_mb == 0
            || !limits.cpu_cores.is_finite()
            || limits.cpu_cores <= 0.0
            || limits.execution_timeout.is_zero()
        {
            return Err(
                "agent memory, CPU and execution limits must be positive and finite".into(),
            );
        }
        let container = match self.tier {
            CommandTier::Docker => Some(&mut self.docker),
            CommandTier::GVisor => Some(&mut self.gvisor.docker),
            _ => None,
        };
        if let Some(config) = container {
            let memory = config
                .memory_limit
                .as_deref()
                .ok_or("missing container memory limit")?;
            let digits = memory.trim_end_matches(['b', 'k', 'm', 'g', 'B', 'K', 'M', 'G']);
            let factor = match memory.as_bytes().last().map(u8::to_ascii_lowercase) {
                Some(b'k') => 1024,
                Some(b'm') => 1024 * 1024,
                Some(b'g') => 1024 * 1024 * 1024,
                _ => 1,
            };
            let bytes = digits
                .parse::<u64>()
                .map_err(|e| e.to_string())?
                .checked_mul(factor)
                .ok_or("container memory limit overflow")?;
            let agent_bytes = (limits.memory_mb as u64)
                .checked_mul(1024 * 1024)
                .ok_or("agent memory limit overflow")?;
            config.memory_limit = Some(bytes.min(agent_bytes).to_string());
            config.cpu_limit = Some(
                config
                    .cpu_limit
                    .unwrap_or(f64::from(limits.cpu_cores))
                    .min(f64::from(limits.cpu_cores)),
            );
            config.max_execution_time = config.max_execution_time.min(limits.execution_timeout);
        }
        #[cfg(target_os = "linux")]
        if self.tier == CommandTier::Landlock {
            self.landlock.memory_mib = self
                .landlock
                .memory_mib
                .min(u32::try_from(limits.memory_mb).unwrap_or(u32::MAX));
            self.landlock.cpu_millis = self
                .landlock
                .cpu_millis
                .min((f64::from(limits.cpu_cores) * 1000.0).floor() as u32);
            self.landlock.max_execution_time = self
                .landlock
                .max_execution_time
                .min(limits.execution_timeout);
        }
        if self.tier == CommandTier::Firecracker {
            let config = self
                .firecracker
                .as_mut()
                .ok_or("missing Firecracker configuration")?;
            if limits.cpu_cores < 1.0 {
                return Err("Firecracker CPU budgets must permit at least one vCPU".into());
            }
            config.mem_mib = config
                .mem_mib
                .min(u32::try_from(limits.memory_mb).unwrap_or(u32::MAX));
            config.vcpus = config.vcpus.min(limits.cpu_cores.floor().min(32.0) as u8);
            config.max_execution_time = config.max_execution_time.min(limits.execution_timeout);
        }
        self.validate()
    }

    pub fn development_host() -> Self {
        Self {
            tier: CommandTier::DevelopmentHost,
            ..Self::default()
        }
    }

    pub fn load(project: &Path) -> Result<Self, String> {
        Self::load_selected(project, None)
    }

    /// Apply a trusted agent definition before validating the selected backend.
    /// An unavailable project default cannot hide an explicit agent selection,
    /// and an unavailable agent selection never falls back to that default.
    pub fn load_for_agent(
        project: &Path,
        settings: &dsl::AgentExecutionSettings,
    ) -> Result<Self, String> {
        Self::load_selected(project, Some(settings))
    }

    fn load_selected(
        project: &Path,
        settings: Option<&dsl::AgentExecutionSettings>,
    ) -> Result<Self, String> {
        let project = project
            .canonicalize()
            .map_err(|e| format!("cannot resolve sandbox project: {e}"))?;
        let path = project.join("symbiont.toml");
        let mut profile = match std::fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => return Err(format!("cannot inspect sandbox configuration: {e}")),
            Ok(_) => {
                let text = std::fs::read_to_string(&path)
                    .map_err(|e| format!("cannot read sandbox configuration: {e}"))?;
                let root: toml::Value = toml::from_str(&text)
                    .map_err(|e| format!("invalid project configuration: {e}"))?;
                match root.get("sandbox") {
                    Some(value) => value
                        .clone()
                        .try_into()
                        .map_err(|e| format!("invalid sandbox configuration: {e}"))?,
                    None => Self::default(),
                }
            }
        };
        if let Some(settings) = settings {
            if let Some(tier) = &settings.sandbox_tier {
                profile.tier = match tier {
                    dsl::SandboxTier::Docker => CommandTier::Docker,
                    dsl::SandboxTier::GVisor => CommandTier::GVisor,
                    dsl::SandboxTier::Firecracker => CommandTier::Firecracker,
                    dsl::SandboxTier::E2B => CommandTier::E2B,
                };
            }
            if let Some(seconds) = settings.timeout_seconds {
                if !(1..=86400).contains(&seconds) {
                    return Err("agent timeout must be between 1 second and 1 day".into());
                }
                let bound = Duration::from_secs(seconds);
                if let Some(config) = profile.firecracker.as_mut() {
                    config.max_execution_time = config.max_execution_time.min(bound);
                }
                profile.docker.max_execution_time = profile.docker.max_execution_time.min(bound);
                profile.gvisor.docker.max_execution_time =
                    profile.gvisor.docker.max_execution_time.min(bound);
                #[cfg(target_os = "linux")]
                {
                    profile.landlock.max_execution_time =
                        profile.landlock.max_execution_time.min(bound);
                }
            }
        }
        if profile.tier == CommandTier::DevelopmentHost
            && std::env::var("SYMBIONT_ALLOW_UNISOLATED").as_deref() != Ok("1")
        {
            return Err("development host execution requires SYMBIONT_ALLOW_UNISOLATED=1".into());
        }
        for name in [
            "symbiont.toml",
            "toolclad.toml",
            "mcp-config.toml",
            "scope",
            "agents",
            "tools",
            "policies",
            ".symbiont",
            ".git",
        ] {
            let path = project.join(name);
            profile
                .protected_paths
                .push(path.canonicalize().unwrap_or(path));
        }
        profile.normalize_roots()?;
        profile.validate()?;
        Ok(profile)
    }

    /// Reconcile the canonical `[sandbox.roots]` with the legacy per-backend
    /// declaration, then push the result down so every backend sees the same
    /// ceilings. Both structs deny unknown fields, so the legacy location has
    /// to keep parsing; it is lifted rather than removed.
    fn normalize_roots(&mut self) -> Result<(), String> {
        if let Some(config) = &self.firecracker {
            let legacy_set = !config.source_roots.is_empty() || !config.output_roots.is_empty();
            let canonical_set =
                !self.roots.source_roots.is_empty() || !self.roots.output_roots.is_empty();
            if legacy_set && canonical_set {
                if config.source_roots != self.roots.source_roots
                    || config.output_roots != self.roots.output_roots
                {
                    return Err(
                        "source and output roots are declared in both [sandbox.roots] \
                                and [sandbox.firecracker] with different values; declare them once"
                            .into(),
                    );
                }
            } else if legacy_set {
                self.roots.source_roots = config.source_roots.clone();
                self.roots.output_roots = config.output_roots.clone();
            }
        }
        if let Some(config) = &mut self.firecracker {
            config.source_roots = self.roots.source_roots.clone();
            config.output_roots = self.roots.output_roots.clone();
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), String> {
        match self.tier {
            CommandTier::DevelopmentHost => {
                if crate::env::is_production().map_err(|e| e.to_string())? {
                    return Err(
                        "development host command execution is forbidden in production".into(),
                    );
                }
                Ok(())
            }
            CommandTier::Docker => self.validate_container(&self.docker),
            CommandTier::GVisor => self.validate_container(&self.gvisor.docker),
            CommandTier::Firecracker => {
                let config = self
                    .firecracker
                    .as_ref()
                    .ok_or("Firecracker tier selected without configuration")?;
                super::FirecrackerRunner::new(config.clone()).map_err(|e| e.to_string())?;
                self.validate_protected_roots(&config.source_roots)?;
                self.validate_protected_roots(&config.output_roots)
            }
            #[cfg(target_os = "linux")]
            CommandTier::Landlock => {
                self.landlock.validate()?;
                super::landlock::validate_roots(&self.landlock, &self.roots)?;
                self.validate_protected_roots(&self.roots.source_roots)?;
                self.validate_protected_roots(&self.roots.output_roots)
            }
            #[cfg(not(target_os = "linux"))]
            CommandTier::Landlock => {
                Err("the landlock tier requires Linux; no host fallback".into())
            }
            CommandTier::E2B => {
                Err("hosted command transport is unavailable; no host fallback".into())
            }
        }
    }

    fn validate_container(&self, config: &DockerConfig) -> Result<(), String> {
        config.validate().map_err(|e| e.to_string())?;
        self.validate_protected_roots(&config.volumes)
    }

    fn validate_protected_roots(&self, roots: &[String]) -> Result<(), String> {
        for mount in roots {
            let source = Path::new(mount.split(':').next().ok_or("missing mount source")?)
                .canonicalize()
                .map_err(|e| format!("cannot resolve mount: {e}"))?;
            if self
                .protected_paths
                .iter()
                .any(|p| source.starts_with(p) || p.starts_with(&source))
            {
                return Err(
                    "sandbox mount exposes protected project configuration or audit storage".into(),
                );
            }
        }
        Ok(())
    }

    /// Cedar supports this descriptor without lossy float/null conversion.
    pub fn descriptor(&self) -> Result<serde_json::Value, String> {
        self.validate()?;
        let value = serde_json::to_value(self).map_err(|e| e.to_string())?;
        let mut descriptor = serde_json::json!({
            "tier": serde_json::to_value(&self.tier).map_err(|e| e.to_string())?,
            "profile_digest": crate::reasoning::prepared::digest_json(&value)?,
        });
        let container = match self.tier {
            CommandTier::Docker => Some(&self.docker),
            CommandTier::GVisor => Some(&self.gvisor.docker),
            _ => None,
        };
        if self.tier == CommandTier::Firecracker {
            let config = self
                .firecracker
                .as_ref()
                .ok_or("missing Firecracker configuration")?;
            #[cfg(unix)]
            let snapshot = config.snapshot.as_ref().map(|s| s.descriptor());
            #[cfg(not(unix))]
            let snapshot: Option<serde_json::Value> = None;
            descriptor["vm"] = serde_json::json!({
                "kernel": config.kernel_image_path, "rootfs": config.rootfs_path,
                "rootfs_read_only": true, "vcpus": config.vcpus,
                "memory_mib": config.mem_mib, "working_dir": config.working_dir,
                "broker_source_roots": config.source_roots,
                "broker_output_roots": config.output_roots,
                "guest_protocol": symbi_sandbox_guest::VERSION,
                "git_snapshot": snapshot,
                "guest_implementation": symbi_sandbox_guest::IMPLEMENTATION,
                "pids_limit": symbi_sandbox_guest::MAX_PROCESSES,
                "open_files_limit": symbi_sandbox_guest::MAX_FILES,
                "network_interfaces": 0,
                "broker_ports": config.services.as_ref().map_or_else(Vec::new, |services| services.ports()),
                "guest_environment": {"PATH":"/usr/bin:/bin","HOME":"/tmp","LANG":"C.UTF-8"}
            });
        }
        #[cfg(target_os = "linux")]
        if self.tier == CommandTier::Landlock {
            // Record the detected ABI, not only the declared floor: a reader
            // must be able to establish what the kernel could enforce at the
            // time, rather than what the configuration asked for.
            descriptor["landlock"] = serde_json::json!({
                "abi_detected": super::landlock::detect_abi(),
                "abi_floor": self.landlock.abi_floor,
                "abi_required": self.landlock.abi_floor.max(super::landlock::MIN_ABI),
                "boundary_version": 5,
                "supervision": "delegated_cgroup_v1",
                "resources": self.landlock.resources(),
                "pids_limit": self.landlock.pids_limit,
                "max_lifetime_ms": self.landlock.max_execution_time.as_millis() as u64,
                "supervisor_state_dir": self.landlock.supervisor.resolved_state_dir().map_err(|e|e.to_string())?,
                "scopes": ["signal", "abstract_unix_socket"],
                "socket_policy": if self.landlock.workspace.as_ref().is_some_and(|w| w.has_channels()) { "private_loopback_and_inherited_brokers" } else if self.landlock.require_network { "private_unix_stream_pair_only" } else { "ip_and_private_unix_stream_pair" },
                "io_uring": "denied",
                "namespace_mutation": "denied_after_setup",
                "inherited_descriptors": if self.landlock.workspace.as_ref().is_some_and(|w| w.has_channels()) { "stdio_and_two_connected_broker_streams" } else { "stdio_only" },
                "network_restricted": self.landlock.require_network,
                "broker_source_roots": self.roots.source_roots,
                "broker_output_roots": self.roots.output_roots,
                "executable": self.landlock.executable(),
                "process_metadata": if self.landlock.executable().is_some() && self.landlock.workspace.is_some() { super::landlock::PRIMARY_PROCESS_METADATA } else { &[] },
                "workspace": self.landlock.workspace.as_ref().map(|workspace| workspace.descriptor()),
            });
        }
        if let Some(config) = container {
            descriptor["container"] = serde_json::json!({
                "image": config.image, "network_mode": config.network_mode,
                "mounts": config.volumes, "working_dir": config.working_dir,
                "memory_limit": config.memory_limit.as_deref().unwrap_or(""),
                "cpu_limit": config.cpu_limit.unwrap_or(0.0).to_string(),
                "pids_limit": config.pids_limit, "user": config.user,
                "max_execution_seconds": config.max_execution_time.as_secs(),
                "max_output_bytes": config.max_output_bytes,
                "max_file_bytes": config.max_file_bytes,
            });
        }
        Ok(descriptor)
    }

    pub async fn execute(
        &self,
        argv: &[String],
        timeout: Duration,
    ) -> Result<ExecutionResult, String> {
        self.validate()?;
        #[cfg(target_os = "linux")]
        if self.tier == CommandTier::Landlock {
            return super::landlock::workspace::execute(
                self.landlock.clone(),
                argv.to_owned(),
                timeout,
            )
            .await;
        }
        if self.tier == CommandTier::Firecracker {
            return self.execute_vm(argv, None, false, timeout).await;
        }

        if timeout.is_zero() {
            return Err("command timed out before execution".into());
        }
        let code = literal_command(argv)?;
        if self.tier == CommandTier::DevelopmentHost {
            let argv = argv.to_owned();
            return tokio::task::spawn_blocking(move || {
                let started = std::time::Instant::now();
                let output = crate::toolclad::process::run_command(
                    "development tool",
                    &argv[0],
                    &argv[1..],
                    timeout,
                )?;
                Ok(ExecutionResult {
                    exit_code: output.status.code().unwrap_or(-1),
                    success: output.status.success(),
                    stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                    execution_time_ms: started.elapsed().as_millis() as u64,
                    stdout_truncated: false,
                    stderr_truncated: false,
                })
            })
            .await
            .map_err(|e| format!("development command worker failed: {e}"))?;
        }
        self.execute_container(code, None, timeout).await
    }

    /// Preserve the parser's filename argument while transferring raw output
    /// over bounded stdin. No host file or parser binary is implicitly mounted.
    pub async fn parse(
        &self,
        path: &str,
        input: &str,
        timeout: Duration,
    ) -> Result<ExecutionResult, String> {
        self.validate()?;
        if input.len() > 10 * 1024 * 1024 {
            return Err("parser input exceeds 10 MiB".into());
        }
        #[cfg(target_os = "linux")]
        if self.tier == CommandTier::Landlock {
            return super::landlock::workspace::parse(&self.landlock, path, input, timeout).await;
        }
        if self.tier == CommandTier::Firecracker {
            return self
                .execute_vm(
                    &[path.into(), symbi_sandbox_guest::INPUT_FILE.into()],
                    Some(input.as_bytes().to_vec()),
                    true,
                    timeout,
                )
                .await;
        }
        if self.tier == CommandTier::DevelopmentHost {
            use std::io::Write;
            let mut file = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
            file.write_all(input.as_bytes())
                .map_err(|e| e.to_string())?;
            return self
                .execute(
                    &[path.into(), file.path().to_string_lossy().into_owned()],
                    timeout,
                )
                .await;
        }
        let command = literal_command(&[path.into(), "/tmp/symbi-parser-input".into()])?;
        self.without_host_mounts()
            .execute_container(
                format!("umask 077; cat > /tmp/symbi-parser-input && {command}"),
                Some(input.as_bytes().to_vec()),
                timeout,
            )
            .await
    }

    async fn execute_vm(
        &self,
        argv: &[String],
        input: Option<Vec<u8>>,
        input_as_file: bool,
        timeout: Duration,
    ) -> Result<ExecutionResult, String> {
        let started = std::time::Instant::now();
        let config = self
            .firecracker
            .as_ref()
            .ok_or("missing Firecracker configuration")?
            .clone();
        let runner = super::FirecrackerRunner::new(config).map_err(|e| e.to_string())?;
        let remaining = timeout.saturating_sub(started.elapsed());
        runner
            .execute_command(argv, HashMap::new(), input, input_as_file, remaining)
            .await
            .map_err(|e| e.to_string())
    }

    async fn initialize_container(
        &self,
        timeout: Duration,
        lifetime: Duration,
    ) -> Result<(DockerRunner, Duration), String> {
        self.validate()?;
        if timeout.is_zero() {
            return Err("command timed out before execution".into());
        }
        let started = std::time::Instant::now();
        let profile = self.clone();
        let initialize = tokio::task::spawn_blocking(move || -> anyhow::Result<DockerRunner> {
            match profile.tier {
                CommandTier::Docker => {
                    let mut config = profile.docker;
                    config.max_execution_time = config.max_execution_time.min(lifetime);
                    DockerRunner::new(config)
                }
                CommandTier::GVisor => {
                    let mut config = profile.gvisor;
                    config.docker.max_execution_time =
                        config.docker.max_execution_time.min(lifetime);
                    Ok(GVisorRunner::new(config)?.into_docker_runner())
                }
                _ => anyhow::bail!("selected command transport unavailable"),
            }
        });
        let runner = tokio::time::timeout(timeout, initialize)
            .await
            .map_err(|_| "sandbox initialization timed out")?
            .map_err(|e| format!("sandbox initialization failed: {e}"))?
            .map_err(|e| e.to_string())?;
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err("sandbox initialization exhausted command deadline".into());
        }
        Ok((runner, remaining))
    }

    /// Reserve delegated capacity and join the worker cgroup before exec.
    #[cfg(target_os = "linux")]
    pub(crate) async fn spawn_landlock(
        &self,
        argv: &[String],
        env: HashMap<String, String>,
        domain: super::landlock::PreparedDomain,
        timeout: Duration,
    ) -> Result<(tokio::process::Child, super::supervisor::Lease), String> {
        let (program, args) = argv.split_first().ok_or("empty command")?;
        let mut command = tokio::process::Command::new(program);
        command
            .args(args)
            .env_clear()
            .envs(env)
            .kill_on_drop(true)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        command.process_group(0);
        super::landlock::spawn(&self.landlock, domain, &mut command, timeout)
            .await
            .map_err(|e| e.to_string())
    }

    #[cfg(feature = "mcp-client")]
    pub(crate) async fn spawn_streams(
        &self,
        argv: &[String],
        env: HashMap<String, String>,
        timeout: Duration,
    ) -> Result<super::streams::StdioStreams, String> {
        self.validate()?;
        #[cfg(target_os = "linux")]
        if self.tier == CommandTier::Landlock {
            if self.landlock.workspace.is_some() {
                let (child, lease) =
                    super::landlock::workspace::spawn(&self.landlock, argv, env, timeout)
                        .await
                        .map_err(|e| e.to_string())?;
                return Ok(super::streams::StdioStreams::host(
                    child,
                    lease,
                    self.landlock.max_output_bytes,
                ));
            }
            let domain = super::landlock::prepare(&self.landlock, &self.roots)?;
            let (child, lease) = self.spawn_landlock(argv, env, domain, timeout).await?;
            return Ok(super::streams::StdioStreams::host(
                child,
                lease,
                self.landlock.max_output_bytes,
            ));
        }
        if self.tier == CommandTier::Firecracker {
            #[cfg(unix)]
            {
                let config = self
                    .firecracker
                    .clone()
                    .ok_or("missing Firecracker configuration")?;
                let runner = super::FirecrackerRunner::new(config).map_err(|e| e.to_string())?;
                return runner
                    .spawn_stdio(argv, env, timeout)
                    .await
                    .map(super::streams::StdioStreams::vm)
                    .map_err(|e| e.to_string());
            }
            #[cfg(not(unix))]
            return Err("Firecracker stdio requires Linux".into());
        }
        self.spawn_stdio(argv, env, timeout)
            .await
            .map(super::streams::StdioStreams::container)
    }

    #[cfg(any(feature = "mcp-client", feature = "cli-executor"))]
    pub(crate) async fn spawn_stdio(
        &self,
        argv: &[String],
        env: HashMap<String, String>,
        timeout: Duration,
    ) -> Result<super::docker::StdioContainer, String> {
        let code = literal_command(argv)?;
        let (runner, remaining) = self.initialize_container(timeout, timeout).await?;
        tokio::time::timeout(
            remaining,
            runner.spawn_stdio(code, env, remaining, remaining, false, None),
        )
        .await
        .map_err(|_| "sandbox stdio initialization timed out".to_string())?
        .map_err(|e| e.to_string())
    }

    /// Allocate the PTY inside the selected worker, retaining its independent
    /// lifetime across calls. Initialization still uses the current call budget.
    #[cfg(feature = "toolclad-session")]
    pub(crate) async fn spawn_terminal(
        &self,
        argv: &[String],
        startup: Duration,
        lifetime: Duration,
        cancellation: tokio::sync::watch::Receiver<bool>,
    ) -> Result<super::streams::StdioStreams, String> {
        self.validate()?;
        let env = HashMap::from([("TERM".into(), "dumb".into())]);
        if self.tier == CommandTier::Firecracker {
            #[cfg(unix)]
            {
                let config = self
                    .firecracker
                    .clone()
                    .ok_or("missing Firecracker configuration")?;
                let runner = super::FirecrackerRunner::new(config).map_err(|e| e.to_string())?;
                return runner
                    .spawn_terminal(argv, env, startup, lifetime, cancellation)
                    .await
                    .map(super::streams::StdioStreams::vm)
                    .map_err(|e| e.to_string());
            }
            #[cfg(not(unix))]
            return Err("Firecracker terminals require Linux".into());
        }
        let started = std::time::Instant::now();
        let command = literal_command(argv)?;
        let code = format!("stty -echo || exit 125; {command}");
        let (runner, remaining) = self.initialize_container(startup, lifetime).await?;
        // The actor awaits initialization even after cancellation so the
        // create/inspect owner can acknowledge cleanup before run termination.
        runner
            .spawn_stdio(
                code,
                env,
                lifetime.saturating_sub(started.elapsed()),
                remaining,
                true,
                Some(cancellation),
            )
            .await
            .map(super::streams::StdioStreams::container)
            .map_err(|e| e.to_string())
    }

    async fn execute_container(
        &self,
        code: String,
        input: Option<Vec<u8>>,
        timeout: Duration,
    ) -> Result<ExecutionResult, String> {
        let (runner, remaining) = self.initialize_container(timeout, timeout).await?;
        tokio::time::timeout(
            remaining,
            runner.execute_with_input(&code, HashMap::new(), input),
        )
        .await
        .map_err(|_| "sandbox command timed out".to_string())?
        .map_err(|e| e.to_string())
    }
}

pub(crate) fn literal_command(argv: &[String]) -> Result<String, String> {
    if argv.is_empty() || argv[0].is_empty() {
        return Err("empty command".into());
    }
    if argv.iter().any(|s| s.contains('\0')) {
        return Err("command contains NUL".into());
    }
    Ok(format!(
        "exec {}",
        argv.iter()
            .map(|s| format!("'{}'", s.replace('\'', "'\\''")))
            .collect::<Vec<_>>()
            .join(" ")
    ))
}

/// Bound trusted runtime preflights; a hung daemon must not freeze dispatch.
pub(super) fn check_binary(binary: &str, argument: &str, timeout: Duration) -> anyhow::Result<()> {
    use std::process::{Command, Stdio};
    if timeout.is_zero() {
        anyhow::bail!("sandbox preflight deadline exhausted");
    }
    let mut command = Command::new(binary);
    command
        .arg(argument)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|e| anyhow::anyhow!("sandbox backend '{binary}' is not available: {e}"))?;
    let started = std::time::Instant::now();
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break Ok(()),
            Ok(Some(status)) => {
                break Err(anyhow::anyhow!(
                    "sandbox backend '{binary}' preflight failed: {status}"
                ))
            }
            Err(e) => break Err(e.into()),
            Ok(None) => {}
        }
        if started.elapsed() >= timeout {
            break Err(anyhow::anyhow!(
                "sandbox backend '{binary}' preflight timed out"
            ));
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    #[cfg(unix)]
    // SAFETY: this child owns the process group created immediately above.
    unsafe {
        libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn landlock_descriptor_records_what_the_kernel_could_actually_enforce() {
        if super::super::landlock::detect_abi() < 6 {
            eprintln!("skipped: kernel Landlock ABI below 6");
            return;
        }
        let boundary = CommandBoundary {
            tier: CommandTier::Landlock,
            ..Default::default()
        };
        let descriptor = boundary.descriptor().expect("descriptor");
        let landlock = &descriptor["landlock"];
        assert_eq!(descriptor["tier"], "landlock");
        assert_eq!(
            landlock["abi_detected"],
            super::super::landlock::detect_abi()
        );
        assert_eq!(landlock["abi_floor"], boundary.landlock.abi_floor);
        assert_eq!(
            landlock["network_restricted"],
            boundary.landlock.require_network
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn landlock_boundary_runs_a_command_and_confines_it_without_a_container() {
        if std::env::var_os("SYMBIONT_TEST_DELEGATED_SERVICE").is_none() {
            eprintln!("skipped: requires explicit delegated service fixture");
            return;
        }
        if super::super::landlock::detect_abi() < 6 {
            eprintln!("skipped: kernel Landlock ABI below 6");
            return;
        }
        let granted = tempfile::tempdir().expect("tempdir");
        let secret = tempfile::tempdir().expect("tempdir");
        std::fs::write(secret.path().join("secret"), b"no").expect("write");

        let mut boundary = CommandBoundary {
            tier: CommandTier::Landlock,
            ..Default::default()
        };
        boundary.roots.source_roots = vec![format!("{}:/workspace:ro", granted.path().display())];
        boundary.validate().expect("boundary must validate");

        let argv = vec![
            "/bin/cat".to_string(),
            secret.path().join("secret").display().to_string(),
        ];
        let domain = super::super::landlock::prepare(&boundary.landlock, &boundary.roots)
            .expect("prepare domain");
        let (child, mut lease) = boundary
            .spawn_landlock(&argv, HashMap::new(), domain, Duration::from_secs(10))
            .await
            .expect("the command must start");
        let output = child.wait_with_output().await.expect("collect output");
        lease.finish().await.expect("independent cleanup");
        // Both halves matter: the command must actually have been refused, and
        // it must not have leaked the content. Asserting only on empty stdout
        // would pass vacuously if the child never started.
        assert!(
            !output.status.success(),
            "reading outside every grant must fail: {output:?}"
        );
        assert!(
            !String::from_utf8_lossy(&output.stdout).contains("no"),
            "the denied file's content must not appear on stdout"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn landlock_boundary_refuses_a_kernel_below_its_floor() {
        let mut boundary = CommandBoundary {
            tier: CommandTier::Landlock,
            ..Default::default()
        };
        boundary.landlock.abi_floor = 99;
        let error = boundary.validate().expect_err("must fail closed");
        assert!(error.contains("99"), "{error}");
    }

    #[test]
    fn roots_live_on_the_boundary_and_are_cleared_with_host_mounts() {
        let mut boundary = CommandBoundary::default();
        boundary
            .roots
            .source_roots
            .push("/srv/src:/workspace:ro".into());
        boundary.roots.output_roots.push("/srv/out:/out".into());
        let stripped = boundary.without_host_mounts();
        assert!(stripped.roots.source_roots.is_empty());
        assert!(stripped.roots.output_roots.is_empty());
    }

    #[test]
    fn agent_selection_precedes_validation_and_freezes_the_profile() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("symbiont.toml");
        std::fs::write(&path, "[sandbox]\ntier='firecracker'").unwrap();
        let settings = dsl::resolve_execution_settings(
            r#"agent a() { with sandbox = "docker", timeout = 2.seconds {} }"#,
            "a",
        )
        .unwrap();
        let boundary = CommandBoundary::load_for_agent(root.path(), &settings).unwrap();
        assert_eq!(boundary.tier, CommandTier::Docker);
        assert_eq!(boundary.docker.max_execution_time, Duration::from_secs(2));
        let descriptor = boundary.descriptor().unwrap();
        std::fs::write(&path, "[sandbox]\ntier='typo'").unwrap();
        assert_eq!(boundary.descriptor().unwrap(), descriptor);
        assert!(CommandBoundary::load_for_agent(root.path(), &settings).is_err());

        std::fs::remove_file(&path).unwrap();
        let unavailable = dsl::resolve_execution_settings(
            r#"agent a() { with sandbox = "firecracker" {} }"#,
            "a",
        )
        .unwrap();
        assert!(CommandBoundary::load_for_agent(root.path(), &unavailable)
            .unwrap_err()
            .contains("Firecracker tier selected without configuration"));
    }

    #[test]
    fn selected_tier_retains_its_own_limits_and_protected_mount_checks() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("symbiont.toml");
        std::fs::write(
            &path,
            "[sandbox.gvisor.docker]\nimage='gvisor-fixture'\nmax_execution_time={secs=1,nanos=0}",
        )
        .unwrap();
        let settings = dsl::resolve_execution_settings(
            r#"agent a() { with sandbox = "gvisor", timeout = 20.seconds {} }"#,
            "a",
        )
        .unwrap();
        let boundary = CommandBoundary::load_for_agent(root.path(), &settings).unwrap();
        assert_eq!(boundary.tier, CommandTier::GVisor);
        assert_eq!(boundary.gvisor.docker.image, "gvisor-fixture");
        assert_eq!(
            boundary.gvisor.docker.max_execution_time,
            Duration::from_secs(1)
        );
        std::fs::write(
            &path,
            format!(
                "[sandbox.gvisor.docker]\nvolumes=['{}:/workspace:rw']",
                root.path().display()
            ),
        )
        .unwrap();
        assert!(CommandBoundary::load_for_agent(root.path(), &settings)
            .unwrap_err()
            .contains("protected project"));
    }

    #[test]
    fn configuration_defaults_strict_errors_and_protected_mounts() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            CommandBoundary::load(root.path()).unwrap().tier,
            CommandTier::Docker
        );
        let path = root.path().join("symbiont.toml");
        for text in [
            "[sandbox]\ntier='typo'",
            "[sandbox]\nvolums=[]",
            "[sandbox.docker]\nnetwork_mod='host'",
            "[sandbox]\ntier='firecracker'",
            "[sandbox]\ntier='e2b'",
            "[sandbox",
        ] {
            std::fs::write(&path, text).unwrap();
            assert!(CommandBoundary::load(root.path()).is_err(), "{text}");
        }
        std::fs::write(
            &path,
            format!(
                "[sandbox.docker]\nvolumes=['{}:/workspace:rw']",
                root.path().display()
            ),
        )
        .unwrap();
        assert!(CommandBoundary::load(root.path())
            .unwrap_err()
            .contains("protected project"));
        let output = root.path().join("output");
        std::fs::create_dir(&output).unwrap();
        std::fs::write(
            &path,
            format!(
                "[sandbox]\ntier='tier1'\n[sandbox.docker]\nvolumes=['{}:/workspace:rw']",
                output.display()
            ),
        )
        .unwrap();
        let profile = CommandBoundary::load(root.path()).unwrap();
        assert_eq!(profile.docker.memory_limit.as_deref(), Some("512m"));
        let mut changed = profile.clone();
        changed.docker.pids_limit = 64;
        assert_ne!(profile.descriptor().unwrap(), changed.descriptor().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn dangling_configuration_and_preflight_deadline_fail_closed() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let root = tempfile::tempdir().unwrap();
        symlink(
            root.path().join("missing"),
            root.path().join("symbiont.toml"),
        )
        .unwrap();
        assert!(CommandBoundary::load(root.path()).is_err());
        let binary = root.path().join("slow-runtime");
        std::fs::write(&binary, "#!/bin/sh\nsleep 60\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        let started = std::time::Instant::now();
        assert!(check_binary(
            binary.to_str().unwrap(),
            "version",
            Duration::from_millis(50)
        )
        .unwrap_err()
        .to_string()
        .contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn unavailable_backend_never_falls_back_to_host() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("effect");
        let mut profile = CommandBoundary::default();
        profile.docker.docker_binary = root.path().join("missing-docker").display().to_string();
        let error = profile
            .execute(
                &["/usr/bin/touch".into(), marker.display().to_string()],
                Duration::from_secs(1),
            )
            .await
            .unwrap_err();
        assert!(error.contains("not available"), "{error}");
        assert!(!marker.exists());
    }
    #[test]
    fn agent_resources_tighten_but_never_expand_container_limits() {
        let mut boundary = CommandBoundary::default();
        boundary.docker.memory_limit = Some("128m".into());
        boundary.docker.cpu_limit = Some(0.25);
        boundary.docker.max_execution_time = Duration::from_secs(10);
        boundary
            .tighten_resources(&crate::types::ResourceLimits::default())
            .unwrap();
        assert_eq!(boundary.docker.memory_limit.as_deref(), Some("134217728"));
        assert_eq!(boundary.docker.cpu_limit, Some(0.25));
        assert_eq!(boundary.docker.max_execution_time, Duration::from_secs(10));
        let limits = crate::types::ResourceLimits {
            memory_mb: 64,
            cpu_cores: 0.125,
            execution_timeout: Duration::from_secs(5),
            ..Default::default()
        };
        boundary.tighten_resources(&limits).unwrap();
        assert_eq!(boundary.docker.memory_limit.as_deref(), Some("67108864"));
        assert_eq!(boundary.docker.cpu_limit, Some(0.125));
        assert_eq!(boundary.docker.max_execution_time, Duration::from_secs(5));
        assert!(boundary
            .tighten_resources(&crate::types::ResourceLimits {
                cpu_cores: f32::NAN,
                ..limits
            })
            .is_err());
    }
}
