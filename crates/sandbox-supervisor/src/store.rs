//! Private, durable lease records. No payload argv or worker secrets are stored.
use crate::protocol::{Create, LABEL};
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    ffi::CString,
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};

const MAX_RECORD: usize = 128 * 1024;
pub const MAX_RECORDS: usize = crate::admission::MAX_WORKERS;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub enum Phase {
    Creating,
    Created {
        container_id: String,
    },
    /// No acknowledgement proves the daemon finished creation. Absence is
    /// insufficient to delete this tombstone: the operation can arrive later.
    Uncertain,
    VmCreating,
    VmCreated {
        pid: u32,
        start_ticks: u64,
        boot_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub lease: uuid::Uuid,
    pub name: String,
    pub docker_binary: PathBuf,
    pub docker_environment: HashMap<String, String>,
    pub state: Phase,
    #[serde(default)]
    pub resources: Option<crate::admission::WorkerResources>,
    #[serde(default)]
    pub jail: Option<crate::host::JailLease>,
    #[serde(default)]
    pub staging: Vec<uuid::Uuid>,
}
impl Record {
    pub fn from_request(request: &Create) -> Self {
        Self {
            lease: request.lease,
            name: request.name.clone(),
            docker_binary: request.docker_binary.clone(),
            docker_environment: request.docker_environment.clone(),
            state: Phase::Creating,
            resources: Some(request.resources),
            jail: None,
            staging: request.staging.clone(),
        }
    }
    pub fn from_vm(request: &crate::protocol::CreateVm) -> Self {
        // Keep the legacy artifact field names on disk for existing container
        // records; the state variant determines the actual backend.
        Self {
            lease: request.lease,
            name: format!("symbi-{}", request.lease),
            docker_binary: request.binary.clone(),
            docker_environment: HashMap::new(),
            state: Phase::VmCreating,
            resources: Some(crate::admission::WorkerResources::vm(
                request.memory_mib,
                request.vcpus,
            )),
            jail: None,
            staging: Vec::new(),
        }
    }
    pub fn validate(&self) -> anyhow::Result<()> {
        crate::protocol::validate_staging_ids(&self.staging)?;
        if let Some(jail) = &self.jail {
            jail.validate()?;
            if !matches!(self.state, Phase::VmCreating | Phase::VmCreated { .. }) {
                anyhow::bail!("jail metadata requires a VM lease");
            }
        }
        if let Some(resources) = self.resources {
            resources.validate()?;
        }
        if self.name != format!("symbi-{}", self.lease) || !self.docker_binary.is_absolute() {
            anyhow::bail!("invalid lease identity or Docker client path");
        }
        crate::protocol::validate_daemon_environment(&self.docker_environment)?;
        if let Phase::Created { container_id } = &self.state {
            if !valid_container_id(container_id) {
                anyhow::bail!("invalid recorded container ID");
            }
        }
        if matches!(self.state, Phase::VmCreating | Phase::VmCreated { .. })
            && !self.docker_environment.is_empty()
        {
            anyhow::bail!("VM lease must not retain ambient environment");
        }
        if let Phase::VmCreated {
            pid,
            start_ticks,
            boot_id,
        } = &self.state
        {
            if *pid <= 1 || *start_ticks == 0 || uuid::Uuid::parse_str(boot_id).is_err() {
                anyhow::bail!("invalid VM process identity");
            }
        }
        Ok(())
    }
    pub fn expected_label(&self) -> (String, String) {
        (LABEL.into(), self.lease.to_string())
    }
}

pub fn valid_container_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub struct Store {
    pub root: PathBuf,
    directory: File,
    _lock: File,
    admission: crate::admission::AdmissionLimits,
}
impl Store {
    pub(crate) fn admission_limits(&self) -> crate::admission::AdmissionLimits {
        self.admission.clone()
    }
    /// The file lock admits one service per private state directory. A caller
    /// finding an existing service must connect to it, never unlink its socket.
    pub fn open(root: &Path) -> anyhow::Result<Option<Self>> {
        crate::protocol::socket_path(root)?;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(root)?;
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(root)?;
        validate_private(&directory, true)?;
        let lock = open_at(
            &directory,
            "supervisor.lock",
            libc::O_RDWR | libc::O_CREAT,
            0o600,
        )?;
        validate_private(&lock, false)?;
        // SAFETY: flock borrows this valid open descriptor; the File retains it.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::WouldBlock {
                return Ok(None);
            }
            return Err(error.into());
        }
        let mut store = Self {
            root: root.to_owned(),
            directory,
            _lock: lock,
            admission: crate::admission::AdmissionLimits::default(),
        };
        store.admission = store.load_admission()?;
        // The exclusive lock proves no other supervisor owns these temporary
        // files. Crashed writers must not leave worker secrets on disk forever.
        for entry in std::fs::read_dir(root)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if name.starts_with(".environment-") || name.starts_with(".write-") {
                let file = open_at(&store.directory, name, libc::O_RDONLY, 0)?;
                validate_private(&file, false)?;
                unlink_at(&store.directory, name)?;
            }
        }
        store.directory.sync_all()?;
        Ok(Some(store))
    }

    pub fn check_identity(&self) -> anyhow::Result<()> {
        let current = std::fs::symlink_metadata(&self.root)?;
        let held = self.directory.metadata()?;
        if !current.is_dir() || current.dev() != held.dev() || current.ino() != held.ino() {
            anyhow::bail!("supervisor state directory was replaced");
        }
        validate_private(&self.directory, true)
    }

    pub fn records(&self) -> anyhow::Result<Vec<Record>> {
        self.check_identity()?;
        read_records(&self.root, &self.directory)
    }

    pub fn insert(&self, record: &Record) -> anyhow::Result<()> {
        let records = self.records()?;
        if records.len() >= MAX_RECORDS {
            anyhow::bail!("durable lease capacity exhausted");
        }
        if records.iter().any(|r| r.lease == record.lease) {
            anyhow::bail!("lease identity already registered");
        }
        let resources = record
            .resources
            .ok_or_else(|| anyhow::anyhow!("new workers require resource admission metadata"))?;
        self.admission
            .admit(records.iter().map(|record| record.resources), resources)?;
        self.write(record)
    }

    fn load_admission(&self) -> anyhow::Result<crate::admission::AdmissionLimits> {
        const NAME: &str = "admission.conf";
        let file = match open_at(&self.directory, NAME, libc::O_RDONLY, 0) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let defaults = crate::admission::AdmissionLimits::default();
                let mut file = open_at(
                    &self.directory,
                    NAME,
                    libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                    0o600,
                )?;
                file.write_all(&serde_json::to_vec(&defaults)?)?;
                file.sync_all()?;
                self.directory.sync_all()?;
                return Ok(defaults);
            }
            Err(error) => return Err(error.into()),
        };
        validate_private(&file, false)?;
        let mut bytes = Vec::new();
        file.take(4097).read_to_end(&mut bytes)?;
        if bytes.len() > 4096 {
            anyhow::bail!("admission configuration exceeds 4 KiB");
        }
        let limits: crate::admission::AdmissionLimits = serde_json::from_slice(&bytes)?;
        limits.validate()?;
        Ok(limits)
    }

    pub fn write(&self, record: &Record) -> anyhow::Result<()> {
        self.check_identity()?;
        record.validate()?;
        let bytes = serde_json::to_vec(record)?;
        if bytes.len() > MAX_RECORD {
            anyhow::bail!("lease record exceeds size limit");
        }
        let temporary = format!(".write-{}", uuid::Uuid::new_v4());
        let destination = format!("{}.json", record.lease);
        let result = (|| -> anyhow::Result<()> {
            let mut file = open_at(
                &self.directory,
                &temporary,
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                0o600,
            )?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            let from = CString::new(temporary.as_str())?;
            let to = CString::new(destination)?;
            // SAFETY: both names are NUL-terminated and relative to an owned
            // directory descriptor; rename never follows a destination symlink.
            if unsafe {
                libc::renameat(
                    self.directory.as_raw_fd(),
                    from.as_ptr(),
                    self.directory.as_raw_fd(),
                    to.as_ptr(),
                )
            } != 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            self.directory.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = unlink_at(&self.directory, &temporary);
        }
        result
    }

    pub fn remove(&self, lease: uuid::Uuid) -> anyhow::Result<()> {
        self.check_identity()?;
        unlink_at(&self.directory, &format!("{lease}.json"))?;
        self.directory.sync_all()?;
        Ok(())
    }
}

