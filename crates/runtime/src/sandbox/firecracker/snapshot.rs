//! Immutable prepared Git snapshot authority. No archive is extracted on the
//! host, and no source directory is mounted in the guest.
use anyhow::Context;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
    },
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};
use symbi_sandbox_guest::snapshot::{Entry, Grant, Kind, Manifest, Receipt};
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Debug)]
pub struct Transfer {
    directory: Arc<symbi_sandbox_supervisor::staging::Lease>,
    entries: Vec<Entry>,
    grant: Grant,
    started: AtomicBool,
    receipt: Mutex<Option<Receipt>>,
    completed: AtomicBool,
}
impl Transfer {
    pub(crate) fn prepare(
        directory: Arc<symbi_sandbox_supervisor::staging::Lease>,
        deadline: Instant,
    ) -> anyhow::Result<Self> {
        let mut entries = Vec::new();
        let mut manifest = Manifest::default();
        for root in ["input", "worktree"] {
            collect(
                directory.path(),
                root,
                deadline,
                &mut entries,
                &mut manifest,
            )?;
        }
        let grant = manifest.grant();
        grant.validate()?;
        Ok(Self {
            directory,
            entries,
            grant,
            started: AtomicBool::new(false),
            receipt: Mutex::new(None),
            completed: AtomicBool::new(false),
        })
    }
    pub(super) fn validate(&self) -> anyhow::Result<()> {
        self.grant.validate()
    }
    pub(super) fn begin(&self) -> anyhow::Result<Grant> {
        self.validate()?;
        anyhow::ensure!(
            !self.started.swap(true, Ordering::AcqRel),
            "guest Git snapshot already consumed"
        );
        Ok(self.grant.clone())
    }
    pub(super) fn observed(&self, receipt: Receipt) -> anyhow::Result<()> {
        receipt.validate(&self.grant)?;
        *self
            .receipt
            .lock()
            .map_err(|_| anyhow::anyhow!("snapshot receipt lock failed"))? = Some(receipt);
        Ok(())
    }
    pub(super) fn complete(&self) {
        self.completed.store(true, Ordering::Release);
    }
    pub(crate) fn receipt(&self) -> Result<Option<Receipt>, String> {
        if !self.completed.load(Ordering::Acquire) {
            return Ok(None);
        }
        Ok(self
            .receipt
            .lock()
            .map_err(|_| "snapshot receipt lock failed")?
            .clone())
    }
    pub(crate) fn descriptor(&self) -> &Grant {
        &self.grant
    }
    pub(super) async fn upload(
        &self,
        writer: &mut (impl AsyncWrite + Unpin),
    ) -> anyhow::Result<()> {
        for entry in &self.entries {
            let path = self.directory.path().join(&entry.path);
            let header = entry.header()?;
            writer.write_u32(header.len() as u32).await?;
            writer.write_all(&header).await?;
            let metadata = std::fs::symlink_metadata(&path)?;
            match entry.kind {
                Kind::Directory => {
                    anyhow::ensure!(metadata.is_dir(), "private snapshot directory changed")
                }
                Kind::Symlink => {
                    anyhow::ensure!(
                        metadata.file_type().is_symlink(),
                        "private snapshot link changed"
                    );
                    let target = std::fs::read_link(&path)?;
                    let bytes = target.as_os_str().as_bytes();
                    anyhow::ensure!(
                        bytes.len() as u64 == entry.length
                            && format!("{:x}", Sha256::digest(bytes)) == entry.sha256,
                        "private snapshot link content changed"
                    );
                    writer.write_all(bytes).await?;
                }
                Kind::File => {
                    let file = open(&path)?;
                    let metadata = file.metadata()?;
                    anyhow::ensure!(
                        metadata.is_file()
                            && metadata.nlink() == 1
                            && metadata.len() == entry.length
                            && (metadata.mode() & 0o111 != 0) == entry.executable,
                        "private snapshot file changed"
                    );
                    let mut file = tokio::fs::File::from_std(file);
                    let mut left = entry.length;
                    let mut buffer = [0; 65536];
                    let mut hash = Sha256::new();
                    while left > 0 {
                        let n = left.min(buffer.len() as u64) as usize;
                        file.read_exact(&mut buffer[..n]).await?;
                        hash.update(&buffer[..n]);
                        writer.write_all(&buffer[..n]).await?;
                        left -= n as u64;
                    }
                    anyhow::ensure!(
                        format!("{:x}", hash.finalize()) == entry.sha256,
                        "private snapshot content changed"
                    );
                }
            }
        }
        Ok(())
    }
}
fn open(path: &Path) -> anyhow::Result<File> {
    Ok(std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)?)
}
fn collect(
    root: &Path,
    path: &str,
    deadline: Instant,
    entries: &mut Vec<Entry>,
    manifest: &mut Manifest,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        Instant::now() < deadline,
        "Git snapshot transfer preparation expired"
    );
    let target = root.join(path);
    let metadata = std::fs::symlink_metadata(&target)?;
    let (kind, length, hash, executable) = if metadata.is_dir() {
        (Kind::Directory, 0, Sha256::digest([]).to_vec(), false)
    } else if metadata.file_type().is_symlink() {
        let text = std::fs::read_link(&target)?;
        let bytes = text.as_os_str().as_bytes();
        (
            Kind::Symlink,
            bytes.len() as u64,
            Sha256::digest(bytes).to_vec(),
            false,
        )
    } else {
        let mut file = open(&target)?;
        let metadata = file.metadata()?;
        anyhow::ensure!(
            metadata.is_file()
                && metadata.nlink() == 1
                && metadata.len() <= symbi_sandbox_guest::snapshot::MAX_FILE_BYTES,
            "invalid private Git snapshot file"
        );
        let mut hash = Sha256::new();
        let mut count = 0u64;
        let mut buffer = [0; 65536];
        loop {
            anyhow::ensure!(
                Instant::now() < deadline,
                "Git snapshot transfer preparation expired"
            );
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            count += n as u64;
            anyhow::ensure!(count <= metadata.len(), "private Git snapshot grew");
            hash.update(&buffer[..n]);
        }
        anyhow::ensure!(count == metadata.len(), "private Git snapshot shrank");
        (
            Kind::File,
            count,
            hash.finalize().to_vec(),
            metadata.mode() & 0o111 != 0,
        )
    };
    let entry = Entry {
        path: path.into(),
        kind,
        length,
        sha256: hex::encode(hash),
        executable,
    };
    manifest.add(&entry, &entry.header()?)?;
    entries.push(entry);
    if kind == Kind::Directory {
        let mut names = Vec::new();
        for child in std::fs::read_dir(&target)? {
            anyhow::ensure!(
                names.len() < symbi_sandbox_guest::snapshot::MAX_ENTRIES,
                "snapshot enumeration exceeds limit"
            );
            names.push(
                child?
                    .file_name()
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("snapshot needs UTF-8 names"))?,
            );
        }
        names.sort();
        for name in names {
            collect(root, &format!("{path}/{name}"), deadline, entries, manifest)
                .with_context(|| format!("snapshot entry {path}/{name}"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};
    fn fixture() -> (
        tempfile::TempDir,
        Arc<symbi_sandbox_supervisor::staging::Lease>,
    ) {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let lease = Arc::new(
            symbi_sandbox_supervisor::staging::Lease::reserve(root.path(), 65536).unwrap(),
        );
        std::fs::create_dir_all(lease.path().join("input/metadata")).unwrap();
        std::fs::create_dir(lease.path().join("worktree")).unwrap();
        std::fs::write(
            lease.path().join("input/metadata/HEAD"),
            b"ref: refs/heads/main\n",
        )
        .unwrap();
        std::fs::write(lease.path().join("worktree/a:b\\c"), b"literal\0bytes").unwrap();
        symlink("/outside/observer", lease.path().join("worktree/link")).unwrap();
        (root, lease)
    }
    #[tokio::test]
    async fn snapshot_stream_preserves_exact_content_and_never_follows_link_targets() {
        let (_root, lease) = fixture();
        let transfer =
            Transfer::prepare(lease, Instant::now() + std::time::Duration::from_secs(5)).unwrap();
        let grant = transfer.begin().unwrap();
        assert!(transfer.begin().is_err());
        let mut stream = Vec::new();
        transfer.upload(&mut stream).await.unwrap();
        let mut reader = &stream[..];
        let mut manifest = Manifest::default();
        let mut paths = Vec::new();
        for _ in 0..grant.entries {
            let length = reader.read_u32().await.unwrap() as usize;
            assert!(length <= symbi_sandbox_guest::snapshot::MAX_HEADER);
            let mut header = vec![0; length];
            tokio::io::AsyncReadExt::read_exact(&mut reader, &mut header)
                .await
                .unwrap();
            let entry: Entry = serde_json::from_slice(&header).unwrap();
            manifest.add(&entry, &header).unwrap();
            let mut bytes = vec![0; entry.length as usize];
            tokio::io::AsyncReadExt::read_exact(&mut reader, &mut bytes)
                .await
                .unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(&bytes)), entry.sha256);
            if entry.path == "worktree/link" {
                assert_eq!(bytes, b"/outside/observer");
            }
            if entry.path == "worktree/a:b\\c" {
                assert_eq!(bytes, b"literal\0bytes");
            }
            paths.push(entry.path);
        }
        assert!(reader.is_empty());
        manifest.verify(&grant).unwrap();
        assert_eq!(paths.len(), 6);
    }
    #[tokio::test]
    async fn snapshot_mutation_and_configuration_rehydration_are_refused() {
        let (_root, lease) = fixture();
        let transfer = Transfer::prepare(
            lease.clone(),
            Instant::now() + std::time::Duration::from_secs(5),
        )
        .unwrap();
        std::fs::write(lease.path().join("worktree/a:b\\c"), b"changed").unwrap();
        assert!(transfer.upload(&mut Vec::new()).await.is_err());
        let config = super::super::FirecrackerConfig {
            snapshot: Some(Arc::new(transfer)),
            ..Default::default()
        };
        let encoded = serde_json::to_value(config).unwrap();
        assert!(encoded.get("snapshot").is_none());
        let restored: super::super::FirecrackerConfig = serde_json::from_value(encoded).unwrap();
        assert!(restored.snapshot.is_none());
    }
}
