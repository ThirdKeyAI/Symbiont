//! Kernel access restrictions and supervised native worker launch.
//!
//! Rules are built in the parent, because each root needs an open directory
//! descriptor. The socket filter is also built before fork, leaving only raw
//! installation syscalls for the pre-exec window in the child.
//! The crate defaults to `CompatLevel::BestEffort`, which silently
//! ignores unsupported requests; every ruleset here sets `HardRequirement`
//! instead so a boundary never enforces less than it claims.

use serde::{Deserialize, Serialize};

pub mod diagnostics;
pub mod workspace;

/// Operator-visible landlock settings. Paths come from `BoundaryRoots`, not
/// from here, so there is one way to declare a ceiling.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LandlockProfile {
    /// Lowest accepted Landlock ABI. Signal isolation always requires at least 6.
    pub abi_floor: u8,
    /// Deny new network sockets. False permits IPv4/IPv6 sockets, never host IPC.
    pub require_network: bool,
    pub memory_mib: u32,
    pub cpu_millis: u32,
    pub pids_limit: u32,
    pub max_execution_time: std::time::Duration,
    pub max_output_bytes: usize,
    pub supervisor: super::supervisor::SupervisorConfig,
    /// Runtime-issued snapshot authority; never restored from configuration.
    #[serde(skip)]
    pub workspace: Option<std::sync::Arc<workspace::Workspace>>,
    #[serde(skip)]
    pub executable: Option<std::path::PathBuf>,
}

impl Default for LandlockProfile {
    fn default() -> Self {
        Self {
            abi_floor: MIN_ABI,
            require_network: true,
            memory_mib: 512,
            cpu_millis: 1000,
            pids_limit: 128,
            max_execution_time: std::time::Duration::from_secs(300),
            max_output_bytes: 10 * 1024 * 1024,
            supervisor: super::supervisor::SupervisorConfig::default(),
            workspace: None,
            executable: None,
        }
    }
}

impl LandlockProfile {
    /// Grant only the operator-selected executable, never its containing directory.
    pub fn allow_executable(&mut self, path: &std::path::Path) -> Result<(), String> {
        use std::os::unix::fs::PermissionsExt;
        if !path.is_absolute() {
            return Err("managed executable must be an absolute file path".into());
        }
        let path = path
            .canonicalize()
            .map_err(|e| format!("cannot resolve managed executable: {e}"))?;
        let text = path
            .to_str()
            .ok_or("managed executable path must be UTF-8")?;
        if text.contains(':') || path.starts_with("/tmp") {
            return Err(
                "managed executable must be outside private /tmp and contain no colon".into(),
            );
        }
        let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
            return Err("managed executable must be an executable regular file".into());
        }
        validate_roots(
            self,
            &crate::sandbox::command::BoundaryRoots {
                source_roots: vec![text.into()],
                output_roots: vec![],
            },
        )?;
        self.executable = Some(path);
        Ok(())
    }

    pub(crate) fn clear_executable(&mut self) {
        self.executable = None;
    }

    pub fn executable(&self) -> Option<&std::path::Path> {
        self.executable.as_deref()
    }

    pub fn resources(&self) -> symbi_sandbox_supervisor::admission::WorkerResources {
        symbi_sandbox_supervisor::admission::WorkerResources {
            memory_bytes: u64::from(self.memory_mib) * 1024 * 1024,
            cpu_nanos: u64::from(self.cpu_millis) * 1_000_000,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        check_kernel(self)?;
        // SAFETY: geteuid has no memory preconditions.
        let uid = unsafe { libc::geteuid() };
        if uid == 0
            || self
                .supervisor
                .service_uid
                .is_some_and(|owner| owner != uid)
        {
            return Err(
                "Landlock workers require an unprivileged, same-UID delegated service".into(),
            );
        }
        self.supervisor.validate().map_err(|e| e.to_string())?;
        if self.memory_mib == 0
            || !(10..=1_024_000).contains(&self.cpu_millis)
            || !(1..=4096).contains(&self.pids_limit)
            || self.max_execution_time.is_zero()
            || self.max_execution_time > std::time::Duration::from_secs(86400)
            || !(1..=10 * 1024 * 1024).contains(&self.max_output_bytes)
        {
            return Err("invalid Landlock memory, CPU, PID, output or lifetime limit".into());
        }
        Ok(())
    }
}

