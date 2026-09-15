//! Bounded project-file reads anchored to a trusted directory descriptor.
use std::path::{Component, Path, PathBuf};

pub struct ProjectReader {
    path: PathBuf,
    #[cfg(unix)]
    root: std::fs::File,
}

impl ProjectReader {
    pub fn open(path: &Path) -> Result<Self, String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let path = path.canonicalize().map_err(|error| error.to_string())?;
            let root = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&path)
                .map_err(|error| error.to_string())?;
            Ok(Self { path, root })
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Err("confined project reads require Unix".into())
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Resolve every component under the pinned root without following links.
    /// Private runtime/configuration paths and hardlinked files are unavailable.
    pub fn read_text(&self, path: &Path, limit: usize) -> Result<String, String> {
        let components = path
            .components()
            .filter(|component| *component != Component::CurDir)
            .map(|component| match component {
                Component::Normal(name) => Ok(name),
                _ => Err("project file path requires ordinary relative components".to_string()),
            })
            .collect::<Result<Vec<_>, _>>()?;
        if components.is_empty() || components.len() > 64 || limit > 1024 * 1024 {
            return Err("project read exceeds its path or byte bound".into());
        }
        if components
            .iter()
            .any(|name| name.to_string_lossy().starts_with('.'))
            || components[0] == "policies"
        {
            return Err("protected project paths are unavailable".into());
        }
        #[cfg(unix)]
        {
            use std::{
                ffi::CString,
                io::Read,
                os::{
                    fd::{AsRawFd, FromRawFd},
                    unix::{ffi::OsStrExt, fs::MetadataExt},
                },
            };
            let mut parent = self.root.try_clone().map_err(|error| error.to_string())?;
            for (index, component) in components.iter().enumerate() {
                let name =
                    CString::new(component.as_bytes()).map_err(|_| "invalid project path")?;
                let last = index + 1 == components.len();
                let flags = libc::O_RDONLY
                    | libc::O_NOFOLLOW
                    | libc::O_CLOEXEC
                    | libc::O_NONBLOCK
                    | if last { 0 } else { libc::O_DIRECTORY };
                // Parent is an owned directory descriptor; the checked single
                // component contains no separators or NUL. Never follow links.
                let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
                if fd < 0 {
                    return Err(std::io::Error::last_os_error().to_string());
                }
                // openat returned a fresh descriptor owned by this File.
                parent = unsafe { std::fs::File::from_raw_fd(fd) };
            }
            let metadata = parent.metadata().map_err(|error| error.to_string())?;
            if !metadata.is_file() || metadata.nlink() != 1 {
                return Err("project reads require a regular file with one link".into());
            }
            let mut bytes = Vec::new();
            parent
                .take(limit as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;
            if bytes.len() > limit {
                return Err(format!("project file exceeds {limit} bytes"));
            }
            String::from_utf8(bytes).map_err(|_| "project file must contain UTF-8 text".into())
        }
        #[cfg(not(unix))]
        {
            Err("confined project reads require Unix".into())
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn pinned_reads_reject_escape_aliases_private_paths_and_oversized_files() {
        let project = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("agent.symbi"), "agent a {}").unwrap();
        std::fs::write(outside.path().join("private.symbi"), "outside").unwrap();
        let reader = ProjectReader::open(project.path()).unwrap();
        assert_eq!(
            reader.read_text(Path::new("agent.symbi"), 100).unwrap(),
            "agent a {}"
        );
        assert_eq!(
            reader.read_text(Path::new("./agent.symbi"), 100).unwrap(),
            "agent a {}"
        );
        assert!(reader.read_text(Path::new("agent.symbi"), 2).is_err());
        std::os::unix::fs::symlink(
            outside.path().join("private.symbi"),
            project.path().join("link.symbi"),
        )
        .unwrap();
        std::os::unix::fs::symlink(outside.path(), project.path().join("directory")).unwrap();
        std::fs::hard_link(
            outside.path().join("private.symbi"),
            project.path().join("hard.symbi"),
        )
        .unwrap();
        for path in [
            "link.symbi",
            "directory/private.symbi",
            "hard.symbi",
            "../private.symbi",
            "/private.symbi",
            ".symbiont/governed/agent.symbi",
            "policies/rule.symbi",
        ] {
            assert!(reader.read_text(Path::new(path), 100).is_err(), "{path}");
        }
        // Replace a traversed directory after the reader was created.
        std::fs::create_dir(project.path().join("agents")).unwrap();
        std::fs::write(project.path().join("agents/a.symbi"), "local").unwrap();
        assert_eq!(
            reader.read_text(Path::new("agents/a.symbi"), 100).unwrap(),
            "local"
        );
        std::fs::rename(
            project.path().join("agents"),
            project.path().join("old-agents"),
        )
        .unwrap();
        std::os::unix::fs::symlink(outside.path(), project.path().join("agents")).unwrap();
        assert!(reader
            .read_text(Path::new("agents/private.symbi"), 100)
            .is_err());
    }
}
