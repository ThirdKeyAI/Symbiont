//! Delegated cgroup ownership for same-UID Landlock workers. The service manager
//! owns the whole service cgroup; workers occupy separate leaves from the manager.
use anyhow::Context;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};

pub use crate::protocol::CgroupIdentity as Identity;
impl Identity {
    pub fn open(&self) -> anyhow::Result<Option<Group>> {
        self.validate()?;
        if self.boot_id != boot_id()? {
            return Ok(None);
        }
        let group = match Group::open(&self.path) {
            Ok(group) => group,
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                return Ok(None)
            }
            Err(error) => return Err(error),
        };
        // A removed cgroup cannot contain live tasks; never act on its successor.
        if group.identity.device != self.device || group.identity.inode != self.inode {
            return Ok(None);
        }
        Ok(Some(group))
    }
}

pub struct Group {
    directory: File,
    pub identity: Identity,
}
fn boot_id() -> anyhow::Result<String> {
    Ok(std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?
        .trim()
        .to_owned())
}
impl Group {
    fn open(path: &Path) -> anyhow::Result<Self> {
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        let metadata = directory.metadata()?;
        // SAFETY: fstatfs writes the initialized buffer using a live descriptor.
        let mut fs = unsafe { std::mem::zeroed::<libc::statfs>() };
        if unsafe { libc::fstatfs(directory.as_raw_fd(), &mut fs) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        // SAFETY: geteuid has no pointer preconditions.
        anyhow::ensure!(
            fs.f_type == libc::CGROUP2_SUPER_MAGIC && metadata.uid() == unsafe { libc::geteuid() },
            "cgroup must be delegated to the service identity"
        );
        Ok(Self {
            identity: Identity {
                path: path.to_owned(),
                boot_id: boot_id()?,
                device: metadata.dev(),
                inode: metadata.ino(),
            },
            directory,
        })
    }
    pub fn file(&self, name: &str, writable: bool) -> anyhow::Result<File> {
        anyhow::ensure!(
            !name.contains('/') && !name.contains('\0'),
            "invalid cgroup field"
        );
        let name = std::ffi::CString::new(name)?;
        // SAFETY: relative NUL-terminated name and live retained directory.
        let fd = unsafe {
            libc::openat(
                self.directory.as_raw_fd(),
                name.as_ptr(),
                (if writable {
                    libc::O_WRONLY
                } else {
                    libc::O_RDONLY
                }) | libc::O_NOFOLLOW
                    | libc::O_CLOEXEC,
                0,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        // SAFETY: openat returned a newly owned descriptor.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    pub fn read(&self, name: &str) -> anyhow::Result<String> {
        let mut text = String::new();
        self.file(name, false)?
            .take(8193)
            .read_to_string(&mut text)?;
        anyhow::ensure!(text.len() <= 8192, "oversized cgroup field");
        Ok(text)
    }
    fn write(&self, name: &str, value: &str) -> anyhow::Result<()> {
        self.file(name, true)?.write_all(value.as_bytes())?;
        Ok(())
    }
    fn child(&self, lease: uuid::Uuid) -> PathBuf {
        self.identity.path.join(format!("worker-{lease}"))
    }
    pub(crate) fn create(&self, spec: &crate::protocol::CreateHost) -> anyhow::Result<Group> {
        anyhow::ensure!(
            self.identity.open()?.is_some(),
            "delegated cgroup parent disappeared"
        );
        let name = std::ffi::CString::new(format!("worker-{}", spec.lease))?;
        // SAFETY: fixed generated component under the retained delegated parent.
        if unsafe { libc::mkdirat(self.directory.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let group = Group::open(&self.child(spec.lease))?;
        for (field, value) in [
            ("memory.max", spec.resources.memory_bytes.to_string()),
            ("memory.swap.max", "0".into()),
            ("memory.oom.group", "1".into()),
            (
                "cpu.max",
                format!("{} 100000", spec.resources.cpu_nanos / 10_000),
            ),
            ("pids.max", spec.pids_limit.to_string()),
        ] {
            group.write(field, &value)?;
            anyhow::ensure!(
                group.read(field)?.trim() == value,
                "cgroup resource limit readback failed: {field}"
            );
        }
        Ok(group)
    }
    pub(crate) async fn remove_child(
        &self,
        lease: uuid::Uuid,
        expected: Option<&Identity>,
    ) -> anyhow::Result<()> {
        if self.identity.open()?.is_none() {
            return Ok(());
        }
        let group = match expected {
            Some(identity) => identity.open()?,
            None => match Group::open(&self.child(lease)) {
                Ok(group) => Some(group),
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
                {
                    None
                }
                Err(error) => return Err(error),
            },
        };
        let Some(group) = group else {
            return Ok(());
        };
        group.write("cgroup.kill", "1")?;
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if group
                    .read("cgroup.events")?
                    .lines()
                    .any(|line| line == "populated 0")
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("delegated worker cleanup remains pending")??;
        // rmdir closes admission too: a retained cgroup.procs descriptor cannot
        // admit a late child after removal. A racing join makes rmdir fail and
        // retains the lease for another kill/reconciliation pass.
        let name = std::ffi::CString::new(format!("worker-{lease}"))?;
        if unsafe {
            libc::unlinkat(
                self.directory.as_raw_fd(),
                name.as_ptr(),
                libc::AT_REMOVEDIR,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
}

pub(crate) fn service_group() -> anyhow::Result<Group> {
    // Same-UID workers must never inherit a privileged supervisor identity.
    anyhow::ensure!(
        unsafe { libc::geteuid() } != 0,
        "delegated Landlock service must be unprivileged"
    );
    anyhow::ensure!(
        std::env::var("WATCHDOG_PID")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            == Some(std::process::id())
            && std::env::var("WATCHDOG_USEC")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .is_some_and(|v| (1..=15_000_000).contains(&v))
            && std::env::var_os("NOTIFY_SOCKET").is_some(),
        "delegated workers require a service-manager watchdog of at most 15 seconds"
    );
    let membership = std::fs::read_to_string("/proc/self/cgroup")?;
    let relative = membership
        .lines()
        .find_map(|line| line.strip_prefix("0::/"))
        .context("cgroup v2 membership unavailable")?;
    let current = Path::new("/sys/fs/cgroup").join(relative);
    anyhow::ensure!(
        current.file_name().is_some_and(|n| n == "manager"),
        "delegated service requires DelegateSubgroup=manager"
    );
    let parent = current.parent().context("missing delegated parent")?;
    anyhow::ensure!(
        parent
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".service")),
        "delegated parent must be a service cgroup"
    );
    // The watchdog is useful only if a failed main process also causes the
    // service manager to kill every worker leaf. Refuse weaker unit settings.
    let unit = parent.file_name().context("missing service unit")?;
    let output = std::process::Command::new("/usr/bin/systemctl")
        .args(["--user", "--no-pager", "show"])
        .arg(unit)
        .arg("--property=Type,KillMode,SendSIGKILL,WatchdogUSec,TimeoutStopUSec")
        .env("LANG", "C")
        .output()?;
    anyhow::ensure!(
        output.status.success() && output.stdout.len() <= 4096,
        "cannot verify delegated service-manager settings"
    );
    let properties = std::str::from_utf8(&output.stdout)?;
    for required in [
        "Type=notify",
        "KillMode=control-group",
        "SendSIGKILL=yes",
        "WatchdogUSec=5s",
        "TimeoutStopUSec=5s",
    ] {
        anyhow::ensure!(
            properties.lines().any(|line| line == required),
            "delegated service requires {required}"
        );
    }
    let group = Group::open(parent)?;
    anyhow::ensure!(
        group.read("cgroup.type")?.trim() == "domain",
        "delegated workers require a domain cgroup"
    );
    let controllers = group.read("cgroup.controllers")?;
    anyhow::ensure!(
        ["cpu", "memory", "pids"]
            .iter()
            .all(|c| controllers.split_whitespace().any(|v| v == *c)),
        "cpu, memory and pids delegation are required"
    );
    group.write("cgroup.subtree_control", "+cpu +memory +pids")?;
    Ok(group)
}
