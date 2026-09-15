//! A separate process owns Docker creation and durable recovery. A disconnected
//! caller cannot abandon an in-flight create or extend a container's lifetime.
use crate::{
    protocol::{self, Create, Reply, Request, IMPLEMENTATION, LABEL, VERSION},
    store::{valid_container_id, Phase, Record, Store},
};
use anyhow::Context;
use serde::Deserialize;
use std::{
    collections::HashSet,
    io::Write,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::Path,
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, BufReader},
    net::{UnixListener, UnixStream},
    process::Command,
    task::JoinSet,
    time::{sleep_until, timeout},
};

#[cfg(target_os = "linux")]
#[path = "virtual_machine.rs"]
mod virtual_machine;

const MAX_CONNECTIONS: usize = 32;
const CLIENT_WAIT: Duration = Duration::from_secs(5);
const DOCKER_WAIT: Duration = Duration::from_secs(15);
const CREATE_WAIT: Duration = Duration::from_secs(30);
const STREAM_LIMIT: usize = 64 * 1024;

pub(crate) struct State {
    store: Arc<Mutex<Store>>,
    active: Mutex<HashSet<uuid::Uuid>>,
    host: Option<Arc<crate::host::HostProfile>>,
}
impl State {
    pub(crate) async fn storage<T: Send + 'static>(
        &self,
        work: impl FnOnce(&Store) -> anyhow::Result<T> + Send + 'static,
    ) -> anyhow::Result<T> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store = store
                .lock()
                .map_err(|_| anyhow::anyhow!("lease store lock poisoned"))?;
            work(&store)
        })
        .await
        .context("lease storage task failed")?
    }
    async fn write(&self, record: &Record) -> anyhow::Result<()> {
        let record = record.clone();
        self.storage(move |s| s.write(&record)).await
    }
    async fn forget(&self, lease: uuid::Uuid) -> anyhow::Result<()> {
        self.storage(move |s| s.remove(lease)).await
    }
}

struct Active {
    state: Arc<State>,
    lease: uuid::Uuid,
}
impl Drop for Active {
    fn drop(&mut self) {
        if let Ok(mut active) = self.state.active.lock() {
            active.remove(&self.lease);
        }
    }
}

/// Run one service under an exclusive private-directory lock. Existing services
/// win startup races; their socket is never replaced by a second process.
pub async fn serve(root: &Path, persistent: bool) -> anyhow::Result<()> {
    serve_configured(root, persistent, None).await
}

#[cfg(target_os = "linux")]
pub async fn serve_host(profile: crate::host::HostProfile) -> anyhow::Result<()> {
    let root = profile.state_dir.clone();
    serve_configured(&root, true, Some(Arc::new(profile))).await
}

#[cfg(target_os = "linux")]
pub(crate) async fn reap_host(profile: crate::host::HostProfile) -> anyhow::Result<()> {
    let store = Store::open(&profile.state_dir)?
        .context("host supervisor is still running; refusing concurrent recovery")?;
    let records = store.records()?;
    let state = State {
        store: Arc::new(Mutex::new(store)),
        active: Mutex::new(HashSet::new()),
        host: Some(Arc::new(profile)),
    };
    for record in records {
        crate::host::reconcile_jail(&state, &record).await?;
    }
    Ok(())
}

