//! Client for the independent container lifetime supervisor.
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{Duration, Instant},
};
use symbi_sandbox_supervisor::protocol::{
    self, Create, CreateVm, Reply, Request, IMPLEMENTATION, VERSION,
};

static EMBEDDED: OnceLock<PathBuf> = OnceLock::new();

/// An authenticated supervisor refused the request before registering or
/// starting this worker. Transport loss is never classified as a refusal.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct CreationRefused(String);

/// Shipping executables may serve the supervisor protocol themselves. SDKs can
/// install the standalone helper or select its path in the command profile.
pub fn use_embedded_helper() -> std::io::Result<()> {
    let _ = EMBEDDED.set(std::env::current_exe()?);
    Ok(())
}
pub use symbi_sandbox_supervisor::{run as run_service, INTERNAL_COMMAND};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SupervisorConfig {
    pub binary: PathBuf,
    pub state_dir: PathBuf,
    /// Connect only to an externally managed service owned by this uid.
    /// No helper is spawned if the service is unavailable.
    pub service_uid: Option<u32>,
}
impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            service_uid: None,
            binary: std::env::var_os("SYMBIONT_SANDBOX_SUPERVISOR")
                .map(PathBuf::from)
                .or_else(|| EMBEDDED.get().cloned())
                .unwrap_or_else(|| "symbi-sandbox-supervisor".into()),
            state_dir: std::env::var_os("SYMBIONT_SANDBOX_STATE_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    dirs::home_dir()
                        .unwrap_or_default()
                        .join(".symbiont/sandbox-leases")
                }),
        }
    }
}
impl SupervisorConfig {
    #[cfg(unix)]
    fn peer_uid(&self) -> u32 {
        // SAFETY: geteuid has no memory preconditions.
        self.service_uid
            .unwrap_or_else(|| unsafe { libc::geteuid() })
    }
    pub fn validate(&self) -> anyhow::Result<()> {
        protocol::socket_path(&self.state_dir)?;
        if self.binary.as_os_str().is_empty() || self.binary.to_str().is_none() {
            anyhow::bail!("invalid sandbox supervisor executable");
        }
        // An operator-selected shared pool is authoritative across backend
        // profiles. A per-project override must not silently mint capacity.
        if let Some(shared) = std::env::var_os("SYMBIONT_SANDBOX_STATE_DIR") {
            if self.state_dir != Path::new(&shared) {
                anyhow::bail!("supervisor state directory conflicts with SYMBIONT_SANDBOX_STATE_DIR; all backend profiles must use the configured shared capacity pool");
            }
        }
        Ok(())
    }
    pub fn resolved_state_dir(&self) -> anyhow::Result<PathBuf> {
        self.validate()?;
        // Resolve existing ancestors without creating anything during policy
        // preparation, so every possible bind overlap can be rejected first.
        let mut ancestor = self.state_dir.as_path();
        let mut suffix = Vec::new();
        loop {
            match std::fs::symlink_metadata(ancestor) {
                Ok(_) => break,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    suffix.push(
                        ancestor
                            .file_name()
                            .ok_or_else(|| anyhow::anyhow!("invalid supervisor state path"))?,
                    );
                    ancestor = ancestor
                        .parent()
                        .ok_or_else(|| anyhow::anyhow!("no supervisor state ancestor"))?;
                }
                Err(e) => return Err(e.into()),
            }
        }
        let mut resolved = ancestor.canonicalize()?;
        for part in suffix.into_iter().rev() {
            resolved.push(part);
        }
        protocol::socket_path(&resolved)?;
        Ok(resolved)
    }
}

pub(crate) fn resolve_binary(binary: &Path) -> anyhow::Result<PathBuf> {
    if binary.is_absolute() || binary.components().count() > 1 {
        return Ok(binary.canonicalize()?);
    }
    let search = std::env::var_os("PATH").unwrap_or_default();
    for directory in std::env::split_paths(&search) {
        let candidate = directory.join(binary);
        if candidate.is_file() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if candidate.metadata()?.permissions().mode() & 0o111 == 0 {
                    continue;
                }
            }
            return Ok(candidate.canonicalize()?);
        }
    }
    anyhow::bail!(
        "sandbox supervisor executable is unavailable: {}",
        binary.display()
    )
}

