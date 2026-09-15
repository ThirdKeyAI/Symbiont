//! Runtime-owned Git snapshot stream. Each bounded entry header precedes its
//! content; a grant hashes the exact ordered headers before any guest execution.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

pub const ROOT: &str = "/tmp/symbi-git-snapshot";
pub const MAX_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_FILES: usize = 10_001;
pub const MAX_ENTRIES: usize = 20_008;
pub const MAX_PATH_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_HEADER: usize = 16 * 1024;
pub const TMPFS_BYTES: u64 = 384 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub entries: usize,
    pub files: usize,
    pub bytes: u64,
    pub path_bytes: usize,
    pub manifest_sha256: String,
}
impl Grant {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.entries > 0
                && self.entries <= MAX_ENTRIES
                && self.files <= MAX_FILES
                && self.bytes <= MAX_BYTES
                && self.path_bytes <= MAX_PATH_BYTES
                && crate::files::valid_hash(&self.manifest_sha256),
            "invalid guest snapshot grant"
        );
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub grant: Grant,
    pub read_only: bool,
    pub filesystem_bytes: u64,
}
impl Receipt {
    pub fn validate(&self, expected: &Grant) -> anyhow::Result<()> {
        expected.validate()?;
        anyhow::ensure!(
            self.grant == *expected
                && self.read_only
                && self.filesystem_bytes > 0
                && self.filesystem_bytes <= TMPFS_BYTES,
            "guest snapshot was not sealed within its admitted filesystem limit"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Directory,
    File,
    Symlink,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub path: String,
    pub kind: Kind,
    pub length: u64,
    pub sha256: String,
    pub executable: bool,
}
impl Entry {
    pub fn validate(&self) -> anyhow::Result<()> {
        let parts: Vec<_> = self.path.split('/').collect();
        anyhow::ensure!(
            self.path.len() <= 4096
                && !self.path.contains('\0')
                && matches!(parts[0], "input" | "worktree")
                && parts.len() <= 68
                && parts
                    .iter()
                    .all(|part| !part.is_empty() && !matches!(*part, "." | "..")),
            "snapshot paths must stay under the fixed input or worktree root"
        );
        anyhow::ensure!(
            crate::files::valid_hash(&self.sha256),
            "invalid snapshot content hash"
        );
        anyhow::ensure!(
            self.length
                <= match self.kind {
                    Kind::Directory => 0,
                    Kind::File => MAX_FILE_BYTES,
                    Kind::Symlink => 4096,
                }
                && (self.kind == Kind::File || !self.executable)
                && (self.kind != Kind::Symlink
                    || (self.path.starts_with("worktree/") && self.length > 0)),
            "invalid guest snapshot entry"
        );
        if self.kind == Kind::Directory {
            anyhow::ensure!(
                self.sha256 == format!("{:x}", Sha256::digest([])),
                "invalid directory digest"
            );
        }
        Ok(())
    }
    pub fn header(&self) -> anyhow::Result<Vec<u8>> {
        self.validate()?;
        let header = serde_json::to_vec(self)?;
        anyhow::ensure!(
            header.len() <= MAX_HEADER,
            "snapshot entry header exceeds limit"
        );
        Ok(header)
    }
}

#[derive(Default)]
pub struct Manifest {
    paths: HashSet<String>,
    files: usize,
    bytes: u64,
    path_bytes: usize,
    hash: Sha256,
}
impl Manifest {
    pub fn add(&mut self, entry: &Entry, header: &[u8]) -> anyhow::Result<()> {
        entry.validate()?;
        anyhow::ensure!(
            !header.is_empty() && header.len() <= MAX_HEADER && self.paths.len() < MAX_ENTRIES,
            "snapshot entry budget exceeded"
        );
        self.path_bytes = self
            .path_bytes
            .checked_add(entry.path.len())
            .ok_or_else(|| anyhow::anyhow!("snapshot path overflow"))?;
        self.bytes = self
            .bytes
            .checked_add(entry.length)
            .ok_or_else(|| anyhow::anyhow!("snapshot byte overflow"))?;
        self.files += usize::from(entry.kind != Kind::Directory);
        anyhow::ensure!(
            self.files <= MAX_FILES
                && self.bytes <= MAX_BYTES
                && self.path_bytes <= MAX_PATH_BYTES
                && self.paths.insert(entry.path.clone()),
            "snapshot aggregate bound or path uniqueness violated"
        );
        self.hash.update((header.len() as u32).to_be_bytes());
        self.hash.update(header);
        Ok(())
    }
    pub fn grant(self) -> Grant {
        Grant {
            entries: self.paths.len(),
            files: self.files,
            bytes: self.bytes,
            path_bytes: self.path_bytes,
            manifest_sha256: format!("{:x}", self.hash.finalize()),
        }
    }
    pub fn verify(self, expected: &Grant) -> anyhow::Result<()> {
        expected.validate()?;
        let actual = self.grant();
        anyhow::ensure!(
            actual.entries == expected.entries
                && actual.files == expected.files
                && actual.bytes == expected.bytes
                && actual.path_bytes == expected.path_bytes
                && actual.manifest_sha256 == expected.manifest_sha256,
            "guest snapshot disagrees with admitted manifest"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(path: &str, kind: Kind) -> Entry {
        Entry {
            path: path.into(),
            kind,
            length: 0,
            sha256: format!("{:x}", Sha256::digest([])),
            executable: false,
        }
    }
    #[test]
    fn snapshot_paths_types_and_manifest_are_bound_before_execution() {
        for path in [
            "../outside",
            "/tmp/outside",
            "input/../outside",
            "worktree//file",
            "worktree/./file",
            "input\0file",
        ] {
            assert!(entry(path, Kind::Directory).validate().is_err(), "{path:?}");
        }
        entry("worktree/a:b\\c", Kind::File).validate().unwrap();
        let mut link = entry("input/metadata/HEAD", Kind::Symlink);
        link.length = 1;
        assert!(link.validate().is_err());
        link.path = "worktree/link".into();
        link.validate().unwrap();
        let directory = entry("input", Kind::Directory);
        let mut manifest = Manifest::default();
        manifest
            .add(&directory, &directory.header().unwrap())
            .unwrap();
        let grant = manifest.grant();
        grant.validate().unwrap();
        let mut duplicate = Manifest::default();
        duplicate
            .add(&directory, &directory.header().unwrap())
            .unwrap();
        assert!(duplicate
            .add(&directory, &directory.header().unwrap())
            .is_err());
        let mut changed = Manifest::default();
        let different = entry("worktree", Kind::Directory);
        changed
            .add(&different, &different.header().unwrap())
            .unwrap();
        assert!(changed.verify(&grant).is_err());
        let mut same = Manifest::default();
        same.add(&directory, &directory.header().unwrap()).unwrap();
        same.verify(&grant).unwrap();
    }
    #[test]
    fn snapshot_payload_and_aggregate_limits_cannot_overflow() {
        let mut manifest = Manifest::default();
        for name in ["a", "b"] {
            let mut file = entry(&format!("worktree/{name}"), Kind::File);
            file.length = MAX_FILE_BYTES;
            manifest.add(&file, &file.header().unwrap()).unwrap();
        }
        let mut extra = entry("worktree/c", Kind::File);
        extra.length = 1;
        assert!(manifest.add(&extra, &extra.header().unwrap()).is_err());
        extra.length = u64::MAX;
        assert!(extra.validate().is_err());
        let mut grant = Grant {
            entries: 1,
            files: 1,
            bytes: 0,
            path_bytes: 1,
            manifest_sha256: "a".repeat(64),
        };
        grant.entries = usize::MAX;
        assert!(grant.validate().is_err());
        grant.entries = 1;
        grant.path_bytes = MAX_PATH_BYTES + 1;
        assert!(grant.validate().is_err());
    }
    #[test]
    fn snapshot_receipt_requires_the_exact_manifest_and_observed_read_only_limit() {
        let entry = entry("input", Kind::Directory);
        let mut manifest = Manifest::default();
        manifest.add(&entry, &entry.header().unwrap()).unwrap();
        let grant = manifest.grant();
        let mut receipt = Receipt {
            grant: grant.clone(),
            read_only: true,
            filesystem_bytes: TMPFS_BYTES,
        };
        receipt.validate(&grant).unwrap();
        receipt.read_only = false;
        assert!(receipt.validate(&grant).is_err());
        receipt.read_only = true;
        receipt.filesystem_bytes = TMPFS_BYTES + 1;
        assert!(receipt.validate(&grant).is_err());
        receipt.filesystem_bytes = TMPFS_BYTES;
        receipt.grant.bytes += 1;
        assert!(receipt.validate(&grant).is_err());
    }
}
