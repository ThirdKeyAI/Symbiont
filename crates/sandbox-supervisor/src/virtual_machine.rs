//! The supervisor creates and owns the VMM, including the parent-death contract.
//! A workload cannot be sent to the guest until its process identity is durable.
use super::{reply, Active, State};
use crate::{
    protocol::{self, CreateVm, Reply, Request},
    store::{Phase, Record},
};
use anyhow::Context;
use std::{
    fs::OpenOptions,
    io::Write,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::fs::{DirBuilderExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::BufReader,
    net::unix::{OwnedReadHalf, OwnedWriteHalf},
    process::Command,
    time::{sleep, sleep_until, timeout},
};

fn directory(root: &Path, lease: uuid::Uuid) -> PathBuf {
    root.join(format!("vm-{lease}"))
}
fn boot_id() -> anyhow::Result<String> {
    Ok(std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?
        .trim()
        .to_owned())
}
fn start_ticks(pid: u32) -> anyhow::Result<Option<u64>> {
    let data = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(data) => data,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let fields = data
        .rsplit_once(") ")
        .context("malformed VMM process identity")?
        .1;
    Ok(Some(
        fields
            .split_whitespace()
            .nth(19)
            .context("missing VMM start identity")?
            .parse()?,
    ))
}

pub(super) async fn handle(
    mut reader: BufReader<OwnedReadHalf>,
    mut writer: OwnedWriteHalf,
    state: Arc<State>,
    spec: CreateVm,
) -> anyhow::Result<()> {
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
    let deadline = started + Duration::from_millis(spec.lifetime_ms);
    let startup = started + Duration::from_millis(spec.startup_ms);
    if !state
        .active
        .lock()
        .map_err(|_| anyhow::anyhow!("lease registry poisoned"))?
        .insert(spec.lease)
    {
        anyhow::bail!("lease already active");
    }
    let _active = Active {
        state: state.clone(),
        lease: spec.lease,
    };
    let request = spec.clone();
    let host = state.host.clone();
    let registration = state
        .storage(move |s| {
            let record = match host {
                Some(profile) => profile.reserve(&request, &s.records()?)?,
                None => Record::from_vm(&request),
            };
            s.insert(&record)?;
            Ok(record)
        })
        .await;
    let mut record = match registration {
        Ok(record) => record,
        Err(error) => {
            reply(
                &mut writer,
                &Reply::Failed {
                    message: error.to_string(),
                },
            )
            .await?;
            return Ok(());
        }
    };
    if reply(&mut writer, &Reply::Registered { lease: spec.lease })
        .await
        .is_err()
    {
        state.forget(spec.lease).await?;
        return Ok(());
    }
    let root = state
        .storage(|s| {
            s.check_identity()?;
            Ok(s.root.clone())
        })
        .await?;
    let work = directory(&root, spec.lease);
    let vsock = work.join("vsock");
    let setup = || -> anyhow::Result<tokio::process::Child> {
        if vsock.as_os_str().as_encoded_bytes().len() >= 104 {
            anyhow::bail!("VM vsock path exceeds Unix socket limit");
        }
        if let Some(host) = &state.host {
            if Instant::now() >= startup {
                anyhow::bail!("VM startup deadline expired before jail creation");
            }
            return Ok(host.prepare(&spec, &record)?.spawn()?);
        }
        for path in [&spec.binary, &spec.kernel, &spec.rootfs] {
            let metadata = std::fs::symlink_metadata(path)?;
            if !metadata.is_file() {
                anyhow::bail!("VM artifacts must be regular files");
            }
        }
        std::fs::DirBuilder::new().mode(0o700).create(&work)?;
        let config = work.join("vm-config.json");
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&config)?;
        file.write_all(&serde_json::to_vec(&spec.vm_config(&vsock))?)?;
        file.sync_all()?;
        // No guest console or VMM logger can fill a host disk or pipe. Command
        // output is carried only by the bounded guest protocol.
        let mut command = Command::new(&spec.binary);
        command
            .args(["--no-api", "--config-file"])
            .arg(&config)
            .env_clear()
            .current_dir(&work)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        // SAFETY: identity lookup has no pointer preconditions.
        let parent = unsafe { libc::getpid() };
        // SAFETY: pre_exec uses only async-signal-safe syscalls; checking the
        // parent after prctl closes the parent-death registration race.
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) != 0
                    || libc::getppid() != parent
                    || libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0
                {
                    return Err(std::io::Error::other("VMM parent-death setup failed"));
                }
                Ok(())
            });
        }
        if Instant::now() >= startup {
            anyhow::bail!("VM startup deadline expired before spawn");
        }
        Ok(command.spawn()?)
    };
    let mut child = match setup() {
        Ok(child) => child,
        Err(error) => {
            let _ = reply(
                &mut writer,
                &Reply::Failed {
                    message: format!("VM startup failed: {error}"),
                },
            )
            .await;
            if reconcile(&state, &record).await? {
                let _ = reply(&mut writer, &Reply::Closed {}).await;
            }
            return Ok(());
        }
    };
    let operation=async {
        let pid=child.id().context("missing VMM process id")?;
        record.state=Phase::VmCreated {pid,start_ticks:start_ticks(pid)?.context("VMM exited during startup")?,boot_id:boot_id()?};
        state.write(&record).await?;
        if let Some(host) = &state.host { host.verify_started(&record,pid,startup).await?; }
        if Instant::now()>=startup {anyhow::bail!("VM startup deadline expired");}
        reply(&mut writer,&Reply::VmCreated {pid,vsock_path:vsock}).await?;
        tokio::select! {
            biased;
            _=protocol::read_frame::<Request>(&mut reader)=>{},
            status=child.wait()=>{anyhow::bail!("VMM exited before explicit release: {:?}",status?.code());},
            _=sleep_until(deadline.into())=>{anyhow::bail!("VM lifetime expired");},
        }
        Ok::<(),anyhow::Error>(())
    }.await;
    // Keep ownership until wait confirms that all VMM threads have stopped.
    let stopped = timeout(Duration::from_secs(10), async {
        if child.try_wait()?.is_none() {
            child.start_kill()?;
        }
        child.wait().await?;
        Ok::<(), anyhow::Error>(())
    })
    .await
    .context("VMM termination remains pending")?;
    stopped?;
    if let Err(error) = operation {
        let _ = reply(
            &mut writer,
            &Reply::Failed {
                message: error.to_string(),
            },
        )
        .await;
    }
    if reconcile(&state, &record).await? {
        let _ = reply(&mut writer, &Reply::Closed {}).await;
    }
    Ok(())
}