pub(crate) fn read_records(root: &Path, directory: &File) -> anyhow::Result<Vec<Record>> {
    let mut records = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let filename = entry.file_name();
        let Some(name) = filename.to_str() else {
            continue;
        };
        let Some(id) = name.strip_suffix(".json") else {
            continue;
        };
        let lease = uuid::Uuid::parse_str(id).context("invalid file in lease store")?;
        if id != lease.to_string() {
            anyhow::bail!("non-canonical lease filename");
        }
        let file = open_at(directory, name, libc::O_RDONLY, 0)?;
        validate_private(&file, false)?;
        let mut bytes = Vec::new();
        file.take((MAX_RECORD + 1) as u64).read_to_end(&mut bytes)?;
        if bytes.len() > MAX_RECORD {
            anyhow::bail!("lease record exceeds size limit");
        }
        let record: Record =
            serde_json::from_slice(&bytes).context("invalid durable lease record")?;
        record.validate()?;
        if record.lease != lease {
            anyhow::bail!("lease record identity mismatch");
        }
        records.push(record);
        if records.len() > MAX_RECORDS {
            anyhow::bail!("lease store capacity exceeded");
        }
    }
    Ok(records)
}

pub(crate) fn validate_private(file: &File, directory: bool) -> anyhow::Result<()> {
    let metadata = file.metadata()?;
    // SAFETY: geteuid has no arguments or memory preconditions.
    let uid = unsafe { libc::geteuid() };
    if metadata.uid() != uid
        // A root-managed service permits traversal to its client-owned socket;
        // directory listing and all lease-file access remain private to root.
        || metadata.mode() & if directory && uid == 0 { 0o066 } else { 0o077 } != 0
        || (directory && !metadata.is_dir())
        || (!directory && (!metadata.is_file() || metadata.nlink() != 1))
    {
        anyhow::bail!("supervisor storage must be private and owned by the runtime user (mode {:o}, owner {}, effective user {}, links {}, directory {})", metadata.mode(), metadata.uid(), uid, metadata.nlink(), directory);
    }
    Ok(())
}