/// Exercise actual admission, kernel restriction, exec and confirmed cleanup.
/// No project files or caller environment are exposed to this diagnostic worker.
pub async fn probe(profile: &LandlockProfile) -> anyhow::Result<()> {
    use std::{process::Stdio, time::Duration};
    let domain = prepare(profile, &Default::default()).map_err(anyhow::Error::msg)?;
    let mut command = tokio::process::Command::new("/usr/bin/true");
    command
        .env_clear()
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let (mut child, mut lease) =
        spawn(profile, domain, &mut command, Duration::from_secs(15)).await?;
    let outcome = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
    lease.finish().await?;
    let status = outcome.map_err(|_| anyhow::anyhow!("Landlock diagnostic worker timed out"))??;
    anyhow::ensure!(
        status.success(),
        "Landlock diagnostic worker failed: {status}"
    );
    Ok(())
}

/// A governed host worker owns its admission until independent cleanup confirms
/// the cgroup is gone. Dropping the lease closes the supervisor control channel.
pub(crate) async fn spawn(
    profile: &LandlockProfile,
    domain: PreparedDomain,
    command: &mut tokio::process::Command,
    timeout: std::time::Duration,
) -> anyhow::Result<(tokio::process::Child, super::supervisor::Lease)> {
    spawn_owned(profile, Some(domain), command, timeout, Vec::new()).await
}

