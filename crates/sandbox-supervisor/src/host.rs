//! Root-owned Firecracker host policy. Client input cannot select privileged artifacts.
use crate::{protocol::CreateVm, store::Record};
use anyhow::Context;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostProfile {
    pub state_dir: PathBuf,
    pub client_uid: u32,
    pub uid_start: u32,
    pub uid_count: u32,
    pub cgroup_parent: String,
    pub memory_overhead_mib: u32,
    pub max_lifetime_ms: u64,
    pub jailer: Artifact,
    pub firecracker: Artifact,
    pub kernel: Artifact,
    pub rootfs: Artifact,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JailLease {
    pub uid: u32,
    pub cgroup_parent: String,
}

fn valid_parent(value: &str) -> bool {
    value.starts_with("symbi")
        && value.ends_with(".slice")
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

impl JailLease {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.uid < 65536 || !valid_parent(&self.cgroup_parent) {
            anyhow::bail!("invalid jailed VM identity or cgroup parent");
        }
        Ok(())
    }
    pub fn cgroup(&self, lease: uuid::Uuid) -> PathBuf {
        Path::new("/sys/fs/cgroup")
            .join(&self.cgroup_parent)
            .join(lease.to_string())
    }
}

/// Reject every link and every unprivileged-writable ancestor before root use.
pub fn trusted_path(path: &Path) -> anyhow::Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        anyhow::bail!("host policy paths must be absolute without traversal");
    }
    for part in path.ancestors() {
        let metadata = std::fs::symlink_metadata(part)?;
        if metadata.file_type().is_symlink() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0
        {
            anyhow::bail!(
                "host policy path must be root-owned and not writable by other users: {}",
                part.display()
            );
        }
    }
    Ok(())
}

impl Artifact {
    fn verify(&self) -> anyhow::Result<()> {
        trusted_path(&self.path)?;
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&self.path)?;
        if !file.metadata()?.is_file() || self.sha256.len() != 64 {
            anyhow::bail!("invalid host artifact");
        }
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 65536];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        if format!("{:x}", hash.finalize()) != self.sha256 {
            anyhow::bail!("host artifact digest mismatch: {}", self.path.display());
        }
        Ok(())
    }
}

