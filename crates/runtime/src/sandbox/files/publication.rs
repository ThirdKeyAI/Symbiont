use super::{linux, PublicationIntent, PublicationState, MAX_FILE_BYTES};
use anyhow::Context;
use sha2::{Digest, Sha256};
use std::{
    ffi::CString,
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::PathBuf,
    sync::Arc,
};

pub(super) struct Candidate {
    pub intent: PublicationIntent,
    output: Arc<linux::Output>,
    name: CString,
    file: File,
    retained: bool,
}

impl Candidate {
    pub fn prepare(
        source: PathBuf,
        output: Arc<linux::Output>,
        limit: u64,
    ) -> anyhow::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(source)?;
        let metadata = file.metadata()?;
        anyhow::ensure!(
            metadata.is_file() && metadata.nlink() == 1 && metadata.len() <= limit,
            "invalid or oversized worker output"
        );
        let mut bytes = Vec::new();
        file.take(limit + 1).read_to_end(&mut bytes)?;
        anyhow::ensure!(
            bytes.len() as u64 <= limit,
            "worker output exceeds the file budget"
        );
        Self::check_parent(&output)?;
        let publication_id = uuid::Uuid::new_v4();
        let candidate_name = format!(".symbi-publish-{publication_id}");
        let candidate = linux::open_at(
            &output.parent,
            &candidate_name,
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
            0o600,
        )?;
        let metadata = candidate.metadata()?;
        let mut prepared = Self {
            name: CString::new(candidate_name.as_str())?,
            intent: PublicationIntent {
                publication_id,
                path: output.path.clone(),
                parent_path: output.parent_path.clone(),
                parent_identity: output.parent_identity,
                candidate_name,
                candidate_identity: (metadata.dev(), metadata.ino()),
                target_name: output.name.to_str()?.to_owned(),
                bytes: bytes.len() as u64,
                sha256: hex::encode(Sha256::digest(&bytes)),
            },
            file: candidate,
            output,
            retained: false,
        };
        prepared.file.write_all(&bytes)?;
        prepared.file.sync_all()?;
        prepared.output.parent.sync_all()?;
        Ok(prepared)
    }

    fn check_parent(output: &linux::Output) -> anyhow::Result<()> {
        let current = linux::directory(&output.parent_path)?.metadata()?;
        anyhow::ensure!(
            (current.dev(), current.ino()) == output.parent_identity,
            "output parent pathname changed; publication requires reconciliation"
        );
        Ok(())
    }

    pub fn retain(&mut self) {
        self.retained = true;
    }

    pub fn commit(&mut self) -> anyhow::Result<serde_json::Value> {
        Self::check_parent(&self.output)?;
        let current = linux::open_at(
            &self.output.parent,
            &self.intent.candidate_name,
            libc::O_RDONLY,
            0,
        )?;
        let metadata = current.metadata()?;
        anyhow::ensure!(
            metadata.is_file()
                && metadata.nlink() == 1
                && (metadata.dev(), metadata.ino()) == self.intent.candidate_identity
                && metadata.len() == self.intent.bytes,
            "publication candidate identity changed"
        );
        let mut bytes = Vec::new();
        current
            .take(self.intent.bytes + 1)
            .read_to_end(&mut bytes)?;
        anyhow::ensure!(
            bytes.len() as u64 == self.intent.bytes
                && hex::encode(Sha256::digest(&bytes)) == self.intent.sha256,
            "publication candidate content changed"
        );
        // SAFETY: pinned parent descriptors and valid, owned C strings. The
        // non-replacing rename cannot overwrite a competing file or link.
        if unsafe {
            libc::renameat2(
                self.output.parent.as_raw_fd(),
                self.name.as_ptr(),
                self.output.parent.as_raw_fd(),
                self.output.name.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        self.output
            .parent
            .sync_all()
            .context("output created but directory durability is uncertain")?;
        Self::check_parent(&self.output)?;
        Ok(
            serde_json::json!([{"path":self.intent.path,"bytes":self.intent.bytes,"sha256":self.intent.sha256}]),
        )
    }
}

impl Drop for Candidate {
    fn drop(&mut self) {
        if !self.retained {
            // SAFETY: delete only this broker-generated candidate name. Once
            // recording starts, uncertainty instead retains it for recovery.
            unsafe {
                libc::unlinkat(self.output.parent.as_raw_fd(), self.name.as_ptr(), 0);
            }
        }
    }
}

fn parent(intent: &PublicationIntent) -> anyhow::Result<File> {
    anyhow::ensure!(
        intent.bytes <= MAX_FILE_BYTES
            && intent.candidate_name == format!(".symbi-publish-{}", intent.publication_id)
            && intent.sha256.len() == 64
            && intent.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            && super::resolve(&intent.path, &Default::default())? == intent.path
            && std::path::Path::new(&intent.path)
                .file_name()
                .and_then(|name| name.to_str())
                == Some(intent.target_name.as_str()),
        "invalid authenticated publication intent"
    );
    let parent = linux::directory(&intent.parent_path)?;
    let metadata = parent.metadata()?;
    anyhow::ensure!(
        (metadata.dev(), metadata.ino()) == intent.parent_identity,
        "publication parent identity changed"
    );
    Ok(parent)
}

fn matches(parent: &File, name: &str, intent: &PublicationIntent) -> anyhow::Result<Option<bool>> {
    let file = match linux::open_at(parent, name, libc::O_RDONLY, 0) {
        Ok(file) => file,
        Err(error) => {
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound)
            {
                return Ok(None);
            }
            return Err(error);
        }
    };
    let metadata = file.metadata()?;
    // SAFETY: geteuid has no preconditions. Recovery never adopts files owned
    // by another identity or writable through an additional hard link.
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
        || metadata.len() != intent.bytes
        || (metadata.dev(), metadata.ino()) != intent.candidate_identity
    {
        return Ok(Some(false));
    }
    let mut bytes = Vec::new();
    file.take(intent.bytes + 1).read_to_end(&mut bytes)?;
    Ok(Some(
        bytes.len() as u64 == intent.bytes && hex::encode(Sha256::digest(bytes)) == intent.sha256,
    ))
}

