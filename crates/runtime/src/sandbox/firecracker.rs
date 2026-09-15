//! Firecracker execution through an independently owned VMM and a versioned
//! guest command protocol. A VMM exit is never a substitute for a guest result.
use super::{
    supervisor::{Lease, SupervisorConfig},
    ExecutionResult, SandboxRunner,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use symbi_sandbox_guest as guest;
use symbi_sandbox_supervisor::protocol::{CreateVm, IMPLEMENTATION, VERSION};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

#[cfg(unix)]
pub mod files;
pub mod services;
#[cfg(unix)]
pub mod snapshot;
#[cfg(unix)]
mod streaming;
#[cfg(unix)]
pub use streaming::{FirecrackerStdio, StdioGuard};

/// Operator-owned artifacts and VM limits. The root image is always read-only;
/// the guest init provisions temporary scratch space inside the VM.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FirecrackerConfig {
    pub kernel_image_path: PathBuf,
    pub rootfs_path: PathBuf,
    pub rootfs_read_only: bool,
    pub boot_args: String,
    pub vcpus: u8,
    pub mem_mib: u32,
    pub firecracker_binary: String,
    pub max_execution_time: Duration,
    pub startup_timeout: Duration,
    pub max_output_bytes: usize,
    /// Legacy work-root setting. State now belongs to the private supervisor;
    /// set supervisor.state_dir instead. A nonempty legacy value is refused.
    pub work_dir: Option<PathBuf>,
    pub working_dir: String,
    /// Explicit host:virtual:ro ceilings for source queries and declared inputs. These are
    /// never mounted in, or automatically transferred to, the guest.
    pub source_roots: Vec<String>,
    /// Separate write-only host ceilings for new-file publication.
    pub output_roots: Vec<String>,
    /// One-use transport authority created by the file broker.
    #[cfg(unix)]
    #[serde(skip)]
    pub files: Option<std::sync::Arc<files::Transfer>>,
    #[cfg(unix)]
    #[serde(skip)]
    pub snapshot: Option<std::sync::Arc<snapshot::Transfer>>,
    pub supervisor: SupervisorConfig,
    /// Opaque runtime-issued broker capabilities, bound into managed admission.
    /// Project configuration cannot create or restore them.
    #[serde(skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub services: Option<services::GuestServices>,
}
impl Default for FirecrackerConfig {
    fn default() -> Self {
        Self {
            kernel_image_path: PathBuf::new(),
            rootfs_path: PathBuf::new(),
            rootfs_read_only: true,
            boot_args: "console=ttyS0 reboot=k panic=1 pci=off ro init=/sbin/symbi-sandbox-guest"
                .into(),
            vcpus: 1,
            mem_mib: 512,
            firecracker_binary: "firecracker".into(),
            max_execution_time: Duration::from_secs(300),
            startup_timeout: Duration::from_secs(15),
            max_output_bytes: guest::MAX_OUTPUT,
            work_dir: None,
            working_dir: "/tmp".into(),
            source_roots: Vec::new(),
            output_roots: Vec::new(),
            #[cfg(unix)]
            files: None,
            #[cfg(unix)]
            snapshot: None,
            supervisor: SupervisorConfig::default(),
            services: None,
        }
    }
}
impl FirecrackerConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        if !cfg!(target_os = "linux") {
            anyhow::bail!("Firecracker requires Linux and KVM");
        }
        if self.kernel_image_path.as_os_str().is_empty() {
            anyhow::bail!("Firecracker sandbox is missing kernel_image_path");
        }
        if self.rootfs_path.as_os_str().is_empty() {
            anyhow::bail!("Firecracker sandbox is missing rootfs_path");
        }
        for path in [&self.kernel_image_path, &self.rootfs_path] {
            if !path.is_absolute() || !path.is_file() {
                anyhow::bail!(
                    "Firecracker artifact must be an existing absolute file: {}",
                    path.display()
                );
            }
        }
        if !self.rootfs_read_only {
            anyhow::bail!("Firecracker rootfs must be read-only; use guest scratch storage");
        }
        if self.work_dir.is_some() {
            anyhow::bail!("Firecracker work_dir is unsupported; configure supervisor.state_dir");
        }
        if !(1..=32).contains(&self.vcpus)
            || !(64..=16384).contains(&self.mem_mib)
            || self.max_execution_time.is_zero()
            || self.max_execution_time > Duration::from_secs(86400)
            || self.startup_timeout.is_zero()
            || self.startup_timeout > Duration::from_secs(60)
            || !(1..=guest::MAX_OUTPUT).contains(&self.max_output_bytes)
            || !self.working_dir.starts_with('/')
            || self.working_dir.contains('\0')
            || self.working_dir.len() > 4096
            || self.boot_args.is_empty()
            || self.boot_args.len() > 4096
            || self.boot_args.contains('\0')
        {
            anyhow::bail!("invalid Firecracker resource, startup or guest limits");
        }
        if self.firecracker_binary.is_empty() || self.firecracker_binary.contains('\0') {
            anyhow::bail!("invalid Firecracker executable");
        }
        self.supervisor.validate()?;
        let supervisor_root = self.supervisor.resolved_state_dir()?;
        for (roots, mode) in [(&self.source_roots, "ro"), (&self.output_roots, "rw")] {
            anyhow::ensure!(roots.len() <= 32, "too many Firecracker broker file roots");
            let mut destinations = std::collections::HashSet::new();
            for root in roots {
                super::docker::canonical_mount(root)?;
                let parts: Vec<_> = root.split(':').collect();
                anyhow::ensure!(parts.get(2).copied().unwrap_or("ro") == mode, "Firecracker source roots must be read-only and output roots explicitly writable");
                anyhow::ensure!(
                    destinations.insert(Path::new(parts[1]).components().collect::<PathBuf>()),
                    "duplicate Firecracker file destination"
                );
                let source = Path::new(parts[0]).canonicalize()?;
                anyhow::ensure!(
                    !source.starts_with(&supervisor_root) && !supervisor_root.starts_with(&source),
                    "file root exposes the protected sandbox supervisor state"
                );
            }
        }
        #[cfg(unix)]
        if let Some(snapshot) = &self.snapshot {
            snapshot.validate()?;
            anyhow::ensure!(
                self.files.is_none() && self.services.is_none(),
                "Git snapshot authority requires an exclusive worker"
            );
        }
        #[cfg(unix)]
        if let Some(files) = &self.files {
            files.validate()?;
        }

        if let Some(services) = &self.services {
            services.validate()?;
        }
        if self
            .supervisor
            .resolved_state_dir()?
            .join(format!("vm-{}", uuid::Uuid::nil()))
            .join("vsock")
            .as_os_str()
            .as_encoded_bytes()
            .len()
            >= if self.services.is_some() { 99 } else { 104 }
        {
            anyhow::bail!("Firecracker supervisor state path is too long for vsock");
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct FirecrackerRunner {
    config: FirecrackerConfig,
}
impl FirecrackerRunner {
    pub fn new(mut config: FirecrackerConfig) -> anyhow::Result<Self> {
        config.validate()?;
        config.kernel_image_path = config.kernel_image_path.canonicalize()?;
        config.rootfs_path = config.rootfs_path.canonicalize()?;
        config.firecracker_binary =
            super::supervisor::resolve_binary(Path::new(&config.firecracker_binary))?
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("Firecracker executable must be UTF-8"))?
                .to_owned();
        config.supervisor.state_dir = config.supervisor.resolved_state_dir()?;
        // No unbounded --version subprocess. Existence is preflight evidence;
        // the actual VMM start and guest handshake establish execution readiness.
        Ok(Self { config })
    }
    fn specification(&self, id: uuid::Uuid, lifetime: Duration) -> CreateVm {
        CreateVm {
            version: VERSION,
            implementation: IMPLEMENTATION.into(),
            lease: id,
            binary: PathBuf::from(&self.config.firecracker_binary),
            kernel: self.config.kernel_image_path.clone(),
            rootfs: self.config.rootfs_path.clone(),
            boot_args: self.config.boot_args.clone(),
            vcpus: self.config.vcpus,
            memory_mib: self.config.mem_mib,
            lifetime_ms: lifetime.as_millis().min(u64::MAX as u128) as u64,
            startup_ms: self
                .config
                .startup_timeout
                .min(lifetime)
                .as_millis()
                .min(u64::MAX as u128) as u64,
        }
    }
    /// VM config contains a concrete vsock device and no network interface.
    pub fn build_vm_config_json(&self, vsock: &Path) -> serde_json::Value {
        self.specification(uuid::Uuid::nil(), self.config.max_execution_time)
            .vm_config(vsock)
    }

    pub async fn execute_command(
        &self,
        argv: &[String],
        environment: HashMap<String, String>,
        input: Option<Vec<u8>>,
        input_as_file: bool,
        budget: Duration,
    ) -> anyhow::Result<ExecutionResult> {
        let started = Instant::now();
        let lifetime = budget.min(self.config.max_execution_time);
        let id = uuid::Uuid::new_v4();
        let input = input.unwrap_or_default();
        let mut environment: std::collections::BTreeMap<String, String> =
            environment.into_iter().collect();
        for (name, value) in [
            ("PATH", "/usr/bin:/bin"),
            ("HOME", "/tmp"),
            ("LANG", "C.UTF-8"),
        ] {
            environment
                .entry(name.into())
                .or_insert_with(|| value.into());
        }
        let request = guest::Command {
            mode: guest::CommandMode::OneShot,
            version: guest::VERSION,
            id: id.to_string(),
            argv: argv.to_vec(),
            environment,
            working_dir: self.config.working_dir.clone(),
            input_length: input.len(),
            input_as_file,
            snapshot: {
                #[cfg(unix)]
                {
                    self.config
                        .snapshot
                        .as_ref()
                        .map(|s| s.begin())
                        .transpose()?
                }
                #[cfg(not(unix))]
                {
                    None
                }
            },
            files: {
                #[cfg(unix)]
                {
                    self.config
                        .files
                        .as_ref()
                        .map(|files| files.begin())
                        .transpose()?
                }
                #[cfg(not(unix))]
                {
                    None
                }
            },
            max_output_bytes: self.config.max_output_bytes,
            timeout_ms: lifetime.as_millis().min(u64::MAX as u128) as u64,
        };
        request.validate()?;
        guest::encode_header(&request)?;
        let specification = self.specification(id, lifetime);
        specification.validate()?;
        let registration = super::command_cleanup::register().map_err(anyhow::Error::msg)?;
        let run_stop = registration
            .as_ref()
            .map_or_else(CancellationToken::new, |r| r.stop.clone());
        let stop = CancellationToken::new();
        let guard = CancelOnDrop(stop.clone());
        let config = self.config.clone();
        let (send, receive) = oneshot::channel();
        tokio::spawn(async move {
            let deadline = started + lifetime;
            let startup = deadline.min(started + config.startup_timeout);
            let acquisition = Lease::register_vm(&config.supervisor, specification, startup).await;
            let (result, cleanup) = match acquisition {
                Ok(mut lease) => {
                    let work = async {
                        let (_, path) = lease.vm_created(startup).await?;
                        #[cfg(unix)]
                        {
                            transact(
                                &path,
                                request,
                                input,
                                startup,
                                config.files.as_deref(),
                                config.snapshot.as_deref(),
                            )
                            .await
                        }
                        #[cfg(not(unix))]
                        {
                            transact(&path, request, input, startup).await
                        }
                    };
                    let result = tokio::select! {
                        biased;
                        _=stop.cancelled()=>Err(anyhow::anyhow!("Firecracker execution cancelled")),
                        _=run_stop.cancelled()=>Err(anyhow::anyhow!("Firecracker execution run closed")),
                        result=tokio::time::timeout_at(deadline.into(),work)=>result.map_err(|_|anyhow::anyhow!("Firecracker execution deadline expired")).and_then(|r|r),
                    };
                    let outcome = lease.finish().await;
                    let cleanup = lease.finish_cleanup().await.map_err(|e| e.to_string());
                    let result = result.and_then(|result| outcome.map(|()| result));
                    (result, cleanup)
                }
                Err(error)
                    if error
                        .downcast_ref::<super::supervisor::CreationRefused>()
                        .is_some() =>
                {
                    (Err(error), Ok(()))
                }
                Err(error) => {
                    let message =
                        format!("VM registration failed without cleanup acknowledgement: {error}");
                    (Err(error), Err(message))
                }
            };
            if let Some(registration) = registration {
                registration.finish(cleanup.clone());
            }
            let result = match cleanup {
                Ok(()) => result,
                Err(error) => Err(anyhow::anyhow!("Firecracker cleanup failed: {error}")),
            };
            #[cfg(unix)]
            if result.is_ok() {
                if let Some(snapshot) = &config.snapshot {
                    snapshot.complete();
                }
            }
            #[cfg(unix)]
            if result.as_ref().is_ok_and(|result| result.success) {
                if let Some(files) = &config.files {
                    files.complete();
                }
            }
            let _ = send.send(result.map(|mut result| {
                result.execution_time_ms =
                    started.elapsed().as_millis().min(u64::MAX as u128) as u64;
                result
            }));
        });
        let result = receive
            .await
            .map_err(|_| anyhow::anyhow!("Firecracker execution owner ended"))?;
        drop(guard);
        result
    }
}
struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

