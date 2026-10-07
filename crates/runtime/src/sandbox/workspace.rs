//! Fixed workspace operations through bounded, retained file capabilities.
//! A plan is prepared before policy review and consumed only by authorized dispatch.
//! No user-supplied program executes in this broker.

use super::command::CommandBoundary;
use serde_json::Value;
#[cfg(target_os = "linux")]
use std::sync::Mutex;
use std::time::Instant;

#[cfg(target_os = "linux")]
const LIMIT: usize = 32768;

pub struct WorkspacePlan {
    descriptor: Value,
    #[cfg(target_os = "linux")]
    operation: Mutex<Option<linux::Operation>>,
}

impl WorkspacePlan {
    pub fn prepare(
        boundary: &CommandBoundary,
        operation: &str,
        path: &str,
        value: Option<&str>,
    ) -> Result<Self, String> {
        boundary.validate()?;
        #[cfg(target_os = "linux")]
        {
            linux::prepare(boundary, operation, path, value).map_err(|e| e.to_string())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (operation, path, value);
            Err("workspace file operations require the Linux file broker".into())
        }
    }

    pub fn descriptor(&self) -> &Value {
        &self.descriptor
    }

    pub fn implementation_hash() -> String {
        crate::reasoning::prepared::digest_json(&serde_json::json!({
            "workspace": include_str!("workspace.rs"), "files": include_str!("files.rs")
        }))
        .expect("literal implementation sources are serializable")
    }

    /// Synchronous and bounded: dropping an async caller cannot detach a write.
    /// The enclosing dispatcher must durably record intent before calling this.
    pub fn execute(&self, deadline: Instant) -> Result<Value, String> {
        if Instant::now() >= deadline {
            return Err("workspace authorization expired before execution".into());
        }
        #[cfg(target_os = "linux")]
        {
            let operation = self
                .operation
                .lock()
                .map_err(|_| "workspace operation ownership failed")?
                .take()
                .ok_or("workspace operation already consumed")?;
            linux::execute(operation, deadline).map_err(|e| e.to_string())
        }
        #[cfg(not(target_os = "linux"))]
        Err("workspace file operations require the Linux file broker".into())
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
        collections::{BTreeMap, HashMap},
        ffi::CString,
        fs::File,
        io::{Read, Seek, SeekFrom, Write},
        os::{fd::AsRawFd, unix::fs::MetadataExt},
        path::Path,
    };

    pub(super) enum Operation {
        Read(String),
        Write {
            path: String,
            parent: File,
            missing: Vec<String>,
            name: String,
            previous: Option<(File, String)>,
            content: Vec<u8>,
        },
    }