pub(super) fn inspect(intent: &PublicationIntent) -> anyhow::Result<PublicationState> {
    let parent = parent(intent)?;
    Ok(
        match (
            matches(&parent, &intent.candidate_name, intent)?,
            matches(&parent, &intent.target_name, intent)?,
        ) {
            (Some(true), None) => PublicationState::ReadyToPublish,
            (None, Some(true)) => PublicationState::Published,
            (None, None) => PublicationState::Missing,
            _ => PublicationState::Conflict,
        },
    )
}

pub(super) fn recover(intent: &PublicationIntent) -> anyhow::Result<serde_json::Value> {
    let directory = parent(intent)?;
    match inspect(intent)? {
        PublicationState::ReadyToPublish => {
            let output = Arc::new(linux::Output {
                path: intent.path.clone(),
                parent_path: intent.parent_path.clone(),
                parent_identity: intent.parent_identity,
                parent: directory,
                name: CString::new(intent.target_name.as_str())?,
            });
            let file = linux::open_at(&output.parent, &intent.candidate_name, libc::O_RDWR, 0)?;
            let mut candidate = Candidate {
                intent: intent.clone(),
                output,
                file,
                name: CString::new(intent.candidate_name.as_str())?,
                retained: true,
            };
            candidate.commit()?;
        }
        PublicationState::Published => directory.sync_all()?,
        _ => anyhow::bail!("publication candidate is missing or conflicts; no file changed"),
    }
    anyhow::ensure!(
        inspect(intent)? == PublicationState::Published,
        "publication did not retain its expected identity and content"
    );
    Ok(serde_json::json!([{"path":intent.path,"bytes":intent.bytes,"sha256":intent.sha256}]))
}
