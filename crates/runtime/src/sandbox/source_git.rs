//! Bounded repository snapshots for fixed Git queries in an isolated worker.
use super::{
    command::{CommandBoundary, CommandTier},
    files::linux::{ceiling as file_ceiling, directory, host_path, open_at},
    source::SourceOperation,
};
use anyhow::Context;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};

const MAX_BYTES: u64 = 128 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_FILES: usize = 10000;
const MAX_ENTRIES: usize = 20000;
const MAX_PATH_BYTES: usize = 1024 * 1024;
const DRIVER: &str = include_str!("source_git.py");
const WORKTREE: &str = "/symbi-source";
const INPUT: &str = "/symbi-git-input";

pub(super) struct GitPlan {
    directory: Arc<symbi_sandbox_supervisor::staging::Lease>,
    pub boundary: CommandBoundary,
    pub descriptor: Value,
    operation: SourceOperation,
    consumed: AtomicBool,
}
impl GitPlan {
    pub fn prepare(
        ceiling: &CommandBoundary,
        operation: SourceOperation,
        deadline: Instant,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            matches!(
                ceiling.tier,
                CommandTier::Docker
                    | CommandTier::GVisor
                    | CommandTier::Firecracker
                    | CommandTier::Landlock
            ),
            "Git source queries require a supported Linux file ceiling"
        );
        let supervisor = match ceiling.tier {
            CommandTier::Landlock => &ceiling.landlock.supervisor,
            CommandTier::Docker => &ceiling.docker.supervisor,
            CommandTier::GVisor => &ceiling.gvisor.docker.supervisor,
            CommandTier::Firecracker => {
                &ceiling
                    .firecracker
                    .as_ref()
                    .context("missing VM configuration")?
                    .supervisor
            }
            _ => unreachable!(),
        };
        let directory = Arc::new(symbi_sandbox_supervisor::staging::Lease::reserve(
            &supervisor.resolved_state_dir()?,
            MAX_BYTES
                + (MAX_ENTRIES as u64 + 8) * 4096
                + if ceiling.tier == CommandTier::Landlock {
                    super::landlock::workspace::SPEC_LIMIT
                } else {
                    0
                },
        )?);
        let mut snapshot = Snapshot {
            ceiling,
            root: directory.path().to_owned(),
            deadline,
            files: Vec::new(),
            bytes: 0,
            entries: 0,
            path_bytes: 0,
            seen: HashSet::new(),
        };
        for path in [
            "input",
            "input/metadata",
            "input/metadata/objects",
            "input/metadata/refs",
            "worktree",
        ] {
            snapshot.mkdir(path)?;
        }
        let git = snapshot.open(".git")?;
        anyhow::ensure!(git.metadata()?.is_dir(),"Git source root requires a real .git directory; external gitdir files and links are unsupported");
        for path in [
            ".git/commondir",
            ".git/objects/info/alternates",
            ".git/objects/info/http-alternates",
        ] {
            anyhow::ensure!(
                !snapshot.exists(path)?,
                "Git snapshot cannot follow external object or common directories"
            );
        }
        snapshot.copy(".git/HEAD", "input/metadata/HEAD", false, 0)?;
        if snapshot.exists(".git/config")? {
            snapshot.copy(".git/config", "input/configuration", false, 0)?;
        } else {
            let configuration = directory.path().join("input/configuration");
            std::fs::write(&configuration, b"")?;
            std::fs::set_permissions(configuration, std::fs::Permissions::from_mode(0o444))?;
        }
        for path in [
            "objects",
            "refs",
            "packed-refs",
            "shallow",
            "info/attributes",
            "info/exclude",
        ] {
            let source = format!(".git/{path}");
            if snapshot.exists(&source)? {
                snapshot.copy(&source, &format!("input/metadata/{path}"), false, 0)?;
            }
        }
        if operation != SourceOperation::GitLog {
            if snapshot.exists(".git/index")? {
                snapshot.copy(".git/index", "input/metadata/index", false, 0)?;
            }
            // Split-index files are required to interpret the index. They carry
            // no external pathname authority and are copied from this gitdir only.
            let git = snapshot.open(".git")?;
            for name in snapshot.names(&git)? {
                if name.starts_with("sharedindex.") {
                    snapshot.copy(
                        &format!(".git/{name}"),
                        &format!("input/metadata/{name}"),
                        false,
                        0,
                    )?;
                }
            }
        }
        let worktree = matches!(
            operation,
            SourceOperation::GitDiff | SourceOperation::GitStatus
        );
        if worktree {
            snapshot.copy(".", "worktree", true, 0)?;
            let (working_dir, roots) = file_ceiling(ceiling, false)?;
            for mount in roots {
                let target = Path::new(
                    mount
                        .split(':')
                        .nth(1)
                        .context("missing Git ceiling destination")?,
                );
                if let Ok(relative) = target.strip_prefix(working_dir) {
                    let path = relative.to_str().context("invalid Git ceiling path")?;
                    if !path.is_empty()
                        && !path.split('/').any(|p| matches!(p, ".git" | ".symbiont"))
                    {
                        snapshot.copy(path, &format!("worktree/{path}"), true, 0)?;
                    }
                }
            }
        }
        snapshot.live()?;
        let mut boundary = ceiling.without_host_mounts();
        match boundary.tier {
            CommandTier::Landlock => {
                use super::landlock::workspace::{Mount, Workspace};
                boundary.landlock.require_network = true;
                boundary.landlock.workspace = Some(Workspace::new(
                    directory.clone(),
                    vec![
                        Mount {
                            source: directory.path().join("input"),
                            destination: "/tmp/symbi-git-input".into(),
                            read_only: true,
                        },
                        Mount {
                            source: directory.path().join("worktree"),
                            destination: "/tmp/symbi-source".into(),
                            read_only: true,
                        },
                    ],
                    "/tmp/symbi-source".into(),
                    MAX_FILE_BYTES,
                )?);
            }
            CommandTier::Firecracker => {
                let config = boundary
                    .firecracker
                    .as_mut()
                    .context("missing VM configuration")?;
                config.working_dir = format!("{}/worktree", symbi_sandbox_guest::snapshot::ROOT);
                config.snapshot = Some(Arc::new(super::firecracker::snapshot::Transfer::prepare(
                    directory.clone(),
                    deadline,
                )?));
            }
            _ => {
                let config = if boundary.tier == CommandTier::Docker {
                    &mut boundary.docker
                } else {
                    &mut boundary.gvisor.docker
                };
                config.working_dir = WORKTREE.into();
                config.network_mode = "none".into();
            }
        }
        let worker_mounts = if matches!(
            boundary.tier,
            CommandTier::Firecracker | CommandTier::Landlock
        ) {
            json!([])
        } else {
            json!([{"destination":INPUT,"read_only":true},{"destination":WORKTREE,"read_only":true}])
        };
        let descriptor = json!({"operation":operation,"transport":"selected_git_snapshot_boundary","host_mounts":[],
            "read":snapshot.files,"create":[],"input_snapshots":true,"atomic_tree_snapshot":false,"worktree_included":worktree,
            "max_snapshot_bytes":MAX_BYTES,"max_file_bytes":MAX_FILE_BYTES,"max_files":MAX_FILES,"max_entries":MAX_ENTRIES,
            "max_depth":64,"max_path_bytes":MAX_PATH_BYTES,"snapshot_bytes":snapshot.bytes,
            "worker_mounts":worker_mounts,
            "repository_config":"isolated_parse_no_includes_then_fixed_configuration",
            "worker_boundary":boundary.descriptor().map_err(anyhow::Error::msg)?});
        Ok(Self {
            directory,
            boundary,
            descriptor,
            operation,
            consumed: AtomicBool::new(false),
        })
    }
    pub async fn execute(&self, name: &str, deadline: Instant) -> Result<Value, String> {
        if Instant::now() >= deadline {
            return Err("Git query authorization expired".into());
        }
        if self.consumed.swap(true, Ordering::AcqRel) {
            return Err("Git query already consumed".into());
        }
        let mut boundary = self.boundary.clone();
        let (input, worktree) = if boundary.tier == CommandTier::Landlock {
            ("/tmp/symbi-git-input".into(), "/tmp/symbi-source".into())
        } else if boundary.tier == CommandTier::Firecracker {
            (
                format!("{}/input", symbi_sandbox_guest::snapshot::ROOT),
                format!("{}/worktree", symbi_sandbox_guest::snapshot::ROOT),
            )
        } else {
            let config = if boundary.tier == CommandTier::Docker {
                &mut boundary.docker
            } else {
                &mut boundary.gvisor.docker
            };
            config.staging = vec![self.directory.id()];
            config.volumes = vec![
                format!(
                    "{}:{INPUT}:ro",
                    self.directory.path().join("input").display()
                ),
                format!(
                    "{}:{WORKTREE}:ro",
                    self.directory.path().join("worktree").display()
                ),
            ];
            (INPUT.into(), WORKTREE.into())
        };
        let operation = serde_json::to_value(self.operation)
            .map_err(|e| e.to_string())?
            .as_str()
            .ok_or("missing Git operation")?
            .to_owned();
        let output = boundary
            .execute(
                &[
                    "python3".into(),
                    "-I".into(),
                    "-c".into(),
                    DRIVER.into(),
                    operation,
                    input,
                    worktree,
                ],
                deadline.saturating_duration_since(Instant::now()),
            )
            .await?;
        Ok(
            json!({"status":if output.success{"success"}else{"error"},"tool":name,
            "execution_transport":"selected_git_snapshot_boundary","created_files":[],
            "output_hash":format!("sha256:{}",hex::encode(Sha256::digest(output.stdout.as_bytes()))),
            "results":{"raw_output":output.stdout,"stderr":output.stderr,"exit_code":output.exit_code},
            "snapshot_bytes":self.descriptor["snapshot_bytes"],
            "snapshot_transfer":boundary.firecracker.as_ref().and_then(|vm|vm.snapshot.as_ref()).map(|s|s.receipt()).transpose()?.flatten()}),
        )
    }
}