pub(crate) fn open_at(
    directory: &File,
    name: &str,
    flags: i32,
    mode: libc::mode_t,
) -> std::io::Result<File> {
    let name = CString::new(name)?;
    // SAFETY: the valid directory descriptor and NUL-terminated relative name
    // are borrowed for this call. A successful returned descriptor is owned.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            mode,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: openat returned a fresh descriptor, transferred exactly once.
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn unlink_at(directory: &File, name: &str) -> std::io::Result<()> {
    let name = CString::new(name)?;
    // SAFETY: descriptor and NUL-terminated name stay live through this call.
    if unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn record() -> Record {
        let lease = uuid::Uuid::new_v4();
        Record {
            lease,
            name: format!("symbi-{lease}"),
            docker_binary: "/usr/bin/docker".into(),
            docker_environment: HashMap::new(),
            state: Phase::Creating,
            resources: Some(crate::admission::WorkerResources::vm(64, 1)),
            jail: None,
            staging: Vec::new(),
        }
    }

    #[test]
    fn durable_reload_lock_and_identity_validation() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let store = Store::open(dir.path()).unwrap().unwrap();
        assert!(Store::open(dir.path()).unwrap().is_none());
        let mut record = record();
        store.insert(&record).unwrap();
        assert!(store.insert(&record).is_err());
        record.state = Phase::Created {
            container_id: "a".repeat(64),
        };
        store.write(&record).unwrap();
        drop(store);
        let store = Store::open(dir.path()).unwrap().unwrap();
        assert!(
            matches!(&store.records().unwrap()[0].state, Phase::Created { container_id } if container_id == &"a".repeat(64))
        );
        store.remove(record.lease).unwrap();
        assert!(store.records().unwrap().is_empty());
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(store.check_identity().is_err());
    }

    #[test]
    fn rejects_symlinks_hardlinks_and_malformed_records() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let alias = dir.path().join("alias");
        symlink(dir.path(), &alias).unwrap();
        assert!(Store::open(&alias).is_err());
        let store = Store::open(dir.path()).unwrap().unwrap();
        let record = record();
        let path = dir.path().join(format!("{}.json", record.lease));
        let target = tempfile::NamedTempFile::new().unwrap();
        symlink(target.path(), &path).unwrap();
        assert!(store.records().is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::hard_link(target.path(), &path).unwrap();
        assert!(store.records().is_err());
        std::fs::remove_file(&path).unwrap();
        store.insert(&record).unwrap();
        std::fs::write(&path, b"truncated").unwrap();
        assert!(store.records().is_err());
    }

    #[test]
    fn recovery_removes_private_temporary_secrets_but_preserves_leases() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let store = Store::open(dir.path()).unwrap().unwrap();
        let record = record();
        store.insert(&record).unwrap();
        let path = dir.path().join(".environment-stale");
        std::fs::write(&path, b"synthetic-secret").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        drop(store);
        let store = Store::open(dir.path()).unwrap().unwrap();
        assert!(!path.exists());
        assert_eq!(store.records().unwrap().len(), 1);
    }

    #[test]
    fn durable_records_exclude_payload_and_worker_environment() {
        let record = record();
        let request = Create {
            staging: Vec::new(),
            version: crate::protocol::VERSION,
            implementation: crate::protocol::IMPLEMENTATION.into(),
            lease: record.lease,
            name: record.name,
            docker_binary: record.docker_binary,
            docker_environment: record.docker_environment,
            arguments: vec!["synthetic-payload-secret".into()],
            environment: HashMap::from([("SECRET".into(), "synthetic-worker-secret".into())]),
            lifetime_ms: 1000,
            startup_ms: 1000,
            resources: crate::admission::WorkerResources::vm(64, 1),
        };
        let serialized = serde_json::to_string(&Record::from_request(&request)).unwrap();
        assert!(!serialized.contains("synthetic"));
        assert!(!serialized.contains("SECRET"));
    }

    #[test]
    fn admission_survives_restart_and_keeps_uncertain_or_legacy_capacity() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut store = Store::open(dir.path()).unwrap().unwrap();
        let limits = crate::admission::AdmissionLimits {
            max_workers: 1,
            memory_bytes: 64 * 1024 * 1024,
            cpu_nanos: 1_000_000_000,
        };
        std::fs::write(
            dir.path().join("admission.conf"),
            serde_json::to_vec(&limits).unwrap(),
        )
        .unwrap();
        store.admission = store.load_admission().unwrap();
        let mut first = record();
        store.insert(&first).unwrap();
        assert!(store.insert(&record()).is_err());
        first.state = Phase::Uncertain;
        store.write(&first).unwrap();
        drop(store);
        let store = Store::open(dir.path()).unwrap().unwrap();
        assert!(store.insert(&record()).is_err());
        store.remove(first.lease).unwrap();
        let second = record();
        store.insert(&second).unwrap();
        store.remove(second.lease).unwrap();
        first.resources = None; // An older lease cannot silently count as zero.
        store.write(&first).unwrap();
        assert!(store.insert(&record()).is_err());
    }

    #[test]
    fn admission_configuration_rejects_unsafe_files_and_invalid_limits() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let store = Store::open(dir.path()).unwrap().unwrap();
        let path = dir.path().join("admission.conf");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::write(
            &path,
            br#"{"max_workers":0,"memory_bytes":1,"cpu_nanos":1}"#,
        )
        .unwrap();
        assert!(store.load_admission().is_err());
        std::fs::remove_file(&path).unwrap();
        let target = tempfile::NamedTempFile::new().unwrap();
        symlink(target.path(), &path).unwrap();
        assert!(store.load_admission().is_err());
    }
}
