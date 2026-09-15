//! Shared private snapshot reservations. Caller locks and durable worker records
//! retain charges until the data directory has actually been removed.
use crate::store::{open_at, read_records, validate_private};
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};
use uuid::Uuid;

const MAX_LEASES: usize = 512;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub max_snapshots: usize,
    /// Reserved payload bytes plus a caller-supplied bounded metadata allowance.
    pub reserved_bytes: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_snapshots: 16,
            reserved_bytes: 1024 * 1024 * 1024,
        }
    }
}
impl Limits {
    fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (1..=MAX_LEASES).contains(&self.max_snapshots) && self.reserved_bytes > 0,
            "invalid shared staging limits"
        );
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reservation {
    id: Uuid,
    reserved_bytes: u64,
}

/// Runtime ownership is not serialized into project configuration. Dropping it
/// attempts cleanup; a worker reference or failed removal keeps the reservation.
#[derive(Debug)]
pub struct Lease {
    root: PathBuf,
    id: Uuid,
    path: PathBuf,
    owner: Option<File>,
}
impl Lease {
    pub fn reserve(root: &Path, reserved_bytes: u64) -> anyhow::Result<Self> {
        anyhow::ensure!(reserved_bytes > 0, "staging reservation must be positive");
        let pool = Pool::open(root)?;
        pool.reap()?;
        let limits: Limits = pool.limits()?;
        let records = pool.reservations()?;
        let total = records
            .iter()
            .try_fold(reserved_bytes, |sum, (_, record)| {
                sum.checked_add(record.reserved_bytes)
                    .context("staging accounting overflow")
            })?;
        anyhow::ensure!(
            records.len() < limits.max_snapshots && total <= limits.reserved_bytes,
            "shared staging capacity exhausted: snapshots={}/{}, reserved_bytes={}/{}",
            records.len() + 1,
            limits.max_snapshots,
            total,
            limits.reserved_bytes
        );
        let id = Uuid::new_v4();
        let directory = pool.path.join(id.to_string());
        std::fs::DirBuilder::new().mode(0o700).create(&directory)?;
        let held = private_directory(&directory)?;
        let mut owner = open_at(
            &held,
            "reservation",
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
            0o600,
        )?;
        lock(&owner, libc::LOCK_SH)?;
        owner.write_all(&serde_json::to_vec(&Reservation { id, reserved_bytes })?)?;
        owner.sync_all()?;
        held.sync_all()?;
        pool.directory.sync_all()?;
        // No payload is created until the complete charge is durable.
        let lease = Self {
            root: root.to_owned(),
            id,
            path: directory.join("data"),
            owner: Some(owner),
        };
        let initialized = std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&lease.path)
            .and_then(|_| held.sync_all());
        drop(pool);
        initialized?;
        Ok(lease)
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn id(&self) -> Uuid {
        self.id
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.owner.take();
        // Failure is conservative: retain both directory and charge. A later
        // allocation or supervisor sweep will retry against durable ownership.
        let _ = reap(&self.root);
    }
}

/// Pins the caller's copies across durable worker registration. Once the record
/// is synced it protects these IDs even if both processes are killed.
pub(crate) fn pin(root: &Path, ids: &[Uuid]) -> anyhow::Result<Vec<File>> {
    crate::protocol::validate_staging_ids(ids)?;
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let pool = Pool::open(root)?;
    ids.iter()
        .map(|id| {
            let directory = private_directory(&pool.path.join(id.to_string()))?;
            let file = open_at(&directory, "reservation", libc::O_RDONLY, 0)?;
            validate_private(&file, false)?;
            lock(&file, libc::LOCK_SH)?;
            let record: Reservation = read_json(&file)?;
            anyhow::ensure!(
                record.id == *id && record.reserved_bytes > 0,
                "invalid staging reservation identity"
            );
            private_directory(&pool.path.join(id.to_string()).join("data"))?;
            Ok(file)
        })
        .collect()
}

/// Missing pools require no creation. Called during supervisor recovery and
/// admission, so caller SIGKILL cannot permanently leak an unreferenced copy.
pub fn reap(root: &Path) -> anyhow::Result<usize> {
    match std::fs::symlink_metadata(root.join("staging")) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e.into()),
        Ok(_) => (),
    }
    Pool::open(root)?.reap()
}

