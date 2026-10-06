//! Private native workspaces. Only staged objects cross into the worker.
//! A trusted, separately exec'd setup process owns namespace construction;
//! arbitrary payloads start only after mounts, limits and Landlock are installed.
use super::LandlockProfile;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    ffi::CString,
    fs::OpenOptions,
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::{fs::OpenOptionsExt, net::UnixStream, process::CommandExt},
    },
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

pub const INTERNAL_COMMAND: &str = "__landlock_worker";
pub(crate) const WORKSPACE: &str = "/tmp/symbi-workspace";
pub(crate) const SPEC_LIMIT: u64 = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Mount {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub read_only: bool,
}

/// An unforgeable-in-configuration reference to a live staging allocation.
#[derive(Debug)]
pub struct Workspace {
    pub(crate) directory: Arc<symbi_sandbox_supervisor::staging::Lease>,
    mounts: Vec<Mount>,
    working_dir: PathBuf,
    file_limit: u64,
    channels: Option<[PathBuf; 2]>,
}
impl Workspace {
    pub(crate) fn descriptor(&self) -> serde_json::Value {
        serde_json::json!({"transport":"private_tmpfs", "working_dir":self.working_dir,
            "network": if self.channels.is_some() { "private_loopback" } else { "selected_profile" },
            "inherited_channels": if self.channels.is_some() { vec!["tools", "inference"] } else { vec![] },
            "staging_id":self.directory.id(), "max_file_bytes":self.file_limit,
            "mounts":self.mounts.iter().map(|mount| serde_json::json!({
                "destination":mount.destination,"read_only":mount.read_only
            })).collect::<Vec<_>>()})
    }
    pub(crate) fn new(
        directory: Arc<symbi_sandbox_supervisor::staging::Lease>,
        mounts: Vec<Mount>,
        working_dir: PathBuf,
        file_limit: u64,
    ) -> anyhow::Result<Arc<Self>> {
        private_path(&working_dir)?;
        let root = directory.path().canonicalize()?;
        for mount in &mounts {
            private_path(&mount.destination)?;
            anyhow::ensure!(
                mount.source.canonicalize()?.starts_with(&root),
                "native mount is outside its staging allocation"
            );
        }
        anyhow::ensure!(
            file_limit > 0 && file_limit <= 64 * 1024 * 1024,
            "invalid native file limit"
        );
        Ok(Arc::new(Self {
            directory,
            mounts,
            working_dir,
            file_limit,
            channels: None,
        }))
    }

    pub(crate) fn managed(
        profile: &LandlockProfile,
        tools: PathBuf,
        inference: PathBuf,
    ) -> anyhow::Result<Arc<Self>> {
        let mut workspace = Self::empty(profile)?;
        Arc::get_mut(&mut workspace)
            .context("native workspace already shared")?
            .channels = Some([tools, inference]);
        Ok(workspace)
    }

    pub(crate) fn has_channels(&self) -> bool {
        self.channels.is_some()
    }