/// pidfd binds the kill to the inspected process even if its numeric PID is
/// subsequently reused. Boot and start identities exclude unrelated processes.
pub(super) async fn reconcile(state: &State, record: &Record) -> anyhow::Result<bool> {
    if record.jail.is_some() {
        return crate::host::reconcile_jail(state, record).await;
    }
    if let Phase::VmCreated {
        pid,
        start_ticks: expected,
        boot_id: boot,
    } = &record.state
    {
        if boot_id()? == *boot {
            // SAFETY: pidfd_open takes scalar arguments and returns an owned FD.
            let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, *pid, 0) as i32 };
            if fd < 0 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::ESRCH) {
                    return Err(error.into());
                }
            } else {
                // SAFETY: the syscall returned a new owned descriptor.
                let fd = unsafe { OwnedFd::from_raw_fd(fd) };
                if start_ticks(*pid)? == Some(*expected) {
                    // SAFETY: pidfd refers to the verified VMM. No siginfo is supplied.
                    if unsafe {
                        libc::syscall(
                            libc::SYS_pidfd_send_signal,
                            fd.as_raw_fd(),
                            libc::SIGKILL,
                            std::ptr::null::<libc::siginfo_t>(),
                            0,
                        )
                    } < 0
                    {
                        let error = std::io::Error::last_os_error();
                        if error.raw_os_error() != Some(libc::ESRCH) {
                            return Err(error.into());
                        }
                    }
                    timeout(Duration::from_secs(10), async {
                        loop {
                            let mut descriptor = libc::pollfd {
                                fd: fd.as_raw_fd(),
                                events: libc::POLLIN,
                                revents: 0,
                            };
                            // SAFETY: a live one-element pollfd array is supplied.
                            let result = unsafe { libc::poll(&mut descriptor, 1, 0) };
                            if result > 0 && descriptor.revents & libc::POLLIN != 0 {
                                return Ok::<(), anyhow::Error>(());
                            }
                            if result < 0 {
                                return Err(std::io::Error::last_os_error().into());
                            }
                            sleep(Duration::from_millis(10)).await;
                        }
                    })
                    .await
                    .context("durable VMM cleanup still pending")??;
                }
            }
        }
    }
    let lease = record.lease;
    state
        .storage(move |s| {
            s.check_identity()?;
            let work = directory(&s.root, lease);
            match std::fs::symlink_metadata(&work) {
                Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(&work)?,
                Ok(_) => anyhow::bail!("VM work directory replaced"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            s.remove(lease)
        })
        .await?;
    Ok(true)
}
