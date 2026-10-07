//! Per-operation file capabilities. Host mounts are ceilings, never implicit grants.

use super::command::{CommandBoundary, CommandTier};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf};

#[cfg(target_os = "linux")]
mod publication;

/// Exact broker-owned candidate recorded before its non-replacing publication.
/// The parent and candidate identities permit recovery without guessing from a
/// pathname or trusting a worker's account of whether publication occurred.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationIntent {
    pub publication_id: uuid::Uuid,
    pub path: String,
    pub parent_path: PathBuf,
    pub parent_identity: (u64, u64),
    pub candidate_name: String,
    pub candidate_identity: (u64, u64),
    pub target_name: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicationState {
    ReadyToPublish,
    Published,
    Missing,
    Conflict,
}

impl PublicationIntent {
    pub fn inspect(&self) -> Result<PublicationState, String> {
        #[cfg(target_os = "linux")]
        return publication::inspect(self).map_err(|error| error.to_string());
        #[cfg(not(target_os = "linux"))]
        Err("file publication inspection requires Linux".into())
    }

    pub(crate) fn recover(&self) -> Result<serde_json::Value, String> {
        #[cfg(target_os = "linux")]
        return publication::recover(self).map_err(|error| error.to_string());
        #[cfg(not(target_os = "linux"))]
        Err("file publication recovery requires Linux".into())
    }
}

const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_INPUT_BYTES: usize = 32 * 1024 * 1024;

/// Relative worker paths, either literal or an entire `{argument}` reference.
/// Creation never overwrites an existing host entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FileAccess {
    pub read: Vec<String>,
    pub create: Vec<String>,
    pub max_file_bytes: u64,
}

impl Default for FileAccess {
    fn default() -> Self {
        Self {
            read: Vec::new(),
            create: Vec::new(),
            max_file_bytes: 8 * 1024 * 1024,
        }
    }
}

