use anyhow::Context;
use std::{
    ffi::CString,
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{
            fs::PermissionsExt,
            process::{CommandExt, ExitStatusExt},
        },
    },
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use symbi_sandbox_guest::{self as wire, Outcome};

mod files;
mod snapshot;
mod streaming;
mod terminal;

fn mount(target: &str, kind: &str, options: &str) -> anyhow::Result<()> {
    let target = CString::new(target)?;
    let kind = CString::new(kind)?;
    let options = CString::new(options)?;
    // SAFETY: all C strings remain valid for this syscall. This runs only as guest PID 1.
    let result = unsafe {
        libc::mount(
            kind.as_ptr(),
            target.as_ptr(),
            kind.as_ptr(),
            libc::MS_NOSUID | libc::MS_NODEV,
            options.as_ptr().cast(),
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

pub fn run() -> anyhow::Result<()> {
    // Prevent accidental execution on an ordinary host or as an unprivileged workload.
    // SAFETY: identity queries take no pointers.
    if unsafe { libc::getpid() != 1 || libc::geteuid() != 0 } {
        anyhow::bail!("guest service must be PID 1 and root inside a dedicated microVM");
    }
    mount("/proc", "proc", "").context("mount /proc")?;
    mount("/sys", "sysfs", "").context("mount /sys")?;
    // devtmpfs needs working device nodes, so MS_NODEV is intentionally absent.
    let dev = CString::new("devtmpfs")?;
    let path = CString::new("/dev")?;
    // Some kernels mount devtmpfs before starting init. Reuse only that exact
    // filesystem and apply the required flags; do not ignore a failed mount.
    let mounted = std::fs::read_to_string("/proc/self/mountinfo")?
        .lines()
        .any(|line| {
            line.split_once(" - ").is_some_and(|(fields, filesystem)| {
                fields.split_whitespace().nth(4) == Some("/dev")
                    && filesystem.split_whitespace().next() == Some("devtmpfs")
            })
        });
    let flags = libc::MS_NOSUID | if mounted { libc::MS_REMOUNT } else { 0 };
    // SAFETY: static mount strings point to valid NUL-terminated buffers.
    if unsafe {
        libc::mount(
            dev.as_ptr(),
            path.as_ptr(),
            dev.as_ptr(),
            flags,
            std::ptr::null(),
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error()).context("mount /dev");
    }
    mount("/tmp", "tmpfs", "mode=1777,size=128m").context("mount /tmp")?;
    enable_loopback().context("enable guest-local loopback")?;
    // SAFETY: umask takes a value, not a pointer.
    unsafe {
        libc::umask(0o077);
    }
    let mut stream = accept_host()?;
    wire::write_header(
        &mut stream,
        &wire::Hello {
            version: wire::VERSION,
            implementation: wire::IMPLEMENTATION.into(),
        },
    )?;
    let request: wire::Command = wire::read_header(&mut stream)?;
    request.validate()?;
    let transfer_deadline = Instant::now() + Duration::from_millis(request.timeout_ms);
    let snapshot = snapshot::receive(&mut stream, request.snapshot.as_ref())?;
    let mut files = files::Files::receive(&mut stream, request.files.as_ref())?;
    if request.mode != wire::CommandMode::OneShot {
        let result = streaming::serve(&mut stream, &request, &mut files);
        kill_workload();
        result?;
        drop(stream);
        park();
    }
    let mut input = vec![0; request.input_length];
    stream.read_exact(&mut input)?;
    let result = execute(&request, input);
    let (mut outcome, stdout, stderr) = match result {
        Ok(result) => result,
        Err(error) => (
            Outcome {
                version: wire::VERSION,
                id: request.id.clone(),
                exit_code: 125,
                stdout_length: 0,
                stderr_length: 0,
                stdout_truncated: false,
                stderr_truncated: false,
                timed_out: false,
                file_output: None,
                snapshot: None,
                finalized: false,
                error: Some(
                    format!("guest execution failed: {error}")
                        .chars()
                        .take(512)
                        .collect(),
                ),
            },
            vec![],
            vec![],
        ),
    };
    // This dedicated VM has no other services. Kill every remaining workload
    // process, including descendants that changed process group or session.
    kill_workload();
    outcome.snapshot = snapshot;
    outcome.stdout_length = stdout.len();
    outcome.stderr_length = stderr.len();
    let exported = match files.capture(&mut outcome) {
        Ok(exported) => exported,
        Err(error) => {
            outcome.exit_code = 125;
            outcome.error = Some(
                format!("guest file output failed: {error}")
                    .chars()
                    .take(512)
                    .collect(),
            );
            None
        }
    };
    outcome.validate(&request)?;
    wire::write_header(&mut stream, &outcome)?;
    stream.write_all(&stdout)?;
    stream.write_all(&stderr)?;
    if let Some(exported) = exported {
        exported.send(&mut stream, transfer_deadline)?;
    }
    stream.flush()?;
    drop(stream);
    // The host must acknowledge VM removal before reporting the outcome. PID 1
    // stays alive until the independently owned VMM is terminated.
    park()
}

/// Local adapters may communicate inside the guest. This does not create a
/// network device, route or host endpoint; external access remains absent.
fn enable_loopback() -> anyhow::Result<()> {
    // SAFETY: scalar socket arguments; a successful result is a fresh owned fd.
    let raw = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    if raw < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: ownership is transferred once, and ifreq is fully initialized.
    let socket = unsafe { OwnedFd::from_raw_fd(raw) };
    let mut request: libc::ifreq = unsafe { std::mem::zeroed() };
    request.ifr_name[0] = b'l' as libc::c_char;
    request.ifr_name[1] = b'o' as libc::c_char;
    // SAFETY: the descriptor and ifreq pointer are valid throughout each ioctl.
    if unsafe { libc::ioctl(socket.as_raw_fd(), libc::SIOCGIFFLAGS, &mut request) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    request.ifr_ifru.ifru_flags =
        unsafe { request.ifr_ifru.ifru_flags } | libc::IFF_UP as libc::c_short;
    if unsafe { libc::ioctl(socket.as_raw_fd(), libc::SIOCSIFFLAGS, &request) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

fn park() -> ! {
    loop {
        std::thread::park_timeout(Duration::from_secs(60));
    }
}

fn accept_host() -> anyhow::Result<File> {
    // SAFETY: socket returns an owned descriptor; sockaddr_vm is fully initialized.
    let fd = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: this is a new descriptor owned exactly once.
    let listener = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut address: libc::sockaddr_vm = unsafe { std::mem::zeroed() };
    address.svm_family = libc::AF_VSOCK as _;
    address.svm_cid = libc::VMADDR_CID_ANY;
    address.svm_port = wire::PORT;
    // SAFETY: address and its supplied size agree; listener stays alive.
    if unsafe {
        libc::bind(
            fd,
            (&address as *const libc::sockaddr_vm).cast(),
            std::mem::size_of_val(&address) as _,
        )
    } != 0
        || unsafe { libc::listen(fd, 1) } != 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: no peer address is requested; the new descriptor is CLOEXEC.
    let accepted = unsafe {
        libc::accept4(
            fd,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            libc::SOCK_CLOEXEC,
        )
    };
    if accepted < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    drop(listener);
    // SAFETY: accept4 transferred sole ownership of this descriptor.
    Ok(unsafe { File::from_raw_fd(accepted) })
}

fn kill_workload() {
    // SAFETY: run() admits only guest PID 1. Linux excludes PID 1 and this caller
    // from kill(-1), so all remaining processes belong to this one guest workload.
    unsafe {
        libc::kill(-1, libc::SIGKILL);
    }
}

fn quiesce_workload() -> anyhow::Result<()> {
    kill_workload();
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        // SAFETY: PID 1 owns all children and no status pointer is requested.
        let result = unsafe { libc::waitpid(-1, std::ptr::null_mut(), libc::WNOHANG) };
        if result > 0 {
            continue;
        }
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ECHILD) {
                return Ok(());
            }
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error.into());
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "guest descendants did not quiesce"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn nonblocking(file: &File) -> anyhow::Result<()> {
    // SAFETY: file retains this open descriptor during both calls.
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
    if flags < 0
        || unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
struct Capture {
    file: File,
    bytes: Vec<u8>,
    closed: bool,
    truncated: bool,
}
impl Capture {
    fn new(fd: OwnedFd) -> anyhow::Result<Self> {
        let file = File::from(fd);
        nonblocking(&file)?;
        Ok(Self {
            file,
            bytes: vec![],
            closed: false,
            truncated: false,
        })
    }
    fn read(&mut self, limit: usize) -> anyhow::Result<()> {
        let mut bytes = [0u8; 8192];
        // Bound work per stream so a continuous writer cannot starve deadlines.
        for _ in 0..8 {
            match self.file.read(&mut bytes) {
                Ok(0) => {
                    self.closed = true;
                    break;
                }
                Ok(n) => {
                    let keep = n.min(limit.saturating_sub(self.bytes.len()));
                    self.bytes.extend_from_slice(&bytes[..keep]);
                    self.truncated |= keep < n;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
}

fn command(request: &wire::Command) -> Command {
    let mut child = Command::new(&request.argv[0]);
    child
        .args(&request.argv[1..])
        .env_clear()
        .envs(&request.environment)
        .current_dir(&request.working_dir);
    let file_limit = request.files.as_ref().map(|files| files.max_file_bytes);
    // SAFETY: only async-signal-safe syscalls are used between fork and exec.
    unsafe {
        child.pre_exec(move || {
            if libc::setgroups(0, std::ptr::null()) != 0
                || libc::setgid(65534) != 0
                || libc::setuid(65534) != 0
                || libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            for (resource, maximum) in [
                (libc::RLIMIT_CORE, 0),
                (libc::RLIMIT_NPROC, wire::MAX_PROCESSES),
                (libc::RLIMIT_NOFILE, wire::MAX_FILES),
            ] {
                let limit = libc::rlimit {
                    rlim_cur: maximum as _,
                    rlim_max: maximum as _,
                };
                if libc::setrlimit(resource, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            if let Some(maximum) = file_limit {
                let limit = libc::rlimit {
                    rlim_cur: maximum as _,
                    rlim_max: maximum as _,
                };
                if libc::setrlimit(libc::RLIMIT_FSIZE, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    child
}

fn spawn_command(request: &wire::Command) -> anyhow::Result<std::process::Child> {
    command(request)
        .stdin(if request.input_as_file {
            Stdio::null()
        } else {
            Stdio::piped()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("cannot start guest command")
}

fn execute(request: &wire::Command, input: Vec<u8>) -> anyhow::Result<(Outcome, Vec<u8>, Vec<u8>)> {
    if request.input_as_file {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(wire::INPUT_FILE)?;
        file.write_all(&input)?;
        file.set_permissions(std::fs::Permissions::from_mode(0o444))?;
    }
    let mut child = spawn_command(request)?;
    let mut stdout = Capture::new(child.stdout.take().context("missing guest stdout")?.into())?;
    let mut stderr = Capture::new(child.stderr.take().context("missing guest stderr")?.into())?;
    let mut stdin = child.stdin.take().map(|s| File::from(OwnedFd::from(s)));
    if let Some(file) = stdin.as_ref() {
        nonblocking(file)?;
    }
    let mut input_offset = 0;
    let started = Instant::now();
    let deadline = started + Duration::from_millis(request.timeout_ms);
    let mut status = None;
    let mut stopping = None;
    let mut timed_out = false;
    loop {
        stdout.read(request.max_output_bytes)?;
        stderr.read(request.max_output_bytes)?;
        if let Some(file) = stdin.as_mut() {
            if input_offset == input.len() {
                stdin = None;
            } else {
                match file.write(&input[input_offset..input.len().min(input_offset + 65536)]) {
                    Ok(n) => input_offset += n,
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                        ) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => stdin = None,
                    Err(e) => return Err(e.into()),
                }
            }
        }
        if status.is_none() {
            status = child.try_wait()?;
        }
        if stopping.is_none()
            && (status.is_some()
                || stdout.truncated
                || stderr.truncated
                || Instant::now() >= deadline)
        {
            timed_out = Instant::now() >= deadline;
            kill_workload();
            stdin = None;
            stopping = Some(Instant::now());
        }
        if status.is_some() && stdout.closed && stderr.closed {
            break;
        }
        if stopping.is_some_and(|at| at.elapsed() >= Duration::from_millis(250)) {
            break;
        }
        let mut polls = [
            libc::pollfd {
                fd: if stdout.closed {
                    -1
                } else {
                    stdout.file.as_raw_fd()
                },
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: if stderr.closed {
                    -1
                } else {
                    stderr.file.as_raw_fd()
                },
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: stdin.as_ref().map_or(-1, AsRawFd::as_raw_fd),
                events: libc::POLLOUT,
                revents: 0,
            },
        ];
        // SAFETY: polls is a live initialized array of the declared length.
        if unsafe { libc::poll(polls.as_mut_ptr(), polls.len() as _, 10) } < 0
            && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    kill_workload();
    if status.is_none() {
        status = child.try_wait()?;
    }
    let incomplete = status.is_none() || !stdout.closed || !stderr.closed;
    let exit_code = status.map_or(125, |s| {
        s.code().unwrap_or_else(|| -s.signal().unwrap_or(1))
    });
    Ok((
        Outcome {
            version: wire::VERSION,
            id: request.id.clone(),
            exit_code,
            stdout_length: stdout.bytes.len(),
            stderr_length: stderr.bytes.len(),
            stdout_truncated: stdout.truncated,
            stderr_truncated: stderr.truncated,
            timed_out,
            error: incomplete.then(|| "guest workload or output cleanup was incomplete".into()),
            file_output: None,
            snapshot: None,
            finalized: false,
        },
        stdout.bytes,
        stderr.bytes,
    ))
}
