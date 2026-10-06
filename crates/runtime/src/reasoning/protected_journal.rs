//! Private, synchronously persisted, signed journals for governed sessions.

use super::loop_types::{JournalEntry, JournalError, JournalWriter, LoopEvent, TerminationReason};
use crate::types::AgentId;
use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use zeroize::Zeroizing;

const MAX_RECORD: usize = 1024 * 1024;
const MAX_FILE: u64 = 64 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    run_id: Option<uuid::Uuid>,
    previous_hash: String,
    entry: JournalEntry,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedRecord {
    payload: Box<serde_json::value::RawValue>,
    signature: String,
}
struct State {
    file: File,
    signer: SigningKey,
    agent_id: AgentId,
    run_id: Option<uuid::Uuid>,
    sequence: u64,
    bytes: u64,
    previous: String,
    failed: bool,
    unconfirmed: bool,
}

/// One exclusive writer per fresh run. Files and the persistent signing key
/// stay in a private runtime-owned directory outside all worker mount grants.
/// Signature verification requires a separately trusted public key.
pub struct ProtectedJournal {
    path: PathBuf,
    public_key: [u8; 32],
    state: Arc<Mutex<State>>,
    sequence: Arc<AtomicU64>,
}

/// Authenticated records plus an explicitly unauthenticated final fragment.
/// This is evidence for inspection, never authority to resume execution.
pub struct VerifiedPrefix {
    pub entries: Vec<JournalEntry>,
    /// SHA-256 of the complete stable snapshot, including any unauthenticated tail.
    pub snapshot_sha256: String,
    pub verified_bytes: u64,
    pub unverified_tail_bytes: u64,
}

impl ProtectedJournal {
    pub fn create(parent: &Path, agent_id: AgentId) -> Result<Self, JournalError> {
        Self::create_named(parent, agent_id, None, format!("{agent_id}.jsonl"))
    }

    /// Create a distinct journal for a repeat invocation of the same principal.
    pub fn create_run(
        parent: &Path,
        agent_id: AgentId,
        run_id: uuid::Uuid,
    ) -> Result<Self, JournalError> {
        Self::create_named(
            parent,
            agent_id,
            Some(run_id),
            format!("{agent_id}.{run_id}.jsonl"),
        )
    }