impl FileAccess {
    pub fn validate(&self) -> Result<(), String> {
        if self.read.len() > 32
            || self.create.len() > 1
            || self.max_file_bytes == 0
            || self.max_file_bytes > MAX_FILE_BYTES
        {
            return Err("filesystem permits at most 32 inputs, one new output and 1–16777216 bytes per file".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct Input {
    path: String,
    bytes: std::sync::Arc<[u8]>,
    sha256: String,
}

#[derive(Debug, Default, Clone)]
pub struct FileAccessPlan {
    inputs: Vec<Input>,
    #[cfg(target_os = "linux")]
    output: Option<std::sync::Arc<linux::Output>>,
    max_file_bytes: u64,
    journal: Option<crate::reasoning::effect_journal::EffectJournal>,
}

impl FileAccessPlan {
    pub(crate) fn check_publication_authority(&self) -> Result<(), String> {
        if self.has_output() {
            self.journal
                .as_ref()
                .ok_or("file publication requires an active audited dispatcher")?
                .check_live()?;
        }
        Ok(())
    }

    pub(crate) fn effect_journal(&self) -> Option<crate::reasoning::effect_journal::EffectJournal> {
        self.journal.clone()
    }
    pub(crate) fn with_journal(
        &self,
        journal: Option<crate::reasoning::effect_journal::EffectJournal>,
    ) -> Self {
        Self {
            journal,
            ..self.clone()
        }
    }
    pub fn prepare(
        boundary: &CommandBoundary,
        access: Option<&FileAccess>,
        args: &HashMap<String, String>,
    ) -> Result<Self, String> {
        boundary.validate()?;
        let Some(access) = access else {
            return Ok(Self::default());
        };
        access.validate()?;
        if !matches!(
            boundary.tier,
            CommandTier::Docker
                | CommandTier::GVisor
                | CommandTier::Firecracker
                | CommandTier::Landlock
        ) {
            return Err("filesystem grants require a supported Linux file broker".into());
        }
        #[cfg(target_os = "linux")]
        {
            linux::prepare(boundary, access, args).map_err(|e| e.to_string())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = args;
            Err("filesystem grants require the Linux file broker".into())
        }
    }

    pub fn descriptor(&self) -> serde_json::Value {
        #[cfg(target_os = "linux")]
        let output: Vec<_> = self.output.iter().map(|o| o.path.as_str()).collect();
        #[cfg(not(target_os = "linux"))]
        let output: Vec<&str> = Vec::new();
        #[cfg(target_os = "linux")]
        let output_parent = self.output.as_ref().map(|o| o.parent_identity);
        #[cfg(not(target_os = "linux"))]
        let output_parent: Option<(u64, u64)> = None;
        serde_json::json!({
            "read": self.inputs.iter().map(|i| serde_json::json!({"path":i.path,"bytes":i.bytes.len(),"sha256":i.sha256})).collect::<Vec<_>>(),
            "create": output, "max_file_bytes": self.max_file_bytes,
            "input_snapshots": true, "overwrite": false,
            "output_parent_identity": output_parent,
        })
    }

    pub fn has_output(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            self.output.is_some()
        }
        #[cfg(not(target_os = "linux"))]
        {
            false
        }
    }

    pub fn stage(&self, boundary: &CommandBoundary) -> Result<StagedFiles, String> {
        let boundary = boundary.without_host_mounts();
        #[cfg(target_os = "linux")]
        {
            linux::stage(self, boundary).map_err(|e| e.to_string())
        }
        #[cfg(not(target_os = "linux"))]
        {
            Ok(StagedFiles { boundary })
        }
    }
}

pub struct StagedFiles {
    pub boundary: CommandBoundary,
    #[cfg(target_os = "linux")]
    directory: Option<std::sync::Arc<symbi_sandbox_supervisor::staging::Lease>>,
    #[cfg(target_os = "linux")]
    output: Option<std::sync::Arc<linux::Output>>,
    #[cfg(target_os = "linux")]
    max_file_bytes: u64,
    #[cfg(target_os = "linux")]
    journal: Option<crate::reasoning::effect_journal::EffectJournal>,
}

impl StagedFiles {
    /// Governed routes require a durable intent before publication, and a
    /// confirmed finish before returning a successful tool result.
    pub(crate) async fn publish_recorded(&self) -> Result<serde_json::Value, String> {
        #[cfg(target_os = "linux")]
        return self.publish_using(self.journal.as_ref()).await;
        #[cfg(not(target_os = "linux"))]
        Ok(serde_json::json!([]))
    }

    pub(crate) async fn publish_using(
        &self,
        journal: Option<&crate::reasoning::effect_journal::EffectJournal>,
    ) -> Result<serde_json::Value, String> {
        #[cfg(target_os = "linux")]
        if let Some(output) = &self.output {
            if let Some(files) = self
                .boundary
                .firecracker
                .as_ref()
                .and_then(|vm| vm.files.as_ref())
            {
                files.check_publication()?;
            }
            let journal =
                journal.ok_or("file publication requires an active audited dispatcher")?;
            journal.check_live()?;
            let mut candidate = publication::Candidate::prepare(
                self.directory
                    .as_ref()
                    .ok_or("missing staged output directory")?
                    .path()
                    .join("output"),
                output.clone(),
                self.max_file_bytes,
            )
            .map_err(|e| e.to_string())?;
            // An acknowledgement can be lost after the signed intent is saved.
            // Retain this exact candidate for inspection on every uncertain exit.
            candidate.retain();
            use crate::reasoning::effect_journal::ToolEffect;
            journal
                .append(ToolEffect::FilePublicationPrepared {
                    intent: candidate.intent.clone(),
                })
                .await?;
            journal.check_live()?;
            let result = candidate
                .commit()
                .map_err(|e| format!("filesystem output publication failed: {e}"));
            journal
                .append(ToolEffect::FilePublicationFinished {
                    publication_id: candidate.intent.publication_id,
                    confirmed: result.is_ok(),
                    error: result.as_ref().err().cloned(),
                })
                .await?;
            return result;
        }
        #[cfg(not(target_os = "linux"))]
        let _ = journal;
        Ok(serde_json::json!([]))
    }
    /// Trusted low-level publication primitive. Governed worker routes use
    /// `publish_recorded` and its live dispatch authority instead. Call only
    /// after successful operation, required parsing and confirmed worker cleanup.
    pub fn publish(&self) -> Result<serde_json::Value, String> {
        #[cfg(target_os = "linux")]
        if let Some(output) = &self.output {
            if let Some(files) = self
                .boundary
                .firecracker
                .as_ref()
                .and_then(|vm| vm.files.as_ref())
            {
                files.check_publication()?;
            }
            return linux::publish(
                self.directory
                    .as_ref()
                    .expect("staged output directory")
                    .path()
                    .join("output"),
                output,
                self.max_file_bytes,
            )
            .map_err(|e| format!("filesystem output publication failed: {e}"));
        }
        Ok(serde_json::json!([]))
    }
}

pub(super) fn resolve(value: &str, args: &HashMap<String, String>) -> anyhow::Result<String> {
    let path = if let Some(name) = value.strip_prefix('{').and_then(|v| v.strip_suffix('}')) {
        args.get(name)
            .ok_or_else(|| anyhow::anyhow!("missing filesystem argument {name}"))?
            .as_str()
    } else {
        value
    };
    if path.is_empty()
        || path.len() > 4096
        || path.contains(['{', '}', '\\', ':', ',', '\0', '\n', '\r'])
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        anyhow::bail!("filesystem grants require normalized relative file paths");
    }
    Ok(path.to_owned())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn fixture() -> (tempfile::TempDir, CommandBoundary) {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("source")).unwrap();
        std::fs::write(
            root.path().join("source").join("input.txt"),
            "permitted input",
        )
        .unwrap();
        std::fs::write(root.path().join("source").join("adjacent.txt"), "ungranted").unwrap();
        let mut boundary = CommandBoundary::default();
        boundary.docker.supervisor.state_dir = root.path().join("leases");
        boundary.docker.volumes = vec![format!(
            "{}:/workspace:rw",
            root.path().join("source").display()
        )];
        (root, boundary)
    }

    #[test]
    fn snapshots_are_immutable_and_only_declared_files_are_staged() {
        let (root, boundary) = fixture();
        let access = FileAccess {
            read: vec!["{input}".into()],
            create: vec!["result.txt".into()],
            ..Default::default()
        };
        let args = HashMap::from([("input".into(), "input.txt".into())]);
        let plan = FileAccessPlan::prepare(&boundary, Some(&access), &args).unwrap();
        let descriptor = plan.descriptor();
        std::fs::write(
            root.path().join("source").join("input.txt"),
            "changed later",
        )
        .unwrap();
        let staged = plan.stage(&boundary).unwrap();
        assert_eq!(staged.boundary.docker.volumes.len(), 2);
        let directory = staged.directory.as_ref().unwrap().path();
        assert_eq!(
            std::fs::read_to_string(directory.join("input-0")).unwrap(),
            "permitted input"
        );
        assert_eq!(
            std::fs::metadata(directory.join("input-0"))
                .unwrap()
                .permissions()
                .mode()
                & 0o222,
            0
        );
        assert_eq!(descriptor, plan.descriptor());
        assert!(!root.path().join("source").join("result.txt").exists());
        std::fs::write(directory.join("output"), "useful result").unwrap();
        let receipt = staged.publish().unwrap();
        assert_eq!(receipt[0]["path"], "result.txt");
        assert_eq!(
            std::fs::read_to_string(root.path().join("source").join("result.txt")).unwrap(),
            "useful result"
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("source").join("adjacent.txt")).unwrap(),
            "ungranted"
        );
        assert!(FileAccessPlan::prepare(&boundary, None, &args)
            .unwrap()
            .stage(&boundary)
            .unwrap()
            .boundary
            .docker
            .volumes
            .is_empty());
    }