async fn spawn_owned(
    profile: &LandlockProfile,
    domain: Option<PreparedDomain>,
    command: &mut tokio::process::Command,
    timeout: std::time::Duration,
    staging: Vec<uuid::Uuid>,
) -> anyhow::Result<(tokio::process::Child, super::supervisor::Lease)> {
    use symbi_sandbox_supervisor::protocol::{CreateHost, IMPLEMENTATION, VERSION};
    profile.validate().map_err(anyhow::Error::msg)?;
    let lifetime = timeout.min(profile.max_execution_time);
    anyhow::ensure!(
        !lifetime.is_zero(),
        "Landlock deadline expired before admission"
    );
    let deadline = std::time::Instant::now() + lifetime;
    let startup = deadline.min(std::time::Instant::now() + std::time::Duration::from_secs(10));
    let request = CreateHost {
        version: VERSION,
        implementation: IMPLEMENTATION.into(),
        lease: uuid::Uuid::new_v4(),
        resources: profile.resources(),
        pids_limit: profile.pids_limit,
        lifetime_ms: lifetime.as_millis().try_into()?,
        startup_ms: startup
            .saturating_duration_since(std::time::Instant::now())
            .as_millis()
            .try_into()?,
        origin: super::worker_origin::current(),
        staging,
    };
    let mut lease =
        super::supervisor::Lease::register_host(&profile.supervisor, request, startup).await?;
    let launch = async {
        let identity = lease.host_created(startup).await?;
        let group = identity
            .open()?
            .ok_or_else(|| anyhow::anyhow!("Landlock worker admission was removed"))?;
        let membership = group.file("cgroup.procs", true)?;
        // Join before restrictions and exec. Writing zero moves only this
        // calling child; a removed leaf rejects a delayed join through this FD.
        unsafe {
            command.pre_exec(move || {
                if libc::write(membership.as_raw_fd(), b"0".as_ptr().cast(), 1) != 1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        if let Some(domain) = domain {
            domain.apply_to(command);
        }
        anyhow::ensure!(
            std::time::Instant::now() < startup,
            "Landlock startup deadline expired"
        );
        Ok::<_, anyhow::Error>(command.spawn()?)
    }
    .await;
    match launch {
        Ok(child) => Ok((child, lease)),
        Err(error) => match lease.finish().await {
            Ok(()) => Err(error),
            Err(cleanup) => Err(anyhow::anyhow!(
                "{error}; delegated cleanup failed: {cleanup}"
            )),
        },
    }
}

/// The kernel's supported Landlock ABI, or 0 when unavailable.
///
/// The landlock crate keeps this private on purpose, to stop callers building
/// rules from an ABI discovered at run time. We only compare it against a
/// declared floor and record it in audit; rule construction always names a
/// fixed `ABI::V*`.
pub fn detect_abi() -> u8 {
    // SAFETY: the version query passes a null attribute pointer and size 0,
    // which the syscall defines as "report the supported ABI".
    let version = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<u8>(),
            0usize,
            1u32, // LANDLOCK_CREATE_RULESET_VERSION
        )
    };
    if version < 0 {
        0
    } else {
        version as u8
    }
}

/// Confirm the running kernel can enforce everything the profile declares.
pub fn check_kernel(profile: &LandlockProfile) -> Result<u8, String> {
    native_audit_arch()?;
    let detected = detect_abi();
    if detected == 0 {
        return Err("Landlock is unavailable on this kernel; no host fallback".into());
    }
    let floor = profile.abi_floor.max(MIN_ABI);
    if detected < floor {
        return Err(format!(
            "Landlock ABI {detected} is below the required floor {floor}; \
             filesystem, signal and socket isolation must all be enforceable"
        ));
    }
    Ok(detected)
}

use landlock::{
    Access, AccessFs, AccessNet, CompatLevel, Compatible, PathBeneath, Ruleset, RulesetAttr,
    RulesetCreated, RulesetCreatedAttr, Scope, ABI,
};
use std::os::{
    fd::{AsRawFd, OwnedFd},
    unix::{fs::OpenOptionsExt, process::CommandExt},
};
use std::sync::Arc;

/// Fixed ABI the rules are written against. Never derived from the kernel: a
/// ruleset built from a detected ABI would silently change meaning per host.
const RULE_ABI: ABI = ABI::V4;
/// ABI 6 introduced signal and abstract Unix socket scopes.
pub const MIN_ABI: u8 = 6;

/// A complete, immutable ruleset, ready to install on a child. The kernel holds
/// the granted objects; replacing a path after preparation cannot redirect it.
#[derive(Clone)]
pub struct PreparedDomain {
    ruleset: Arc<OwnedFd>,
    socket_filter: Arc<Vec<libc::sock_filter>>,
}

/// seccomp syscall numbers and argument offsets must match the worker ABI.
/// Reject compatibility ABIs instead of interpreting them with native numbers.
fn native_audit_arch() -> Result<u32, String> {
    match (
        std::env::consts::ARCH,
        cfg!(all(target_endian = "little", target_pointer_width = "64")),
    ) {
        ("x86_64", true) => Ok(0xc000_003e),
        ("aarch64", true) => Ok(0xc000_00b7),
        _ => {
            Err("Landlock socket isolation requires native little-endian x86_64 or aarch64".into())
        }
    }
}

/// Landlock's TCP rules do not cover UDP or pathname Unix sockets. Deny socket
/// creation except private Unix stream pairs and explicitly opted-in IP access.
/// io_uring must also be refused: it can perform socket operations without a
/// socket syscall. Existing descriptors are removed at exec by close_range.
fn socket_filter(require_network: bool) -> Result<Vec<libc::sock_filter>, String> {
    let stmt = |code: u32, k| libc::sock_filter {
        code: code as u16,
        jt: 0,
        jf: 0,
        k,
    };
    let eq = |k, jt, jf| libc::sock_filter {
        code: (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16,
        jt,
        jf,
        k,
    };
    let load = libc::BPF_LD | libc::BPF_W | libc::BPF_ABS;
    let ret = libc::BPF_RET | libc::BPF_K;
    let deny = libc::SECCOMP_RET_ERRNO | libc::EACCES as u32;
    let allow = libc::SECCOMP_RET_ALLOW;
    let mut filter = vec![
        stmt(load, std::mem::offset_of!(libc::seccomp_data, arch) as u32),
        eq(native_audit_arch()?, 1, 0),
        stmt(ret, libc::SECCOMP_RET_KILL_PROCESS),
        stmt(load, std::mem::offset_of!(libc::seccomp_data, nr) as u32),
    ];
    if cfg!(target_arch = "x86_64") {
        // x32 shares AUDIT_ARCH_X86_64 but sets this bit in the syscall number.
        filter.push(libc::sock_filter {
            code: (libc::BPF_JMP | libc::BPF_JSET | libc::BPF_K) as u16,
            jt: 0,
            jf: 1,
            k: 0x4000_0000,
        });
        filter.push(stmt(ret, libc::SECCOMP_RET_KILL_PROCESS));
    }
    for syscall in [
        libc::SYS_io_uring_setup,
        libc::SYS_io_uring_enter,
        libc::SYS_io_uring_register,
        libc::SYS_mount,
        libc::SYS_umount2,
        libc::SYS_pivot_root,
        libc::SYS_chroot,
        libc::SYS_unshare,
        libc::SYS_setns,
        libc::SYS_fsopen,
        libc::SYS_fsconfig,
        libc::SYS_fsmount,
        libc::SYS_open_tree,
        libc::SYS_move_mount,
        libc::SYS_mount_setattr,
    ] {
        filter.extend([eq(syscall as u32, 0, 1), stmt(ret, deny)]);
    }
    let first_argument = std::mem::offset_of!(libc::seccomp_data, args) as u32;
    // Force libc's inspectable clone fallback. Namespace creation must not
    // restore mount capabilities after native setup drops them.
    filter.extend([
        eq(libc::SYS_clone3 as u32, 0, 1),
        stmt(ret, libc::SECCOMP_RET_ERRNO | libc::ENOSYS as u32),
        eq(libc::SYS_clone as u32, 0, 4),
        stmt(load, first_argument),
        libc::sock_filter {
            code: (libc::BPF_JMP | libc::BPF_JSET | libc::BPF_K) as u16,
            jt: 0,
            jf: 1,
            k: (libc::CLONE_NEWUSER
                | libc::CLONE_NEWNS
                | libc::CLONE_NEWNET
                | libc::CLONE_NEWPID
                | libc::CLONE_NEWIPC
                | libc::CLONE_NEWUTS
                | libc::CLONE_NEWCGROUP) as u32,
        },
        stmt(ret, deny),
        stmt(ret, allow),
    ]);
    // Datagram pairs can send to unrelated pathname sockets with sendto();
    // connected stream pairs cannot change their peer. Preserve creation flags.
    filter.extend([
        eq(libc::SYS_socketpair as u32, 0, 8),
        stmt(load, first_argument),
        eq(libc::AF_UNIX as u32, 1, 0),
        stmt(ret, deny),
        stmt(load, first_argument + 8),
        stmt(
            libc::BPF_ALU | libc::BPF_AND | libc::BPF_K,
            !(libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK) as u32,
        ),
        eq(libc::SOCK_STREAM as u32, 1, 0),
        stmt(ret, deny),
        stmt(ret, allow),
    ]);
    if require_network {
        filter.extend([eq(libc::SYS_socket as u32, 0, 1), stmt(ret, deny)]);
    } else {
        filter.extend([
            eq(libc::SYS_socket as u32, 0, 5),
            stmt(load, first_argument),
            eq(libc::AF_INET as u32, 2, 0),
            eq(libc::AF_INET6 as u32, 1, 0),
            stmt(ret, deny),
            stmt(ret, allow),
        ]);
    }
    filter.push(stmt(ret, allow));
    Ok(filter)
}

/// Take the host path from a `host:virtual:ro` entry.
fn host_path(entry: &str) -> &str {
    entry.split(':').next().unwrap_or(entry)
}

/// Exact read-only metadata pinned to the managed primary process before exec.
pub(super) const PRIMARY_PROCESS_METADATA: &[&str] = &["/proc/self/maps", "/proc/self/stat"];

/// Read and execute access the child needs before it can run at all: its own
/// interpreter, the loader and the shared libraries they pull in. Without
/// these a dynamically linked program cannot even be exec'd, so the domain
/// would deny every command rather than confine it.
///
/// Deliberately narrow. It carries no writable path, nothing under a home
/// directory, and no broad `/etc` grant; anything else a workload needs is an
/// explicit boundary root.
const SYSTEM_PATHS: &[&str] = &[
    "/usr",
    "/bin",
    "/sbin",
    "/lib",
    "/lib64",
    "/etc/ld.so.cache",
    "/etc/ld.so.conf",
    "/etc/ld.so.conf.d",
    "/dev/null",
    "/dev/zero",
    "/dev/urandom",
];

/// Open once and derive the rule's file/directory rights from that descriptor.
/// Only absent optional system paths may be skipped. Declared roots must open;
/// the convenience `path_beneath_rules` helper silently discards open failures.
fn add_path(
    ruleset: &mut RulesetCreated,
    path: &str,
    writable: bool,
    optional: bool,
) -> Result<(), String> {
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if optional && error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("cannot open Landlock root {path:?}: {error}")),
    };
    let mut access = if writable {
        AccessFs::from_write(RULE_ABI)
    } else {
        AccessFs::from_read(RULE_ABI)
    };
    if !file.metadata().map_err(|e| e.to_string())?.is_dir() {
        access &= AccessFs::from_file(RULE_ABI);
    }
    ruleset
        .add_rule(PathBeneath::new(file, access))
        .map_err(|e| format!("cannot grant Landlock root {path:?}: {e}"))?;
    Ok(())
}