    fn create_named(
        parent: &Path,
        agent_id: AgentId,
        run_id: Option<uuid::Uuid>,
        filename: String,
    ) -> Result<Self, JournalError> {
        private_directory(parent).map_err(failure)?;
        let signer = load_key(parent).map_err(failure)?;
        let public_key = signer.verifying_key().to_bytes();
        let path = parent.join(filename);
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
            .map_err(failure)?;
        file.sync_all().map_err(failure)?;
        File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(failure)?;
        Ok(Self {
            path,
            public_key,
            sequence: Arc::new(AtomicU64::new(0)),
            state: Arc::new(Mutex::new(State {
                file,
                signer,
                agent_id,
                run_id,
                sequence: 0,
                bytes: 0,
                previous: "0".repeat(64),
                failed: false,
                unconfirmed: false,
            })),
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn public_key(&self) -> [u8; 32] {
        self.public_key
    }

    /// Verify every persisted record against an externally trusted key.
    /// Missing terminal records remain visible to the caller as interrupted
    /// evidence; they are never silently converted into completed sessions.
    pub fn verify(path: &Path, public_key: &[u8; 32]) -> Result<Vec<JournalEntry>, JournalError> {
        Self::verify_inner(path, public_key, None, false).map(|prefix| prefix.entries)
    }

    /// Verify an expected invocation identity as well as signatures and chain.
    /// Legacy version-one journals have no signed run ID and cannot satisfy this check.
    pub fn verify_run(
        path: &Path,
        public_key: &[u8; 32],
        run_id: uuid::Uuid,
    ) -> Result<Vec<JournalEntry>, JournalError> {
        Self::verify_inner(path, public_key, Some(run_id), false).map(|prefix| prefix.entries)
    }

    /// Inspect a stable invocation snapshot. Only an unterminated final fragment
    /// may be excluded; a complete malformed or invalidly signed record fails.
    /// The original file is never changed, and its tail is not called authentic.
    pub fn verify_run_prefix(
        path: &Path,
        public_key: &[u8; 32],
        run_id: uuid::Uuid,
    ) -> Result<VerifiedPrefix, JournalError> {
        Self::verify_inner(path, public_key, Some(run_id), true)
    }

    fn verify_inner(
        path: &Path,
        public_key: &[u8; 32],
        expected_run: Option<uuid::Uuid>,
        allow_fragment: bool,
    ) -> Result<VerifiedPrefix, JournalError> {
        let key = VerifyingKey::from_bytes(public_key).map_err(failure)?;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(path)
            .map_err(failure)?;
        let before = file.metadata().map_err(failure)?;
        if !before.is_file() || before.len() > MAX_FILE {
            return Err(failure(
                "journal must be a regular file within its size limit",
            ));
        }
        let mut reader = BufReader::new(file);
        let mut entries = Vec::new();
        let mut previous = "0".repeat(64);
        let mut principal = None;
        let mut run_identity = None;
        let mut total = 0u64;
        let mut unverified_tail_bytes = 0;
        let mut snapshot_hash = Sha256::new();
        loop {
            let mut line = Vec::new();
            let size = (&mut reader)
                .take((MAX_RECORD + 1) as u64)
                .read_until(b'\n', &mut line)
                .map_err(failure)?;
            if size == 0 {
                break;
            }
            total = total.saturating_add(size as u64);
            snapshot_hash.update(&line);
            if total > MAX_FILE || size > MAX_RECORD {
                return Err(failure("incomplete or oversized signed journal record"));
            }
            if line.last() != Some(&b'\n') {
                if !allow_fragment {
                    return Err(failure("incomplete signed journal record"));
                }
                unverified_tail_bytes = size as u64;
                break;
            }
            let record: SignedRecord = serde_json::from_slice(&line).map_err(failure)?;
            let encoded = record.payload.get().as_bytes();
            let signature = STANDARD.decode(record.signature).map_err(failure)?;
            let signature = Signature::from_slice(&signature).map_err(failure)?;
            key.verify_strict(encoded, &signature).map_err(failure)?;
            let payload: Payload = serde_json::from_str(record.payload.get()).map_err(failure)?;
            let valid_version =
                matches!((payload.version, payload.run_id), (1, None) | (2, Some(_)));
            if !valid_version
                || expected_run.is_some_and(|id| payload.run_id != Some(id))
                || run_identity.is_some_and(|id| id != payload.run_id)
                || payload.previous_hash != previous
                || payload.entry.sequence != entries.len() as u64
                || principal.is_some_and(|id| id != payload.entry.agent_id)
            {
                return Err(failure("signed journal chain or principal mismatch"));
            }
            principal = Some(payload.entry.agent_id);
            run_identity = Some(payload.run_id);
            previous = hex::encode(Sha256::digest(&line));
            entries.push(payload.entry);
        }
        if entries.is_empty() {
            return Err(failure("signed journal has no records"));
        }
        let after = reader.get_ref().metadata().map_err(failure)?;
        if before.len() != after.len()
            || total != before.len()
            || before.mtime() != after.mtime()
            || before.mtime_nsec() != after.mtime_nsec()
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
        {
            return Err(failure(
                "journal changed during verification; inspect a stable snapshot",
            ));
        }
        Ok(VerifiedPrefix {
            entries,
            snapshot_sha256: hex::encode(snapshot_hash.finalize()),
            verified_bytes: total - unverified_tail_bytes,
            unverified_tail_bytes,
        })
    }
}

#[async_trait::async_trait]
impl JournalWriter for ProtectedJournal {
    fn audit_reference(&self) -> Option<super::run_audit::RunAuditReference> {
        let run_id = self.state.lock().ok()?.run_id?;
        Some(super::run_audit::RunAuditReference {
            run_id,
            path: self.path.clone(),
            public_key: hex::encode(self.public_key),
        })
    }
    async fn append(&self, mut entry: JournalEntry) -> Result<(), JournalError> {
        let state = self.state.clone();
        let sequence = self.sequence.clone();
        tokio::task::spawn_blocking(move || {
            let mut state = state
                .lock()
                .map_err(|_| failure("journal writer poisoned"))?;
            if state.failed {
                return Err(failure("journal writer stopped after a failed append"));
            }
            if state.unconfirmed && starts_effect(&entry.event) {
                return Err(failure(
                    "execution stopped after an unconfirmed outcome; reconciliation is required",
                ));
            }
            let result = (|| {
                if entry.agent_id != state.agent_id {
                    return Err(failure("journal principal mismatch"));
                }
                if state.file.metadata().map_err(failure)?.len() != state.bytes {
                    return Err(failure("journal length changed outside its writer"));
                }
                entry.sequence = state.sequence;
                let unconfirmed = has_unconfirmed_outcome(&entry.event);
                let payload = Payload {
                    version: if state.run_id.is_some() { 2 } else { 1 },
                    run_id: state.run_id,
                    previous_hash: state.previous.clone(),
                    entry,
                };
                // Serialize once, then sign and persist these exact bytes. In
                // particular, typed f32 JSON and serde_json::Value can differ.
                let payload = serde_json::value::RawValue::from_string(
                    super::prepared::canonical_json(
                        &serde_json::to_value(payload).map_err(failure)?,
                    )
                    .map_err(failure)?,
                )
                .map_err(failure)?;
                let signature =
                    STANDARD.encode(state.signer.sign(payload.get().as_bytes()).to_bytes());
                let mut encoded =
                    serde_json::to_vec(&SignedRecord { payload, signature }).map_err(failure)?;
                encoded.push(b'\n');
                if encoded.len() > MAX_RECORD
                    || state.bytes.saturating_add(encoded.len() as u64) > MAX_FILE
                {
                    return Err(failure("signed journal capacity exceeded"));
                }
                state.file.write_all(&encoded).map_err(failure)?;
                state.file.sync_data().map_err(failure)?;
                state.bytes += encoded.len() as u64;
                state.sequence += 1;
                sequence.store(state.sequence, Ordering::Release);
                state.previous = hex::encode(Sha256::digest(&encoded));
                state.unconfirmed |= unconfirmed;
                Ok(())
            })();
            if result.is_err() {
                state.failed = true;
            }
            result
        })
        .await
        .map_err(failure)?
    }
    async fn next_sequence(&self) -> u64 {
        self.sequence.load(Ordering::Acquire)
    }
}

fn starts_effect(event: &LoopEvent) -> bool {
    matches!(
        event,
        LoopEvent::Started { .. }
            | LoopEvent::ToolDispatchStarted { .. }
            | LoopEvent::DelegationStarted { .. }
            | LoopEvent::ResponseDeliveryStarted { .. }
            | LoopEvent::InferenceRequested { .. }
            | LoopEvent::BudgetReservationStarted { .. }
            | LoopEvent::DirectInferenceRequested { .. }
            | LoopEvent::ToolEffect {
                effect: super::effect_journal::ToolEffect::NetworkRequestStarted { .. }
                    | super::effect_journal::ToolEffect::FilePublicationPrepared { .. },
                ..
            }
    )
}

fn has_unconfirmed_outcome(event: &LoopEvent) -> bool {
    match event {
        LoopEvent::ToolEffect {
            effect:
                super::effect_journal::ToolEffect::FilePublicationFinished {
                    confirmed, error, ..
                },
            ..
        } => !confirmed || error.is_some(),
        LoopEvent::ToolDispatchFinished { is_error, .. } => *is_error,
        LoopEvent::ResponseDeliveryFinished { confirmed, .. } => !confirmed,
        LoopEvent::DelegationFinished { reason, .. }
        | LoopEvent::DirectInferenceFinished { reason, .. }
        | LoopEvent::Terminated { reason, .. } => !matches!(reason, TerminationReason::Completed),
        LoopEvent::ToolEffect {
            effect:
                super::effect_journal::ToolEffect::NetworkRequestFinished {
                    status,
                    response_hash,
                    error,
                    ..
                },
            ..
        } => status.is_none() || response_hash.is_none() || error.is_some(),
        _ => false,
    }
}

pub(crate) fn private_directory(path: &Path) -> Result<(), String> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|e| e.to_string())?;
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    // SAFETY: geteuid has no memory-safety preconditions.
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err("journal directory must be private and runtime-owned".into());
    }
    Ok(())
}