struct Snapshot<'a> {
    ceiling: &'a CommandBoundary,
    root: PathBuf,
    deadline: Instant,
    files: Vec<Value>,
    bytes: u64,
    entries: usize,
    path_bytes: usize,
    seen: HashSet<String>,
}
impl Snapshot<'_> {
    fn live(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            Instant::now() < self.deadline,
            "Git snapshot preparation deadline expired"
        );
        Ok(())
    }
    fn mkdir(&self, path: &str) -> anyhow::Result<()> {
        let mut directory = self.root.clone();
        for component in Path::new(path).components() {
            let std::path::Component::Normal(name) = component else {
                anyhow::bail!("invalid private snapshot directory");
            };
            directory.push(name);
            std::fs::create_dir_all(&directory)?;
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755))?;
        }
        Ok(())
    }
    fn open(&self, path: &str) -> anyhow::Result<File> {
        self.live()?;
        let host = host_path(self.ceiling, path, false)?;
        let parent = directory(host.parent().context("Git snapshot parent missing")?)?;
        open_at(
            &parent,
            host.file_name()
                .and_then(|n| n.to_str())
                .context("Git snapshot name missing")?,
            libc::O_PATH,
            0,
        )
    }
    fn exists(&self, path: &str) -> anyhow::Result<bool> {
        match self.open(path) {
            Ok(_) => Ok(true),
            Err(e)
                if e.downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }
    fn names(&mut self, file: &File) -> anyhow::Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(format!("/proc/self/fd/{}", file.as_raw_fd()))? {
            self.live()?;
            self.entries += 1;
            anyhow::ensure!(
                self.entries <= MAX_ENTRIES,
                "Git snapshot entry budget exceeded"
            );
            let name = entry?
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("Git snapshot requires UTF-8 paths"))?;
            names.push(name);
        }
        names.sort();
        Ok(names)
    }
    fn copy(
        &mut self,
        source: &str,
        target: &str,
        worktree: bool,
        depth: usize,
    ) -> anyhow::Result<()> {
        self.live()?;
        if !self.seen.insert(source.to_owned()) {
            return Ok(());
        }
        anyhow::ensure!(depth <= 64, "Git snapshot depth budget exceeded");
        self.path_bytes = self
            .path_bytes
            .checked_add(source.len())
            .context("Git snapshot path overflow")?;
        anyhow::ensure!(
            self.path_bytes <= MAX_PATH_BYTES,
            "Git snapshot path budget exceeded"
        );
        let file = self.open(source)?;
        let info = file.metadata()?;
        if info.is_dir() {
            self.mkdir(target)?;
            for name in self.names(&file)? {
                if worktree && matches!(name.as_str(), ".git" | ".symbiont") {
                    continue;
                }
                let source = if source == "." {
                    name.clone()
                } else {
                    format!("{source}/{name}")
                };
                anyhow::ensure!(
                    worktree
                        || (!source.ends_with("/alternates")
                            && !source.ends_with("/http-alternates")
                            && !source.ends_with(".promisor")),
                    "Git snapshot cannot use external or partial object stores"
                );
                self.copy(&source, &format!("{target}/{name}"), worktree, depth + 1)?;
            }
            return Ok(());
        }
        anyhow::ensure!(
            self.files.len() < MAX_FILES,
            "Git snapshot file budget exceeded"
        );
        let destination = self.root.join(target);
        self.mkdir(
            Path::new(target)
                .parent()
                .and_then(|p| p.to_str())
                .context("snapshot destination parent missing")?,
        )?;
        if worktree && info.file_type().is_symlink() {
            let mut bytes = vec![0u8; 4097];
            let empty = c"";
            let count = unsafe {
                libc::readlinkat(
                    file.as_raw_fd(),
                    empty.as_ptr(),
                    bytes.as_mut_ptr().cast(),
                    bytes.len(),
                )
            };
            anyhow::ensure!(
                (0..=4096).contains(&count),
                "Git snapshot link target exceeds its bound"
            );
            bytes.truncate(count as usize);
            self.bytes += bytes.len() as u64;
            anyhow::ensure!(self.bytes <= MAX_BYTES, "Git snapshot byte budget exceeded");
            use std::os::unix::ffi::OsStrExt;
            std::os::unix::fs::symlink(std::ffi::OsStr::from_bytes(&bytes), destination)?;
            self.files.push(json!({"path":source,"kind":"symlink","bytes":bytes.len(),"sha256":hex::encode(Sha256::digest(&bytes))}));
            return Ok(());
        }
        anyhow::ensure!(info.is_file()&&info.nlink()==1,"Git snapshots require regular files without hard links; metadata links and special files are unsupported");
        anyhow::ensure!(
            info.len() <= MAX_FILE_BYTES,
            "Git snapshot file byte budget exceeded"
        );
        let mut input = File::open(format!("/proc/self/fd/{}", file.as_raw_fd()))?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&destination)?;
        let mut hash = Sha256::new();
        let mut size = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            self.live()?;
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            size += count as u64;
            self.bytes += count as u64;
            anyhow::ensure!(
                size <= MAX_FILE_BYTES && self.bytes <= MAX_BYTES,
                "Git snapshot byte budget exceeded"
            );
            hash.update(&buffer[..count]);
            output.write_all(&buffer[..count])?;
        }
        let mode = if info.mode() & 0o111 != 0 {
            0o555
        } else {
            0o444
        };
        output.set_permissions(std::fs::Permissions::from_mode(mode))?;
        self.files.push(json!({"path":source,"kind":"file","bytes":size,"sha256":hex::encode(hash.finalize()),"executable":mode==0o555}));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::symlink, time::Duration};
    fn fixture() -> (tempfile::TempDir, CommandBoundary) {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        fs::create_dir_all(source.join(".git/objects/info")).unwrap();
        fs::create_dir_all(source.join(".git/refs/heads")).unwrap();
        fs::write(source.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(
            source.join(".git/config"),
            "[core]\nrepositoryformatversion=0\n",
        )
        .unwrap();
        fs::write(source.join(".git/index"), "synthetic index").unwrap();
        fs::write(source.join("a"), "original").unwrap();
        let mut boundary = CommandBoundary::default();
        boundary.docker.volumes = vec![format!("{}:/workspace:ro", source.display())];
        boundary.docker.supervisor.state_dir = root.path().join("leases");
        (root, boundary)
    }
    fn plan(b: &CommandBoundary, op: SourceOperation) -> anyhow::Result<GitPlan> {
        GitPlan::prepare(b, op, Instant::now() + Duration::from_secs(5))
    }
    #[test]
    fn git_snapshot_grants_only_operation_inputs_and_retains_original_bytes() {
        let (root, b) = fixture();
        for op in [
            SourceOperation::GitLog,
            SourceOperation::GitStagedDiff,
            SourceOperation::GitDiff,
            SourceOperation::GitStatus,
        ] {
            let p = plan(&b, op).unwrap();
            let paths = p.descriptor["read"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v["path"].as_str().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(
                paths.contains(&"a"),
                matches!(op, SourceOperation::GitDiff | SourceOperation::GitStatus)
            );
            assert_eq!(paths.contains(&".git/index"), op != SourceOperation::GitLog);
            assert!(!p.directory.path().join("input/metadata/config").exists());
            assert_eq!(p.boundary.docker.network_mode, "none");
            assert!(p.boundary.docker.volumes.is_empty());
        }
        let p = plan(&b, SourceOperation::GitDiff).unwrap();
        fs::write(root.path().join("source/a"), "changed").unwrap();
        assert_eq!(
            fs::read_to_string(p.directory.path().join("worktree/a")).unwrap(),
            "original"
        );
    }
    #[test]
    fn git_snapshot_refuses_external_metadata_and_hard_links() {
        let (root, b) = fixture();
        let source = root.path().join("source");
        for path in [
            ".git/commondir",
            ".git/objects/info/alternates",
            ".git/objects/info/http-alternates",
        ] {
            fs::write(source.join(path), "/outside").unwrap();
            assert!(plan(&b, SourceOperation::GitLog).is_err());
            fs::remove_file(source.join(path)).unwrap();
        }
        fs::write(root.path().join("secret"), "canary").unwrap();
        fs::hard_link(root.path().join("secret"), source.join("hard")).unwrap();
        assert!(plan(&b, SourceOperation::GitDiff).is_err());
        fs::remove_file(source.join("hard")).unwrap();
        fs::remove_file(source.join(".git/index")).unwrap();
        symlink(root.path().join("secret"), source.join(".git/index")).unwrap();
        assert!(plan(&b, SourceOperation::GitStatus).is_err());
        assert_eq!(
            fs::read_to_string(root.path().join("secret")).unwrap(),
            "canary"
        );
    }
    #[test]
    fn git_snapshot_preserves_symlink_value_without_reading_target() {
        let (root, b) = fixture();
        fs::write(root.path().join("secret"), "canary").unwrap();
        symlink(root.path().join("secret"), root.path().join("source/link")).unwrap();
        let p = plan(&b, SourceOperation::GitDiff).unwrap();
        assert_eq!(
            fs::read_link(p.directory.path().join("worktree/link")).unwrap(),
            root.path().join("secret")
        );
        let record = p.descriptor["read"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["path"] == "link")
            .unwrap();
        assert_eq!(record["kind"], "symlink");
        assert_eq!(
            record["bytes"],
            root.path().join("secret").as_os_str().len()
        );
    }
    #[test]
    fn git_snapshot_applies_overlay_and_virtual_mounts() {
        let (root, mut b) = fixture();
        let overlay = root.path().join("overlay");
        fs::create_dir(&overlay).unwrap();
        fs::write(overlay.join("visible"), "overlay").unwrap();
        fs::create_dir(root.path().join("source/sub")).unwrap();
        fs::write(root.path().join("source/sub/hidden"), "hidden").unwrap();
        b.docker.volumes.extend([
            format!("{}:/workspace/sub:ro", overlay.display()),
            format!("{}:/workspace/virtual/deep:ro", overlay.display()),
        ]);
        let p = plan(&b, SourceOperation::GitStatus).unwrap();
        assert!(!p.directory.path().join("worktree/sub/hidden").exists());
        assert_eq!(
            fs::read_to_string(p.directory.path().join("worktree/virtual/deep/visible")).unwrap(),
            "overlay"
        );
    }
    #[test]
    fn git_snapshot_preserves_literal_names_and_readable_empty_configuration() {
        let (root, b) = fixture();
        std::fs::remove_file(root.path().join("source/.git/config")).unwrap();
        for name in ["colon:name", "back\\slash"] {
            std::fs::write(root.path().join("source").join(name), "literal").unwrap();
        }
        let p = plan(&b, SourceOperation::GitDiff).unwrap();
        for name in ["colon:name", "back\\slash"] {
            assert_eq!(
                std::fs::read_to_string(p.directory.path().join("worktree").join(name)).unwrap(),
                "literal"
            );
        }
        assert_eq!(
            std::fs::metadata(p.directory.path().join("input/configuration"))
                .unwrap()
                .mode()
                & 0o777,
            0o444
        );
    }

    #[test]
    fn git_snapshot_refuses_large_inputs_and_pointer_roots() {
        let (root, b) = fixture();
        let f = File::create(root.path().join("source/huge")).unwrap();
        f.set_len(MAX_FILE_BYTES + 1).unwrap();
        assert!(plan(&b, SourceOperation::GitStatus).is_err());
        assert!(plan(&b, SourceOperation::GitLog).is_ok());
        fs::rename(
            root.path().join("source/.git"),
            root.path().join("metadata"),
        )
        .unwrap();
        fs::write(root.path().join("source/.git"), "gitdir: ../metadata").unwrap();
        assert!(plan(&b, SourceOperation::GitLog).is_err());
    }
    #[test]
    fn vm_git_uses_only_selected_roots_and_binds_an_immutable_transfer() {
        let (root, mut boundary) = fixture();
        let kernel = root.path().join("kernel");
        let rootfs = root.path().join("rootfs");
        fs::write(&kernel, b"fixture").unwrap();
        fs::write(&rootfs, b"fixture").unwrap();
        boundary.tier = CommandTier::Firecracker;
        let mut vm = super::super::firecracker::FirecrackerConfig {
            kernel_image_path: kernel,
            rootfs_path: rootfs,
            firecracker_binary: "/usr/bin/true".into(),
            ..Default::default()
        };
        vm.supervisor.state_dir = root.path().join("vm-leases");
        boundary.firecracker = Some(vm);
        assert!(plan(&boundary, SourceOperation::GitLog).is_err());
        boundary.firecracker.as_mut().unwrap().source_roots =
            vec![format!("{}:/tmp:ro", root.path().join("source").display())];
        let snapshot = plan(&boundary, SourceOperation::GitDiff).unwrap();
        assert_eq!(snapshot.descriptor["worker_mounts"], json!([]));
        let vm = snapshot.boundary.firecracker.as_ref().unwrap();
        assert!(vm.source_roots.is_empty() && vm.output_roots.is_empty());
        let transfer = vm.snapshot.as_ref().unwrap();
        assert_eq!(
            transfer.descriptor().bytes,
            snapshot.descriptor["snapshot_bytes"].as_u64().unwrap()
        );
        assert_eq!(
            snapshot.descriptor["worker_boundary"]["vm"]["git_snapshot"],
            serde_json::to_value(transfer.descriptor()).unwrap()
        );
        fs::write(root.path().join("source/a"), b"changed").unwrap();
        assert_eq!(
            fs::read(snapshot.directory.path().join("worktree/a")).unwrap(),
            b"original"
        );
        assert!(snapshot
            .boundary
            .without_host_mounts()
            .firecracker
            .unwrap()
            .snapshot
            .is_none());
    }
}
