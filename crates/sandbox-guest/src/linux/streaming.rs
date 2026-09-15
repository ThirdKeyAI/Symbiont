use super::{kill_workload, nonblocking, spawn_command};
use anyhow::Context;
use std::{
    collections::VecDeque,
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, OwnedFd},
        unix::process::ExitStatusExt,
    },
    time::{Duration, Instant},
};
use symbi_sandbox_guest::{
    self as wire,
    stream::{self, Kind},
};

struct Pending {
    bytes: Vec<u8>,
    offset: usize,
}
impl Pending {
    fn new(kind: Kind, bytes: &[u8]) -> anyhow::Result<Self> {
        Ok(Self {
            bytes: stream::encode(kind, bytes)?,
            offset: 0,
        })
    }
    fn write(&mut self, file: &mut File) -> anyhow::Result<bool> {
        match file.write(&self.bytes[self.offset..]) {
            Ok(0) => anyhow::bail!("guest stream write made no progress"),
            Ok(n) => self.offset += n,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) => {}
            Err(e) => return Err(e.into()),
        }
        Ok(self.offset == self.bytes.len())
    }
}

struct Output {
    file: File,
    count: usize,
    closed: bool,
    truncated: bool,
    kind: Kind,
    terminal: bool,
}
impl Output {
    fn new(fd: OwnedFd, kind: Kind, terminal: bool) -> anyhow::Result<Self> {
        let file = File::from(fd);
        nonblocking(&file)?;
        Ok(Self {
            file,
            count: 0,
            closed: false,
            truncated: false,
            kind,
            terminal,
        })
    }
    fn read(&mut self, queue: &mut VecDeque<Pending>, limit: usize) -> anyhow::Result<()> {
        if self.closed || queue.len() >= 4 {
            return Ok(());
        }
        let mut buffer = [0; 8192];
        match self.file.read(&mut buffer) {
            Ok(0) => self.closed = true,
            Ok(n) => {
                let keep = n.min(limit.saturating_sub(self.count));
                self.truncated |= keep != n;
                self.count += keep;
                if keep != 0 {
                    queue.push_back(Pending::new(self.kind, &buffer[..keep])?);
                }
            }
            Err(e) if self.terminal && e.raw_os_error() == Some(libc::EIO) => self.closed = true,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) => {}
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }
}