async fn serve_configured(
    root: &Path,
    persistent: bool,
    host: Option<Arc<crate::host::HostProfile>>,
) -> anyhow::Result<()> {
    let Some(store) = Store::open(root)? else {
        return Ok(());
    };
    store.records()?; // Malformed durable state prevents new execution.
                      // SAFETY: geteuid has no memory preconditions.
    let client_uid = host
        .as_ref()
        .map_or_else(|| unsafe { libc::geteuid() }, |h| h.client_uid);
    let socket = protocol::socket_path(root)?;
    match std::fs::symlink_metadata(&socket) {
        Ok(metadata) => {
            // SAFETY: geteuid has no memory preconditions.
            if !metadata.file_type().is_socket() || metadata.uid() != client_uid {
                anyhow::bail!("unexpected entry at supervisor socket path");
            }
            std::fs::remove_file(&socket)?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let listener = UnixListener::bind(&socket)?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    if host.is_some() {
        let name = std::ffi::CString::new(socket.as_os_str().as_encoded_bytes())?;
        // SAFETY: the root-owned directory prevents path replacement.
        if unsafe { libc::chown(name.as_ptr(), client_uid, 0) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o711))?;
    }
    let state = Arc::new(State {
        store: Arc::new(Mutex::new(store)),
        active: Mutex::new(HashSet::new()),
        host,
    });
    let mut clients = JoinSet::new();
    let mut recovery = JoinSet::new();
    let mut reaper = tokio::time::interval(Duration::from_secs(1));
    let mut idle = Instant::now();
    let mut recovery_cursor = 0usize;
    #[cfg(target_os = "linux")]
    if state.host.is_some() {
        crate::host::notify("READY=1\nWATCHDOG=1")?;
    }
    loop {
        tokio::select! {
            accepted=listener.accept() => {
                let (mut stream,_)=accepted?;
                // A worker never gets this socket. Peer credentials also prevent
                // a differently owned process from impersonating the runtime.
                // SAFETY: geteuid has no memory preconditions.
                if stream.peer_cred()?.uid()!=client_uid { continue; }
                if clients.len()>=MAX_CONNECTIONS {
                    let _=timeout(CLIENT_WAIT,reply(&mut stream,&Reply::Failed { message:"supervisor connection capacity exhausted".into() })).await;
                    continue;
                }
                idle=Instant::now();
                let state=state.clone();
                clients.spawn(async move { handle(stream,state).await });
            }
            Some(result)=clients.join_next(), if !clients.is_empty() => {
                report_task(result);
            }
            Some(result)=recovery.join_next(), if !recovery.is_empty() => { report_task(result); }
            _=reaper.tick() => {
                #[cfg(target_os = "linux")]
                if state.host.is_some() { crate::host::notify("WATCHDOG=1")?; }
                let mut records=state.storage(|s| {
                    if let Err(error) = crate::staging::reap(&s.root) { eprintln!("staging recovery retained unresolved data: {error}"); }
                    s.records()
                }).await?;
                records.sort_by_key(|record| record.lease);
                for _ in 0..records.len() {
                    if recovery.len() >= MAX_CONNECTIONS { break; }
                    recovery_cursor %= records.len();
                    let record = &records[recovery_cursor];
                    recovery_cursor += 1;
                    let inserted=state.active.lock().map_err(|_| anyhow::anyhow!("lease registry poisoned"))?.insert(record.lease);
                    if inserted {
                        let active=Active { state:state.clone(),lease:record.lease };
                        let record=record.clone();
                        recovery.spawn(async move { reconcile(&active.state,&record).await.map(|_| ()) });
                    }
                }
                if !persistent && clients.is_empty() && recovery.is_empty() && records.is_empty() && idle.elapsed()>=Duration::from_secs(3) { break; }
            }
        }
    }
    std::fs::remove_file(socket)?;
    Ok(())
}

fn report_task(result: Result<anyhow::Result<()>, tokio::task::JoinError>) {
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => eprintln!("sandbox supervisor operation failed: {error}"),
        Err(error) => eprintln!("sandbox supervisor task failed: {error}"),
    }
}

async fn reply(writer: &mut (impl AsyncWrite + Unpin), value: &Reply) -> anyhow::Result<()> {
    timeout(CLIENT_WAIT, protocol::write_frame(writer, value))
        .await
        .context("supervisor response timed out")?
}