    fn empty(profile: &LandlockProfile) -> anyhow::Result<Arc<Self>> {
        let directory = Arc::new(symbi_sandbox_supervisor::staging::Lease::reserve(
            &profile.supervisor.resolved_state_dir()?,
            SPEC_LIMIT + 32768,
        )?);
        Self::new(directory, Vec::new(), WORKSPACE.into(), 16 * 1024 * 1024)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Spec {
    profile: LandlockProfile,
    mounts: Vec<Mount>,
    working_dir: PathBuf,
    file_limit: u64,
    scratch_bytes: u64,
    argv: Vec<String>,
    environment: HashMap<String, String>,
    #[serde(default)]
    channel_fds: Option<[i32; 2]>,
    #[serde(default)]
    executable: Option<PathBuf>,
}

fn private_path(path: &Path) -> anyhow::Result<()> {
    anyhow::ensure!(
        path.starts_with("/tmp")
            && path != Path::new("/tmp")
            && !path
                .components()
                .any(|part| matches!(part, Component::ParentDir | Component::CurDir)),
        "native workspace destination must be below private /tmp"
    );
    Ok(())
}

pub(crate) async fn spawn(
    profile: &LandlockProfile,
    argv: &[String],
    environment: HashMap<String, String>,
    timeout: Duration,
) -> anyhow::Result<(tokio::process::Child, crate::sandbox::supervisor::Lease)> {
    profile.validate().map_err(anyhow::Error::msg)?;
    crate::sandbox::command::literal_command(argv).map_err(anyhow::Error::msg)?;
    let workspace = match &profile.workspace {
        Some(workspace) => workspace.clone(),
        None => Workspace::empty(profile)?,
    };
    let channels = workspace
        .channels
        .as_ref()
        .map(|paths| {
            Ok::<_, std::io::Error>([
                UnixStream::connect(&paths[0])?,
                UnixStream::connect(&paths[1])?,
            ])
        })
        .transpose()?;
    let channel_fds = channels
        .as_ref()
        .map(|streams| [streams[0].as_raw_fd(), streams[1].as_raw_fd()]);
    let spec = Spec {
        profile: profile.clone(),
        mounts: workspace.mounts.clone(),
        working_dir: workspace.working_dir.clone(),
        file_limit: workspace.file_limit,
        scratch_bytes: (u64::from(profile.memory_mib) * 1024 * 1024 / 2)
            .clamp(1024 * 1024, 128 * 1024 * 1024),
        argv: argv.to_owned(),
        environment,
        channel_fds,
        executable: profile.executable.clone(),
    };
    let encoded = serde_json::to_vec(&spec)?;
    anyhow::ensure!(
        encoded.len() as u64 <= SPEC_LIMIT,
        "native launch contract exceeds 1 MiB"
    );
    let path = workspace.directory.path().join("launch.json");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .context("native workspace is single-use")?;
    file.write_all(&encoded)?;
    let mut command =
        tokio::process::Command::new(crate::sandbox::supervisor::native_worker_binary()?);
    command
        .arg(INTERNAL_COMMAND)
        .arg(&path)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LANG", "C.UTF-8")
        .current_dir("/")
        .process_group(0)
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(channels) = channels {
        // Only these connected, session-scoped sockets survive the helper exec.
        // The closure retains ownership and performs no allocation after fork.
        unsafe {
            command.pre_exec(move || {
                if libc::syscall(
                    libc::SYS_close_range,
                    3u32,
                    u32::MAX,
                    libc::CLOSE_RANGE_CLOEXEC,
                ) != 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                for stream in &channels {
                    if libc::fcntl(stream.as_raw_fd(), libc::F_SETFD, 0) == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
    }
    super::spawn_owned(
        profile,
        None,
        &mut command,
        timeout,
        vec![workspace.directory.id()],
    )
    .await
}

fn mount(
    source: Option<&str>,
    target: &Path,
    kind: Option<&str>,
    flags: libc::c_ulong,
    data: Option<&str>,
) -> anyhow::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let source = source.map(CString::new).transpose()?;
    let target = CString::new(target.as_os_str().as_bytes())?;
    let kind = kind.map(CString::new).transpose()?;
    let data = data.map(CString::new).transpose()?;
    // SAFETY: all C strings remain live through this synchronous syscall.
    let result = unsafe {
        libc::mount(
            source.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
            target.as_ptr(),
            kind.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
            flags,
            data.as_ref()
                .map_or(std::ptr::null(), |s| s.as_ptr().cast()),
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error()).context("private workspace mount failed");
    }
    Ok(())
}

fn enable_loopback() -> anyhow::Result<()> {
    use std::os::fd::FromRawFd;
    // SAFETY: the socket and zeroed ifreq are used only with Linux interface ioctls.
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0);
        anyhow::ensure!(fd >= 0, "cannot open private interface control socket");
        let socket = std::os::fd::OwnedFd::from_raw_fd(fd);
        let mut request: libc::ifreq = std::mem::zeroed();
        request.ifr_name[0] = b'l' as libc::c_char;
        request.ifr_name[1] = b'o' as libc::c_char;
        anyhow::ensure!(
            libc::ioctl(socket.as_raw_fd(), libc::SIOCGIFFLAGS, &mut request) == 0,
            "cannot read private loopback flags"
        );
        request.ifr_ifru.ifru_flags |= libc::IFF_UP as libc::c_short;
        anyhow::ensure!(
            libc::ioctl(socket.as_raw_fd(), libc::SIOCSIFFLAGS, &request) == 0,
            "cannot enable private loopback"
        );
    }
    Ok(())
}

fn drop_capabilities() -> anyhow::Result<()> {
    #[repr(C)]
    struct Header {
        version: u32,
        pid: i32,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Data {
        effective: u32,
        permitted: u32,
        inheritable: u32,
    }
    let header = Header {
        version: 0x20080522,
        pid: 0,
    };
    let data = [Data {
        effective: 0,
        permitted: 0,
        inheritable: 0,
    }; 2];
    // SAFETY: Linux capability ABI v3 consists of one header and two data words.
    anyhow::ensure!(
        unsafe { libc::syscall(libc::SYS_capset, &header, data.as_ptr()) } == 0,
        "cannot drop native setup capabilities: {}",
        std::io::Error::last_os_error()
    );
    Ok(())
}

async fn read_bounded(
    mut reader: impl tokio::io::AsyncRead + Unpin,
    limit: usize,
) -> anyhow::Result<String> {
    use tokio::io::AsyncReadExt;
    let mut output = Vec::new();
    (&mut reader)
        .take(limit as u64 + 1)
        .read_to_end(&mut output)
        .await?;
    anyhow::ensure!(output.len() <= limit, "native worker output limit exceeded");
    Ok(String::from_utf8_lossy(&output).into_owned())
}

pub(crate) async fn execute(
    profile: LandlockProfile,
    argv: Vec<String>,
    timeout: Duration,
) -> Result<crate::sandbox::ExecutionResult, String> {
    use crate::sandbox::{command_cleanup, worker_origin, ExecutionResult};
    let registration = command_cleanup::register()?;
    let (keep_alive, mut cancelled) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(worker_origin::inherit(async move {
        let started = std::time::Instant::now();
        let lifetime = timeout.min(profile.max_execution_time);
        let (mut child, mut lease) = match spawn(&profile, &argv, HashMap::new(), lifetime).await {
            Ok(worker) => worker,
            Err(error) => {
                if error
                    .downcast_ref::<crate::sandbox::supervisor::CreationRefused>()
                    .is_some()
                {
                    if let Some(owner) = &registration {
                        owner.finish(Ok(()));
                    }
                }
                return Err(error);
            }
        };
        drop(child.stdin.take());
        let stdout = child.stdout.take().context("missing native stdout")?;
        let stderr = child.stderr.take().context("missing native stderr")?;
        let stop = registration
            .as_ref()
            .map(|owner| owner.stop.clone())
            .unwrap_or_default();
        let output = async {
            tokio::try_join!(
                read_bounded(stdout, profile.max_output_bytes),
                read_bounded(stderr, profile.max_output_bytes),
                async {
                    let status = child.wait().await?;
                    // Descendants may retain the output pipes. Revoke their
                    // lifetime as soon as the main process exits, before drain.
                    lease.finish().await?;
                    Ok::<_, anyhow::Error>(status)
                },
            )
        };
        let outcome = tokio::select! {
            biased;
            _ = &mut cancelled => Err(anyhow::anyhow!("native execution cancelled")),
            _ = stop.cancelled() => Err(anyhow::anyhow!("native execution run cancelled")),
            result = tokio::time::timeout(lifetime.saturating_sub(started.elapsed()), output) =>
                result.unwrap_or_else(|_| Err(anyhow::anyhow!("native execution timed out"))),
        };
        let cleanup = lease.finish().await;
        let reaped = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
        let cleanup = cleanup.and_then(|_| {
            reaped.context("native child reaping timed out")??;
            Ok(())
        });
        if let Some(owner) = registration {
            owner.finish(
                cleanup
                    .as_ref()
                    .map(|_| ())
                    .map_err(|error| error.to_string()),
            );
        }
        cleanup?;
        let (stdout, stderr, status) = outcome?;
        Ok::<_, anyhow::Error>(ExecutionResult {
            stdout,
            stderr,
            exit_code: status.code().unwrap_or(-1),
            success: status.success(),
            execution_time_ms: started.elapsed().as_millis() as u64,
            stdout_truncated: false,
            stderr_truncated: false,
        })
    }));
    let result = task
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string());
    drop(keep_alive);
    result
}

pub(crate) async fn parse(
    profile: &LandlockProfile,
    program: &str,
    input: &str,
    timeout: Duration,
) -> Result<crate::sandbox::ExecutionResult, String> {
    let directory = Arc::new(
        symbi_sandbox_supervisor::staging::Lease::reserve(
            &profile
                .supervisor
                .resolved_state_dir()
                .map_err(|error| error.to_string())?,
            SPEC_LIMIT + input.len() as u64 + 32768,
        )
        .map_err(|error| error.to_string())?,
    );
    let source = directory.path().join("parser-input");
    std::fs::write(&source, input).map_err(|error| error.to_string())?;
    let mut selected = profile.clone();
    // The parser gets only the captured result, never the tool's file grants.
    selected.workspace = Some(
        Workspace::new(
            directory,
            vec![Mount {
                source,
                destination: "/tmp/symbi-parser-input".into(),
                read_only: true,
            }],
            WORKSPACE.into(),
            16 * 1024 * 1024,
        )
        .map_err(|error| error.to_string())?,
    );
    execute(
        selected,
        vec![program.into(), "/tmp/symbi-parser-input".into()],
        timeout,
    )
    .await
}

/// Shipping entry point, called before loading project files or environment.
pub fn run(path: &Path) -> anyhow::Result<()> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    anyhow::ensure!(
        file.metadata()?.is_file(),
        "native launch contract is not a file"
    );
    let mut bytes = Vec::new();
    file.take(SPEC_LIMIT + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= SPEC_LIMIT,
        "native launch contract exceeds limit"
    );
    let mut spec: Spec = serde_json::from_slice(&bytes)?;
    if let Some(path) = &spec.executable {
        spec.profile
            .allow_executable(path)
            .map_err(anyhow::Error::msg)?;
    }
    spec.profile.validate().map_err(anyhow::Error::msg)?;
    crate::sandbox::command::literal_command(&spec.argv).map_err(anyhow::Error::msg)?;
    private_path(&spec.working_dir)?;
    anyhow::ensure!(
        (1024 * 1024..=128 * 1024 * 1024).contains(&spec.scratch_bytes)
            && (1..=64 * 1024 * 1024).contains(&spec.file_limit),
        "invalid native scratch limits"
    );
    // SAFETY: this setup entry point is single-threaded and executes no user code.
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
    anyhow::ensure!(
        unsafe {
            libc::unshare(
                libc::CLONE_NEWUSER
                    | libc::CLONE_NEWNS
                    | if spec.channel_fds.is_some() {
                        libc::CLONE_NEWNET
                    } else {
                        0
                    },
            )
        } == 0,
        "native workspaces require unprivileged user/mount namespaces: {}",
        std::io::Error::last_os_error()
    );
    std::fs::write("/proc/self/setgroups", "deny")?;
    std::fs::write("/proc/self/uid_map", format!("{uid} {uid} 1\n"))?;
    std::fs::write("/proc/self/gid_map", format!("{gid} {gid} 1\n"))?;
    mount(
        None,
        Path::new("/"),
        None,
        libc::MS_REC | libc::MS_PRIVATE,
        None,
    )?;
    // Open sources in this mount namespace before shadowing /tmp, where an
    // operator may keep the pool. Descriptors opened before unshare still
    // belong to the old mount namespace and cannot be bind-mounted here.
    let sources = spec
        .mounts
        .iter()
        .map(|item| {
            private_path(&item.destination)?;
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_PATH | libc::O_CLOEXEC | libc::O_NOFOLLOW)
                .open(&item.source)?;
            let metadata = file.metadata()?;
            anyhow::ensure!(
                metadata.is_file() || (item.read_only && metadata.is_dir()),
                "invalid native snapshot object"
            );
            Ok((file, metadata.is_dir()))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    mount(
        Some("tmpfs"),
        Path::new("/tmp"),
        Some("tmpfs"),
        libc::MS_NOSUID | libc::MS_NODEV,
        Some(&format!(
            "size={},nr_inodes=4096,mode=700,uid={uid},gid={gid}",
            spec.scratch_bytes
        )),
    )?;
    for (item, (source, directory)) in spec.mounts.iter().zip(&sources) {
        std::fs::create_dir_all(
            item.destination
                .parent()
                .context("missing native target parent")?,
        )?;
        if *directory {
            std::fs::create_dir(&item.destination)?;
        } else {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&item.destination)?;
        }
        mount(
            Some(&format!("/proc/self/fd/{}", source.as_raw_fd())),
            &item.destination,
            None,
            libc::MS_BIND,
            None,
        )?;
        let flags = libc::MS_BIND
            | libc::MS_REMOUNT
            | libc::MS_NOSUID
            | libc::MS_NODEV
            | if item.read_only {
                libc::MS_RDONLY | libc::MS_NOEXEC
            } else {
                0
            };
        mount(None, &item.destination, None, flags, None)?;
    }
    std::fs::create_dir_all(&spec.working_dir)?;
    std::fs::create_dir_all("/tmp/symbi-home")?;
    let limit = libc::rlimit {
        rlim_cur: spec.file_limit,
        rlim_max: spec.file_limit,
    };
    // SAFETY: valid limit pointer; both limits apply to this process and descendants.
    anyhow::ensure!(
        unsafe { libc::setrlimit(libc::RLIMIT_FSIZE, &limit) } == 0,
        "cannot enforce native file size limit"
    );
    if spec.channel_fds.is_some() {
        enable_loopback()?;
    }
    drop_capabilities()?;
    let mut roots = crate::sandbox::command::BoundaryRoots {
        source_roots: vec!["/tmp".into()],
        output_roots: vec!["/tmp".into()],
    };
    if let Some(path) = spec.profile.executable() {
        roots
            .source_roots
            .push(path.to_str().context("invalid executable encoding")?.into());
    }
    // Only this freshly created private tmpfs bypasses the host-root validator.
    // Read-only snapshot mounts independently deny mutation even within /tmp.
    let mut payload_profile = spec.profile.clone();
    if spec.channel_fds.is_some() {
        // IP sockets operate only in the freshly created loopback namespace.
        payload_profile.require_network = false;
    }
    let domain = super::build_domain(&payload_profile, &roots, spec.executable.is_some())
        .map_err(anyhow::Error::msg)?;
    let mut command = std::process::Command::new(&spec.argv[0]);
    command
        .args(&spec.argv[1..])
        .env_clear()
        .envs(spec.environment)
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("LANG", "C.UTF-8")
        .env("HOME", "/tmp/symbi-home")
        .env("TMPDIR", "/tmp")
        .current_dir(&spec.working_dir);
    if let Some(fds) = spec.channel_fds {
        anyhow::ensure!(
            fds[0] > 2 && fds[1] > 2 && fds[0] != fds[1],
            "invalid broker descriptors"
        );
        command
            .env("SYMBI_TOOLS_FD", fds[0].to_string())
            .env("SYMBI_INFERENCE_FD", fds[1].to_string());
        domain.apply_to_std(&mut command);
        // This hook runs after the domain has marked all other FDs close-on-exec.
        unsafe {
            command.pre_exec(move || {
                for fd in fds {
                    if libc::fcntl(fd, libc::F_SETFD, 0) == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
    } else {
        domain.apply_to_std(&mut command);
    }
    Err(command.exec()).context("restricted native payload exec failed")
}