pub(super) fn validate_roots(
    profile: &LandlockProfile,
    roots: &crate::sandbox::command::BoundaryRoots,
) -> Result<(), String> {
    let supervisor = profile
        .supervisor
        .resolved_state_dir()
        .map_err(|e| e.to_string())?;
    for root in roots.source_roots.iter().chain(&roots.output_roots) {
        let path = std::path::Path::new(host_path(root))
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if [
            std::path::Path::new("/proc"),
            std::path::Path::new("/sys"),
            supervisor.as_path(),
        ]
        .iter()
        .any(|protected| path.starts_with(protected) || protected.starts_with(&path))
        {
            return Err("Landlock root exposes kernel controls or supervisor state".into());
        }
    }
    Ok(())
}

/// Resolve a boundary's roots into a domain that can be applied to children.
pub fn prepare(
    profile: &LandlockProfile,
    roots: &crate::sandbox::command::BoundaryRoots,
) -> Result<PreparedDomain, String> {
    check_kernel(profile)?;
    validate_roots(profile, roots)?;
    build_domain(profile, roots, false)
}

fn build_domain(
    profile: &LandlockProfile,
    roots: &crate::sandbox::command::BoundaryRoots,
    own_process_metadata: bool,
) -> Result<PreparedDomain, String> {
    let mut ruleset = Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .scope(Scope::Signal | Scope::AbstractUnixSocket)
        .map_err(|e| e.to_string())?
        .handle_access(AccessFs::from_all(RULE_ABI))
        .map_err(|e| e.to_string())?;
    if profile.require_network {
        ruleset = ruleset
            .handle_access(AccessNet::from_all(RULE_ABI))
            .map_err(|e| e.to_string())?;
    }
    let mut ruleset = ruleset.create().map_err(|e| e.to_string())?;
    for path in SYSTEM_PATHS {
        add_path(&mut ruleset, path, false, true)?;
    }
    // Process launchers use O_RDWR for discarded streams. This device grants
    // no persistent filesystem storage or access to other device nodes.
    add_path(&mut ruleset, "/dev/null", true, false)?;
    for path in &roots.source_roots {
        add_path(&mut ruleset, host_path(path), false, false)?;
    }
    for path in &roots.output_roots {
        add_path(&mut ruleset, host_path(path), true, false)?;
    }
    if own_process_metadata {
        for path in PRIMARY_PROCESS_METADATA {
            // Managed runtimes inspect their loaded image and memory usage. Pin
            // only this process's inodes; exec preserves them, forks get no new
            // grant. Configuration roots still cannot expose /proc or /sys.
            let file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_PATH | libc::O_CLOEXEC | libc::O_NOFOLLOW)
                .open(path)
                .map_err(|e| format!("cannot open managed process metadata {path}: {e}"))?;
            (&mut ruleset)
                .add_rule(PathBeneath::new(file, AccessFs::ReadFile))
                .map_err(|e| format!("cannot grant managed process metadata {path}: {e}"))?;
        }
    }
    // HardRequirement rejects any unsupported access during construction.
    // Keep only the CLOEXEC descriptor so installation cannot rebuild rules or
    // take an allocator lock in the forked child.
    let ruleset: Option<OwnedFd> = ruleset.into();
    Ok(PreparedDomain {
        ruleset: Arc::new(ruleset.ok_or("Landlock did not create an enforceable ruleset")?),
        socket_filter: Arc::new(socket_filter(profile.require_network)?),
    })
}