async fn handle(stream: UnixStream, state: Arc<State>) -> anyhow::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let request = timeout(CLIENT_WAIT, protocol::read_frame::<Request>(&mut reader)).await??;
    let spec = match request {
        Some(Request::Ping {
            version,
            implementation,
        }) if version == VERSION && implementation == IMPLEMENTATION => {
            reply(
                &mut writer,
                &Reply::Ready {
                    version: VERSION,
                    implementation: IMPLEMENTATION.into(),
                    jailed_vms: state.host.is_some(),
                },
            )
            .await?;
            return Ok(());
        }
        Some(Request::Create(spec)) => spec,
        Some(Request::Inspect {
            version,
            implementation,
            lease,
        }) if version == VERSION && implementation == IMPLEMENTATION => {
            let result = match lease {
                Some(lease) => crate::inspection::usage(state, lease)
                    .await
                    .map(|measurement| Reply::Measurement { measurement }),
                None => crate::inspection::capacity(&state)
                    .await
                    .map(|snapshot| Reply::Capacity { snapshot }),
            };
            reply(
                &mut writer,
                &result.unwrap_or_else(|error| Reply::Failed {
                    message: error.to_string(),
                }),
            )
            .await?;
            return Ok(());
        }
        Some(Request::CreateVm(spec)) => {
            #[cfg(target_os = "linux")]
            return virtual_machine::handle(reader, writer, state, *spec).await;
            #[cfg(not(target_os = "linux"))]
            {
                let _ = spec;
                anyhow::bail!("VM supervision requires Linux");
            }
        }
        _ => anyhow::bail!("invalid supervisor initial request"),
    };
    if let Err(error) = spec.validate() {
        reply(
            &mut writer,
            &Reply::Failed {
                message: error.to_string(),
            },
        )
        .await?;
        return Ok(());
    }
    let started = Instant::now();
    if state.host.is_some() {
        reply(
            &mut writer,
            &Reply::Failed {
                message: "jailed VM host service accepts only approved Firecracker requests".into(),
            },
        )
        .await?;
        return Ok(());
    }
    let lifetime = started + Duration::from_millis(spec.lifetime_ms);
    let startup = started + Duration::from_millis(spec.startup_ms);
    let mut record = Record::from_request(&spec);
    let inserted = state
        .active
        .lock()
        .map_err(|_| anyhow::anyhow!("lease registry poisoned"))?
        .insert(spec.lease);
    if !inserted {
        anyhow::bail!("lease already active");
    }
    let _active = Active {
        state: state.clone(),
        lease: spec.lease,
    };
    let registration = record.clone();
    if let Err(error) = state
        .storage(move |s| {
            let _pins = crate::staging::pin(&s.root, &registration.staging)?;
            s.insert(&registration)
        })
        .await
    {
        reply(
            &mut writer,
            &Reply::Failed {
                message: error.to_string(),
            },
        )
        .await?;
        return Ok(());
    }
    if reply(&mut writer, &Reply::Registered { lease: spec.lease })
        .await
        .is_err()
    {
        // The create process has not been started, so absence is definitive.
        state.forget(spec.lease).await?;
        return Ok(());
    }
    let root = state
        .storage(|s| {
            s.check_identity()?;
            Ok(s.root.clone())
        })
        .await?;
    let mut creation = tokio::spawn(async move { create_container(&spec, &root).await });
    let mut disconnected = Box::pin(protocol::read_frame::<Request>(&mut reader));
    let mut cancelled = false;
    let created = tokio::select! {
        biased;
        _=&mut disconnected => { cancelled=true; creation.await },
        _=sleep_until(startup.into()) => { cancelled=true; creation.await },
        result=&mut creation => result,
    };
    let id = match created {
        Ok(Ok(id)) => id,
        result => {
            record.state = Phase::Uncertain;
            state.write(&record).await?;
            let message = match result {
                Ok(Err(e)) => e.to_string(),
                Err(e) => format!("creation owner failed: {e}"),
                _ => unreachable!(),
            };
            let _ = reply(
                &mut writer,
                &Reply::Failed {
                    message: format!(
                        "container creation failed; durable cleanup retained: {message}"
                    ),
                },
            )
            .await;
            if reconcile(&state, &record).await? {
                let _ = reply(&mut writer, &Reply::Closed {}).await;
            }
            return Ok(());
        }
    };
    record.state = Phase::Created {
        container_id: id.clone(),
    };
    state.write(&record).await?;
    let ready = inspect(&record).await.and_then(|inspection| {
        let inspection = inspection
            .ok_or_else(|| anyhow::anyhow!("created container disappeared before verification"))?;
        let resources = record
            .resources
            .ok_or_else(|| anyhow::anyhow!("container has no admission reservation"))?;
        if inspection.memory_bytes == 0
            || inspection.memory_bytes > resources.memory_bytes
            || inspection.cpu_nanos == 0
            || inspection.cpu_nanos > resources.cpu_nanos
        {
            anyhow::bail!("actual Docker limits exceed the shared admission reservation");
        }
        if inspection.mounts.iter().any(|mount| {
            !matches!(
                mount.get("Type").and_then(|v| v.as_str()),
                Some("bind" | "tmpfs")
            )
        }) {
            anyhow::bail!(
                "Docker image declares an unapproved volume; use explicit bounded mounts"
            );
        }
        Ok(())
    });
    let ready = ready.and_then(|()| {
        if cancelled || Instant::now() >= startup || Instant::now() >= lifetime {
            anyhow::bail!("container initialization cancelled or timed out");
        }
        Ok(())
    });
    match ready {
        Ok(()) => {
            if reply(&mut writer, &Reply::Created { id }).await.is_ok() {
                tokio::select! { biased; _=&mut disconnected=>{}, _=sleep_until(lifetime.into())=>{} }
            }
        }
        Err(error) => {
            let _ = reply(
                &mut writer,
                &Reply::Failed {
                    message: error.to_string(),
                },
            )
            .await;
        }
    }
    if reconcile(&state, &record).await? {
        let _ = reply(&mut writer, &Reply::Closed {}).await;
    }
    Ok(())
}

