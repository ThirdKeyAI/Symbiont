//! Operator attestations close uncertainty without granting replay authority.
//! Original claim and journal bytes remain intact. A signed, atomically published
//! sidecar binds the attestation to their reviewed snapshots.

use super::*;
use crate::reasoning::{protected_journal::load_key, recovery::RecoveryReport};
use ed25519_dalek::{Signature, Signer, VerifyingKey};
use sha2::{Digest, Sha256};
use std::io::{Seek, SeekFrom};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationSnapshot {
    pub scope: String,
    pub id: Uuid,
    pub request_hash: String,
    pub agent_id: AgentId,
    pub claim_sha256: String,
    pub journal_sha256: String,
    pub audit: RunAuditReference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionOutcome {
    Completed,
    Failed,
    NoEffects,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolutionEvidence {
    /// Operator-retained evidence location; never fetched or executed by the runtime.
    pub reference: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolutionReview {
    pub snapshot_hash: String,
    pub outcome: ResolutionOutcome,
    pub rationale: String,
    pub evidence: Vec<ResolutionEvidence>,
    /// Operator assertion that workers and pending external operations are stopped
    /// or settled. Losing the invocation owner's lock does not establish this.
    pub effects_stopped: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolutionReceipt {
    pub version: u32,
    pub snapshot: InvocationSnapshot,
    pub review: ResolutionReview,
    pub operator_uid: u32,
    pub recorded_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedResolution {
    payload: ResolutionReceipt,
    signature: String,
}

#[derive(Debug, Serialize)]
pub struct InvocationInspection {
    pub status: String,
    pub snapshot_hash: String,
    pub snapshot: InvocationSnapshot,
    /// Absent only when the original journal is empty. Empty evidence proves no
    /// outcome; an operator must establish cleanup and effects independently.
    pub recovery: Option<RecoveryReport>,
    pub resolution: Option<ResolutionReceipt>,
}

fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn snapshot_hash(snapshot: &InvocationSnapshot) -> Result<String, String> {
    digest_json(&serde_json::to_value(snapshot).map_err(|e| e.to_string())?)
}

pub(super) fn receipt_path(store: &Path, scope: &str, id: Uuid) -> Result<PathBuf, String> {
    let digest = digest_json(&serde_json::json!([scope, id]))?;
    Ok(store.join(format!(
        "{}.resolved.json",
        digest.trim_start_matches("sha256:")
    )))
}

pub(super) fn locked_claim(project: &Path, scope: &str, id: Uuid) -> Result<File, String> {
    if scope.is_empty() || scope.len() > 256 {
        return Err("invalid invocation scope".into());
    }
    let store = project.join(".symbiont/invocations");
    if !store.try_exists().map_err(|e| e.to_string())? {
        return Err("invocation store does not exist".into());
    }
    private_directory(&store)?;
    let _guard = store_lock(&store)?;
    let digest = digest_json(&serde_json::json!([scope, id]))?;
    let file = options()
        .open(store.join(format!("{}.jsonl", digest.trim_start_matches("sha256:"))))
        .map_err(|e| e.to_string())?;
    validate_file(&file)?;
    if !try_lock(&file)? {
        return Err("invocation is still owned; reconciliation refused".into());
    }
    Ok(file)
}

pub(super) fn capture(
    project: &Path,
    file: &mut File,
    scope: &str,
    id: Uuid,
) -> Result<InvocationInspection, String> {
    validate_file(file)?;
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_RECORD_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err("invocation claim exceeds its size limit".into());
    }
    let complete = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    let records = bytes[..complete]
        .split_inclusive(|b| *b == b'\n')
        .map(serde_json::from_slice::<Record>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("invalid invocation evidence: {e}"))?;
    let [Record::Claim {
        version: 1,
        scope: saved_scope,
        id: saved_id,
        request_hash,
        agent_id,
        run_id,
    }, Record::Audit { reference }, rest @ ..] = records.as_slice()
    else {
        return Err("reconciliation requires a complete claim identity and audit reference".into());
    };
    if saved_scope != scope
        || *saved_id != id
        || reference.run_id != *run_id
        || reference.path
            != project
                .join(".symbiont/governed")
                .join(format!("{agent_id}.{run_id}.jsonl"))
        || !(rest.is_empty() || matches!(rest, [Record::Result { .. }]))
    {
        return Err("invalid invocation evidence identity or record order".into());
    }
    let key: [u8; 32] = hex::decode(&reference.public_key)
        .map_err(|e| e.to_string())?
        .try_into()
        .map_err(|_| "invalid invocation audit key")?;
    // Inspect empty journals separately: there is no signed prefix to classify.
    let journal = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(&reference.path)
        .map_err(|e| e.to_string())?;
    let meta = journal.metadata().map_err(|e| e.to_string())?;
    // SAFETY: geteuid has no memory-safety preconditions.
    if !meta.is_file()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o077 != 0
        || meta.nlink() != 1
    {
        return Err("invocation journal must be private, runtime-owned and singly linked".into());
    }
    let (journal_sha256, recovery) = if meta.len() == 0 {
        (hash(&[]), None)
    } else {
        let prefix = ProtectedJournal::verify_run_prefix(&reference.path, &key, *run_id)
            .map_err(|e| e.to_string())?;
        match prefix.entries.first().map(|e| &e.event) {
            Some(LoopEvent::Started {
                execution_context, ..
            }) if execution_context.get("invocation")
                == Some(
                    &serde_json::json!({"id": id, "scope": scope, "request_hash": request_hash}),
                ) => {}
            _ => return Err("signed run does not bind the invocation claim".into()),
        }
        let digest = prefix.snapshot_sha256.clone();
        let report =
            crate::reasoning::recovery::classify(prefix, *run_id).map_err(|e| e.to_string())?;
        if report.agent_id != *agent_id {
            return Err("invocation principal differs from its journal".into());
        }
        (digest, Some(report))
    };
    let snapshot = InvocationSnapshot {
        scope: scope.into(),
        id,
        request_hash: request_hash.clone(),
        agent_id: *agent_id,
        claim_sha256: hash(&bytes),
        journal_sha256,
        audit: reference.clone(),
    };
    let status = if complete == bytes.len()
        && matches!(rest, [Record::Result { .. }])
        && recovery
            .as_ref()
            .is_some_and(|r| !r.requires_reconciliation)
    {
        "recorded"
    } else {
        "unresolved"
    }
    .into();
    Ok(InvocationInspection {
        status,
        snapshot_hash: snapshot_hash(&snapshot)?,
        snapshot,
        recovery,
        resolution: None,
    })
}

fn signing_bytes(receipt: &ResolutionReceipt) -> Result<Vec<u8>, String> {
    let mut bytes = b"symbi-invocation-resolution:v1\n".to_vec();
    bytes.extend(serde_json::to_vec(receipt).map_err(|e| e.to_string())?);
    Ok(bytes)
}

fn validate_review(review: &ResolutionReview) -> Result<(), String> {
    if !review.effects_stopped
        || review.rationale.trim().is_empty()
        || review.rationale.len() > 8192
        || review.evidence.is_empty()
        || review.evidence.len() > 32
        || review.evidence.iter().any(|e| {
            e.reference.trim().is_empty()
                || e.reference.len() > 2048
                || e.sha256.len() != 64
                || !e.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        })
    {
        return Err("resolution requires stopped/settled effects, a bounded rationale and 1–32 evidence references with SHA-256 digests".into());
    }
    Ok(())
}

pub fn read_review(path: &Path) -> Result<ResolutionReview, String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("review must be a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(131073)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 131072 {
        return Err("review exceeds 128 KiB".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

fn read_signed(
    project: &Path,
    inspection: &InvocationInspection,
) -> Result<Option<ResolutionReceipt>, String> {
    let path = receipt_path(
        &project.join(".symbiont/invocations"),
        &inspection.snapshot.scope,
        inspection.snapshot.id,
    )?;
    let file = match options().open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    validate_file(&file)?;
    let mut bytes = Vec::new();
    file.take(MAX_RECORD_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err("oversized resolution receipt".into());
    }
    let signed: SignedResolution = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let key: [u8; 32] = hex::decode(&inspection.snapshot.audit.public_key)
        .map_err(|e| e.to_string())?
        .try_into()
        .map_err(|_| "invalid resolution audit key")?;
    VerifyingKey::from_bytes(&key)
        .map_err(|e| e.to_string())?
        .verify_strict(
            &signing_bytes(&signed.payload)?,
            &Signature::from_slice(&hex::decode(&signed.signature).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    validate_review(&signed.payload.review)?;
    if signed.payload.version != 1
        || snapshot_hash(&signed.payload.snapshot)? != inspection.snapshot_hash
        || signed.payload.review.snapshot_hash != inspection.snapshot_hash
    {
        return Err(
            "reconciled evidence changed; original outcome requires renewed investigation".into(),
        );
    }
    Ok(Some(signed.payload))
}

pub(super) fn read_resolution(
    project: &Path,
    file: &mut File,
    scope: &str,
    id: Uuid,
    request_hash: &str,
) -> Result<Option<ResolutionReceipt>, String> {
    // Avoid journal verification overhead on ordinary unresolved lookups.
    match std::fs::symlink_metadata(receipt_path(
        &project.join(".symbiont/invocations"),
        scope,
        id,
    )?) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
        Ok(_) => {}
    }
    let inspection = capture(project, file, scope, id)?;
    if inspection.snapshot.request_hash != request_hash {
        return Err("invocation ID conflicts with a different request".into());
    }
    read_signed(project, &inspection)
}

/// Read one inactive invocation without needing its original input payload.
/// The project must be the same canonical directory used for admission.
pub fn inspect_invocation(
    project: &Path,
    scope: &str,
    id: Uuid,
) -> Result<InvocationInspection, String> {
    let mut file = locked_claim(project, scope, id)?;
    let mut inspection = capture(project, &mut file, scope, id)?;
    inspection.resolution = read_signed(project, &inspection)?;
    if inspection.resolution.is_some() {
        inspection.status = "reconciled".into();
    }
    Ok(inspection)
}

/// Record an operator's assessment. This API is runtime/operator authority and
/// must never be exposed as a model-callable tool or unauthenticated endpoint.
/// Repeating the same review returns its receipt; a different review is refused.
pub fn reconcile_invocation(
    project: &Path,
    scope: &str,
    id: Uuid,
    review: ResolutionReview,
) -> Result<ResolutionReceipt, String> {
    validate_review(&review)?;
    let mut file = locked_claim(project, scope, id)?;
    let inspection = capture(project, &mut file, scope, id)?;
    if let Some(receipt) = read_signed(project, &inspection)? {
        return if receipt.review == review {
            Ok(receipt)
        } else {
            Err("invocation already has a different resolution".into())
        };
    }
    if review.snapshot_hash != inspection.snapshot_hash {
        return Err("invocation evidence changed since review; inspect it again".into());
    }
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    if matches!(
        read_claim(
            project,
            &mut file,
            scope,
            id,
            &inspection.snapshot.request_hash
        )?,
        ExistingInvocation::Recorded { .. }
    ) {
        return Err("invocation already has a verified recorded result".into());
    }
    let parent = project.join(".symbiont/governed");
    private_directory(&parent)?;
    std::fs::symlink_metadata(parent.join("audit-signing.key"))
        .map_err(|_| "original audit signing key is unavailable")?;
    let key = load_key(&parent)?;
    if hex::encode(key.verifying_key().to_bytes()) != inspection.snapshot.audit.public_key {
        return Err("current signing key differs from the original invocation key".into());
    }
    let payload = ResolutionReceipt {
        version: 1,
        snapshot: inspection.snapshot,
        review,
        // SAFETY: geteuid has no memory-safety preconditions.
        operator_uid: unsafe { libc::geteuid() },
        recorded_at: chrono::Utc::now(),
    };
    let signature = hex::encode(key.sign(&signing_bytes(&payload)?).to_bytes());
    let encoded = serde_json::to_vec(&SignedResolution {
        payload: payload.clone(),
        signature,
    })
    .map_err(|e| e.to_string())?;
    let store = project.join(".symbiont/invocations");
    let _guard = store_lock(&store)?;
    let current = capture(project, &mut file, scope, id)?;
    if current.snapshot_hash != payload.review.snapshot_hash {
        return Err("invocation evidence changed during reconciliation".into());
    }
    capacity(&store, 1, encoded.len() as u64)?;
    let mut temp = tempfile::NamedTempFile::new_in(&store).map_err(|e| e.to_string())?;
    temp.write_all(&encoded)
        .and_then(|_| temp.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    temp.persist_noclobber(receipt_path(&store, scope, id)?)
        .map_err(|e| e.to_string())?;
    File::open(&store)
        .and_then(|dir| dir.sync_all())
        .map_err(|e| e.to_string())?;
    read_signed(project, &current)?.ok_or_else(|| "resolution publication is unconfirmed".into())
}
