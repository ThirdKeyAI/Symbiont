//! Read-only source queries. Preparation captures bounded inputs or results;
//! authorized dispatch uses fixed brokers or isolated Git snapshot workers.
use super::command::CommandBoundary;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Mutex, time::Instant};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceOperation {
    ReadFile,
    ListFiles,
    GrepFiles,
    GitDiff,
    GitStagedDiff,
    GitLog,
    GitStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceQuery {
    pub operation: SourceOperation,
}

pub struct SourcePlan {
    descriptor: Value,
    result: Mutex<Option<Value>>,
    #[cfg(target_os = "linux")]
    git: Option<super::source_git::GitPlan>,
}

impl SourcePlan {
    pub fn prepare(
        boundary: &CommandBoundary,
        query: &SourceQuery,
        args: &HashMap<String, String>,
        deadline: Instant,
    ) -> Result<Self, String> {
        boundary.validate()?;
        #[cfg(target_os = "linux")]
        {
            if matches!(
                query.operation,
                SourceOperation::GitDiff
                    | SourceOperation::GitStagedDiff
                    | SourceOperation::GitLog
                    | SourceOperation::GitStatus
            ) {
                if !args.is_empty() {
                    return Err("Git source queries accept no arguments".into());
                }
                let git = super::source_git::GitPlan::prepare(boundary, query.operation, deadline)
                    .map_err(|e| e.to_string())?;
                return Ok(Self {
                    descriptor: git.descriptor.clone(),
                    result: Mutex::new(None),
                    git: Some(git),
                });
            }
            linux::prepare(boundary, query.operation, args, deadline).map_err(|e| e.to_string())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (query, args, deadline);
            Err("source queries require the Linux file broker".into())
        }
    }

    pub fn descriptor(&self) -> &Value {
        &self.descriptor
    }

    pub fn implementation_hash() -> String {
        crate::reasoning::prepared::digest_json(&json!({
            "source": include_str!("source.rs"), "files": include_str!("files.rs"),
            "git":include_str!("source_git.rs"),"git_driver":include_str!("source_git.py"),
            "git_transfer":include_str!("firecracker/snapshot.rs")
        }))
        .expect("literal implementation sources are serializable")
    }

    pub fn execution_boundary(&self, default: &CommandBoundary) -> CommandBoundary {
        #[cfg(target_os = "linux")]
        if let Some(git) = &self.git {
            return git.boundary.clone();
        }
        default.clone()
    }
    pub fn transport(&self) -> &str {
        self.descriptor["transport"]
            .as_str()
            .unwrap_or("fixed_source_broker")
    }
    pub async fn execute_prepared(&self, name: &str, deadline: Instant) -> Result<Value, String> {
        #[cfg(target_os = "linux")]
        if let Some(git) = &self.git {
            return git.execute(name, deadline).await;
        }
        self.execute(name, deadline)
    }

    pub fn execute(&self, name: &str, deadline: Instant) -> Result<Value, String> {
        if Instant::now() >= deadline {
            return Err("source query authorization expired".into());
        }
        let result = self
            .result
            .lock()
            .map_err(|_| "source query ownership failed")?
            .take()
            .ok_or("source query already consumed")?;
        Ok(json!({"status":"success", "tool":name, "results":result,
            "output_hash":self.descriptor["result_sha256"], "created_files":[],
            "execution_transport":"fixed_source_broker"}))
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use crate::sandbox::files::{
        linux::{ceiling, directory, host_path, open_at},
        resolve,
    };
    use anyhow::Context;
    use sha2::{Digest, Sha256};
    use std::{
        collections::{BTreeSet, HashSet},
        fs::File,
        io::{Read, Seek, SeekFrom},
        os::{fd::AsRawFd, unix::fs::MetadataExt},
        path::Path,
    };

    const MAX_ENTRIES: usize = 10000;
    const MAX_FILES: usize = 2000;
    const MAX_PATH_BYTES: usize = 65536;
    const MAX_FILE_BYTES: usize = 1024 * 1024;
    const MAX_SCAN_BYTES: usize = 8 * 1024 * 1024;
    const MAX_MATCHES: usize = 200;

    fn hash(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }
    fn live(deadline: Instant) -> anyhow::Result<()> {
        anyhow::ensure!(
            Instant::now() < deadline,
            "source query preparation deadline expired"
        );
        Ok(())
    }
    fn safe(path: &str) -> anyhow::Result<()> {
        if path != "." {
            resolve(path, &HashMap::new())?;
        }
        anyhow::ensure!(
            !path.split('/').any(|p| matches!(p, ".git" | ".symbiont")),
            "source control directories are unavailable"
        );
        Ok(())
    }
    fn open(boundary: &CommandBoundary, path: &str) -> anyhow::Result<File> {
        let host = host_path(boundary, path, false)?;
        let parent = directory(host.parent().context("source parent missing")?)?;
        // O_PATH does not open devices or FIFOs for I/O. Metadata is inspected
        // before a regular file or directory is reopened through its retained fd.
        open_at(
            &parent,
            host.file_name()
                .and_then(|n| n.to_str())
                .context("source name missing")?,
            libc::O_PATH,
            0,
        )
    }
    fn reader(file: &File) -> anyhow::Result<File> {
        Ok(File::open(format!("/proc/self/fd/{}", file.as_raw_fd()))?)
    }
    fn number(
        args: &HashMap<String, String>,
        name: &str,
        default: u64,
        min: u64,
        max: u64,
    ) -> anyhow::Result<u64> {
        let n = args
            .get(name)
            .filter(|s| !s.is_empty())
            .map(|v| v.parse::<u64>())
            .transpose()?
            .unwrap_or(default);
        anyhow::ensure!((min..=max).contains(&n), "invalid source {name}");
        Ok(n)
    }

    pub(super) fn prepare(
        boundary: &CommandBoundary,
        operation: SourceOperation,
        args: &HashMap<String, String>,
        deadline: Instant,
    ) -> anyhow::Result<SourcePlan> {
        live(deadline)?;
        let (working_dir, configured_roots) = ceiling(boundary, false)?;
        let allowed: &[&str] = match operation {
            SourceOperation::ReadFile => &["path", "offset", "limit"],
            SourceOperation::ListFiles => &[],
            SourceOperation::GrepFiles => &["needle"],
            _ => anyhow::bail!("Git queries require repository snapshot preparation"),
        };
        anyhow::ensure!(
            args.keys().all(|k| allowed.contains(&k.as_str())),
            "unknown source query argument"
        );
        let mut descriptor = json!({"operation":operation, "transport":"fixed_source_broker", "host_mounts":[],
            "read":[], "create":[], "input_snapshots":true, "atomic_tree_snapshot":false,
            "max_entries":MAX_ENTRIES,"max_files":MAX_FILES,"max_depth":32,
            "max_path_bytes":MAX_PATH_BYTES,"max_file_bytes":MAX_FILE_BYTES,
            "max_scan_bytes":MAX_SCAN_BYTES,"max_matches":MAX_MATCHES,
            "excluded_directories":[".git",".symbiont"], "working_dir":working_dir});
        let result = if operation == SourceOperation::ReadFile {
            let path = args.get("path").context("missing source path")?;
            anyhow::ensure!(path != ".", "source read requires a file path");
            safe(path)?;
            let offset = number(args, "offset", 0, 0, 10000000)?;
            let limit = number(args, "limit", 32768, 1, 65536)?;
            let file = open(boundary, path)?;
            let info = file.metadata()?;
            anyhow::ensure!(
                info.is_file() && info.nlink() == 1,
                "source must be a regular file without links"
            );
            let mut file = reader(&file)?;
            file.seek(SeekFrom::Start(offset))?;
            let mut bytes = Vec::new();
            file.take(limit).read_to_end(&mut bytes)?;
            descriptor["read"] = json!([{"path":path,"offset":offset,"bytes":bytes.len(),"sha256":hash(&bytes),"observed_size":info.len()}]);
            json!({"path":path,"offset":offset,"bytes":bytes.len(),"size":info.len(),"text":String::from_utf8_lossy(&bytes)})
        } else {
            let needle = if operation == SourceOperation::GrepFiles {
                let n = args.get("needle").context("missing source needle")?;
                anyhow::ensure!(
                    !n.is_empty() && n.len() <= 1024 && !n.contains('\0'),
                    "invalid source needle"
                );
                Some(n.as_bytes())
            } else {
                None
            };
            let cwd = Path::new(working_dir);
            let mut roots = BTreeSet::new();
            for mount in configured_roots {
                let destination = Path::new(
                    mount
                        .split(':')
                        .nth(1)
                        .context("missing mount destination")?,
                );
                if cwd.starts_with(destination) {
                    roots.insert(".".to_owned());
                } else if let Ok(relative) = destination.strip_prefix(cwd) {
                    let path = relative.to_str().context("invalid source path encoding")?;
                    if safe(path).is_ok() {
                        roots.insert(path.to_owned());
                    }
                }
            }
            anyhow::ensure!(
                !roots.is_empty(),
                "source query has no configured mount ceiling"
            );
            let mut scan = Scan::default();
            for path in roots {
                live(deadline)?;
                // A missing or unusable declared root is an error, never an empty success.
                let file = open(boundary, &path)?;
                scan.walk(boundary, file, &path, needle, 0, deadline)?;
                if scan.stopped {
                    break;
                }
            }
            descriptor["read"] = json!(scan.inputs);
            descriptor["metadata_only"] = json!(needle.is_none());
            descriptor["truncated"] = json!(scan.truncated);
            descriptor["skipped"] = json!(scan.skipped);
            if needle.is_some() {
                json!({"matches":scan.matches,"truncated":scan.truncated,"skipped":scan.skipped,"files_scanned":scan.files})
            } else {
                json!({"files":scan.paths,"truncated":scan.truncated,"skipped":scan.skipped})
            }
        };
        live(deadline)?;
        descriptor["result_sha256"] =
            json!(crate::reasoning::prepared::digest_json(&result).map_err(anyhow::Error::msg)?);
        Ok(SourcePlan {
            descriptor,
            result: Mutex::new(Some(result)),
            git: None,
        })
    }

    #[derive(Default)]
    struct Scan {
        paths: Vec<String>,
        matches: Vec<Value>,
        inputs: Vec<Value>,
        seen: HashSet<String>,
        entries: usize,
        files: usize,
        bytes: usize,
        path_bytes: usize,
        skipped: usize,
        truncated: bool,
        stopped: bool,
    }
    impl Scan {
        fn stop(&mut self) {
            self.truncated = true;
            self.stopped = true;
        }
        fn walk(
            &mut self,
            boundary: &CommandBoundary,
            file: File,
            path: &str,
            needle: Option<&[u8]>,
            depth: usize,
            deadline: Instant,
        ) -> anyhow::Result<()> {
            live(deadline)?;
            if self.stopped || !self.seen.insert(path.into()) {
                return Ok(());
            }
            if depth > 32
                || self.files >= MAX_FILES
                || self.path_bytes + path.len() > MAX_PATH_BYTES
            {
                self.stop();
                return Ok(());
            }
            let info = file.metadata()?;
            if info.is_dir() {
                let file = reader(&file)?;
                let mut names = Vec::new();
                for entry in std::fs::read_dir(format!("/proc/self/fd/{}", file.as_raw_fd()))? {
                    live(deadline)?;
                    if self.entries >= MAX_ENTRIES {
                        self.stop();
                        break;
                    }
                    self.entries += 1;
                    names.push(entry?.file_name());
                }
                names.sort();
                // An entry limit is a partial result, not a claim that no files exist.
                for name in names {
                    if self.stopped {
                        break;
                    }
                    let Some(name) = name.to_str() else {
                        self.skipped += 1;
                        self.truncated = true;
                        continue;
                    };
                    if matches!(name, ".git" | ".symbiont") {
                        self.skipped += 1;
                        continue;
                    }
                    let child = if path == "." {
                        name.into()
                    } else {
                        format!("{path}/{name}")
                    };
                    // Re-resolve overlay destinations so a parent mount never reveals
                    // files hidden by a more specific configured mount.
                    match open(boundary, &child) {
                        Ok(f) => self.walk(boundary, f, &child, needle, depth + 1, deadline)?,
                        Err(_) => {
                            self.skipped += 1;
                            self.truncated = true;
                        }
                    }
                }
            } else if info.is_file() && info.nlink() == 1 {
                self.files += 1;
                self.path_bytes += path.len();
                if let Some(needle) = needle {
                    if info.len() > MAX_FILE_BYTES as u64 {
                        self.skipped += 1;
                        self.truncated = true;
                        return Ok(());
                    }
                    if self.bytes >= MAX_SCAN_BYTES {
                        self.stop();
                        return Ok(());
                    }
                    let cap = (MAX_FILE_BYTES + 1).min(MAX_SCAN_BYTES - self.bytes);
                    let mut bytes = Vec::new();
                    reader(&file)?.take(cap as u64).read_to_end(&mut bytes)?;
                    self.bytes += bytes.len();
                    let partial = bytes.len() > MAX_FILE_BYTES || (bytes.len() as u64) < info.len();
                    self.truncated |= partial;
                    self.inputs.push(json!({"path":path,"bytes":bytes.len(),"sha256":hash(&bytes),"truncated":partial}));
                    for (line, value) in bytes.split(|b| *b == b'\n').enumerate() {
                        if value.windows(needle.len()).any(|w| w == needle) {
                            self.matches.push(json!({"path":path,"line":line+1,"text":String::from_utf8_lossy(&value[..value.len().min(512)])}));
                            if self.matches.len() >= MAX_MATCHES {
                                self.stop();
                                break;
                            }
                        }
                    }
                } else {
                    self.paths.push(path.into());
                }
            } else {
                self.skipped += 1;
                self.truncated = true;
            }
            Ok(())
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use crate::sandbox::command::CommandTier;
    use std::{fs, os::unix::fs::symlink, time::Duration};

    fn fixture() -> (tempfile::TempDir, CommandBoundary) {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("source")).unwrap();
        let mut boundary = CommandBoundary::default();
        boundary.docker.working_dir = "/source".into();
        boundary.docker.volumes = vec![format!(
            "{}:/source:ro",
            root.path().join("source").display()
        )];
        (root, boundary)
    }
    fn plan(
        boundary: &CommandBoundary,
        op: SourceOperation,
        args: Value,
    ) -> Result<SourcePlan, String> {
        let args = args
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
            .collect();
        SourcePlan::prepare(
            boundary,
            &SourceQuery { operation: op },
            &args,
            Instant::now() + Duration::from_secs(10),
        )
    }
    fn result(plan: &SourcePlan) -> Value {
        plan.execute("fixture", Instant::now() + Duration::from_secs(1))
            .unwrap()["results"]
            .clone()
    }

    fn vm_fixture() -> (tempfile::TempDir, CommandBoundary) {
        let (root, mut boundary) = fixture();
        for name in ["kernel", "rootfs"] {
            fs::write(root.path().join(name), b"test artifact").unwrap();
        }
        let config = crate::sandbox::FirecrackerConfig {
            kernel_image_path: root.path().join("kernel"),
            rootfs_path: root.path().join("rootfs"),
            firecracker_binary: "/bin/true".into(),
            working_dir: "/source".into(),
            source_roots: boundary.docker.volumes.clone(),
            supervisor: crate::sandbox::supervisor::SupervisorConfig {
                state_dir: root.path().join("state"),
                ..Default::default()
            },
            ..Default::default()
        };
        boundary.tier = CommandTier::Firecracker;
        boundary.firecracker = Some(config);
        // A different tier's configuration must not grant any source authority.
        boundary.docker.volumes = vec![format!("{}:/source:ro", root.path().display())];
        (root, boundary)
    }

    #[test]
    fn firecracker_source_uses_only_explicit_roots_and_keeps_workers_mount_free() {
        let (root, b) = vm_fixture();
        fs::write(root.path().join("source/a"), "selected input").unwrap();
        fs::write(root.path().join("secret"), "other tier authority").unwrap();
        let p = plan(&b, SourceOperation::ReadFile, json!({"path":"a"})).unwrap();
        fs::write(root.path().join("source/a"), "replaced").unwrap();
        assert_eq!(result(&p)["text"], "selected input");
        assert!(plan(&b, SourceOperation::ReadFile, json!({"path":"secret"})).is_err());
        assert_eq!(
            result(&plan(&b, SourceOperation::ListFiles, json!({})).unwrap())["files"],
            json!(["a"])
        );
        assert_eq!(
            result(&plan(&b, SourceOperation::GrepFiles, json!({"needle":"replace"})).unwrap())
                ["matches"][0]["path"],
            "a"
        );
        let cleared = b.without_host_mounts();
        assert!(cleared
            .firecracker
            .as_ref()
            .unwrap()
            .source_roots
            .is_empty());
        assert!(plan(&cleared, SourceOperation::ListFiles, json!({})).is_err());
        assert_eq!(cleared.descriptor().unwrap()["vm"]["network_interfaces"], 0);
        assert!(plan(&b, SourceOperation::GitLog, json!({})).is_err());
    }

    #[test]
    fn firecracker_source_roots_cannot_expose_project_control_files() {
        let (root, b) = vm_fixture();
        let project = root.path().join("control");
        fs::create_dir_all(project.join("agents")).unwrap();
        let config = b.firecracker.unwrap();
        fs::write(project.join("symbiont.toml"), format!(
            "[sandbox]\ntier='firecracker'\n[sandbox.firecracker]\nkernel_image_path='{}'\nrootfs_path='{}'\nfirecracker_binary='/bin/true'\nworking_dir='/source'\nsource_roots=['{}:/source:ro']\n[sandbox.firecracker.supervisor]\nstate_dir='{}'\n",
            config.kernel_image_path.display(), config.rootfs_path.display(), project.join("agents").display(), config.supervisor.state_dir.display()
        )).unwrap();
        assert!(CommandBoundary::load(&project)
            .unwrap_err()
            .contains("protected project"));
    }

    #[test]
    fn firecracker_source_roots_reject_unsafe_and_writable_authority() {
        let (root, mut b) = vm_fixture();
        let mut config = b.firecracker.clone().unwrap();
        for source in [
            format!("{}:/source:rw", root.path().join("source").display()),
            "/etc:/source:ro".into(),
            format!("{}:/source:ro", root.path().display()),
        ] {
            config.source_roots = vec![source];
            assert!(config.validate().is_err());
        }
        config.source_roots = b.firecracker.as_ref().unwrap().source_roots.clone();
        config.source_roots.push(config.source_roots[0].clone());
        assert!(config.validate().is_err());
        config.source_roots[1] = config.source_roots[1].replace("/source:ro", "/source/:ro");
        assert!(config.validate().is_err());
        fs::write(root.path().join("source/a"), "input").unwrap();
        symlink(root.path().join("kernel"), root.path().join("source/link")).unwrap();
        fs::hard_link(root.path().join("kernel"), root.path().join("source/hard")).unwrap();
        for path in ["../kernel", "link", "hard", ".symbiont/key"] {
            assert!(plan(&b, SourceOperation::ReadFile, json!({"path":path})).is_err());
        }
        let overlay = root.path().join("overlay");
        fs::create_dir(&overlay).unwrap();
        fs::write(overlay.join("visible"), "overlay").unwrap();
        b.firecracker
            .as_mut()
            .unwrap()
            .source_roots
            .push(format!("{}:/source/sub:ro", overlay.display()));
        assert_eq!(
            result(&plan(&b, SourceOperation::ListFiles, json!({})).unwrap())["files"],
            json!(["a", "sub/visible"])
        );
    }

    #[test]
    fn source_read_retains_exact_range_after_replacement_and_consumes_once() {
        let (root, b) = fixture();
        let path = root.path().join("source/input");
        fs::write(&path, "before content").unwrap();
        let p = plan(
            &b,
            SourceOperation::ReadFile,
            json!({"path":"input","offset":"7","limit":"7"}),
        )
        .unwrap();
        fs::remove_file(&path).unwrap();
        fs::write(&path, "changed").unwrap();
        assert_eq!(p.descriptor()["read"][0]["bytes"], 7);
        assert_eq!(result(&p)["text"], "content");
        assert!(p
            .execute("fixture", Instant::now() + Duration::from_secs(1))
            .is_err());
    }
    #[test]
    fn source_refuses_traversal_links_special_files_and_private_paths() {
        let (root, b) = fixture();
        let src = root.path().join("source");
        fs::write(root.path().join("secret"), "canary").unwrap();
        symlink(root.path().join("secret"), src.join("symlink")).unwrap();
        fs::hard_link(root.path().join("secret"), src.join("hardlink")).unwrap();
        fs::create_dir(src.join(".git")).unwrap();
        fs::write(src.join(".git/config"), "private").unwrap();
        let fifo = std::ffi::CString::new(src.join("fifo").to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        for path in [
            "../secret",
            "/etc/passwd",
            "symlink",
            "hardlink",
            "fifo",
            ".git/config",
            ".",
        ] {
            assert!(
                plan(&b, SourceOperation::ReadFile, json!({"path":path})).is_err(),
                "{path}"
            );
        }
        let p = plan(&b, SourceOperation::ListFiles, json!({})).unwrap();
        assert_eq!(result(&p)["files"], json!([]));
        assert_eq!(
            fs::read_to_string(root.path().join("secret")).unwrap(),
            "canary"
        );
    }
    #[test]
    fn source_listing_is_metadata_only_and_search_returns_literal_matches() {
        let (root, b) = fixture();
        let src = root.path().join("source");
        fs::write(src.join("a"), "first\nneedle.* here\nlast").unwrap();
        fs::write(src.join("b"), "unrelated text").unwrap();
        let p = plan(&b, SourceOperation::ListFiles, json!({})).unwrap();
        assert_eq!(p.descriptor()["read"], json!([]));
        assert_eq!(p.descriptor()["metadata_only"], true);
        assert_eq!(result(&p)["files"], json!(["a", "b"]));
        let p = plan(&b, SourceOperation::GrepFiles, json!({"needle":"needle.*"})).unwrap();
        assert_eq!(p.descriptor()["read"].as_array().unwrap().len(), 2);
        let value = result(&p);
        assert_eq!(
            value["matches"],
            json!([{"path":"a","line":2,"text":"needle.* here"}])
        );
        assert_eq!(value["truncated"], false);
    }
    #[test]
    fn source_overlay_does_not_disclose_hidden_parent_files() {
        let (root, mut b) = fixture();
        let src = root.path().join("source");
        fs::create_dir(src.join("sub")).unwrap();
        fs::write(src.join("sub/hidden"), "secret").unwrap();
        let overlay = root.path().join("overlay");
        fs::create_dir(&overlay).unwrap();
        fs::write(overlay.join("visible"), "visible").unwrap();
        b.docker
            .volumes
            .push(format!("{}:/source/sub:ro", overlay.display()));
        assert_eq!(
            result(&plan(&b, SourceOperation::ListFiles, json!({})).unwrap())["files"],
            json!(["sub/visible"])
        );
        assert!(plan(&b, SourceOperation::ReadFile, json!({"path":"sub/hidden"})).is_err());
    }
    #[test]
    fn source_nested_mount_without_parent_and_gvisor_use_same_ceiling() {
        let (root, mut b) = fixture();
        fs::write(root.path().join("source/a"), "data").unwrap();
        b.docker.volumes = vec![format!(
            "{}:/source/sub/deep:ro",
            root.path().join("source").display()
        )];
        b.gvisor.docker = b.docker.clone();
        b.tier = CommandTier::GVisor;
        assert_eq!(
            result(&plan(&b, SourceOperation::ListFiles, json!({})).unwrap())["files"],
            json!(["sub/deep/a"])
        );
    }
    #[test]
    fn source_reports_partial_search_and_bounded_matches() {
        let (root, b) = fixture();
        fs::write(
            root.path().join("source/large"),
            vec![b'x'; 1024 * 1024 + 1],
        )
        .unwrap();
        fs::write(root.path().join("source/matches"), "hit\n".repeat(201)).unwrap();
        let p = plan(&b, SourceOperation::GrepFiles, json!({"needle":"hit"})).unwrap();
        let r = result(&p);
        assert_eq!(r["matches"].as_array().unwrap().len(), 200);
        assert_eq!(r["truncated"], true);
        assert_eq!(r["skipped"], 1);
    }
    #[test]
    fn source_refuses_missing_ceiling_bad_bounds_and_expired_authority() {
        let (root, mut b) = fixture();
        fs::write(root.path().join("source/a"), "data").unwrap();
        for args in [
            json!({"path":"a","limit":"65537"}),
            json!({"path":"a","offset":"-1"}),
            json!({"path":"a","command":"anything"}),
        ] {
            assert!(plan(&b, SourceOperation::ReadFile, args).is_err());
        }
        let p = plan(&b, SourceOperation::ListFiles, json!({})).unwrap();
        assert!(p.execute("fixture", Instant::now()).is_err());
        assert!(SourcePlan::prepare(
            &b,
            &SourceQuery {
                operation: SourceOperation::ListFiles
            },
            &HashMap::new(),
            Instant::now()
        )
        .is_err());
        b.docker.volumes.clear();
        assert!(plan(&b, SourceOperation::ListFiles, json!({})).is_err());
    }
}