    fn hash(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn snapshot(file: &mut File) -> anyhow::Result<Vec<u8>> {
        snapshot_up_to(file, LIMIT + 1)
    }

    fn snapshot_up_to(file: &mut File, limit: usize) -> anyhow::Result<Vec<u8>> {
        let metadata = file.metadata()?;
        anyhow::ensure!(
            metadata.is_file() && metadata.nlink() == 1,
            "workspace operations require regular files without hard links"
        );
        file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        file.take(limit as u64).read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    fn identity(file: &File) -> anyhow::Result<(u64, u64)> {
        let info = file.metadata()?;
        Ok((info.dev(), info.ino()))
    }

    fn safe_path(path: &str, search: bool) -> anyhow::Result<()> {
        if search && path == "." {
            return Ok(());
        }
        resolve(path, &HashMap::new())?;
        anyhow::ensure!(
            !path.split('/').any(|s| matches!(s, ".git" | ".symbiont")),
            "workspace control directories are unavailable"
        );
        Ok(())
    }

    fn parent(path: &Path, create: bool) -> anyhow::Result<(File, Vec<String>)> {
        anyhow::ensure!(path.is_absolute(), "workspace host parent must be absolute");
        let mut current = File::open("/")?;
        let mut missing = Vec::new();
        for component in path.components() {
            let component = match component {
                std::path::Component::RootDir => continue,
                std::path::Component::Normal(component) => component,
                _ => anyhow::bail!("workspace host parent cannot traverse directories"),
            };
            let name = component
                .to_str()
                .context("invalid workspace path encoding")?;
            if !missing.is_empty() {
                missing.push(name.to_owned());
                continue;
            }
            match open_at(&current, name, libc::O_RDONLY | libc::O_DIRECTORY, 0) {
                Ok(child) => current = child,
                Err(e)
                    if create
                        && e.downcast_ref::<std::io::Error>()
                            .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
                {
                    missing.push(name.to_owned())
                }
                Err(e) => return Err(e.context("cannot open workspace parent")),
            }
        }
        anyhow::ensure!(
            missing.len() <= 16,
            "workspace creation exceeds directory depth limit"
        );
        Ok((current, missing))
    }

    pub(super) fn prepare(
        boundary: &CommandBoundary,
        operation: &str,
        path: &str,
        value: Option<&str>,
    ) -> anyhow::Result<WorkspacePlan> {
        let (working_dir, configured_roots) = ceiling(boundary, false)?;
        anyhow::ensure!(
            working_dir == "/workspace",
            "workspace file tools require /workspace as working directory"
        );
        safe_path(path, operation == "search")?;
        let mut descriptor = serde_json::json!({"operation": operation,"path":path,"max_file_bytes":LIMIT+1,"max_write_bytes":LIMIT,"max_output_bytes":3*LIMIT+256,"host_mounts":[],"transport":"fixed_file_broker","read":[],"create":[],"update":[],"overwrite":false,"input_snapshots":true});
        let operation = match operation {
            "read_file" => {
                let host = host_path(boundary, path, false)?;
                let parent = directory(host.parent().context("missing workspace parent")?)?;
                let mut file = open_at(
                    &parent,
                    host.file_name()
                        .and_then(|s| s.to_str())
                        .context("missing file name")?,
                    libc::O_RDONLY,
                    0,
                )?;
                let bytes = snapshot(&mut file)?;
                descriptor["read"] = serde_json::json!([{"path":path,"bytes":bytes.len(),"sha256":hash(&bytes),"truncated":bytes.len()>LIMIT}]);
                let mut text =
                    String::from_utf8_lossy(&bytes[..bytes.len().min(LIMIT)]).into_owned();
                text.push('\n');
                if bytes.len() > LIMIT {
                    text.push_str("[file truncated at 32768 bytes]\n");
                }
                Operation::Read(text)
            }
            "search" => {
                let query = value.context("missing workspace query")?;
                anyhow::ensure!(
                    !query.is_empty() && query.len() <= LIMIT && !query.contains('\0'),
                    "invalid workspace query"
                );
                let mut search = Search::default();
                let requested = Path::new("/workspace").join(path);
                let mut roots = BTreeMap::new();
                if configured_roots.iter().any(|mount| {
                    mount
                        .split(':')
                        .nth(1)
                        .is_some_and(|destination| requested.starts_with(destination))
                }) {
                    roots.insert(path.to_owned(), host_path(boundary, path, false)?);
                }
                for mount in configured_roots {
                    let parts: Vec<_> = mount.split(':').collect();
                    let guest = Path::new(parts[1]);
                    if guest.starts_with(&requested) && guest.starts_with("/workspace") {
                        let relative = guest
                            .strip_prefix("/workspace")?
                            .to_str()
                            .context("invalid mount encoding")?;
                        if relative
                            .split('/')
                            .any(|name| matches!(name, ".git" | ".symbiont" | "target"))
                        {
                            continue;
                        }
                        let relative = if relative.is_empty() { "." } else { relative };
                        roots.insert(relative.into(), host_path(boundary, relative, false)?);
                    }
                }
                anyhow::ensure!(
                    !roots.is_empty(),
                    "workspace search has no configured mount ceiling"
                );
                for (relative, host) in roots {
                    let parent = directory(host.parent().context("missing search parent")?)?;
                    let file = open_at(
                        &parent,
                        host.file_name()
                            .and_then(|s| s.to_str())
                            .context("missing search root")?,
                        libc::O_RDONLY,
                        0,
                    )?;
                    search.walk(boundary, file, &relative, query, 0)?;
                    if search.limited {
                        break;
                    }
                }
                descriptor["read"] = serde_json::json!(search.inputs);
                descriptor["limited"] = serde_json::json!(search.limited);
                descriptor["skipped"] = serde_json::json!(search.skipped);
                Operation::Read(format!(
                    "{}[searched {} files; skipped {} entries; limited={}]\n",
                    search.text, search.files, search.skipped, search.limited
                ))
            }
            "edit_file" | "save_artifact" => {
                let bytes = value.context("missing workspace content")?.as_bytes();
                anyhow::ensure!(
                    bytes.len() <= LIMIT && !bytes.contains(&0),
                    "invalid workspace content"
                );
                let host = host_path(boundary, path, true)?;
                let (parent, missing) = parent(
                    host.parent().context("missing write parent")?,
                    operation == "save_artifact",
                )?;
                let name = host
                    .file_name()
                    .and_then(|s| s.to_str())
                    .context("missing write file name")?
                    .to_owned();
                let previous = if missing.is_empty() {
                    match open_at(&parent, &name, libc::O_RDWR, 0) {
                        Ok(mut file) => {
                            let old = snapshot(&mut file)?;
                            anyhow::ensure!(
                                old.len() <= LIMIT,
                                "existing workspace file exceeds edit budget"
                            );
                            Some((file, hash(&old)))
                        }
                        Err(e)
                            if e.downcast_ref::<std::io::Error>()
                                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
                        {
                            None
                        }
                        Err(e) => return Err(e),
                    }
                } else {
                    None
                };
                descriptor["write"] = serde_json::json!({"path":path,"bytes":bytes.len(),"sha256":hash(bytes),"parent_identity":identity(&parent)?,"create_directories":missing,"previous":previous.as_ref().map(|(f,h)|identity(f).map(|id|serde_json::json!({"identity":id,"sha256":h}))).transpose()?,"mode":if previous.is_some(){"in_place"}else{"create_no_replace"}});
                descriptor[if previous.is_some() {
                    "update"
                } else {
                    "create"
                }] = serde_json::json!([path]);
                descriptor["overwrite"] = serde_json::json!(previous.is_some());
                Operation::Write {
                    path: path.into(),
                    parent,
                    missing,
                    name,
                    previous,
                    content: bytes.to_vec(),
                }
            }
            _ => anyhow::bail!("unknown workspace file operation"),
        };
        Ok(WorkspacePlan {
            descriptor,
            operation: Mutex::new(Some(operation)),
        })
    }

    #[derive(Default)]
    struct Search {
        text: String,
        inputs: Vec<Value>,
        files: usize,
        entries: usize,
        bytes: usize,
        skipped: usize,
        limited: bool,
        seen: std::collections::HashSet<String>,
    }
    impl Search {
        fn walk(
            &mut self,
            boundary: &CommandBoundary,
            mut file: File,
            path: &str,
            query: &str,
            depth: usize,
        ) -> anyhow::Result<()> {
            if self.limited || !self.seen.insert(path.to_owned()) {
                return Ok(());
            }
            if depth > 16 || self.entries >= 10000 || self.files >= 512 || self.bytes >= 1024 * 1024
            {
                self.limited = true;
                return Ok(());
            }
            let info = file.metadata()?;
            if info.is_dir() {
                // /proc resolves the retained directory descriptor; children are
                // opened relative to that same descriptor without following links.
                let mut names = Vec::new();
                for entry in std::fs::read_dir(format!("/proc/self/fd/{}", file.as_raw_fd()))? {
                    self.entries += 1;
                    if self.entries > 10000 {
                        self.limited = true;
                        break;
                    }
                    names.push(entry?.file_name());
                }
                names.sort();
                for name in names {
                    let Some(name) = name.to_str() else {
                        self.skipped += 1;
                        continue;
                    };
                    if matches!(name, ".git" | ".symbiont" | "target") {
                        self.skipped += 1;
                        continue;
                    }
                    let relative = if path == "." {
                        name.into()
                    } else {
                        format!("{path}/{name}")
                    };
                    // Resolve every child against the most specific ceiling;
                    // overlapping mounts cannot reveal hidden parent contents.
                    let host = host_path(boundary, &relative, false)?;
                    let parent = directory(host.parent().context("missing search child parent")?)?;
                    match open_at(
                        &parent,
                        host.file_name()
                            .and_then(|s| s.to_str())
                            .context("invalid search child")?,
                        libc::O_RDONLY,
                        0,
                    ) {
                        Ok(child) => self.walk(boundary, child, &relative, query, depth + 1)?,
                        Err(_) => {
                            self.skipped += 1;
                        }
                    }
                    if self.limited {
                        break;
                    }
                }
            } else if info.is_file() && info.nlink() == 1 {
                let bytes = snapshot_up_to(&mut file, (LIMIT + 1).min(1024 * 1024 - self.bytes))?;
                self.files += 1;
                self.bytes += bytes.len();
                let truncated = bytes.len() > LIMIT || info.len() > bytes.len() as u64;
                self.inputs.push(serde_json::json!({"path":path,"bytes":bytes.len(),"sha256":hash(&bytes),"truncated":truncated}));
                if bytes.contains(&0) {
                    self.skipped += 1;
                    return Ok(());
                }
                for (i, line) in String::from_utf8_lossy(&bytes[..bytes.len().min(LIMIT)])
                    .lines()
                    .enumerate()
                {
                    if line.contains(query) {
                        let line = format!("{path}:{}: {line}\n", i + 1);
                        if self.text.len() + line.len() > LIMIT {
                            self.limited = true;
                            break;
                        }
                        self.text.push_str(&line);
                    }
                }
                if truncated || self.bytes >= 1024 * 1024 {
                    self.limited = true;
                }
            } else {
                self.skipped += 1;
            }
            Ok(())
        }
    }

    fn check_deadline(deadline: Instant) -> anyhow::Result<()> {
        anyhow::ensure!(
            Instant::now() < deadline,
            "workspace authorization expired before write"
        );
        Ok(())
    }
    pub(super) fn execute(operation: Operation, deadline: Instant) -> anyhow::Result<Value> {
        let (text, receipt) = match operation {
            Operation::Read(text) => (text, None),
            Operation::Write {
                path,
                mut parent,
                missing,
                name,
                previous,
                content,
            } => {
                for component in missing {
                    check_deadline(deadline)?;
                    let c = CString::new(component.as_str())?;
                    // SAFETY: retained parent and validated single component.
                    if unsafe { libc::mkdirat(parent.as_raw_fd(), c.as_ptr(), 0o700) } != 0 {
                        return Err(std::io::Error::last_os_error().into());
                    }
                    parent.sync_all()?;
                    parent = open_at(&parent, &component, libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
                }
                let mode = if let Some((mut file, expected)) = previous {
                    // The lock serializes independent broker plans for this inode.
                    // Other host writers need to honor flock for serialized edits.
                    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0
                    {
                        anyhow::bail!("workspace file is busy; prepare another edit");
                    }
                    let current = open_at(&parent, &name, libc::O_RDONLY, 0)?;
                    anyhow::ensure!(
                        identity(&current)? == identity(&file)?
                            && hash(&snapshot(&mut file)?) == expected,
                        "workspace file changed since authorization"
                    );
                    check_deadline(deadline)?;
                    file.seek(SeekFrom::Start(0))?;
                    file.write_all(&content)?;
                    file.set_len(content.len() as u64)?;
                    file.sync_all()?;
                    let current = open_at(&parent, &name, libc::O_RDONLY, 0)?;
                    anyhow::ensure!(
                        identity(&current)? == identity(&file)? && file.metadata()?.nlink() == 1,
                        "workspace pathname changed during edit; effect requires reconciliation"
                    );
                    anyhow::ensure!(
                        hash(&snapshot(&mut file)?) == hash(&content),
                        "workspace content changed during edit; effect requires reconciliation"
                    );
                    "in_place"
                } else {
                    let temporary = format!(".symbi-write-{}", uuid::Uuid::new_v4());
                    let old = CString::new(temporary.as_str())?;
                    let new = CString::new(name.as_str())?;
                    check_deadline(deadline)?;
                    let mut file = open_at(
                        &parent,
                        &temporary,
                        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                        0o600,
                    )?;
                    let published = (|| -> anyhow::Result<()> {
                        file.write_all(&content)?;
                        file.sync_all()?;
                        check_deadline(deadline)?;
                        // SAFETY: retained parent and broker-owned candidate. A
                        // competing target is never overwritten or followed.
                        if unsafe {
                            crate::sandbox::rename_noreplace_at(
                                parent.as_raw_fd(),
                                old.as_ptr(),
                                parent.as_raw_fd(),
                                new.as_ptr(),
                            )
                        } != 0
                        {
                            return Err(std::io::Error::last_os_error().into());
                        }
                        parent
                            .sync_all()
                            .context("workspace file created but durability is uncertain")?;
                        Ok(())
                    })();
                    unsafe {
                        libc::unlinkat(parent.as_raw_fd(), old.as_ptr(), 0);
                    }
                    published?;
                    "create_no_replace"
                };
                (
                    format!("Wrote {} bytes to /workspace/{path}", content.len()),
                    Some(
                        serde_json::json!({"path":path,"bytes":content.len(),"sha256":hash(&content),"mode":mode}),
                    ),
                )
            }
        };
        Ok(
            serde_json::json!({"status":"success","execution_status":"broker_completed","exit_code":null,"results":{"raw_output":text},"written_file":receipt}),
        )
    }
}
