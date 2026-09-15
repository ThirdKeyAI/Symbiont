//! Runtime-owned guest file transfer. Host paths never cross the guest protocol.
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use symbi_sandbox_guest::files::{Grant, Receipt};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWrite, AsyncWriteExt};

#[derive(Debug)]
pub struct Transfer {
    grant: Grant,
    inputs: Vec<PathBuf>,
    output: Option<File>,
    // Retained by the independent runtime owner through transport and VM cleanup.
    // The VM receives byte copies, never mounts of this private host directory.
    _directory: Arc<symbi_sandbox_supervisor::staging::Lease>,
    started: AtomicBool,
    received: AtomicBool,
    completed: AtomicBool,
}
impl Transfer {
    pub(crate) fn new(
        grant: Grant,
        inputs: Vec<PathBuf>,
        output: Option<File>,
        directory: Arc<symbi_sandbox_supervisor::staging::Lease>,
    ) -> anyhow::Result<Self> {
        grant.validate()?;
        anyhow::ensure!(
            inputs.len() == grant.inputs.len() && output.is_some() == grant.output.is_some(),
            "guest file transfer does not match its grant"
        );
        Ok(Self {
            grant,
            inputs,
            output,
            _directory: directory,
            started: AtomicBool::new(false),
            received: AtomicBool::new(false),
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
            "guest file transfer already consumed"
        );
        Ok(self.grant.clone())
    }
    pub(super) fn has_output(&self) -> bool {
        self.grant.output.is_some()
    }
    pub(super) async fn upload(
        &self,
        writer: &mut (impl AsyncWrite + Unpin),
    ) -> anyhow::Result<()> {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        for (path, expected) in self.inputs.iter().zip(&self.grant.inputs) {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(path)?;
            let metadata = file.metadata()?;
            anyhow::ensure!(
                metadata.is_file() && metadata.nlink() == 1 && metadata.len() == expected.length,
                "private guest input changed"
            );
            let mut file = tokio::fs::File::from_std(file);
            let mut left = expected.length;
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
                format!("{:x}", hash.finalize()) == expected.sha256,
                "private guest input hash changed"
            );
        }
        Ok(())
    }
    pub(super) async fn receive(
        &self,
        reader: &mut (impl AsyncRead + Unpin),
        receipt: &Receipt,
    ) -> anyhow::Result<()> {
        receipt.validate(&self.grant)?;
        let file = self
            .output
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("unexpected guest output"))?
            .try_clone()?;
        let mut file = tokio::fs::File::from_std(file);
        file.set_len(0).await?;
        file.seek(std::io::SeekFrom::Start(0)).await?;
        let mut left = receipt.length;
        let mut buffer = [0; 65536];
        let mut hash = Sha256::new();
        while left > 0 {
            let n = left.min(buffer.len() as u64) as usize;
            reader.read_exact(&mut buffer[..n]).await?;
            hash.update(&buffer[..n]);
            file.write_all(&buffer[..n]).await?;
            left -= n as u64;
        }
        anyhow::ensure!(
            format!("{:x}", hash.finalize()) == receipt.sha256,
            "guest output hash mismatch"
        );
        file.sync_all().await?;
        self.received.store(true, Ordering::Release);
        Ok(())
    }
    pub(super) fn complete(&self) {
        self.completed.store(true, Ordering::Release);
    }
    pub(crate) fn check_publication(&self) -> Result<(), String> {
        if self.has_output()
            && (!self.received.load(Ordering::Acquire) || !self.completed.load(Ordering::Acquire))
        {
            return Err(
                "guest output requires a verified transfer and confirmed VM cleanup".into(),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
pub(super) fn fixture() -> (tempfile::TempDir, Transfer) {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let directory =
        Arc::new(symbi_sandbox_supervisor::staging::Lease::reserve(root.path(), 65536).unwrap());
    let input = directory.path().join("input");
    std::fs::write(&input, b"a\0b").unwrap();
    let output = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(directory.path().join("output"))
        .unwrap();
    let grant = Grant {
        inputs: vec![symbi_sandbox_guest::files::Input {
            path: "/tmp/input".into(),
            length: 3,
            sha256: format!("{:x}", Sha256::digest(b"a\0b")),
        }],
        output: Some("/tmp/result".into()),
        max_file_bytes: 1024,
    };
    (
        root,
        Transfer::new(grant, vec![input], Some(output), directory).unwrap(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn transfer_requires_exact_bytes_one_use_and_confirmed_completion() {
        let (_root, transfer) = fixture();
        transfer.begin().unwrap();
        assert!(transfer.begin().is_err());
        let mut uploaded = Vec::new();
        transfer.upload(&mut uploaded).await.unwrap();
        assert_eq!(uploaded, b"a\0b");
        assert!(transfer.check_publication().is_err());
        let receipt = Receipt {
            path: "/tmp/result".into(),
            length: 3,
            sha256: format!("{:x}", Sha256::digest(b"x\0y")),
        };
        for payload in [b"x".as_slice(), b"bad"] {
            assert!(transfer.receive(&mut &payload[..], &receipt).await.is_err());
            assert!(transfer.check_publication().is_err());
        }
        transfer.receive(&mut &b"x\0y"[..], &receipt).await.unwrap();
        assert!(transfer.check_publication().is_err());
        transfer.complete();
        transfer.check_publication().unwrap();
        assert_eq!(
            std::fs::read(transfer._directory.path().join("output")).unwrap(),
            b"x\0y"
        );
    }
    #[tokio::test]
    async fn mutated_input_is_refused_and_configuration_cannot_restore_authority() {
        let (_root, transfer) = fixture();
        std::fs::write(&transfer.inputs[0], b"bad").unwrap();
        assert!(transfer.upload(&mut Vec::new()).await.is_err());
        let config = super::super::FirecrackerConfig {
            files: Some(Arc::new(transfer)),
            ..Default::default()
        };
        let encoded = serde_json::to_value(config).unwrap();
        assert!(encoded.get("files").is_none());
        let decoded: super::super::FirecrackerConfig = serde_json::from_value(encoded).unwrap();
        assert!(decoded.files.is_none());
    }
}