async fn create_container(spec: &Create, root: &Path) -> anyhow::Result<String> {
    let mut command = daemon_command(&Record::from_request(spec));
    let mut arguments = spec.arguments.clone();
    arguments.splice(4..4, ["--label".into(), format!("{LABEL}={}", spec.lease)]);
    // The independent process owns this file for the entire client request.
    // Values are never exposed in process argv or persisted in lease records.
    let environment = if spec.environment.is_empty() {
        None
    } else {
        let mut file = tempfile::Builder::new()
            .prefix(".environment-")
            .tempfile_in(root)?;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
        for (key, value) in &spec.environment {
            writeln!(file, "{key}={value}")?;
        }
        file.flush()?;
        arguments.splice(
            4..4,
            [
                "--env-file".into(),
                file.path().to_string_lossy().into_owned(),
            ],
        );
        Some(file)
    };
    command.args(arguments);
    // Caller expiry never drops this request. A hard client failure instead
    // leaves an uncertain record for the service's durable reconciliation loop.
    let output = timeout(CREATE_WAIT, collect(command))
        .await
        .context("Docker create client timed out")??;
    drop(environment);
    let id = output.stdout.trim();
    if !output.success || !valid_container_id(id) {
        anyhow::bail!("Docker create failed: {}", output.stderr);
    }
    Ok(id.into())
}

#[derive(Deserialize)]
struct Inspection {
    id: String,
    labels: std::collections::HashMap<String, String>,
    mounts: Vec<serde_json::Value>,
    memory_bytes: u64,
    cpu_nanos: u64,
}

async fn inspect(record: &Record) -> anyhow::Result<Option<Inspection>> {
    let target = match &record.state {
        Phase::Created { container_id } => container_id.as_str(),
        _ => record.name.as_str(),
    };
    let mut command = daemon_command(record);
    command.args([
        "inspect",
        "--type",
        "container",
        "--format",
        r#"{"id":"{{.Id}}","labels":{{json .Config.Labels}},"mounts":{{json .Mounts}},"memory_bytes":{{.HostConfig.Memory}},"cpu_nanos":{{.HostConfig.NanoCpus}}}"#,
        target,
    ]);
    let output = timeout(DOCKER_WAIT, collect(command))
        .await
        .context("Docker inspection timed out")??;
    if !output.success {
        if output.stderr.contains("No such container") || output.stderr.contains("No such object") {
            return Ok(None);
        }
        anyhow::bail!("Docker inspection failed: {}", output.stderr);
    }
    let inspection: Inspection =
        serde_json::from_str(output.stdout.trim()).context("invalid Docker inspection")?;
    let (label, value) = record.expected_label();
    if !valid_container_id(&inspection.id) || inspection.labels.get(&label) != Some(&value) {
        anyhow::bail!("container ownership does not match the durable lease");
    }
    if let Phase::Created { container_id } = &record.state {
        if &inspection.id != container_id {
            anyhow::bail!("container identity changed");
        }
    }
    Ok(Some(inspection))
}

/// Unknown creation is resolved only after its owned container is observed and
/// removed. A missing name alone never clears a potentially delayed request.
async fn reconcile(state: &State, record: &Record) -> anyhow::Result<bool> {
    if matches!(record.state, Phase::VmCreating | Phase::VmCreated { .. }) {
        #[cfg(target_os = "linux")]
        return virtual_machine::reconcile(state, record).await;
        #[cfg(not(target_os = "linux"))]
        anyhow::bail!("VM recovery requires Linux");
    }
    if let Some(inspection) = inspect(record).await? {
        let mut command = daemon_command(record);
        command.args(["rm", "--force", "--volumes", &inspection.id]);
        let output = timeout(DOCKER_WAIT, collect(command))
            .await
            .context("Docker container removal timed out")??;
        if !output.success && !output.stderr.contains("No such container") {
            anyhow::bail!("Docker container removal failed: {}", output.stderr);
        }
        state.forget(record.lease).await?;
        return Ok(true);
    }
    if matches!(record.state, Phase::Created { .. }) {
        state.forget(record.lease).await?;
        return Ok(true);
    }
    Ok(false)
}

pub(crate) fn daemon_command(record: &Record) -> Command {
    let mut command = Command::new(&record.docker_binary);
    command
        .env_clear()
        .envs(&record.docker_environment)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}
pub(crate) struct Output {
    pub(crate) success: bool,
    pub(crate) stdout: String,
    stderr: String,
}
async fn read_limited(mut reader: impl AsyncRead + Unpin) -> anyhow::Result<String> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            return Ok(String::from_utf8_lossy(&bytes).into_owned());
        }
        if bytes.len().saturating_add(count) > STREAM_LIMIT {
            anyhow::bail!("Docker control output exceeded 64 KiB");
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
}
pub(crate) async fn collect(mut command: Command) -> anyhow::Result<Output> {
    let mut child = command.spawn()?;
    let stdout = child
        .stdout
        .take()
        .context("missing Docker control stdout")?;
    let stderr = child
        .stderr
        .take()
        .context("missing Docker control stderr")?;
    let (stdout, stderr) = tokio::try_join!(read_limited(stdout), read_limited(stderr))?;
    let status = child.wait().await?;
    Ok(Output {
        success: status.success(),
        stdout,
        stderr,
    })
}
