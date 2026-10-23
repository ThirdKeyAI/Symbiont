//! Reads confined to an operator-selected skill directory, including during renames.
use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use std::{
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

use super::scanner::ScanLimits;

pub(super) struct SkillDirectory(Dir);

impl SkillDirectory {
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        // Resolve the operator-selected parent first (system paths such as
        // /var on macOS can be links). Pin that parent by walking its canonical
        // path without following links, then open the skill root without following
        // a link. Every descendant read remains relative to this retained root.
        let absolute = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()?.join(path)
        };
        let root_name = absolute
            .file_name()
            .ok_or_else(|| invalid("skill directory requires a final name"))?;
        let parent = absolute
            .parent()
            .ok_or_else(|| invalid("skill directory requires a parent"))?
            .canonicalize()?;
        let mut components = parent.components();
        let mut anchor = PathBuf::new();
        let first = components
            .next()
            .ok_or_else(|| invalid("empty skill parent"))?;
        anchor.push(first.as_os_str());
        if matches!(first, Component::Prefix(_)) {
            let root = components
                .next()
                .ok_or_else(|| invalid("missing filesystem root"))?;
            if root != Component::RootDir {
                return Err(invalid("skill parent must be absolute"));
            }
            anchor.push(root.as_os_str());
        }
        let mut directory = Dir::open_ambient_dir(&anchor, cap_std::ambient_authority())?;
        for component in components {
            let Component::Normal(name) = component else {
                return Err(invalid("invalid skill parent component"));
            };
            directory = directory.open_dir_nofollow(name)?;
        }
        let directory = directory.open_dir_nofollow(root_name)?;
        Ok(Self(directory))
    }

    pub(super) fn read_text(&self, path: &Path, limit: u64) -> io::Result<String> {
        String::from_utf8(self.read_bytes(path, limit)?)
            .map_err(|_| invalid("skill text must be UTF-8"))
    }

    fn read_bytes(&self, path: &Path, limit: u64) -> io::Result<Vec<u8>> {
        let mut components = path.components().peekable();
        let mut parent = self.0.try_clone()?;
        while let Some(component) = components.next() {
            let Component::Normal(name) = component else {
                return Err(invalid(
                    "skill paths must have ordinary relative components",
                ));
            };
            if components.peek().is_some() {
                parent = parent.open_dir_nofollow(name)?;
                continue;
            }
            let mut options = OpenOptions::new();
            options.read(true).follow(FollowSymlinks::No);
            #[cfg(unix)]
            {
                use cap_std::fs::OpenOptionsExt;
                // A file replaced by a FIFO must not block before type checking.
                options.custom_flags(libc::O_NONBLOCK);
            }
            let file = parent.open_with(name, &options)?;
            let metadata = file.metadata()?;
            if !metadata.is_file() || metadata.nlink() != 1 {
                return Err(invalid("skill reads require a regular file with one link"));
            }
            if metadata.len() > limit {
                return Err(invalid("skill file exceeds the byte limit"));
            }
            let mut bytes = Vec::new();
            file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > limit {
                return Err(invalid("skill file exceeds the byte limit"));
            }
            return Ok(bytes);
        }
        Err(invalid("skill file path is empty"))
    }

    pub(super) fn text_paths(&self, limits: &ScanLimits) -> io::Result<Vec<PathBuf>> {
        self.paths(limits, text_file)
    }

    fn paths(&self, limits: &ScanLimits, include: fn(&Path) -> bool) -> io::Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        // Bound discovery as well as reads, including binary files and empty
        // directories. Allow each text file a full chain of parent directories.
        let mut entries_left = limits
            .max_files
            .saturating_mul(limits.max_depth.saturating_add(1));
        walk(
            &self.0,
            Path::new(""),
            0,
            limits,
            include,
            &mut entries_left,
            &mut files,
        )?;
        files.sort();
        Ok(files)
    }

    pub(super) fn verification_snapshot(&self) -> io::Result<tempfile::TempDir> {
        self.snapshot(
            &ScanLimits {
                max_files: 1_000,
                max_depth: 20,
                max_file_size: 16 * 1024 * 1024,
            },
            64 * 1024 * 1024,
        )
    }

    pub(super) fn signature_manifest(
        &self,
    ) -> io::Result<std::collections::BTreeMap<String, String>> {
        self.signature_manifest_with_size()
            .map(|(manifest, _)| manifest)
    }

    pub(super) fn signature_manifest_with_size(
        &self,
    ) -> io::Result<(std::collections::BTreeMap<String, String>, u64)> {
        use sha2::Digest;
        let limits = ScanLimits {
            max_files: 1_000,
            max_depth: 20,
            max_file_size: 16 * 1024 * 1024,
        };
        let mut remaining = 64 * 1024 * 1024;
        let mut manifest = std::collections::BTreeMap::new();
        for path in self.paths(&limits, |path| path != Path::new(".schemapin.sig"))? {
            let name = path
                .components()
                .map(|component| {
                    component
                        .as_os_str()
                        .to_str()
                        .filter(|s| !s.contains('\\'))
                        .ok_or_else(|| invalid("signed skill paths must be portable UTF-8"))
                })
                .collect::<io::Result<Vec<_>>>()?
                .join("/");
            let bytes = self.read_bytes(&path, limits.max_file_size.min(remaining))?;
            remaining -= bytes.len() as u64;
            manifest.insert(
                name,
                format!("sha256:{}", hex::encode(sha2::Sha256::digest(bytes))),
            );
        }
        Ok((manifest, 64 * 1024 * 1024 - remaining))
    }

    pub(super) fn write_signature(&self, bytes: &[u8]) -> io::Result<()> {
        use std::io::Write;
        let temporary = format!(".schemapin.sig.tmp-{}", uuid::Uuid::new_v4());
        let result = (|| {
            let mut file = self
                .0
                .open_with(&temporary, OpenOptions::new().write(true).create_new(true))?;
            file.write_all(bytes)?;
            file.sync_all()?;
            self.0.rename(&temporary, &self.0, ".schemapin.sig")
        })();
        if result.is_err() {
            let _ = self.0.remove_file(&temporary);
        }
        result
    }

    fn snapshot(&self, limits: &ScanLimits, total_limit: u64) -> io::Result<tempfile::TempDir> {
        // The verifier only receives this private, bounded tree. Never give its
        // path-based canonicalizer access to the mutable source directory.
        let paths = self.paths(limits, |_| true)?;
        let snapshot = tempfile::Builder::new()
            .prefix("skill-verification-")
            .tempdir()?;
        let mut remaining = total_limit;
        for path in paths {
            let bytes = self.read_bytes(&path, limits.max_file_size.min(remaining))?;
            remaining -= bytes.len() as u64;
            let destination = snapshot.path().join(&path);
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)?;
            }
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(destination)?;
            file.write_all(&bytes)?;
        }
        Ok(snapshot)
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn walk(
    directory: &Dir,
    relative: &Path,
    depth: usize,
    limits: &ScanLimits,
    include: fn(&Path) -> bool,
    entries_left: &mut usize,
    files: &mut Vec<PathBuf>,
) -> io::Result<()> {
    for entry in directory.entries()? {
        let entry = entry?;
        *entries_left = entries_left
            .checked_sub(1)
            .ok_or_else(|| invalid("skill discovery entry limit exceeded"))?;
        let name = entry.file_name();
        let path = relative.join(&name);
        let kind = entry.file_type()?;
        if kind.is_dir() {
            if depth >= limits.max_depth {
                return Err(invalid("skill directory depth limit exceeded"));
            }
            let child = directory.open_dir_nofollow(&name)?;
            walk(
                &child,
                &path,
                depth + 1,
                limits,
                include,
                entries_left,
                files,
            )?;
        } else if !kind.is_file() {
            return Err(invalid(
                "skill directories cannot contain links or special files",
            ));
        } else if include(&path) {
            if files.len() >= limits.max_files {
                return Err(invalid("skill file count limit exceeded"));
            }
            files.push(path);
        }
    }
    Ok(())
}