/// One nonblocking loop multiplexes all streams. Only four outgoing chunks and
/// one incoming frame can wait in memory; blocked readers cannot starve expiry.
pub(super) fn serve(
    socket: &mut File,
    request: &wire::Command,
    files: &mut super::files::Files,
) -> anyhow::Result<()> {
    let terminal = request.mode == wire::CommandMode::Pty;
    let (mut child, input_file, output_file, error_file) = if terminal {
        let (child, master) = super::terminal::spawn(request)?;
        (child, master.try_clone()?, master, File::open("/dev/null")?)
    } else {
        let mut child = spawn_command(request)?;
        let input = File::from(OwnedFd::from(child.stdin.take().context("missing stdin")?));
        let output = File::from(OwnedFd::from(
            child.stdout.take().context("missing stdout")?,
        ));
        let error = File::from(OwnedFd::from(
            child.stderr.take().context("missing stderr")?,
        ));
        (child, input, output, error)
    };
    nonblocking(socket)?;
    nonblocking(&input_file)?;
    let mut stdin = Some(input_file);
    let mut stdout = Output::new(output_file.into(), Kind::Stdout, terminal)?;
    let mut stderr = Output::new(error_file.into(), Kind::Stderr, false)?;
    let started = stream::Started {
        version: wire::VERSION,
        id: request.id.clone(),
    };
    let mut queue = VecDeque::from([Pending::new(Kind::Started, &serde_json::to_vec(&started)?)?]);
    let mut decoder = stream::Decoder::default();
    let mut input = Vec::new();
    let mut input_offset = 0;
    let mut input_count = 0usize;
    let mut input_closed = false;
    let mut status = None;
    let mut stopping = None;
    let mut outcome_sent = false;
    let mut finalized = false;
    let mut exported = None;
    let deadline = Instant::now() + Duration::from_millis(request.timeout_ms);
    loop {
        if Instant::now() >= deadline {
            anyhow::bail!("guest stdio deadline expired");
        }
        if let Some(pending) = queue.front_mut() {
            if pending.write(socket)? {
                queue.pop_front();
            }
        }
        if outcome_sent && queue.is_empty() {
            if let Some(exported) = exported.take() {
                super::files::Export::send(exported, socket, deadline)?;
            }
            return Ok(());
        }
        if !outcome_sent {
            stdout.read(&mut queue, request.max_output_bytes)?;
            stderr.read(&mut queue, request.max_output_bytes)?;
        }
        if stopping.is_none() && input_offset == input.len() {
            if let Some((kind, bytes)) = decoder.read(socket)? {
                match kind {
                    Kind::Input if !input_closed => {
                        input_count = input_count
                            .checked_add(bytes.len())
                            .context("guest input overflow")?;
                        if input_count > wire::MAX_INPUT {
                            anyhow::bail!("guest input limit exceeded");
                        }
                        input = bytes;
                        input_offset = 0;
                    }
                    Kind::InputClosed if !input_closed => {
                        input_closed = true;
                        stdin = None;
                        // PTYs have no independent write-half close. End the
                        // workload rather than inventing a terminal control byte.
                        if terminal {
                            kill_workload();
                            stopping = Some(Instant::now());
                        }
                    }
                    Kind::Finalize => {
                        let control: stream::Started = serde_json::from_slice(&bytes)?;
                        anyhow::ensure!(
                            control.version == wire::VERSION
                                && control.id == request.id
                                && request
                                    .files
                                    .as_ref()
                                    .is_some_and(|grant| grant.output.is_some()),
                            "invalid guest file finalization"
                        );
                        finalized = true;
                        kill_workload();
                        stdin = None;
                        stopping = Some(Instant::now());
                    }
                    _ => anyhow::bail!("invalid host-to-guest stream frame"),
                }
            }
        }
        if input_offset < input.len() {
            if let Some(file) = &mut stdin {
                match file.write(&input[input_offset..]) {
                    Ok(0) => anyhow::bail!("guest stdin write made no progress"),
                    Ok(n) => input_offset += n,
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                        ) => {}
                    Err(e)
                        if e.kind() == std::io::ErrorKind::BrokenPipe
                            || (terminal && e.raw_os_error() == Some(libc::EIO)) =>
                    {
                        stdin = None;
                        input_offset = input.len();
                    }
                    Err(e) => return Err(e.into()),
                }
            } else {
                // A child may deliberately close stdin before exiting. Keep
                // validating and bounding the host stream until its EOF.
                input_offset = input.len();
            }
        }
        if status.is_none() {
            status = child.try_wait()?;
        }
        if stopping.is_none() && (status.is_some() || stdout.truncated || stderr.truncated) {
            kill_workload();
            stdin = None;
            stopping = Some(Instant::now());
        }
        if let Some(status) = status.filter(|_| !outcome_sent && stdout.closed && stderr.closed) {
            let mut outcome = wire::Outcome {
                version: wire::VERSION,
                id: request.id.clone(),
                exit_code: status
                    .code()
                    .unwrap_or_else(|| -status.signal().unwrap_or(1)),
                stdout_length: stdout.count,
                stderr_length: stderr.count,
                stdout_truncated: stdout.truncated,
                stderr_truncated: stderr.truncated,
                timed_out: false,
                error: None,
                file_output: None,
                snapshot: None,
                finalized,
            };
            exported = files.capture(&mut outcome)?;
            outcome.validate(request)?;
            queue.push_back(Pending::new(Kind::Outcome, &serde_json::to_vec(&outcome)?)?);
            outcome_sent = true;
        }
        if stopping.is_some_and(|at| at.elapsed() > Duration::from_millis(250)) && !outcome_sent {
            anyhow::bail!("guest stdio process or output cleanup was incomplete");
        }
        let mut polls = [
            libc::pollfd {
                fd: socket.as_raw_fd(),
                events: (if stopping.is_none() && input_offset == input.len() {
                    libc::POLLIN
                } else {
                    0
                }) | (if queue.is_empty() { 0 } else { libc::POLLOUT }),
                revents: 0,
            },
            libc::pollfd {
                fd: if stdout.closed || queue.len() >= 4 {
                    -1
                } else {
                    stdout.file.as_raw_fd()
                },
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: if stderr.closed || queue.len() >= 4 {
                    -1
                } else {
                    stderr.file.as_raw_fd()
                },
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: if input_offset < input.len() {
                    stdin.as_ref().map_or(-1, AsRawFd::as_raw_fd)
                } else {
                    -1
                },
                events: libc::POLLOUT,
                revents: 0,
            },
        ];
        // SAFETY: the array is initialized and its descriptors remain owned.
        if unsafe { libc::poll(polls.as_mut_ptr(), polls.len() as _, 10) } < 0
            && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
}