    #[test]
    fn links_directories_traversal_and_oversized_files_cannot_be_granted() {
        let (root, boundary) = fixture();
        symlink("input.txt", root.path().join("source").join("link.txt")).unwrap();
        std::fs::create_dir(root.path().join("source").join("directory")).unwrap();
        symlink(root.path(), root.path().join("source").join("linked-dir")).unwrap();
        let fifo =
            std::ffi::CString::new(root.path().join("source").join("pipe").to_str().unwrap())
                .unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        for path in [
            "../input.txt",
            "/input.txt",
            "./input.txt",
            "directory/../input.txt",
            "directory",
            "link.txt",
            "linked-dir/input.txt",
            "pipe",
        ] {
            let access = FileAccess {
                read: vec![path.into()],
                ..Default::default()
            };
            assert!(
                FileAccessPlan::prepare(&boundary, Some(&access), &HashMap::new()).is_err(),
                "accepted {path}"
            );
        }
        let access = FileAccess {
            read: vec!["input.txt".into()],
            max_file_bytes: 2,
            ..Default::default()
        };
        assert!(FileAccessPlan::prepare(&boundary, Some(&access), &HashMap::new()).is_err());
        std::fs::hard_link(
            root.path().join("source").join("input.txt"),
            root.path().join("source").join("hard.txt"),
        )
        .unwrap();
        let access = FileAccess {
            read: vec!["input.txt".into()],
            ..Default::default()
        };
        assert!(FileAccessPlan::prepare(&boundary, Some(&access), &HashMap::new()).is_err());
    }