pub(crate) fn daemon_environment() -> std::collections::HashMap<String, String> {
    let mut environment = std::collections::HashMap::from([
        ("PATH".into(), "/usr/bin:/bin".into()),
        ("LANG".into(), "C.UTF-8".into()),
    ]);
    for key in [
        "HOME",
        "DOCKER_HOST",
        "DOCKER_CONTEXT",
        "DOCKER_CONFIG",
        "DOCKER_CERT_PATH",
        "DOCKER_TLS_VERIFY",
        "DOCKER_API_VERSION",
    ] {
        if let Ok(value) = std::env::var(key) {
            environment.insert(key.into(), value);
        }
    }
    environment
}

#[cfg(unix)]
mod client {
    use super::*;
    use anyhow::Context;
    use std::process::Stdio;
    use tokio::{
        io::BufReader,
        net::{unix::OwnedWriteHalf, UnixStream},
        sync::{oneshot, watch},
        time::{timeout, timeout_at},
    };

    async fn connect(path: &Path, uid: u32) -> anyhow::Result<UnixStream> {
        let stream = UnixStream::connect(path).await?;
        if stream.peer_cred()?.uid() != uid {
            anyhow::bail!("sandbox supervisor peer has another owner");
        }
        Ok(stream)
    }

    /// Read the existing authenticated service. Inspection must not bootstrap
    /// a helper, create lease storage or turn unavailable accounting into zero.
    pub async fn inspect(
        config: &SupervisorConfig,
        lease: Option<uuid::Uuid>,
    ) -> anyhow::Result<serde_json::Value> {
        let root = config.resolved_state_dir()?;
        let socket = protocol::socket_path(&root)?;
        timeout(Duration::from_secs(6), async {
            let stream = connect(&socket, config.peer_uid())
                .await
                .context("sandbox supervisor is unavailable; no service was started")?;
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            protocol::write_frame(
                &mut writer,
                &Request::Inspect {
                    version: VERSION,
                    implementation: IMPLEMENTATION.into(),
                    lease,
                },
            )
            .await?;
            match protocol::read_frame::<Reply>(&mut reader).await? {
                Some(Reply::Capacity { snapshot }) if lease.is_none() => {
                    Ok(serde_json::to_value(snapshot)?)
                }
                Some(Reply::Measurement { measurement }) if lease == Some(measurement.lease) => {
                    Ok(serde_json::to_value(measurement)?)
                }
                Some(Reply::Failed { message }) => anyhow::bail!(message),
                _ => anyhow::bail!("sandbox inspection protocol mismatch or unavailable evidence"),
            }
        })
        .await
        .context("sandbox inspection timed out")?
    }
    async fn ping(stream: UnixStream, deadline: Instant, require_jail: bool) -> anyhow::Result<()> {
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        timeout_at(deadline.into(),async {
            protocol::write_frame(&mut writer,&Request::Ping { version: VERSION, implementation: IMPLEMENTATION.into() }).await?;
            match protocol::read_frame::<Reply>(&mut reader).await? {
                Some(Reply::Ready { version,implementation,jailed_vms }) if version==VERSION && implementation==IMPLEMENTATION && (!require_jail || jailed_vms)=>Ok(()),
                _=>anyhow::bail!("sandbox supervisor protocol or implementation mismatch; drain the older service before upgrading"),
            }
        }).await.context("sandbox supervisor handshake timed out")?
    }