pub(crate) fn load_key(parent: &Path) -> Result<SigningKey, String> {
    let path = parent.join("audit-signing.key");
    if !path.try_exists().map_err(|e| e.to_string())? {
        let mut bytes = Zeroizing::new([0u8; 32]);
        rand::rngs::OsRng.fill_bytes(bytes.as_mut());
        let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        temp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        temp.write_all(bytes.as_ref()).map_err(|e| e.to_string())?;
        temp.as_file().sync_all().map_err(|e| e.to_string())?;
        match temp.persist_noclobber(&path) {
            Ok(_) => File::open(parent)
                .and_then(|dir| dir.sync_all())
                .map_err(|e| e.to_string())?,
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.error.to_string()),
        }
    }
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    // SAFETY: geteuid has no memory-safety preconditions.
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
        || metadata.len() != 32
    {
        return Err("invalid protected journal signing key".into());
    }
    let mut bytes = Zeroizing::new([0u8; 32]);
    file.read_exact(bytes.as_mut()).map_err(|e| e.to_string())?;
    Ok(SigningKey::from_bytes(&bytes))
}

fn failure(error: impl std::fmt::Display) -> JournalError {
    JournalError::WriteFailed(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrent_repeat_runs_share_one_key_and_never_replace_a_run() {
        let root = tempfile::tempdir().unwrap();
        let principal = AgentId::new();
        let path = root.path().join("journals");
        std::thread::scope(|scope| {
            let tasks: Vec<_> = (0..8)
                .map(|_| {
                    let path = &path;
                    scope.spawn(move || {
                        let id = uuid::Uuid::new_v4();
                        let journal = ProtectedJournal::create_run(path, principal, id).unwrap();
                        assert!(ProtectedJournal::create_run(path, principal, id).is_err());
                        (journal.path().to_owned(), journal.public_key())
                    })
                })
                .collect();
            let records: Vec<_> = tasks.into_iter().map(|t| t.join().unwrap()).collect();
            assert!(records.iter().all(|r| r.1 == records[0].1));
            assert_eq!(
                records
                    .iter()
                    .map(|r| &r.0)
                    .collect::<std::collections::HashSet<_>>()
                    .len(),
                8
            );
        });
    }
    #[tokio::test]
    async fn signed_run_identity_rejects_substitution_and_preserves_legacy_verification() {
        use super::super::loop_types::LoopEvent;
        let root = tempfile::tempdir().unwrap();
        let audit = root.path().join("private-audit");
        let agent_id = AgentId::new();
        let run_id = uuid::Uuid::new_v4();
        let entry = JournalEntry {
            sequence: 0,
            timestamp: chrono::Utc::now(),
            agent_id,
            iteration: 0,
            event: LoopEvent::ObservationsCollected {
                iteration: 0,
                observation_count: 0,
            },
        };
        let journal = ProtectedJournal::create_run(&audit, agent_id, run_id).unwrap();
        journal.append(entry.clone()).await.unwrap();
        let key = journal.public_key();
        assert_eq!(
            ProtectedJournal::verify_run(journal.path(), &key, run_id)
                .unwrap()
                .len(),
            1
        );
        assert!(ProtectedJournal::verify_run(journal.path(), &key, uuid::Uuid::new_v4()).is_err());
        let copied = root.path().join("renamed.jsonl");
        std::fs::copy(journal.path(), &copied).unwrap();
        assert!(ProtectedJournal::verify_run(&copied, &key, uuid::Uuid::new_v4()).is_err());
        let mut record: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&copied).unwrap()).unwrap();
        record["payload"]["run_id"] = serde_json::json!(uuid::Uuid::new_v4());
        std::fs::write(&copied, format!("{}\n", record)).unwrap();
        assert!(ProtectedJournal::verify(&copied, &key).is_err());

        let legacy = ProtectedJournal::create(&audit, agent_id).unwrap();
        legacy.append(entry).await.unwrap();
        assert_eq!(
            ProtectedJournal::verify(legacy.path(), &key).unwrap().len(),
            1
        );
        assert!(ProtectedJournal::verify_run(legacy.path(), &key, run_id).is_err());
    }
}