#[cfg(unix)]
async fn read_header<T: serde::de::DeserializeOwned>(
    stream: &mut tokio::net::UnixStream,
) -> anyhow::Result<T> {
    use tokio::io::AsyncReadExt;
    let length = stream.read_u32().await? as usize;
    if length == 0 || length > guest::MAX_HEADER {
        anyhow::bail!("invalid guest header length");
    }
    let mut data = vec![0; length];
    stream.read_exact(&mut data).await?;
    Ok(serde_json::from_slice(&data)?)
}
#[cfg(unix)]
async fn connect_guest(path: &Path, startup: Instant) -> anyhow::Result<tokio::net::UnixStream> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = loop {
        if Instant::now() >= startup {
            anyhow::bail!("Firecracker guest did not become ready before startup deadline");
        }
        let attempt = async {
            let mut stream = tokio::net::UnixStream::connect(path).await?;
            stream
                .write_all(format!("CONNECT {}\n", guest::PORT).as_bytes())
                .await?;
            let mut line = Vec::new();
            loop {
                let byte = stream.read_u8().await?;
                if byte == b'\n' {
                    break;
                }
                if line.len() >= 64 {
                    anyhow::bail!("invalid vsock acknowledgement");
                }
                line.push(byte);
            }
            let line = std::str::from_utf8(&line)?;
            line.strip_prefix("OK ")
                .ok_or_else(|| anyhow::anyhow!("vsock connection refused"))?
                .parse::<u32>()?;
            Ok::<_, anyhow::Error>(stream)
        };
        match tokio::time::timeout_at(
            startup
                .min(Instant::now() + Duration::from_millis(250))
                .into(),
            attempt,
        )
        .await
        {
            Ok(Ok(stream)) => break stream,
            _ => tokio::time::sleep(Duration::from_millis(10)).await,
        }
    };
    let hello: guest::Hello = tokio::time::timeout_at(startup.into(), read_header(&mut stream))
        .await
        .map_err(|_| anyhow::anyhow!("Firecracker guest handshake deadline expired"))??;
    if hello.version != guest::VERSION || hello.implementation != guest::IMPLEMENTATION {
        anyhow::bail!("Firecracker guest protocol mismatch");
    }
    Ok(stream)
}
#[cfg(unix)]
async fn transact(
    path: &Path,
    mut request: guest::Command,
    input: Vec<u8>,
    startup: Instant,
    files: Option<&files::Transfer>,
    snapshot: Option<&snapshot::Transfer>,
) -> anyhow::Result<ExecutionResult> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let connected = Instant::now();
    let mut stream = connect_guest(path, startup).await?;
    request.timeout_ms = request
        .timeout_ms
        .saturating_sub(connected.elapsed().as_millis().min(u64::MAX as u128) as u64);
    request.validate()?;
    let header = guest::encode_header(&request)?;
    stream.write_u32(header.len() as u32).await?;
    stream.write_all(&header).await?;
    if let Some(snapshot) = snapshot {
        snapshot.upload(&mut stream).await?;
    }
    if let Some(files) = files {
        files.upload(&mut stream).await?;
    }
    stream.write_all(&input).await?;
    let result: guest::Outcome = read_header(&mut stream).await?;
    result.validate(&request)?;
    let mut stdout = vec![0; result.stdout_length];
    let mut stderr = vec![0; result.stderr_length];
    stream.read_exact(&mut stdout).await?;
    stream.read_exact(&mut stderr).await?;
    if let Some(receipt) = &result.file_output {
        files
            .ok_or_else(|| anyhow::anyhow!("unsolicited guest file output"))?
            .receive(&mut stream, receipt)
            .await?;
    }
    if stream.read(&mut [0; 1]).await? != 0 {
        anyhow::bail!("unexpected bytes after guest outcome");
    }
    if let Some(receipt) = result.snapshot.clone() {
        snapshot
            .ok_or_else(|| anyhow::anyhow!("unexpected guest snapshot receipt"))?
            .observed(receipt)?;
    }
    let mut stderr = String::from_utf8_lossy(&stderr).into_owned();
    if let Some(error) = &result.error {
        stderr.push_str(&format!("\n{error}"));
    }
    if result.timed_out {
        stderr.push_str("\nguest command deadline expired");
    }
    Ok(ExecutionResult {
        exit_code: result.exit_code,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr,
        execution_time_ms: 0,
        success: result.success(),
        stdout_truncated: result.stdout_truncated,
        stderr_truncated: result.stderr_truncated,
    })
}
#[cfg(not(unix))]
async fn transact(
    _: &Path,
    _: guest::Command,
    _: Vec<u8>,
    _: Instant,
) -> anyhow::Result<ExecutionResult> {
    anyhow::bail!("Firecracker guest transport requires Linux")
}