impl PreparedDomain {
    /// Enforce the prepared domain without allocation or path resolution.
    fn restrict(&self) -> std::io::Result<()> {
        let program = libc::sock_fprog {
            len: self.socket_filter.len() as u16,
            filter: self.socket_filter.as_ptr().cast_mut(),
        };
        // SAFETY: live descriptors and an immutable filter copied by the kernel;
        // no allocation, formatting, or library locks after fork. CLOEXEC keeps
        // Command's error pipe usable until exec, when all extra FDs close.
        unsafe {
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0
                || libc::syscall(
                    libc::SYS_landlock_restrict_self,
                    self.ruleset.as_raw_fd(),
                    0u32,
                ) != 0
                || libc::syscall(
                    libc::SYS_close_range,
                    3u32,
                    u32::MAX,
                    libc::CLOSE_RANGE_CLOEXEC,
                ) != 0
                || libc::prctl(
                    libc::PR_SET_SECCOMP,
                    libc::SECCOMP_MODE_FILTER,
                    &program,
                    0,
                    0,
                ) != 0
            {
                return Err(std::io::Error::last_os_error());
            }
        }
        Ok(())
    }

    /// Install on a std Command.
    pub fn apply_to_std(&self, command: &mut std::process::Command) {
        let domain = self.clone();
        // SAFETY: the closure runs between fork and exec and performs only
        // restriction syscalls using state prepared before the fork.
        unsafe {
            command.pre_exec(move || domain.restrict());
        }
    }

    /// Install on a tokio Command.
    pub fn apply_to(&self, command: &mut tokio::process::Command) {
        let domain = self.clone();
        // SAFETY: as above.
        unsafe {
            command.pre_exec(move || domain.restrict());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_below_the_floor_is_refused_rather_than_degraded() {
        let profile = LandlockProfile {
            abi_floor: 99,
            require_network: true,
            ..Default::default()
        };
        let error = check_kernel(&profile).expect_err("an impossible floor must fail");
        assert!(
            error.contains("99"),
            "error must name the required floor: {error}"
        );
    }

    #[test]
    fn a_lower_configured_floor_cannot_disable_ipc_isolation() {
        let profile = LandlockProfile {
            abi_floor: 1,
            require_network: false,
            ..Default::default()
        };
        if detect_abi() < MIN_ABI {
            assert!(check_kernel(&profile).is_err());
        } else {
            assert!(check_kernel(&profile).is_ok());
        }
    }
}
