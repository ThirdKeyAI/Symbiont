//! Evidence capture for `[tool.evidence]` (ToolClad TC-19 design).
//!
//! Tools run inside an isolated boundary, so the evidence a call produces is
//! what the runtime captured: its stdout (or HTTP response body). When a
//! manifest declares `[tool.evidence] capture = true` that output is written
//! under a per-call directory below the evidence root:
//!
//! * the root is `TOOLCLAD_EVIDENCE_DIR`, or `symbiont-evidence-<uid>` in the
//!   system temp directory, created mode 0700 and refused when it is a
//!   symlink, not owned by the runtime user, or group/other accessible;
//! * the per-call directory is created mode 0700 and must not already exist;
//! * the output file is opened `O_CREAT | O_EXCL | O_NOFOLLOW` mode 0600, so
//!   a pre-planted symlink cannot redirect the write;
//! * the scan identifier carries 128 bits of entropy.

use std::path::{Component, Path, PathBuf};

/// Environment variable naming the evidence root.
pub const EVIDENCE_DIR_ENV: &str = "TOOLCLAD_EVIDENCE_DIR";

/// Render `output_dir` (which must start with `{evidence_dir}/` and may use
/// only `{evidence_dir}` and `{scan_id}`) into a path relative to the root.
pub(crate) fn relative_dir(template: &str, scan_id: &str) -> Result<PathBuf, String> {
    let rest = template
        .strip_prefix("{evidence_dir}/")
        .ok_or("tool.evidence.output_dir must start with {evidence_dir}/")?;
    let rendered = rest.replace("{scan_id}", scan_id);
    if rendered.contains(['{', '}']) || rendered.is_empty() {
        return Err(
            "tool.evidence.output_dir may only use the {evidence_dir} and {scan_id} placeholders"
                .into(),
        );
    }
    let path = PathBuf::from(&rendered);
    if path
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("tool.evidence.output_dir must stay below the evidence root".into());
    }
    Ok(path)
}

/// The evidence root, created or verified as a private directory.
fn evidence_root() -> Result<PathBuf, String> {
    let root = match std::env::var_os(EVIDENCE_DIR_ENV).filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => {
            #[cfg(unix)]
            // SAFETY: geteuid has no memory preconditions.
            let uid = unsafe { libc::geteuid() };
            #[cfg(not(unix))]
            let uid = 0;
            std::env::temp_dir().join(format!("symbiont-evidence-{uid}"))
        }
    };
    if !root.is_absolute() {
        return Err(format!("{EVIDENCE_DIR_ENV} must be an absolute path"));
    }
    create_private_dir(&root, true)?;
    Ok(root)
}

fn create_private_dir(path: &Path, existing_ok: bool) -> Result<(), String> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && existing_ok => {
            verify_private_dir(path)
        }
        Err(e) => Err(format!(
            "cannot create evidence directory {}: {e}",
            path.display()
        )),
    }
}

fn verify_private_dir(path: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| format!("cannot inspect evidence directory {}: {e}", path.display()))?;
    if !meta.file_type().is_dir() {
        return Err(format!(
            "evidence root {} is not a directory (symlinks are refused)",
            path.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: geteuid has no memory preconditions.
        let uid = unsafe { libc::geteuid() };
        if meta.uid() != uid || meta.mode() & 0o077 != 0 {
            return Err(format!(
                "evidence root {} must be owned by the runtime user with mode 0700",
                path.display()
            ));
        }
    }
    Ok(())
}

/// A stored evidence record.
#[derive(Debug, Clone)]
pub struct Stored {
    pub dir: PathBuf,
    pub file: PathBuf,
}

/// Store captured output for one call. `None` when the manifest declares no
/// capture.
pub fn store(
    manifest: &super::manifest::Manifest,
    scan_id: &str,
    output: &[u8],
) -> Result<Option<Stored>, String> {
    let Some(evidence) = manifest.tool.evidence.as_ref().filter(|e| e.capture) else {
        return Ok(None);
    };
    let root = evidence_root()?;
    store_under(&root, &evidence.output_dir, scan_id, output).map(Some)
}

fn store_under(
    root: &Path,
    template: &str,
    scan_id: &str,
    output: &[u8],
) -> Result<Stored, String> {
    let relative = relative_dir(template, scan_id)?;
    let mut dir = root.to_path_buf();
    let components: Vec<_> = relative.components().collect();
    for (i, component) in components.iter().enumerate() {
        dir.push(component);
        // Intermediate directories may be shared; the per-call leaf is new.
        create_private_dir(&dir, i + 1 < components.len())?;
    }
    let file = dir.join("output");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut handle = options
        .open(&file)
        .map_err(|e| format!("cannot create evidence file {}: {e}", file.display()))?;
    std::io::Write::write_all(&mut handle, output)
        .map_err(|e| format!("cannot write evidence file {}: {e}", file.display()))?;
    Ok(Stored { dir, file })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_dir_must_stay_below_the_root() {
        assert_eq!(
            relative_dir("{evidence_dir}/{scan_id}-nmap", "1-ab").unwrap(),
            PathBuf::from("1-ab-nmap")
        );
        for bad in [
            "/tmp/x",
            "{evidence_dir}/../x",
            "{evidence_dir}/{other}",
            "{evidence_dir}/",
            "{evidence_dir}//abs",
        ] {
            assert!(relative_dir(bad, "1").is_err(), "{bad}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn evidence_is_private_and_never_follows_symlinks() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let stored = store_under(root.path(), "{evidence_dir}/{scan_id}", "id1", b"out").unwrap();
        assert_eq!(std::fs::read(&stored.file).unwrap(), b"out");
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&stored.dir), 0o700);
        assert_eq!(mode(&stored.file), 0o600);
        // A second call with the same id cannot reuse or clobber the record.
        assert!(store_under(root.path(), "{evidence_dir}/{scan_id}", "id1", b"x").is_err());
        // A pre-planted symlink at the per-call path is refused.
        let target = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(target.path(), root.path().join("id2")).unwrap();
        assert!(store_under(root.path(), "{evidence_dir}/{scan_id}", "id2", b"x").is_err());
        // A group-readable root is refused.
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(verify_private_dir(root.path()).is_err());
    }
}