fn text_file(path: &Path) -> bool {
    if path
        .file_name()
        .is_some_and(|name| name == ".schemapin.sig")
    {
        return false;
    }
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    [
        "md", "txt", "yaml", "yml", "json", "toml", "sh", "bash", "py", "js", "ts", "rs", "go",
        "rb", "conf", "cfg", "ini", "",
    ]
    .contains(&extension)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn system_parent_aliases_are_supported_but_skill_root_links_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real");
        std::fs::create_dir_all(real.join("skill")).unwrap();
        std::fs::write(real.join("skill/SKILL.md"), "safe").unwrap();
        std::os::unix::fs::symlink(&real, temp.path().join("parent-alias")).unwrap();
        let directory = SkillDirectory::open(&temp.path().join("parent-alias/skill")).unwrap();
        assert_eq!(
            directory.read_text(Path::new("SKILL.md"), 64).unwrap(),
            "safe"
        );
        std::os::unix::fs::symlink(real.join("skill"), temp.path().join("skill-alias")).unwrap();
        assert!(SkillDirectory::open(&temp.path().join("skill-alias")).is_err());
    }

    #[test]
    fn snapshot_bounds_binary_files_count_depth_and_combined_bytes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("assets")).unwrap();
        std::fs::write(dir.path().join("SKILL.md"), b"safe").unwrap();
        std::fs::write(dir.path().join("assets/data.bin"), [0, 255, 0, 255]).unwrap();
        let directory = SkillDirectory::open(dir.path()).unwrap();
        let limits = ScanLimits {
            max_files: 2,
            max_depth: 1,
            max_file_size: 4,
        };
        let snapshot = directory.snapshot(&limits, 8).unwrap();
        assert_eq!(
            schemapin::skill::canonicalize_skill(dir.path()).unwrap(),
            schemapin::skill::canonicalize_skill(snapshot.path()).unwrap()
        );
        assert!(directory.snapshot(&limits, 7).is_err());
        assert!(directory
            .snapshot(
                &ScanLimits {
                    max_file_size: 3,
                    ..limits.clone()
                },
                8
            )
            .is_err());
        assert!(directory
            .snapshot(
                &ScanLimits {
                    max_files: 1,
                    ..limits.clone()
                },
                8
            )
            .is_err());
        assert!(directory
            .snapshot(
                &ScanLimits {
                    max_depth: 0,
                    ..limits
                },
                8
            )
            .is_err());
        std::fs::write(dir.path().join("SKILL.md"), "changed source").unwrap();
        assert_eq!(
            std::fs::read(snapshot.path().join("SKILL.md")).unwrap(),
            b"safe"
        );
    }

    #[test]
    #[cfg(unix)]
    fn snapshot_refuses_linked_binary_content_and_retains_the_original_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("skill");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("SKILL.md"), "safe").unwrap();
        let directory = SkillDirectory::open(&root).unwrap();
        let outside = dir.path().join("outside.bin");
        std::fs::write(&outside, [0, 255]).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("linked.bin")).unwrap();
        assert!(directory.verification_snapshot().is_err());
        std::fs::remove_file(root.join("linked.bin")).unwrap();
        std::fs::hard_link(&outside, root.join("linked.bin")).unwrap();
        assert!(directory.verification_snapshot().is_err());
        std::fs::remove_file(root.join("linked.bin")).unwrap();
        std::fs::rename(&root, dir.path().join("original")).unwrap();
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("SKILL.md"), "replacement").unwrap();
        let snapshot = directory.verification_snapshot().unwrap();
        assert_eq!(
            std::fs::read(snapshot.path().join("SKILL.md")).unwrap(),
            b"safe"
        );
    }
}