    #[test]
    fn publication_never_overwrites_a_competing_output_or_symlink() {
        let (root, boundary) = fixture();
        let access = FileAccess {
            create: vec!["result.txt".into()],
            ..Default::default()
        };
        let first = FileAccessPlan::prepare(&boundary, Some(&access), &HashMap::new())
            .unwrap()
            .stage(&boundary)
            .unwrap();
        let second = FileAccessPlan::prepare(&boundary, Some(&access), &HashMap::new())
            .unwrap()
            .stage(&boundary)
            .unwrap();
        std::fs::write(
            first.directory.as_ref().unwrap().path().join("output"),
            "first",
        )
        .unwrap();
        std::fs::write(
            second.directory.as_ref().unwrap().path().join("output"),
            "second",
        )
        .unwrap();
        first.publish().unwrap();
        assert!(second.publish().is_err());
        assert_eq!(
            std::fs::read_to_string(root.path().join("source").join("result.txt")).unwrap(),
            "first"
        );
        std::fs::remove_file(root.path().join("source").join("result.txt")).unwrap();
        symlink(
            "adjacent.txt",
            root.path().join("source").join("result.txt"),
        )
        .unwrap();
        assert!(second.publish().is_err());
        assert_eq!(
            std::fs::read_to_string(root.path().join("source").join("adjacent.txt")).unwrap(),
            "ungranted"
        );
        let mut readonly = boundary.clone();
        readonly.docker.volumes[0] =
            format!("{}:/workspace:ro", root.path().join("source").display());
        assert!(FileAccessPlan::prepare(&readonly, Some(&access), &HashMap::new()).is_err());
    }
}

