//! Persistent invocation claims. Only the process that durably creates a claim
//! can execute it. Existing, interrupted and malformed claims never grant replay.

use super::{
    loop_types::{JournalEntry, JournalError, JournalWriter, LoopEvent},
    prepared::digest_json,
    protected_journal::{private_directory, ProtectedJournal},
    recovery::inspect_run,
    run_audit::RunAuditReference,
};
use crate::types::AgentId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use uuid::Uuid;
pub use uuid::Uuid as InvocationId;

pub mod file_recovery;
pub mod reconciliation;

const MAX_CLAIMS: usize = 4096;
const MAX_STORE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_RECORD_BYTES: u64 = 2 * 1024 * 1024;
const MAX_RESULT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Record {
    Claim {
        version: u32,
        scope: String,
        id: Uuid,
        request_hash: String,
        agent_id: AgentId,
        run_id: Uuid,
    },
    Audit {
        reference: RunAuditReference,
    },
    Result {
        payload: Value,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ExistingInvocation {
    /// An owner still holds the claim; no second owner can execute it.
    InProgress,
    /// No cacheable receipt exists. Inspect the original run before retrying work.
    Unresolved { audit: Option<RunAuditReference> },
    /// A previously persisted result. Returning it does not execute the request.
    Recorded {
        audit: RunAuditReference,
        result: Value,
    },
    /// An operator attestation, distinct from a recorded execution result.
    /// The original ID remains closed to execution.
    Reconciled {
        audit: RunAuditReference,
        resolution: Box<reconciliation::ResolutionReceipt>,
    },
}

pub enum OpenInvocation {
    Fresh(Box<Invocation>),
    Existing(ExistingInvocation),
}

/// Holds the exclusive claim lock until the result is durably recorded or the
/// owner exits. Dropping the handle never removes or resets the claim.
pub struct Invocation {
    journal: Arc<dyn JournalWriter>,
    audit: RunAuditReference,
    agent_id: AgentId,
    store: PathBuf,
    file: File,
    bytes: u64,
}

struct InvocationJournal {
    writer: ProtectedJournal,
    context: Value,
    phase: tokio::sync::Mutex<InvocationPhase>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InvocationPhase {
    Ready,
    Running,
    Closed,
}

#[async_trait::async_trait]
impl JournalWriter for InvocationJournal {
    fn audit_reference(&self) -> Option<RunAuditReference> {
        self.writer.audit_reference()
    }
    async fn append(&self, mut entry: JournalEntry) -> Result<(), JournalError> {
        let mut phase = self.phase.lock().await;
        let next = match &mut entry.event {
            LoopEvent::Started {
                execution_context, ..
            } if *phase == InvocationPhase::Ready => {
                execution_context.insert("invocation".into(), self.context.clone());
                InvocationPhase::Running
            }
            LoopEvent::Started { .. } => {
                return Err(JournalError::WriteFailed(
                    "invocation already started; a claim cannot execute twice".into(),
                ))
            }
            LoopEvent::Terminated { .. } if *phase == InvocationPhase::Running => {
                InvocationPhase::Closed
            }
            _ if *phase == InvocationPhase::Running => InvocationPhase::Running,
            _ => {
                return Err(JournalError::WriteFailed(
                    "invocation journal is not running".into(),
                ))
            }
        };
        let result = self.writer.append(entry).await;
        *phase = if result.is_ok() {
            next
        } else {
            InvocationPhase::Closed
        };
        result
    }
    async fn next_sequence(&self) -> u64 {
        self.writer.next_sequence().await
    }
}

/// `scope` comes from the trusted entry point and authenticated caller, not from
/// model output. A reused ID with a different request is always refused.
pub async fn open_invocation(
    project: &Path,
    scope: &str,
    id: Uuid,
    request: &Value,
    agent_id: AgentId,
) -> Result<OpenInvocation, String> {
    if scope.is_empty() || scope.len() > 256 {
        return Err("invalid invocation scope".into());
    }
    if serde_json::to_vec(request)
        .map_err(|e| e.to_string())?
        .len()
        > MAX_RESULT_BYTES
    {
        return Err("invocation request identity exceeds 1 MiB".into());
    }
    let request_hash = digest_json(request)?;
    let project = project.to_owned();
    let scope = scope.to_owned();
    tokio::task::spawn_blocking(move || open(&project, &scope, id, &request_hash, agent_id))
        .await
        .map_err(|e| e.to_string())?
}

/// Inspect an existing claim without creating a claim or granting execution.
/// Absence is distinct from an unresolved claim; callers must retain that distinction.
pub async fn lookup_invocation(
    project: &Path,
    scope: &str,
    id: Uuid,
    request: &Value,
) -> Result<Option<ExistingInvocation>, String> {
    if scope.is_empty()
        || scope.len() > 256
        || serde_json::to_vec(request)
            .map_err(|e| e.to_string())?
            .len()
            > MAX_RESULT_BYTES
    {
        return Err("invalid invocation lookup identity".into());
    }
    let request_hash = digest_json(request)?;
    let store = project.join(".symbiont/invocations");
    let project = project.to_owned();
    let scope = scope.to_owned();
    tokio::task::spawn_blocking(move || {
        match std::fs::symlink_metadata(&store) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        }
        private_directory(&store)?;
        let store_guard = store_lock(&store)?;
        let digest = digest_json(&serde_json::json!([scope, id]))?;
        let path = store.join(format!("{}.jsonl", digest.trim_start_matches("sha256:")));
        let mut file = match options().open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        validate_file(&file)?;
        if !try_lock(&file)? {
            return Ok(Some(ExistingInvocation::InProgress));
        }
        drop(store_guard);
        read_existing(&project, &mut file, &scope, id, &request_hash).map(Some)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn open(
    project: &Path,
    scope: &str,
    id: Uuid,
    request_hash: &str,
    agent_id: AgentId,
) -> Result<OpenInvocation, String> {
    let store = project.join(".symbiont/invocations");
    private_directory(&store)?;
    let _store_lock = store_lock(&store)?;
    let digest = digest_json(&serde_json::json!([scope, id]))?;
    let path = store.join(format!("{}.jsonl", digest.trim_start_matches("sha256:")));
    let run_id = Uuid::new_v4();
    let claim = Record::Claim {
        version: 1,
        scope: scope.into(),
        id,
        request_hash: request_hash.into(),
        agent_id,
        run_id,
    };
    let claim_bytes = serde_json::to_vec(&claim).map_err(|e| e.to_string())?.len() as u64 + 1;
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match std::fs::symlink_metadata(reconciliation::receipt_path(&store, scope, id)?) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
                Ok(_) => {
                    return Err(
                        "reconciled invocation claim is missing; no replay permitted".into(),
                    )
                }
            }
            capacity(&store, 1, claim_bytes)?
        }
        Err(error) => return Err(error.to_string()),
        Ok(_) => {}
    }
    let created = options().create_new(true).open(&path);
    let mut file = match created {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let mut file = options().open(&path).map_err(|e| e.to_string())?;
            validate_file(&file)?;
            if !try_lock(&file)? {
                return Ok(OpenInvocation::Existing(ExistingInvocation::InProgress));
            }
            drop(_store_lock);
            return read_existing(project, &mut file, scope, id, request_hash)
                .map(OpenInvocation::Existing);
        }
        Err(error) => return Err(error.to_string()),
    };
    if !try_lock(&file)? {
        return Err("new invocation claim is unexpectedly locked".into());
    }
    // The empty claim is deliberately retained on any subsequent failure.
    // An uncertain creation cannot authorize a second execution.
    let mut bytes = 0;
    append(&store, &mut file, &mut bytes, &claim)?;
    File::open(&store)
        .and_then(|dir| dir.sync_all())
        .map_err(|e| e.to_string())?;
    let journal =
        ProtectedJournal::create_run(&project.join(".symbiont/governed"), agent_id, run_id)
            .map_err(|e| e.to_string())?;
    let audit = RunAuditReference {
        run_id,
        path: journal.path().to_owned(),
        public_key: hex::encode(journal.public_key()),
    };
    append(
        &store,
        &mut file,
        &mut bytes,
        &Record::Audit {
            reference: audit.clone(),
        },
    )?;
    Ok(OpenInvocation::Fresh(Box::new(Invocation {
        journal: Arc::new(InvocationJournal {
            writer: journal,
            context: serde_json::json!({"id": id, "scope": scope, "request_hash": request_hash}),
            phase: tokio::sync::Mutex::new(InvocationPhase::Ready),
        }),
        audit,
        agent_id,
        store,
        file,
        bytes,
    })))
}

impl Invocation {
    pub fn journal(&self) -> Arc<dyn JournalWriter> {
        self.journal.clone()
    }
    pub fn audit(&self) -> &RunAuditReference {
        &self.audit
    }
    pub fn agent_id(&self) -> AgentId {
        self.agent_id
    }

    /// Persist the result only after verifying the run. Unknown effects keep the
    /// invocation unresolved, even if the reasoning loop reported completion.
    pub async fn finish(mut self, result: Value) -> Result<ExistingInvocation, String> {
        if serde_json::to_vec(&result)
            .map_err(|e| e.to_string())?
            .len()
            > MAX_RESULT_BYTES
        {
            return Err(
                "invocation result exceeds 1 MiB; original claim remains unresolved".into(),
            );
        }
        tokio::task::spawn_blocking(move || {
            if requires_reconciliation(&self.audit)? {
                return Ok(ExistingInvocation::Unresolved {
                    audit: Some(self.audit),
                });
            }
            let _store_lock = store_lock(&self.store)?;
            append(
                &self.store,
                &mut self.file,
                &mut self.bytes,
                &Record::Result {
                    payload: result.clone(),
                },
            )?;
            Ok(ExistingInvocation::Recorded {
                audit: self.audit,
                result,
            })
        })
        .await
        .map_err(|e| e.to_string())?
    }
}

fn read_existing(
    project: &Path,
    file: &mut File,
    scope: &str,
    id: Uuid,
    request_hash: &str,
) -> Result<ExistingInvocation, String> {
    let existing = read_claim(project, file, scope, id, request_hash)?;
    if matches!(existing, ExistingInvocation::Unresolved { .. }) {
        if let Some(resolution) =
            reconciliation::read_resolution(project, file, scope, id, request_hash)?
        {
            return Ok(ExistingInvocation::Reconciled {
                audit: resolution.snapshot.audit.clone(),
                resolution: Box::new(resolution),
            });
        }
    }
    Ok(existing)
}

fn read_claim(
    project: &Path,
    file: &mut File,
    scope: &str,
    id: Uuid,
    request_hash: &str,
) -> Result<ExistingInvocation, String> {
    let mut bytes = Vec::new();
    file.take(MAX_RECORD_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err("invocation claim exceeds its size limit".into());
    }
    if bytes.is_empty() || bytes.last() != Some(&b'\n') {
        return Ok(ExistingInvocation::Unresolved { audit: None });
    }
    let records: Vec<Record> = bytes
        .split_inclusive(|b| *b == b'\n')
        .map(serde_json::from_slice)
        .collect::<Result<_, _>>()
        .map_err(|e| format!("invalid invocation claim; no retry permitted: {e}"))?;
    let Some(Record::Claim {
        version: 1,
        scope: saved_scope,
        id: saved_id,
        request_hash: saved_hash,
        agent_id,
        run_id,
    }) = records.first()
    else {
        return Err("invalid invocation claim identity".into());
    };
    if saved_scope != scope || *saved_id != id || saved_hash != request_hash {
        return Err("invocation ID conflicts with a different request".into());
    }
    let audit = match records.get(1) {
        Some(Record::Audit { reference })
            if reference.run_id == *run_id
                && reference.path
                    == project
                        .join(".symbiont/governed")
                        .join(format!("{agent_id}.{run_id}.jsonl")) =>
        {
            Some(reference.clone())
        }
        None => None,
        _ => return Err("invalid invocation audit reference".into()),
    };
    match (records.get(2), audit) {
        (Some(Record::Result { payload }), Some(audit)) if records.len() == 3 => {
            if requires_reconciliation(&audit)? {
                Ok(ExistingInvocation::Unresolved { audit: Some(audit) })
            } else {
                Ok(ExistingInvocation::Recorded {
                    audit,
                    result: payload.clone(),
                })
            }
        }
        (None, audit) => Ok(ExistingInvocation::Unresolved { audit }),
        _ => Err("invalid invocation result records".into()),
    }
}

fn requires_reconciliation(audit: &RunAuditReference) -> Result<bool, String> {
    let key: [u8; 32] = hex::decode(&audit.public_key)
        .map_err(|e| e.to_string())?
        .try_into()
        .map_err(|_| "invalid invocation audit key")?;
    inspect_run(&audit.path, &key, audit.run_id)
        .map(|report| report.requires_reconciliation)
        .map_err(|e| e.to_string())
}

fn options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
    options
}