    async fn ensure_service(
        config: &SupervisorConfig,
        deadline: Instant,
    ) -> anyhow::Result<std::path::PathBuf> {
        let root = config.resolved_state_dir()?;
        let socket = protocol::socket_path(&root)?;
        match UnixStream::connect(&socket).await {
            Ok(stream) => {
                if stream.peer_cred()?.uid() != config.peer_uid() {
                    anyhow::bail!("sandbox supervisor peer has another owner");
                }
                ping(stream, deadline, config.service_uid == Some(0)).await?;
                return Ok(socket);
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) => {}
            Err(e) => return Err(e.into()),
        }
        if config.service_uid.is_some() {
            anyhow::bail!("managed sandbox supervisor unavailable; restore its host service before admitting work");
        }
        let binary =
            resolve_binary(&config.binary).context("sandbox supervisor executable unavailable")?;
        let mut command = tokio::process::Command::new(binary);
        command
            .arg(INTERNAL_COMMAND)
            .arg("--state-dir")
            .arg(&root)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("LANG", "C.UTF-8")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(false)
            .process_group(0);
        let mut child = command
            .spawn()
            .context("cannot start independent sandbox supervisor")?;
        tokio::spawn(async move {
            let _ = child.wait().await;
        });
        loop {
            if Instant::now() >= deadline {
                anyhow::bail!("sandbox supervisor unavailable before startup deadline; verify helper and private state permissions");
            }
            if let Ok(stream) = connect(&socket, config.peer_uid()).await {
                ping(stream, deadline, false).await?;
                return Ok(socket);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    enum Worker {
        Container(String),
        VirtualMachine(u32, PathBuf),
    }
    type Completion = Option<Result<Option<String>, String>>;
    pub(crate) struct Lease {
        writer: Option<OwnedWriteHalf>,
        created: Option<oneshot::Receiver<Result<Worker, String>>>,
        // The outer result is removal acknowledgement; an acknowledged VM
        // removal may still carry a failed workload/lifetime outcome.
        done: watch::Receiver<Completion>,
    }
    impl Lease {
        pub async fn register(
            config: &SupervisorConfig,
            request: Create,
            deadline: Instant,
        ) -> anyhow::Result<Self> {
            Self::register_request(config, Request::Create(Box::new(request)), deadline).await
        }
        pub async fn register_vm(
            config: &SupervisorConfig,
            request: CreateVm,
            deadline: Instant,
        ) -> anyhow::Result<Self> {
            Self::register_request(config, Request::CreateVm(Box::new(request)), deadline).await
        }
        async fn register_request(
            config: &SupervisorConfig,
            mut request: Request,
            deadline: Instant,
        ) -> anyhow::Result<Self> {
            let started = Instant::now();
            let socket = ensure_service(config, deadline).await?;
            let stream = timeout_at(deadline.into(), connect(&socket, config.peer_uid()))
                .await
                .context("sandbox supervisor connection timed out")??;
            let (reader, mut writer) = stream.into_split();
            let elapsed = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
            let (lease, lifetime, is_vm) = match &mut request {
                Request::Create(spec) => {
                    spec.lifetime_ms = spec.lifetime_ms.saturating_sub(elapsed);
                    spec.startup_ms = spec.startup_ms.saturating_sub(elapsed);
                    spec.validate()?;
                    (spec.lease, spec.lifetime_ms, false)
                }
                Request::CreateVm(spec) => {
                    spec.lifetime_ms = spec.lifetime_ms.saturating_sub(elapsed);
                    spec.startup_ms = spec.startup_ms.saturating_sub(elapsed);
                    spec.validate()?;
                    (spec.lease, spec.lifetime_ms, true)
                }
                _ => anyhow::bail!("invalid lease registration request"),
            };
            let expected_vsock = config
                .resolved_state_dir()?
                .join(format!("vm-{lease}"))
                .join("vsock");
            let reader_budget = Duration::from_millis(lifetime) + Duration::from_secs(90);
            timeout_at(
                deadline.into(),
                protocol::write_frame(&mut writer, &request),
            )
            .await
            .context("sandbox supervisor registration timed out")??;
            let (registered, registration) = oneshot::channel();
            let (created, creation) = oneshot::channel();
            let (finished, done) = watch::channel(None);
            tokio::spawn(async move {
                let mut registered = Some(registered);
                let mut created = Some(created);
                let read = async {
                    let mut reader = BufReader::new(reader);
                    let mut seen_registration = false;
                    let mut seen_creation = false;
                    let mut operation_failed = false;
                    let mut vm_failure = None;
                    loop {
                        match protocol::read_frame::<Reply>(&mut reader).await? {
                            Some(Reply::Registered { lease: received }) if received==lease && !seen_registration=>{
                                seen_registration=true;
                                if let Some(sender)=registered.take() { let _=sender.send(Ok(())); }
                            }
                            Some(Reply::Created { id }) if !is_vm && seen_registration && !seen_creation && !operation_failed=>{
                                if id.len()!=64 || !id.bytes().all(|b|b.is_ascii_hexdigit()) { anyhow::bail!("invalid supervised container identity"); }
                                seen_creation=true;
                                if let Some(sender)=created.take() { let _=sender.send(Ok(Worker::Container(id))); }
                            }
                            Some(Reply::VmCreated {pid,vsock_path}) if is_vm && seen_registration && !seen_creation && !operation_failed=>{
                                if pid<=1 || vsock_path!=expected_vsock {anyhow::bail!("invalid supervised VM identity");}
                                seen_creation=true;
                                if let Some(sender)=created.take() {let _=sender.send(Ok(Worker::VirtualMachine(pid,vsock_path)));}
                            }
                            Some(Reply::Failed { message })=>{
                                if !seen_registration {
                                    if let Some(sender)=registered.take() { let _=sender.send(Err(anyhow::Error::new(CreationRefused(message.clone())))); }
                                    if let Some(sender)=created.take() { let _=sender.send(Err(message)); }
                                    return Ok(None);
                                }
                                operation_failed=true;
                                if let Some(sender)=created.take() { let _=sender.send(Err(message.clone())); }
                                if is_vm {vm_failure=Some(message);} else if seen_creation { return Err(anyhow::anyhow!(message)); }
                            }
                            Some(Reply::Closed {}) if seen_registration=>return Ok(vm_failure),
                            None=>anyhow::bail!("supervisor connection ended without cleanup acknowledgement; durable recovery is required"),
                            _=>anyhow::bail!("unexpected supervisor lifecycle response"),
                        }
                    }
                };
                let result = match timeout(reader_budget, read).await {
                    Ok(result) => result.map_err(|e| e.to_string()),
                    Err(_) => Err(
                        "supervisor cleanup acknowledgement timed out; durable recovery retained"
                            .into(),
                    ),
                };
                if let Some(sender) = registered {
                    let _ = sender.send(Err(anyhow::Error::msg(
                        result
                            .clone()
                            .err()
                            .unwrap_or_else(|| "lease closed before registration".into()),
                    )));
                }
                if let Some(sender) = created {
                    let _ = sender
                        .send(Err(result.clone().err().unwrap_or_else(|| {
                            "lease closed before container readiness".into()
                        })));
                }
                finished.send_replace(Some(result));
            });
            timeout_at(deadline.into(), registration)
                .await
                .context("supervisor registration acknowledgement timed out")?
                .context("supervisor registration task ended")??;
            Ok(Self {
                writer: Some(writer),
                created: Some(creation),
                done,
            })
        }
        async fn worker(&mut self, deadline: Instant) -> anyhow::Result<Worker> {
            let created = self
                .created
                .take()
                .context("worker readiness already consumed")?;
            timeout_at(deadline.into(), created)
                .await
                .context("supervised worker creation timed out")?
                .context("worker creation owner ended")?
                .map_err(anyhow::Error::msg)
        }
        pub async fn created(&mut self, deadline: Instant) -> anyhow::Result<String> {
            match self.worker(deadline).await? {
                Worker::Container(id) => Ok(id),
                _ => anyhow::bail!("expected a supervised container"),
            }
        }
        pub async fn vm_created(&mut self, deadline: Instant) -> anyhow::Result<(u32, PathBuf)> {
            match self.worker(deadline).await? {
                Worker::VirtualMachine(pid, path) => Ok((pid, path)),
                _ => anyhow::bail!("expected a supervised VM"),
            }
        }
        pub async fn finish(&mut self) -> anyhow::Result<()> {
            match self.completion().await? {
                Some(message) => Err(anyhow::Error::msg(message)),
                None => Ok(()),
            }
        }
        /// Confirm removal independently of the supervised VM's outcome.
        pub async fn finish_cleanup(&mut self) -> anyhow::Result<()> {
            self.completion().await.map(|_| ())
        }
        async fn completion(&mut self) -> anyhow::Result<Option<String>> {
            self.writer.take(); // EOF transfers cancellation independently of Tokio.
            timeout(Duration::from_secs(20), async {
                loop {
                    if let Some(result) = self.done.borrow().clone() {
                        return result.map_err(anyhow::Error::msg);
                    }
                    self.done
                        .changed()
                        .await
                        .context("cleanup acknowledgement owner ended")?;
                }
            })
            .await
            .context("container cleanup still pending in durable supervisor")?
        }
    }
}
#[cfg(unix)]
pub use client::inspect;
#[cfg(unix)]
pub(crate) use client::Lease;

#[cfg(not(unix))]
pub(crate) struct Lease;
#[cfg(not(unix))]
impl Lease {
    pub async fn register(_: &SupervisorConfig, _: Create, _: Instant) -> anyhow::Result<Self> {
        anyhow::bail!("container supervision requires Unix local sockets")
    }
    pub async fn register_vm(
        _: &SupervisorConfig,
        _: CreateVm,
        _: Instant,
    ) -> anyhow::Result<Self> {
        anyhow::bail!("VM supervision requires Linux local sockets")
    }
    pub async fn vm_created(&mut self, _: Instant) -> anyhow::Result<(u32, PathBuf)> {
        anyhow::bail!("VM supervision requires Linux local sockets")
    }
    pub async fn created(&mut self, _: Instant) -> anyhow::Result<String> {
        anyhow::bail!("container supervision requires Unix local sockets")
    }
    pub async fn finish(&mut self) -> anyhow::Result<()> {
        anyhow::bail!("container supervision requires Unix local sockets")
    }
}