#[async_trait]
impl SandboxRunner for FirecrackerRunner {
    async fn execute(
        &self,
        code: &str,
        environment: HashMap<String, String>,
    ) -> anyhow::Result<ExecutionResult> {
        self.execute_command(
            &["/bin/sh".into(), "-c".into(), code.into()],
            environment,
            None,
            false,
            self.config.max_execution_time,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_artifacts_and_unsafe_limits_are_refused() {
        assert!(FirecrackerConfig::default()
            .validate()
            .unwrap_err()
            .to_string()
            .contains("kernel_image_path"));
        let kernel = tempfile::NamedTempFile::new().unwrap();
        let rootfs = tempfile::NamedTempFile::new().unwrap();
        let mut config = FirecrackerConfig {
            kernel_image_path: kernel.path().into(),
            rootfs_path: rootfs.path().into(),
            ..Default::default()
        };
        config.validate().unwrap();
        config.rootfs_read_only = false;
        assert!(config.validate().is_err());
        config.rootfs_read_only = true;
        config.max_output_bytes = usize::MAX;
        assert!(config.validate().is_err());
        config.max_output_bytes = 1024;
        config.max_execution_time = Duration::ZERO;
        assert!(config.validate().is_err());
    }
    #[test]
    fn vm_configuration_has_a_real_guest_channel_without_network() {
        let runner = FirecrackerRunner {
            config: FirecrackerConfig::default(),
        };
        let value = runner.build_vm_config_json(Path::new("/private/vsock"));
        assert_eq!(value["vsock"]["uds_path"], "/private/vsock");
        assert_eq!(value["drives"][0]["is_read_only"], true);
        assert!(value.get("network-interfaces").is_none());
        assert!(value.get("logger").is_none());
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn guest_response_must_be_complete_correlated_and_bounded() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for mode in [
            "valid",
            "wrong_id",
            "wrong_version",
            "oversized_output",
            "oversized_header",
            "zero_header",
            "malformed_header",
            "unknown_field",
            "truncated_stream",
            "extra_stream",
            "stale_hello",
            "stalled_hello",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let socket = directory.path().join("vsock");
            let listener = tokio::net::UnixListener::bind(&socket).unwrap();
            let request = guest::Command {
                mode: guest::CommandMode::OneShot,
                version: guest::VERSION,
                id: uuid::Uuid::new_v4().to_string(),
                argv: vec!["/bin/true".into()],
                environment: Default::default(),
                working_dir: "/tmp".into(),
                input_length: 0,
                input_as_file: false,
                files: None,
                snapshot: None,
                max_output_bytes: 4,
                timeout_ms: 1000,
            };
            let expected_id = request.id.clone();
            let server = tokio::spawn(async move {
                let (mut peer, _) = listener.accept().await.unwrap();
                let mut connect = Vec::new();
                loop {
                    let byte = peer.read_u8().await.unwrap();
                    connect.push(byte);
                    if byte == b'\n' {
                        break;
                    }
                }
                assert_eq!(connect, b"CONNECT 4050\n");
                peer.write_all(b"OK 12345\n").await.unwrap();
                if mode == "stalled_hello" {
                    let mut received = Vec::new();
                    peer.read_to_end(&mut received).await.unwrap();
                    assert!(received.is_empty(), "payload sent before handshake");
                    return;
                }
                let hello = guest::Hello {
                    version: guest::VERSION,
                    implementation: if mode == "stale_hello" {
                        "stale".into()
                    } else {
                        guest::IMPLEMENTATION.into()
                    },
                };
                let mut bytes = vec![];
                guest::write_header(&mut bytes, &hello).unwrap();
                peer.write_all(&bytes).await.unwrap();
                if mode == "stale_hello" {
                    let mut received = Vec::new();
                    peer.read_to_end(&mut received).await.unwrap();
                    assert!(received.is_empty(), "payload sent to a stale guest");
                    return;
                }
                let actual: guest::Command = read_header(&mut peer).await.unwrap();
                assert_eq!(actual.id, expected_id);
                let mut outcome = serde_json::json!({"version":guest::VERSION,"id":expected_id,
                    "exit_code":0,"stdout_length":2,"stderr_length":0,
                    "stdout_truncated":false,"stderr_truncated":false,"timed_out":false,"error":null,"file_output":null,"finalized":false});
                match mode {
                    "wrong_id" => outcome["id"] = uuid::Uuid::new_v4().to_string().into(),
                    "wrong_version" => outcome["version"] = 99.into(),
                    "oversized_output" => outcome["stdout_length"] = 5.into(),
                    "unknown_field" => outcome["invented_success"] = true.into(),
                    "oversized_header" => {
                        let _ = peer.write_u32(guest::MAX_HEADER as u32 + 1).await;
                        return;
                    }
                    "zero_header" => {
                        let _ = peer.write_u32(0).await;
                        return;
                    }
                    "malformed_header" => {
                        let _ = peer.write_all(b"\x00\x00\x00\x01{").await;
                        return;
                    }
                    _ => {}
                }
                bytes.clear();
                guest::write_header(&mut bytes, &outcome).unwrap();
                bytes.extend_from_slice(match mode {
                    "truncated_stream" => b"o",
                    "extra_stream" => b"ok!",
                    _ => b"ok",
                });
                let _ = peer.write_all(&bytes).await;
            });
            let result = tokio::time::timeout(
                Duration::from_secs(2),
                transact(
                    &socket,
                    request,
                    vec![],
                    Instant::now() + Duration::from_millis(200),
                    None,
                    None,
                ),
            )
            .await
            .expect("guest frame processing hung");
            if mode == "valid" {
                let result = result.unwrap();
                assert!(result.success);
                assert_eq!(result.stdout, "ok");
            } else {
                assert!(result.is_err(), "accepted invalid response: {mode}");
            }
            server.await.unwrap();
        }
    }
}