#[cfg(target_os = "linux")]
pub(super) mod linux {
    use super::*;
    use anyhow::Context;
    use sha2::{Digest, Sha256};
    use std::{
        ffi::CString,
        fs::{File, OpenOptions},
        io::{Read, Write},
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        },
        path::Path,
        sync::Arc,
    };

    #[derive(Debug)]
    pub(super) struct Output {
        pub path: String,
        pub parent_identity: (u64, u64),
        pub(super) parent_path: PathBuf,
        pub(super) parent: File,
        pub(super) name: CString,
    }

    pub(in crate::sandbox) fn open_at(
        parent: &File,
        name: &str,
        flags: i32,
        mode: u32,
    ) -> anyhow::Result<File> {
        let name = CString::new(name)?;
        // SAFETY: valid descriptor and nul-terminated name; returned fd is owned.
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
                mode,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    pub(in crate::sandbox) fn directory(path: &Path) -> anyhow::Result<File> {
        anyhow::ensure!(path.is_absolute(), "host grant root must be absolute");
        let mut dir = File::open("/")?;
        for part in path.components() {
            match part {
                std::path::Component::RootDir => {}
                std::path::Component::Normal(name) => {
                    dir = open_at(
                        &dir,
                        name.to_str().context("non-UTF8 grant root")?,
                        libc::O_RDONLY | libc::O_DIRECTORY,
                        0,
                    )?
                }
                _ => anyhow::bail!("invalid host grant root"),
            }
        }
        Ok(dir)
    }

    /// The selected backend supplies its own file ceiling. VM broker roots are
    /// host authority only and never become Docker mounts or guest devices.
    pub(in crate::sandbox) fn ceiling(
        boundary: &CommandBoundary,
        create: bool,
    ) -> anyhow::Result<(&str, &[String])> {
        match boundary.tier {
            CommandTier::Landlock => {
                let roots = if create {
                    &boundary.roots.output_roots
                } else {
                    &boundary.roots.source_roots
                };
                anyhow::ensure!(roots.len() <= 32, "too many Landlock file ceilings");
                let mut destinations = std::collections::HashSet::new();
                for root in roots {
                    super::super::docker::canonical_mount(root)?;
                    let parts: Vec<_> = root.split(':').collect();
                    anyhow::ensure!(parts.get(2).copied().unwrap_or("ro") == if create { "rw" } else { "ro" }, "Landlock source ceilings must be read-only and output ceilings explicitly writable");
                    anyhow::ensure!(
                        destinations.insert(Path::new(parts[1]).components().collect::<PathBuf>()),
                        "duplicate Landlock file destination"
                    );
                }
                Ok(("/workspace", roots))
            }
            CommandTier::Docker => Ok((&boundary.docker.working_dir, &boundary.docker.volumes)),
            CommandTier::GVisor => Ok((
                &boundary.gvisor.docker.working_dir,
                &boundary.gvisor.docker.volumes,
            )),
            CommandTier::Firecracker => {
                let config = boundary
                    .firecracker
                    .as_ref()
                    .context("missing Firecracker configuration")?;
                Ok((
                    &config.working_dir,
                    if create {
                        &config.output_roots
                    } else {
                        &config.source_roots
                    },
                ))
            }
            _ => anyhow::bail!("selected backend has no supported file ceiling"),
        }
    }

    pub(in crate::sandbox) fn host_path(
        boundary: &CommandBoundary,
        path: &str,
        create: bool,
    ) -> anyhow::Result<PathBuf> {
        let (working_dir, roots) = ceiling(boundary, create)?;
        let guest = Path::new(working_dir).join(path);
        let mount = roots
            .iter()
            .filter_map(|v| {
                let parts: Vec<_> = v.split(':').collect();
                let destination = Path::new(*parts.get(1)?);
                guest
                    .strip_prefix(destination)
                    .ok()
                    .map(|relative| (parts, relative.to_owned(), destination.components().count()))
            })
            .max_by_key(|(_, _, depth)| *depth)
            .context("filesystem path is outside configured mount ceilings")?;
        if create && mount.0.get(2).copied() != Some("rw") {
            anyhow::bail!("filesystem creation requires a writable mount ceiling");
        }
        let source = Path::new(mount.0[0]);
        let host = source.join(&mount.1);
        // Recheck project protection before opening any input, including sources
        // changed after the executor captured its configuration.
        boundary.validate().map_err(anyhow::Error::msg)?;
        Ok(host)
    }

    fn target(
        boundary: &CommandBoundary,
        path: &str,
        create: bool,
    ) -> anyhow::Result<(File, String)> {
        let host = host_path(boundary, path, create)?;
        let (parent, name) = (
            host.parent().context("missing host file parent")?,
            host.file_name()
                .and_then(|v| v.to_str())
                .context("missing host file name")?,
        );
        Ok((directory(parent)?, name.to_owned()))
    }

    fn absent(parent: &File, name: &CString) -> anyhow::Result<()> {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: output points to writable storage; no symlink is followed.
        if unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } == 0
        {
            anyhow::bail!("filesystem output already exists; overwriting is forbidden");
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::NotFound {
            return Err(error.into());
        }
        Ok(())
    }

    pub(super) fn prepare(
        boundary: &CommandBoundary,
        access: &FileAccess,
        args: &HashMap<String, String>,
    ) -> anyhow::Result<FileAccessPlan> {
        let mut plan = FileAccessPlan {
            max_file_bytes: access.max_file_bytes,
            ..Default::default()
        };
        let mut total = 0usize;
        let mut paths = std::collections::HashSet::new();
        for value in &access.read {
            let path = resolve(value, args)?;
            anyhow::ensure!(paths.insert(path.clone()), "duplicate filesystem grant");
            let (parent, name) = target(boundary, &path, false)?;
            let file =
                open_at(&parent, &name, libc::O_RDONLY, 0).context("cannot open declared input")?;
            let metadata = file.metadata()?;
            anyhow::ensure!(
                metadata.is_file()
                    && metadata.nlink() == 1
                    && metadata.len() <= access.max_file_bytes,
                "input must be a bounded regular file without hard links"
            );
            let mut bytes = Vec::new();
            file.take(access.max_file_bytes + 1)
                .read_to_end(&mut bytes)?;
            total = total
                .checked_add(bytes.len())
                .context("input size overflow")?;
            anyhow::ensure!(
                bytes.len() as u64 <= access.max_file_bytes && total <= MAX_INPUT_BYTES,
                "filesystem input snapshot exceeds its byte budget"
            );
            let sha256 = format!("{:x}", Sha256::digest(&bytes));
            plan.inputs.push(Input {
                path,
                bytes: bytes.into(),
                sha256,
            });
        }
        if let Some(value) = access.create.first() {
            let path = resolve(value, args)?;
            anyhow::ensure!(paths.insert(path.clone()), "duplicate filesystem grant");
            let (parent, name) = target(boundary, &path, true)?;
            let name = CString::new(name)?;
            absent(&parent, &name)?;
            let metadata = parent.metadata()?;
            let parent_identity = (metadata.dev(), metadata.ino());
            plan.output = Some(Arc::new(Output {
                path,
                parent,
                name,
                parent_identity,
                parent_path: host_path(boundary, &resolve(value, args)?, true)?
                    .parent()
                    .context("missing output parent")?
                    .to_owned(),
            }));
        }
        Ok(plan)
    }

    pub(super) fn stage(
        plan: &FileAccessPlan,
        mut boundary: CommandBoundary,
    ) -> anyhow::Result<StagedFiles> {
        if plan.inputs.is_empty() && plan.output.is_none() {
            return Ok(StagedFiles {
                boundary,
                directory: None,
                output: None,
                max_file_bytes: plan.max_file_bytes,
                journal: plan.journal.clone(),
            });
        }
        let (state_dir, working_dir) = match boundary.tier {
            CommandTier::Docker => (
                boundary.docker.supervisor.resolved_state_dir()?,
                boundary.docker.working_dir.clone(),
            ),
            CommandTier::GVisor => (
                boundary.gvisor.docker.supervisor.resolved_state_dir()?,
                boundary.gvisor.docker.working_dir.clone(),
            ),
            CommandTier::Firecracker => {
                let config = boundary
                    .firecracker
                    .as_ref()
                    .context("missing VM configuration")?;
                (
                    config.supervisor.resolved_state_dir()?,
                    config.working_dir.clone(),
                )
            }
            CommandTier::Landlock => (
                boundary.landlock.supervisor.resolved_state_dir()?,
                super::super::landlock::workspace::WORKSPACE.into(),
            ),
            _ => anyhow::bail!("file staging transport unavailable"),
        };
        let reserved_bytes = plan
            .inputs
            .iter()
            .map(|i| i.bytes.len() as u64)
            .sum::<u64>()
            + if plan.output.is_some() {
                plan.max_file_bytes
            } else {
                0
            }
            + (plan.inputs.len() as u64 + 4) * 4096
            + if boundary.tier == CommandTier::Landlock {
                super::super::landlock::workspace::SPEC_LIMIT
            } else {
                0
            };
        let directory = Arc::new(symbi_sandbox_supervisor::staging::Lease::reserve(
            &state_dir,
            reserved_bytes,
        )?);
        let mut volumes = Vec::new();
        let mut imports = Vec::new();
        let mut sources = Vec::new();
        for (index, input) in plan.inputs.iter().enumerate() {
            let source = directory.path().join(format!("input-{index}"));
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&source)?;
            file.write_all(&input.bytes)?;
            file.set_permissions(std::fs::Permissions::from_mode(0o444))?;
            let target = Path::new(&working_dir)
                .join(&input.path)
                .to_str()
                .context("invalid guest file path")?
                .to_owned();
            volumes.push(format!("{}:{target}:ro", source.display()));
            imports.push(symbi_sandbox_guest::files::Input {
                path: target,
                length: input.bytes.len() as u64,
                sha256: input.sha256.clone(),
            });
            sources.push(source);
        }
        let mut destination = None;
        let mut output_file = None;
        if let Some(output) = &plan.output {
            let source = directory.path().join("output");
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&source)?;
            file.set_permissions(std::fs::Permissions::from_mode(0o666))?;
            let target = Path::new(&working_dir)
                .join(&output.path)
                .to_str()
                .context("invalid guest output path")?
                .to_owned();
            volumes.push(format!("{}:{target}:rw", source.display()));
            destination = Some(target);
            output_file = Some(file);
        }
        match boundary.tier {
            CommandTier::Landlock => {
                use super::super::landlock::workspace::{Mount, Workspace};
                let mounts = sources
                    .into_iter()
                    .zip(imports)
                    .map(|(source, input)| Mount {
                        source,
                        destination: input.path.into(),
                        read_only: true,
                    })
                    .chain(destination.into_iter().map(|target| Mount {
                        source: directory.path().join("output"),
                        destination: target.into(),
                        read_only: false,
                    }))
                    .collect();
                boundary.landlock.workspace = Some(Workspace::new(
                    directory.clone(),
                    mounts,
                    working_dir.into(),
                    plan.max_file_bytes,
                )?);
            }
            CommandTier::Docker | CommandTier::GVisor => {
                let config = if boundary.tier == CommandTier::Docker {
                    &mut boundary.docker
                } else {
                    &mut boundary.gvisor.docker
                };
                config.max_file_bytes = config.max_file_bytes.min(plan.max_file_bytes);
                config.volumes = volumes;
                config.staging = vec![directory.id()];
            }
            CommandTier::Firecracker => {
                let grant = symbi_sandbox_guest::files::Grant {
                    inputs: imports,
                    output: destination,
                    max_file_bytes: plan.max_file_bytes,
                };
                boundary
                    .firecracker
                    .as_mut()
                    .context("missing VM configuration")?
                    .files = Some(Arc::new(super::super::firecracker::files::Transfer::new(
                    grant,
                    sources,
                    output_file,
                    directory.clone(),
                )?));
            }
            _ => unreachable!(),
        }
        boundary.validate().map_err(anyhow::Error::msg)?;
        Ok(StagedFiles {
            boundary,
            directory: Some(directory),
            output: plan.output.clone(),
            max_file_bytes: plan.max_file_bytes,
            journal: plan.journal.clone(),
        })
    }

    pub(super) fn publish(
        source: PathBuf,
        output: &Output,
        limit: u64,
    ) -> anyhow::Result<serde_json::Value> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
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
        absent(&output.parent, &output.name)?;
        let temporary = format!(".symbi-publish-{}", uuid::Uuid::new_v4());
        let temporary_name = CString::new(temporary.as_str())?;
        let mut candidate = open_at(
            &output.parent,
            &temporary,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
            0o600,
        )?;
        let result = (|| -> anyhow::Result<()> {
            candidate.write_all(&bytes)?;
            candidate.sync_all()?;
            // SAFETY: pinned parent and valid names. RENAME_NOREPLACE atomically
            // rejects a competing file, symlink or directory without overwriting.
            if unsafe {
                crate::sandbox::rename_noreplace_at(
                    output.parent.as_raw_fd(),
                    temporary_name.as_ptr(),
                    output.parent.as_raw_fd(),
                    output.name.as_ptr(),
                )
            } != 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            output
                .parent
                .sync_all()
                .context("output created but directory durability is uncertain")?;
            Ok(())
        })();
        // SAFETY: remove only the unique broker-owned candidate, never the target.
        unsafe {
            libc::unlinkat(output.parent.as_raw_fd(), temporary_name.as_ptr(), 0);
        }
        result?;
        Ok(
            serde_json::json!([{"path":output.path,"bytes":bytes.len(),"sha256":format!("{:x}", Sha256::digest(&bytes))}]),
        )
    }
}