fn validate_file(file: &File) -> Result<(), String> {
    let meta = file.metadata().map_err(|e| e.to_string())?;
    // SAFETY: geteuid has no memory-safety preconditions.
    if !meta.is_file()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o077 != 0
        || meta.nlink() != 1
        || meta.len() > MAX_RECORD_BYTES
    {
        return Err(
            "invocation file must be private, runtime-owned, have one link and be bounded".into(),
        );
    }
    Ok(())
}

fn try_lock(file: &File) -> Result<bool, String> {
    // SAFETY: flock acts only on the live file descriptor. Closing it releases the lock.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::WouldBlock {
        Ok(false)
    } else {
        Err(error.to_string())
    }
}

fn store_lock(store: &Path) -> Result<File, String> {
    let lock = options()
        .create(true)
        .truncate(false)
        .open(store.join("store.lock"))
        .map_err(|e| e.to_string())?;
    validate_file(&lock)?;
    let started = Instant::now();
    while !try_lock(&lock)? {
        if started.elapsed() >= Duration::from_secs(2) {
            return Err("invocation store busy; retry with the same ID".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(lock)
}

/// Called only under the store lock. The retained claims and their total bytes
/// are bounded; exhaustion refuses new work or result delivery without deletion.
fn append(store: &Path, file: &mut File, bytes: &mut u64, record: &Record) -> Result<(), String> {
    validate_file(file)?;
    if file.metadata().map_err(|e| e.to_string())?.len() != *bytes {
        return Err("invocation file changed outside its owner".into());
    }
    let mut encoded = serde_json::to_vec(record).map_err(|e| e.to_string())?;
    encoded.push(b'\n');
    if bytes.saturating_add(encoded.len() as u64) > MAX_RECORD_BYTES {
        return Err("invocation record limit exceeded".into());
    }
    capacity(store, 0, encoded.len() as u64)?;
    file.write_all(&encoded)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    *bytes += encoded.len() as u64;
    Ok(())
}

fn capacity(store: &Path, additional_files: usize, additional_bytes: u64) -> Result<(), String> {
    let mut count = additional_files;
    let mut total = additional_bytes;
    for item in std::fs::read_dir(store).map_err(|e| e.to_string())? {
        let item = item.map_err(|e| e.to_string())?;
        if item.file_name() == "store.lock" {
            continue;
        }
        let meta = std::fs::symlink_metadata(item.path()).map_err(|e| e.to_string())?;
        // SAFETY: geteuid has no memory-safety preconditions.
        if !meta.is_file()
            || meta.nlink() != 1
            || meta.mode() & 0o077 != 0
            || meta.uid() != unsafe { libc::geteuid() }
        {
            return Err("unsafe invocation store entry".into());
        }
        count += 1;
        total = total.saturating_add(meta.len());
        if count > MAX_CLAIMS || total > MAX_STORE_BYTES {
            return Err("invocation store capacity exhausted; retained identities must not be silently discarded".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reasoning::loop_types::{JournalEntry, LoopConfig, LoopEvent, TerminationReason};

    async fn append_event(run: &Invocation, event: LoopEvent) {
        run.journal
            .append(JournalEntry {
                sequence: 0,
                timestamp: chrono::Utc::now(),
                agent_id: run.agent_id,
                iteration: 0,
                event,
            })
            .await
            .unwrap();
    }

    async fn terminate(run: &Invocation, unknown: bool) {
        append_event(
            run,
            LoopEvent::Started {
                agent_id: run.agent_id,
                config: Box::new(LoopConfig::default()),
                execution_context: Default::default(),
            },
        )
        .await;
        if unknown {
            append_event(
                run,
                LoopEvent::ToolDispatchStarted {
                    dispatch_id: Uuid::new_v4(),
                    run_key: "run".into(),
                    call_id: "call".into(),
                    call_fingerprint: "fingerprint".into(),
                    tool_name: "fixture".into(),
                },
            )
            .await;
        }
        append_event(
            run,
            LoopEvent::Terminated {
                reason: TerminationReason::Completed,
                iterations: 1,
                total_usage: Default::default(),
                duration: Default::default(),
            },
        )
        .await;
    }

    #[tokio::test]
    async fn concurrent_retries_have_one_owner_and_completed_results_are_cached() {
        let root = tempfile::tempdir().unwrap();
        let id = Uuid::new_v4();
        let request = serde_json::json!({"task": "useful work"});
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..8 {
            let project = root.path().to_owned();
            let request = request.clone();
            tasks.spawn(async move {
                open_invocation(&project, "fixture", id, &request, AgentId::new())
                    .await
                    .unwrap()
            });
        }
        let mut owner = None;
        let mut blocked = 0;
        while let Some(result) = tasks.join_next().await {
            match result.unwrap() {
                OpenInvocation::Fresh(run) => {
                    assert!(owner.replace(*run).is_none());
                }
                OpenInvocation::Existing(ExistingInvocation::InProgress) => blocked += 1,
                _ => panic!("unexpected concurrent retry outcome"),
            }
        }
        assert_eq!(blocked, 7);
        let owner = owner.unwrap();
        let audit = owner.audit.clone();
        terminate(&owner, false).await;
        assert!(owner
            .journal
            .append(JournalEntry {
                sequence: 0,
                timestamp: chrono::Utc::now(),
                agent_id: owner.agent_id,
                iteration: 0,
                event: LoopEvent::Started {
                    agent_id: owner.agent_id,
                    config: Box::new(LoopConfig::default()),
                    execution_context: Default::default()
                }
            })
            .await
            .is_err());
        let key: [u8; 32] = hex::decode(&audit.public_key).unwrap().try_into().unwrap();
        let entries = ProtectedJournal::verify_run(&audit.path, &key, audit.run_id).unwrap();
        let LoopEvent::Started {
            execution_context, ..
        } = &entries[0].event
        else {
            panic!()
        };
        assert_eq!(execution_context["invocation"]["id"], serde_json::json!(id));
        assert_eq!(execution_context["invocation"]["scope"], "fixture");
        assert_eq!(entries.len(), 2);
        let payload = serde_json::json!({"sum": 10});
        assert!(matches!(
            owner.finish(payload.clone()).await.unwrap(),
            ExistingInvocation::Recorded { .. }
        ));
        let OpenInvocation::Existing(ExistingInvocation::Recorded {
            audit: cached,
            result,
        }) = open_invocation(root.path(), "fixture", id, &request, AgentId::new())
            .await
            .unwrap()
        else {
            panic!("completed invocation was not cached");
        };
        assert_eq!(cached.run_id, audit.run_id);
        assert_eq!(result, payload);
        assert!(open_invocation(
            root.path(),
            "fixture",
            id,
            &serde_json::json!({"task": "different"}),
            AgentId::new()
        )
        .await
        .is_err());
        assert!(matches!(
            open_invocation(root.path(), "another-scope", id, &request, AgentId::new())
                .await
                .unwrap(),
            OpenInvocation::Fresh(_)
        ));
    }

    #[tokio::test]
    async fn missing_outcomes_and_damaged_records_never_create_another_owner() {
        let root = tempfile::tempdir().unwrap();
        let request = serde_json::json!({"task": "effect"});
        for state in ["empty", "unknown_effect", "terminal_without_receipt"] {
            let id = Uuid::new_v4();
            let OpenInvocation::Fresh(run) =
                open_invocation(root.path(), "fixture", id, &request, AgentId::new())
                    .await
                    .unwrap()
            else {
                panic!()
            };
            let run_id = run.audit.run_id;
            if state == "unknown_effect" {
                terminate(&run, true).await;
                assert!(matches!(
                    run.finish(serde_json::json!({"ignored": true}))
                        .await
                        .unwrap(),
                    ExistingInvocation::Unresolved { .. }
                ));
            } else {
                if state == "terminal_without_receipt" {
                    terminate(&run, false).await;
                }
                drop(run);
            }
            let OpenInvocation::Existing(ExistingInvocation::Unresolved { audit: Some(audit) }) =
                open_invocation(root.path(), "fixture", id, &request, AgentId::new())
                    .await
                    .unwrap()
            else {
                panic!()
            };
            assert_eq!(audit.run_id, run_id);
            let hash = digest_json(&serde_json::json!(["fixture", id])).unwrap();
            let path = root
                .path()
                .join(".symbiont/invocations")
                .join(format!("{}.jsonl", hash.trim_start_matches("sha256:")));
            std::fs::write(&path, b"{").unwrap();
            assert!(matches!(
                open_invocation(root.path(), "fixture", id, &request, AgentId::new())
                    .await
                    .unwrap(),
                OpenInvocation::Existing(ExistingInvocation::Unresolved { .. })
            ));
            std::fs::write(&path, b"{}\n").unwrap();
            assert!(
                open_invocation(root.path(), "fixture", id, &request, AgentId::new())
                    .await
                    .is_err()
            );
        }
    }

    #[tokio::test]
    async fn unsafe_claims_and_exhausted_capacity_refuse_without_creating_more_records() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let store = root.path().join(".symbiont/invocations");
        private_directory(&store).unwrap();
        let id = Uuid::new_v4();
        let hash = digest_json(&serde_json::json!(["fixture", id])).unwrap();
        let path = store.join(format!("{}.jsonl", hash.trim_start_matches("sha256:")));
        let target = root.path().join("outside");
        std::fs::write(&target, b"protected").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(
            open_invocation(root.path(), "fixture", id, &Value::Null, AgentId::new())
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"protected");
        std::fs::remove_file(&path).unwrap();
        std::fs::hard_link(&target, &path).unwrap();
        assert!(
            open_invocation(root.path(), "fixture", id, &Value::Null, AgentId::new())
                .await
                .is_err()
        );
        std::fs::remove_file(&path).unwrap();
        let full = options()
            .create_new(true)
            .open(store.join("full.jsonl"))
            .unwrap();
        full.set_len(MAX_STORE_BYTES).unwrap();
        let before = std::fs::read_dir(&store).unwrap().count();
        assert!(
            open_invocation(root.path(), "fixture", id, &Value::Null, AgentId::new())
                .await
                .is_err()
        );
        assert_eq!(std::fs::read_dir(&store).unwrap().count(), before);
        std::fs::remove_file(store.join("full.jsonl")).unwrap();
        std::fs::set_permissions(&store, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(
            open_invocation(root.path(), "fixture", id, &Value::Null, AgentId::new())
                .await
                .is_err()
        );
    }
}
