//! Guest PID 1 imports immutable declared inputs and retains the exact new output
//! inode. No path supplied by a workload is used to select a host destination.
use anyhow::Context;
use sha2::{Digest, Sha256};
use std::{
    ffi::CString,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, PermissionsExt},
    },
    time::Instant,
};
use symbi_sandbox_guest::{
    files::{Grant, Receipt},
    Outcome,
};

pub(super) struct Files {
    output: Option<(String, File, u64)>,
}
pub(super) struct Export {
    file: File,
    length: u64,
}
impl Files {
    pub fn receive(reader: &mut impl Read, grant: Option<&Grant>) -> anyhow::Result<Self> {
        let Some(grant) = grant else {
            return Ok(Self { output: None });
        };
        grant.validate()?;
        for input in &grant.inputs {
            let (parent, name) = parent(&input.path, true)?;
            let mut file = open(
                &parent,
                &name,
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                0o600,
            )?;
            let mut hash = Sha256::new();
            let mut remaining = input.length;
            let mut buffer = [0; 65536];
            while remaining > 0 {
                let n = remaining.min(buffer.len() as u64) as usize;
                reader.read_exact(&mut buffer[..n])?;
                file.write_all(&buffer[..n])?;
                hash.update(&buffer[..n]);
                remaining -= n as u64;
            }
            anyhow::ensure!(
                format!("{:x}", hash.finalize()) == input.sha256,
                "guest input hash mismatch"
            );
            file.set_permissions(std::fs::Permissions::from_mode(0o444))?;
        }
        let output = if let Some(path) = &grant.output {
            let (parent, name) = parent(path, true)?;
            let file = open(
                &parent,
                &name,
                libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
                0o600,
            )?;
            // Root ownership and sticky /tmp or root-owned parent directories
            // permit content writes while preventing unlink/rename replacement.
            file.set_permissions(std::fs::Permissions::from_mode(0o666))?;
            Some((path.clone(), file, grant.max_file_bytes))
        } else {
            None
        };
        Ok(Self { output })
    }
    pub fn capture(&mut self, outcome: &mut Outcome) -> anyhow::Result<Option<Export>> {
        if !outcome.files_succeeded() {
            return Ok(None);
        }
        let Some((path, file, maximum)) = &mut self.output else {
            return Ok(None);
        };
        super::quiesce_workload()?;
        let metadata = file.metadata()?;
        anyhow::ensure!(
            metadata.is_file() && metadata.nlink() == 1 && metadata.len() <= *maximum,
            "invalid retained guest output"
        );
        let (parent, name) = parent(path, false)?;
        let current = open(&parent, &name, libc::O_PATH, 0)?.metadata()?;
        anyhow::ensure!(
            current.is_file() && current.dev() == metadata.dev() && current.ino() == metadata.ino(),
            "guest output path changed"
        );
        file.seek(SeekFrom::Start(0))?;
        let mut hash = Sha256::new();
        let mut left = metadata.len();
        let mut buffer = [0; 65536];
        while left > 0 {
            let n = left.min(buffer.len() as u64) as usize;
            file.read_exact(&mut buffer[..n])?;
            hash.update(&buffer[..n]);
            left -= n as u64;
        }
        file.seek(SeekFrom::Start(0))?;
        outcome.file_output = Some(Receipt {
            path: path.clone(),
            length: metadata.len(),
            sha256: format!("{:x}", hash.finalize()),
        });
        Ok(Some(Export {
            file: file.try_clone()?,
            length: metadata.len(),
        }))
    }
}
impl Export {
    pub fn send(mut self, socket: &mut File, deadline: Instant) -> anyhow::Result<()> {
        super::nonblocking(socket)?;
        let mut buffer = [0; 65536];
        while self.length > 0 {
            let n = self.length.min(buffer.len() as u64) as usize;
            self.file.read_exact(&mut buffer[..n])?;
            let mut sent = 0;
            while sent < n {
                anyhow::ensure!(
                    Instant::now() < deadline,
                    "guest file transfer deadline expired"
                );
                match socket.write(&buffer[sent..n]) {
                    Ok(0) => anyhow::bail!("guest file output made no progress"),
                    Ok(written) => sent += written,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        let mut poll = libc::pollfd {
                            fd: socket.as_raw_fd(),
                            events: libc::POLLOUT,
                            revents: 0,
                        };
                        // SAFETY: the initialized descriptor remains owned by socket.
                        if unsafe { libc::poll(&mut poll, 1, 10) } < 0
                            && std::io::Error::last_os_error().kind()
                                != std::io::ErrorKind::Interrupted
                        {
                            return Err(std::io::Error::last_os_error().into());
                        }
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            self.length -= n as u64;
        }
        Ok(())
    }
}
fn parent(path: &str, create: bool) -> anyhow::Result<(File, CString)> {
    symbi_sandbox_guest::files::validate_path(path)?;
    let mut parts: Vec<_> = path
        .strip_prefix("/tmp/")
        .context("guest file root missing")?
        .split('/')
        .collect();
    let name = CString::new(parts.pop().context("guest file leaf missing")?)?;
    let mut directory = File::open("/tmp")?;
    for part in parts {
        let component = CString::new(part)?;
        let mut created = false;
        if create {
            // SAFETY: pinned directory and valid relative NUL-terminated name.
            created =
                unsafe { libc::mkdirat(directory.as_raw_fd(), component.as_ptr(), 0o755) } == 0;
            if !created
                && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        directory = open(
            &directory,
            &component,
            libc::O_RDONLY | libc::O_DIRECTORY,
            0,
        )?;
        if created {
            directory.set_permissions(std::fs::Permissions::from_mode(0o755))?;
        }
        anyhow::ensure!(
            directory.metadata()?.uid() == 0 && directory.metadata()?.mode() & 0o022 == 0,
            "guest file parent must remain root-owned and immutable to the workload"
        );
    }
    Ok((directory, name))
}
fn open(parent: &File, name: &CString, flags: i32, mode: libc::mode_t) -> anyhow::Result<File> {
    // SAFETY: valid owned parent and C string; successful descriptor is fresh.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            mode,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: sole ownership transfers exactly once from openat.
    Ok(unsafe { File::from_raw_fd(fd) })
}
