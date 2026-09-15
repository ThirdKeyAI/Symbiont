//! Populate one fixed, read-only guest tmpfs before starting any workload.
use anyhow::Context;
use sha2::{Digest, Sha256};
use std::{
    ffi::CString,
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::PermissionsExt,
    },
};
use symbi_sandbox_guest::snapshot::{self as wire, Entry, Grant, Kind, Manifest, Receipt};

pub(super) fn receive(
    reader: &mut impl Read,
    grant: Option<&Grant>,
) -> anyhow::Result<Option<Receipt>> {
    let Some(grant) = grant else {
        return Ok(None);
    };
    grant.validate()?;
    std::fs::create_dir(wire::ROOT)?;
    std::fs::set_permissions(wire::ROOT, std::fs::Permissions::from_mode(0o755))?;
    super::mount(
        wire::ROOT,
        "tmpfs",
        &format!("mode=0755,size={}", wire::TMPFS_BYTES),
    )?;
    let root = File::open(wire::ROOT)?;
    let mut manifest = Manifest::default();
    for _ in 0..grant.entries {
        let mut length = [0; 4];
        reader.read_exact(&mut length)?;
        let length = u32::from_be_bytes(length) as usize;
        anyhow::ensure!(
            (1..=wire::MAX_HEADER).contains(&length),
            "invalid snapshot entry header length"
        );
        let mut header = vec![0; length];
        reader.read_exact(&mut header)?;
        let entry: Entry = serde_json::from_slice(&header)?;
        manifest.add(&entry, &header)?;
        let (parent, name) = parent(&root, &entry.path)?;
        match entry.kind {
            Kind::Directory => {
                // SAFETY: root-pinned parent and a validated single component.
                if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o755) } != 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
                let dir = open(&parent, &name, libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
                dir.set_permissions(std::fs::Permissions::from_mode(0o755))?;
            }
            Kind::File => {
                let mut file = open(
                    &parent,
                    &name,
                    libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                    0o600,
                )?;
                let mut remaining = entry.length;
                let mut hash = Sha256::new();
                let mut buffer = [0; 65536];
                while remaining > 0 {
                    let n = remaining.min(buffer.len() as u64) as usize;
                    reader.read_exact(&mut buffer[..n])?;
                    hash.update(&buffer[..n]);
                    file.write_all(&buffer[..n])?;
                    remaining -= n as u64;
                }
                anyhow::ensure!(
                    format!("{:x}", hash.finalize()) == entry.sha256,
                    "guest snapshot file hash mismatch"
                );
                file.set_permissions(std::fs::Permissions::from_mode(if entry.executable {
                    0o555
                } else {
                    0o444
                }))?;
            }
            Kind::Symlink => {
                let mut target = vec![0; entry.length as usize];
                reader.read_exact(&mut target)?;
                anyhow::ensure!(
                    format!("{:x}", Sha256::digest(&target)) == entry.sha256,
                    "guest snapshot link hash mismatch"
                );
                let target = CString::new(target)?;
                // SAFETY: link text is data, not a path to open; parent is confined.
                if unsafe { libc::symlinkat(target.as_ptr(), parent.as_raw_fd(), name.as_ptr()) }
                    != 0
                {
                    return Err(std::io::Error::last_os_error().into());
                }
            }
        }
    }
    manifest.verify(grant)?;
    let target = CString::new(wire::ROOT)?;
    // SAFETY: the fixed mount contains no live workload. Seal all entries before exec.
    if unsafe {
        libc::mount(
            std::ptr::null(),
            target.as_ptr(),
            std::ptr::null(),
            libc::MS_REMOUNT | libc::MS_RDONLY | libc::MS_NOSUID | libc::MS_NODEV,
            std::ptr::null(),
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error()).context("seal guest snapshot");
    }
    // SAFETY: fstatvfs initializes this struct for the retained snapshot root.
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::fstatvfs(root.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: the successful call initialized every field.
    let stat = unsafe { stat.assume_init() };
    let receipt = Receipt {
        grant: grant.clone(),
        read_only: stat.f_flag & libc::ST_RDONLY != 0,
        filesystem_bytes: stat
            .f_blocks
            .checked_mul(stat.f_frsize)
            .context("snapshot filesystem size overflow")?,
    };
    receipt.validate(grant)?;
    Ok(Some(receipt))
}
fn parent(root: &File, path: &str) -> anyhow::Result<(File, CString)> {
    let mut parts: Vec<_> = path.split('/').collect();
    let leaf = CString::new(parts.pop().context("snapshot leaf missing")?)?;
    let mut parent = root.try_clone()?;
    for name in parts {
        parent = open(
            &parent,
            &CString::new(name)?,
            libc::O_RDONLY | libc::O_DIRECTORY,
            0,
        )?;
    }
    Ok((parent, leaf))
}
fn open(parent: &File, name: &CString, flags: i32, mode: libc::mode_t) -> anyhow::Result<File> {
    // SAFETY: valid retained directory and single-component C string.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            mode,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: transfer sole ownership of the freshly opened descriptor.
    Ok(unsafe { File::from_raw_fd(fd) })
}