struct Pool {
    root: PathBuf,
    state: File,
    path: PathBuf,
    directory: File,
    _lock: File,
}
impl Pool {
    fn open(root: &Path) -> anyhow::Result<Self> {
        crate::protocol::socket_path(root)?;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(root)?;
        let state = private_directory(root)?;
        let mutex = open_at(&state, "staging.lock", libc::O_RDWR | libc::O_CREAT, 0o600)?;
        validate_private(&mutex, false)?;
        lock(&mutex, libc::LOCK_EX)?;
        let path = root.join("staging");
        match std::fs::DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => state.sync_all()?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.into()),
        }
        let directory = private_directory(&path)?;
        Ok(Self {
            root: root.to_owned(),
            state,
            path,
            directory,
            _lock: mutex,
        })
    }
    fn limits(&self) -> anyhow::Result<Limits> {
        let file = match open_at(&self.state, "staging.conf", libc::O_RDONLY, 0) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let limits = Limits::default();
                let mut file = open_at(
                    &self.state,
                    "staging.conf",
                    libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                    0o600,
                )?;
                file.write_all(&serde_json::to_vec(&limits)?)?;
                file.sync_all()?;
                self.state.sync_all()?;
                return Ok(limits);
            }
            Err(e) => return Err(e.into()),
        };
        validate_private(&file, false)?;
        let limits: Limits = read_json(&file)?;
        limits.validate()?;
        Ok(limits)
    }
    fn directories(&self) -> anyhow::Result<Vec<(Uuid, PathBuf)>> {
        let mut result = Vec::new();
        for entry in std::fs::read_dir(&self.path)? {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("invalid staging directory name"))?;
            let id = Uuid::parse_str(&name)?;
            anyhow::ensure!(name == id.to_string(), "noncanonical staging identity");
            result.push((id, entry.path()));
            anyhow::ensure!(
                result.len() <= MAX_LEASES,
                "staging directory capacity exceeded"
            );
        }
        Ok(result)
    }
    fn reservations(&self) -> anyhow::Result<Vec<(Uuid, Reservation)>> {
        self.directories()?
            .into_iter()
            .map(|(id, path)| {
                let directory = private_directory(&path)?;
                let file = open_at(&directory, "reservation", libc::O_RDONLY, 0)?;
                validate_private(&file, false)?;
                let record: Reservation = read_json(&file)?;
                anyhow::ensure!(
                    record.id == id && record.reserved_bytes > 0,
                    "invalid staging charge"
                );
                Ok((id, record))
            })
            .collect()
    }
    fn reap(&self) -> anyhow::Result<usize> {
        let retained: HashSet<Uuid> = read_records(&self.root, &self.state)?
            .into_iter()
            .flat_map(|record| record.staging)
            .collect();
        self.reap_unreferenced(&retained)
    }
    fn reap_unreferenced(&self, retained: &HashSet<Uuid>) -> anyhow::Result<usize> {
        let mut removed = 0;
        for (id, path) in self.directories()? {
            if retained.contains(&id) {
                continue;
            }
            let directory = private_directory(&path)?;
            let file = match open_at(&directory, "reservation", libc::O_RDONLY, 0) {
                Ok(file) => Some(file),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.into()),
            };
            if let Some(file) = &file {
                validate_private(file, false)?;
                if !try_exclusive(file)? {
                    continue;
                }
            }
            // Registration can finish after the initial record snapshot and
            // release its pin before this exclusive lock is acquired. Re-read
            // while exclusive ownership and the pool lock prohibit new pins.
            if read_records(&self.root, &self.state)?
                .iter()
                .any(|record| record.staging.contains(&id))
            {
                continue;
            }
            // A crash before the charge was durable can leave an empty or
            // partially written reservation, but payload must then be absent.
            let data_exists = match std::fs::symlink_metadata(path.join("data")) {
                Ok(_) => true,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
                Err(e) => return Err(e.into()),
            };
            if data_exists {
                let record: Reservation = read_json(
                    file.as_ref()
                        .context("uncharged staging data requires operator reconciliation")?,
                )?;
                anyhow::ensure!(
                    record.id == id && record.reserved_bytes > 0,
                    "invalid orphan staging charge"
                );
                private_directory(&path.join("data"))?;
                std::fs::remove_dir_all(path.join("data"))?;
                directory.sync_all()?;
            }
            // Keep the charge until data removal has been synced. The private
            // container never mounts this directory or the reservation file.
            if file.is_some() {
                std::fs::remove_file(path.join("reservation"))?;
            }
            std::fs::remove_dir(&path)?;
            self.directory.sync_all()?;
            removed += 1;
        }
        Ok(removed)
    }
}
fn private_directory(path: &Path) -> anyhow::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    validate_private(&file, true)?;
    // Staging data and accounting never permit even root-service traversal by
    // other identities; root-managed VM source transport needs its own broker.
    anyhow::ensure!(
        file.metadata()?.mode() & 0o077 == 0,
        "staging directories must be mode 0700"
    );
    Ok(file)
}
fn read_json<T: serde::de::DeserializeOwned>(file: &File) -> anyhow::Result<T> {
    // Each reader has its own open file description; never seek a shared owner.
    let mut bytes = Vec::new();
    file.take(4097).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 4096, "staging metadata exceeds 4 KiB");
    Ok(serde_json::from_slice(&bytes)?)
}
fn lock(file: &File, mode: i32) -> anyhow::Result<()> {
    // SAFETY: flock borrows a valid owned descriptor for the duration of this call.
    anyhow::ensure!(
        unsafe { libc::flock(file.as_raw_fd(), mode) } == 0,
        "staging lock failed: {}",
        std::io::Error::last_os_error()
    );
    Ok(())
}
fn try_exclusive(file: &File) -> anyhow::Result<bool> {
    // SAFETY: descriptor remains live; nonblocking flock never waits for a caller.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::WouldBlock {
        Ok(false)
    } else {
        Err(error.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Phase, Record, Store};
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn limits(root: &Path, bytes: u64, slots: usize) {
        std::fs::write(
            root.join("staging.conf"),
            serde_json::to_vec(&Limits {
                max_snapshots: slots,
                reserved_bytes: bytes,
            })
            .unwrap(),
        )
        .unwrap();
        std::fs::set_permissions(
            root.join("staging.conf"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }
    #[test]
    fn reservations_compete_and_live_copies_cannot_be_reaped() {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        limits(root.path(), 100, 2);
        let a = Lease::reserve(root.path(), 60).unwrap();
        std::fs::write(a.path().join("input"), b"retained").unwrap();
        assert!(Lease::reserve(root.path(), 41)
            .unwrap_err()
            .to_string()
            .contains("capacity exhausted"));
        let b = Lease::reserve(root.path(), 40).unwrap();
        assert_eq!(reap(root.path()).unwrap(), 0);
        assert!(Lease::reserve(root.path(), 1).is_err());
        drop(a);
        assert!(b.path().exists());
        let c = Lease::reserve(root.path(), 60).unwrap();
        drop((b, c));
        assert_eq!(
            std::fs::read_dir(root.path().join("staging"))
                .unwrap()
                .count(),
            0
        );
    }
    #[test]
    fn durable_uncertain_workers_retain_dead_callers_and_registration_pins() {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let store = Store::open(root.path()).unwrap().unwrap();
        let lease = Lease::reserve(root.path(), 100).unwrap();
        let id = lease.id();
        let path = lease.path().to_owned();
        let pins = pin(root.path(), &[id]).unwrap();
        drop(lease);
        assert!(path.exists());
        let worker = Uuid::new_v4();
        store
            .insert(&Record {
                lease: worker,
                name: format!("symbi-{worker}"),
                docker_binary: "/usr/bin/docker".into(),
                docker_environment: Default::default(),
                state: Phase::Uncertain,
                resources: Some(crate::admission::WorkerResources::vm(64, 1)),
                jail: None,
                staging: vec![id],
            })
            .unwrap();
        drop(pins);
        // Exercise a stale sweep snapshot taken before registration completed.
        // It must not delete the now durably referenced directory.
        let pool = Pool::open(root.path()).unwrap();
        assert_eq!(pool.reap_unreferenced(&HashSet::new()).unwrap(), 0);
        drop(pool);
        drop(store);
        assert_eq!(reap(root.path()).unwrap(), 0);
        assert!(path.exists());
        let store = Store::open(root.path()).unwrap().unwrap();
        store.remove(worker).unwrap();
        assert_eq!(reap(root.path()).unwrap(), 1);
        assert!(!path.exists());
        assert!(pin(root.path(), &[id]).is_err());
    }
    #[test]
    fn abandoned_preparation_is_reaped_and_unsafe_data_preserves_charge() {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut lease = Lease::reserve(root.path(), 100).unwrap();
        let data = lease.path().to_owned();
        lease.owner.take(); // Model process loss without running the destructor.
        assert_eq!(reap(root.path()).unwrap(), 1);
        assert!(!data.exists());
        let mut lease = Lease::reserve(root.path(), 100).unwrap();
        let data = lease.path().to_owned();
        std::fs::remove_dir(&data).unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("canary"), "protected").unwrap();
        symlink(outside.path(), &data).unwrap();
        lease.owner.take();
        assert!(reap(root.path()).is_err());
        assert!(data.parent().unwrap().join("reservation").exists());
        assert_eq!(
            std::fs::read_to_string(outside.path().join("canary")).unwrap(),
            "protected"
        );
        assert!(Lease::reserve(root.path(), 1).is_err());
    }
    #[test]
    fn interrupted_registration_and_unsafe_configuration_refuse_without_payload() {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let pool = Pool::open(root.path()).unwrap();
        let directory = pool.path.join(Uuid::new_v4().to_string());
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        let file = private_directory(&directory).unwrap();
        open_at(&file, "reservation", libc::O_WRONLY | libc::O_CREAT, 0o600)
            .unwrap()
            .write_all(b"{partial")
            .unwrap();
        assert_eq!(pool.reap().unwrap(), 1);
        drop(pool);
        limits(root.path(), 0, 1);
        assert!(Lease::reserve(root.path(), 1).is_err());
        limits(root.path(), 100, 1);
        std::fs::set_permissions(
            root.path().join("staging.conf"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert!(Lease::reserve(root.path(), 1).is_err());
    }
}