impl HostProfile {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        Self::load_mode(path, true)
    }
    pub(crate) fn load_for_recovery(path: &Path) -> anyhow::Result<Self> {
        Self::load_mode(path, false)
    }
    fn load_mode(path: &Path, launch: bool) -> anyhow::Result<Self> {
        // SAFETY: geteuid has no memory preconditions.
        if unsafe { libc::geteuid() } != 0 {
            anyhow::bail!("jailed VM host service requires root");
        }
        if launch
            && (std::env::var("WATCHDOG_PID")
                .ok()
                .and_then(|v| v.parse::<u32>().ok())
                != Some(std::process::id())
                || !std::env::var("WATCHDOG_USEC")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .is_some_and(|v| (1..=15_000_000).contains(&v)))
        {
            anyhow::bail!("jailed VM host requires an independent service manager with a watchdog of at most 15 seconds");
        }
        trusted_path(path)?;
        let mut bytes = Vec::new();
        File::open(path)?.take(16385).read_to_end(&mut bytes)?;
        if bytes.len() > 16384 {
            anyhow::bail!("host profile exceeds 16 KiB");
        }
        let profile: Self = serde_json::from_slice(&bytes)?;
        profile.validate()?;
        trusted_path(&profile.state_dir)?;
        if !launch {
            return Ok(profile);
        }
        let accounts = std::fs::read_to_string("/etc/passwd")?;
        let groups = std::fs::read_to_string("/etc/group")?;
        let prefix = profile.cgroup_parent.trim_end_matches(".slice");
        for offset in 0..profile.uid_count {
            let id = (profile.uid_start + offset).to_string();
            let name = format!("{prefix}v{offset}");
            let account_matches = accounts.lines().any(|line| {
                let fields: Vec<_> = line.split(':').collect();
                fields.len() == 7
                    && fields[0] == name
                    && fields[2] == id
                    && fields[3] == id
                    && fields[6] == "/usr/sbin/nologin"
            });
            let group_matches = groups.lines().any(|line| {
                let fields: Vec<_> = line.split(':').collect();
                fields.len() == 4 && fields[0] == name && fields[2] == id && fields[3].is_empty()
            });
            if !account_matches || !group_matches {
                anyhow::bail!("VM identity {name} is not exclusively provisioned at uid/gid {id}");
            }
        }
        for artifact in [
            &profile.jailer,
            &profile.firecracker,
            &profile.kernel,
            &profile.rootfs,
        ] {
            artifact.verify()?;
        }
        trusted_path(&profile.cgroup_root())?;
        let memory = std::fs::read_to_string(profile.cgroup_root().join("memory.max"))?;
        let cpu = std::fs::read_to_string(profile.cgroup_root().join("cpu.max"))?;
        if memory.trim().parse::<u64>().unwrap_or(0) == 0
            || cpu
                .split_whitespace()
                .next()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0)
                == 0
        {
            anyhow::bail!("host cgroup parent requires finite positive memory and CPU limits");
        }
        std::fs::write(
            profile.cgroup_root().join("cgroup.subtree_control"),
            "+cpu +memory +pids",
        )?;
        Ok(profile)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        crate::protocol::socket_path(&self.state_dir)?;
        let end = self
            .uid_start
            .checked_add(self.uid_count)
            .context("VM identity range overflow")?;
        if self.client_uid == 0
            || self.uid_start < 65536
            || self.uid_count == 0
            || self.uid_count > 512
            || (self.uid_start..end).contains(&self.client_uid)
            || !valid_parent(&self.cgroup_parent)
            || !(64..=4096).contains(&self.memory_overhead_mib)
            || self.max_lifetime_ms == 0
            || self.max_lifetime_ms > 86_400_000
            || self.firecracker.path.file_name().and_then(|v| v.to_str()) != Some("firecracker")
        {
            anyhow::bail!("invalid jailed VM host policy");
        }
        Ok(())
    }

    pub fn cgroup_root(&self) -> PathBuf {
        Path::new("/sys/fs/cgroup").join(&self.cgroup_parent)
    }

    pub fn reserve(&self, spec: &CreateVm, records: &[Record]) -> anyhow::Result<Record> {
        if spec.binary != self.firecracker.path
            || spec.kernel != self.kernel.path
            || spec.rootfs != self.rootfs.path
            || spec.lifetime_ms > self.max_lifetime_ms
        {
            anyhow::bail!("VM request does not match the approved host artifacts or lifetime");
        }
        let uid = (self.uid_start..self.uid_start + self.uid_count)
            .find(|uid| {
                !records
                    .iter()
                    .any(|r| r.jail.as_ref().is_some_and(|jail| jail.uid == *uid))
            })
            .context("dedicated VM identities exhausted; cleanup must complete before reuse")?;
        let mut record = Record::from_vm(spec);
        record
            .resources
            .as_mut()
            .context("missing VM resources")?
            .memory_bytes += u64::from(self.memory_overhead_mib) * 1024 * 1024;
        record.jail = Some(JailLease {
            uid,
            cgroup_parent: self.cgroup_parent.clone(),
        });
        Ok(record)
    }

    pub fn jail_root(root: &Path, lease: uuid::Uuid) -> PathBuf {
        root.join("jails/firecracker")
            .join(lease.to_string())
            .join("root")
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn prepare(
        &self,
        spec: &CreateVm,
        record: &Record,
    ) -> anyhow::Result<tokio::process::Command> {
        let jail = record
            .jail
            .as_ref()
            .context("missing VM jail reservation")?;
        let root = Self::jail_root(&self.state_dir, spec.lease);
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o711)
            .create(&root)?;
        for ancestor in [
            self.state_dir.join("jails"),
            self.state_dir.join("jails/firecracker"),
            root.parent().context("missing jail parent")?.to_owned(),
        ] {
            std::fs::set_permissions(ancestor, std::fs::Permissions::from_mode(0o711))?;
        }
        for (source, destination) in [(&self.kernel.path, "kernel"), (&self.rootfs.path, "rootfs")]
        {
            // Artifacts and jails are provisioned on one filesystem. Root-owned
            // read-only links avoid unbounded image copies during admission.
            std::fs::hard_link(source, root.join(destination))
                .context("host artifacts and jails must share a filesystem")?;
        }
        let mut guest_spec = spec.clone();
        guest_spec.kernel = "/kernel".into();
        guest_spec.rootfs = "/rootfs".into();
        let mut config = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o444)
            .open(root.join("vm-config.json"))?;
        config.write_all(&serde_json::to_vec(
            &guest_spec.vm_config(Path::new("/vsock")),
        )?)?;
        config.sync_all()?;
        config.set_permissions(std::fs::Permissions::from_mode(0o444))?;
        std::os::unix::fs::symlink(&root, self.state_dir.join(format!("vm-{}", spec.lease)))?;
        let memory = record
            .resources
            .context("missing VM resource reservation")?
            .memory_bytes;
        let mut command = tokio::process::Command::new(&self.jailer.path);
        command
            .args(["--id", &spec.lease.to_string(), "--exec-file"])
            .arg(&self.firecracker.path)
            .args([
                "--uid",
                &jail.uid.to_string(),
                "--gid",
                &jail.uid.to_string(),
                "--chroot-base-dir",
            ])
            .arg(self.state_dir.join("jails"))
            .args([
                "--cgroup-version",
                "2",
                "--parent-cgroup",
                &jail.cgroup_parent,
                "--cgroup",
                &format!("memory.max={memory}"),
                "--cgroup",
                "memory.swap.max=0",
                "--cgroup",
                "memory.oom.group=1",
                "--cgroup",
                "pids.max=64",
                "--cgroup",
                &format!("cpu.max={} 100000", u32::from(spec.vcpus) * 100000),
                "--resource-limit",
                "no-file=256",
                "--resource-limit",
                "fsize=0",
                "--",
                "--no-api",
                "--config-file",
                "/vm-config.json",
            ])
            .env_clear()
            .current_dir(&self.state_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        // SAFETY: identity lookup has no pointer preconditions.
        let parent = unsafe { libc::getpid() };
        // SAFETY: only async-signal-safe syscalls run after fork. Jailer creates
        // its own mount namespace; this adds a separate empty network namespace.
        unsafe {
            command.pre_exec(move || {
                libc::umask(0o077);
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) != 0
                    || libc::getppid() != parent
                    || libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0
                    || libc::unshare(libc::CLONE_NEWNET) != 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Ok(command)
    }

    #[cfg(target_os = "linux")]
    pub(crate) async fn verify_started(
        &self,
        record: &Record,
        pid: u32,
        deadline: std::time::Instant,
    ) -> anyhow::Result<()> {
        let jail = record.jail.as_ref().context("missing jail")?;
        let socket = Self::jail_root(&self.state_dir, record.lease).join("vsock");
        loop {
            if socket.exists() {
                break;
            }
            if std::time::Instant::now() >= deadline {
                anyhow::bail!("jailed VM did not become ready before startup deadline");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let proc = PathBuf::from(format!("/proc/{pid}"));
        let status = std::fs::read_to_string(proc.join("status"))?;
        let field = |name: &str| {
            status
                .lines()
                .find_map(|line| line.strip_prefix(name))
                .map(str::trim)
        };
        let uid = jail.uid.to_string();
        let expected_uid = [uid.as_str(); 4].join("\t");
        if field("Uid:") != Some(expected_uid.as_str())
            || field("Gid:") != Some(expected_uid.as_str())
            || field("Groups:").is_none_or(|groups| groups.split_whitespace().any(|g| g != uid))
            || ["CapEff:", "CapPrm:", "CapAmb:"]
                .iter()
                .any(|name| field(name) != Some("0000000000000000"))
            || field("NoNewPrivs:") != Some("1")
            || field("Seccomp:") != Some("2")
        {
            anyhow::bail!("VMM privilege or seccomp verification failed");
        }
        if std::fs::read_link(proc.join("root"))? != Self::jail_root(&self.state_dir, record.lease)
            || std::fs::read_link(proc.join("ns/mnt"))? == std::fs::read_link("/proc/self/ns/mnt")?
            || std::fs::read_link(proc.join("ns/net"))? == std::fs::read_link("/proc/self/ns/net")?
        {
            anyhow::bail!("VMM jail or namespace verification failed");
        }
        let cgroup = jail.cgroup(record.lease);
        let membership = std::fs::read_to_string(proc.join("cgroup"))?;
        let expected = format!("0::/{}/{}", jail.cgroup_parent, record.lease);
        let resources = record.resources.context("missing resources")?;
        if membership.trim() != expected
            || std::fs::read_to_string(cgroup.join("memory.max"))?.trim()
                != record
                    .resources
                    .context("missing resources")?
                    .memory_bytes
                    .to_string()
            || std::fs::read_to_string(cgroup.join("memory.swap.max"))?.trim() != "0"
            || std::fs::read_to_string(cgroup.join("memory.oom.group"))?.trim() != "1"
            || std::fs::read_to_string(cgroup.join("cpu.max"))?.trim()
                != format!("{} 100000", resources.cpu_nanos / 10000)
            || std::fs::read_to_string(cgroup.join("pids.max"))?.trim() != "64"
        {
            anyhow::bail!("VMM host cgroup verification failed");
        }
        // The runtime receives access only to this private guest channel.
        use std::os::{fd::AsRawFd, unix::fs::FileTypeExt};
        let socket_fd = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&socket)?;
        let metadata = socket_fd.metadata()?;
        if !metadata.file_type().is_socket() || metadata.uid() != jail.uid {
            anyhow::bail!("unexpected jailed VM socket identity or permissions");
        }
        // chmod through the held descriptor's procfs link pins the inode even
        // on kernels without fchmodat2(AT_EMPTY_PATH) for O_PATH descriptors.
        std::fs::set_permissions(
            format!("/proc/self/fd/{}", socket_fd.as_raw_fd()),
            std::fs::Permissions::from_mode(0o600),
        )?;
        // SAFETY: O_PATH pins the checked socket. A guest replacing its path
        // with a symlink cannot redirect this ownership change to a host file.
        if unsafe {
            libc::fchownat(
                socket_fd.as_raw_fd(),
                c"".as_ptr(),
                self.client_uid,
                0,
                libc::AT_EMPTY_PATH | libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        std::fs::set_permissions(
            Self::jail_root(&self.state_dir, record.lease),
            std::fs::Permissions::from_mode(0o711),
        )?;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn notify(message: &str) -> anyhow::Result<()> {
    use std::os::{
        linux::net::SocketAddrExt,
        unix::net::{SocketAddr, UnixDatagram},
    };
    let destination =
        std::env::var("NOTIFY_SOCKET").context("missing service-manager notification channel")?;
    let address = match destination.strip_prefix('@') {
        Some(name) => SocketAddr::from_abstract_name(name.as_bytes())?,
        None => SocketAddr::from_pathname(destination)?,
    };
    let socket = UnixDatagram::unbound()?;
    socket.set_nonblocking(true)?;
    socket.connect_addr(&address)?;
    socket.send(message.as_bytes())?;
    Ok(())
}

#[cfg(target_os = "linux")]
pub(crate) async fn reconcile_jail(
    state: &crate::service::State,
    record: &Record,
) -> anyhow::Result<bool> {
    // SAFETY: geteuid has no memory preconditions.
    if unsafe { libc::geteuid() } != 0 {
        anyhow::bail!("jailed VM cleanup requires its root host service");
    }
    let jail = record
        .jail
        .as_ref()
        .context("missing jail cleanup identity")?;
    jail.validate()?;
    let cgroup = jail.cgroup(record.lease);
    match std::fs::symlink_metadata(&cgroup) {
        Ok(_) => {
            trusted_path(&cgroup)?;
            std::fs::write(cgroup.join("cgroup.kill"), "1")?;
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let events = std::fs::read_to_string(cgroup.join("cgroup.events"))?;
                    if events.lines().any(|line| line == "populated 0") {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Ok::<_, anyhow::Error>(())
            })
            .await
            .context("jailed VMM cgroup cleanup remains pending")??;
            std::fs::remove_dir(&cgroup)?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let lease = record.lease;
    state
        .storage(move |s| {
            s.check_identity()?;
            let jail_root = HostProfile::jail_root(&s.root, lease);
            let alias = s.root.join(format!("vm-{lease}"));
            match std::fs::symlink_metadata(&alias) {
                Ok(metadata)
                    if metadata.file_type().is_symlink()
                        && std::fs::read_link(&alias)? == jail_root =>
                {
                    std::fs::remove_file(alias)?
                }
                Ok(_) => anyhow::bail!("jailed VM channel alias was replaced"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            let directory = jail_root.parent().context("missing jail parent")?;
            match std::fs::symlink_metadata(directory) {
                Ok(metadata) if metadata.is_dir() && metadata.uid() == 0 => {
                    std::fs::remove_dir_all(directory)?
                }
                Ok(_) => anyhow::bail!("jailed VM owner directory was replaced"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            s.remove(lease)
        })
        .await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (HostProfile, CreateVm) {
        let artifact = |name: &str| Artifact {
            path: Path::new("/var/lib/symbivmm/artifacts").join(name),
            sha256: "0".repeat(64),
        };
        let profile = HostProfile {
            state_dir: "/var/lib/symbivmm/state".into(),
            client_uid: 1000,
            uid_start: 200000,
            uid_count: 2,
            cgroup_parent: "symbivmm.slice".into(),
            memory_overhead_mib: 128,
            max_lifetime_ms: 300000,
            jailer: artifact("jailer"),
            firecracker: artifact("firecracker"),
            kernel: artifact("kernel"),
            rootfs: artifact("rootfs"),
        };
        let request = CreateVm {
            origin: None,
            version: crate::protocol::VERSION,
            implementation: crate::protocol::IMPLEMENTATION.into(),
            lease: uuid::Uuid::new_v4(),
            binary: profile.firecracker.path.clone(),
            kernel: profile.kernel.path.clone(),
            rootfs: profile.rootfs.path.clone(),
            boot_args: String::new(),
            vcpus: 1,
            memory_mib: 256,
            lifetime_ms: 10000,
            startup_ms: 5000,
        };
        (profile, request)
    }

    #[test]
    fn approved_artifacts_overhead_and_retained_identities_are_authoritative() {
        let (profile, mut request) = fixture();
        profile.validate().unwrap();
        let first = profile.reserve(&request, &[]).unwrap();
        assert_eq!(first.resources.unwrap().memory_bytes, 384 * 1024 * 1024);
        assert_eq!(first.jail.as_ref().unwrap().uid, 200000);
        request.lease = uuid::Uuid::new_v4();
        let second = profile
            .reserve(&request, std::slice::from_ref(&first))
            .unwrap();
        assert_eq!(second.jail.as_ref().unwrap().uid, 200001);
        assert!(profile.reserve(&request, &[first, second.clone()]).is_err());
        assert_eq!(
            profile
                .reserve(&request, &[second])
                .unwrap()
                .jail
                .unwrap()
                .uid,
            200000
        );
        request.rootfs = "/etc/shadow".into();
        assert!(profile.reserve(&request, &[]).is_err());
        request.rootfs = profile.rootfs.path.clone();
        request.lifetime_ms = profile.max_lifetime_ms + 1;
        assert!(profile.reserve(&request, &[]).is_err());
    }

    #[test]
    fn dangerous_identity_ranges_and_cleanup_paths_are_refused() {
        let (profile, _) = fixture();
        let mut invalid = profile.clone();
        invalid.uid_start = u32::MAX;
        assert!(invalid.validate().is_err());
        invalid = profile.clone();
        invalid.client_uid = invalid.uid_start;
        assert!(invalid.validate().is_err());
        invalid = profile.clone();
        invalid.memory_overhead_mib = 0;
        assert!(invalid.validate().is_err());
        for parent in [
            "../system.slice",
            "system.slice",
            "symbi/../../system.slice",
            "symbi.slice/child",
        ] {
            assert!(JailLease {
                uid: 200000,
                cgroup_parent: parent.into()
            }
            .validate()
            .is_err());
        }
        assert!(JailLease {
            uid: 0,
            cgroup_parent: profile.cgroup_parent
        }
        .validate()
        .is_err());
    }
}
