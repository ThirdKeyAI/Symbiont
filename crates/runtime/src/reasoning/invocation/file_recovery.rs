//! Complete one authenticated broker publication without repeating its tool.
use super::*;
use crate::{
    reasoning::{protected_journal::load_key, recovery::EffectIdentity},
    sandbox::files::{PublicationIntent, PublicationState},
};
use ed25519_dalek::{Signature, Signer, VerifyingKey};
use reconciliation::{InvocationInspection, InvocationSnapshot};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileRecoveryReceipt {
    pub version: u32,
    pub snapshot: InvocationSnapshot,
    pub publication: PublicationIntent,
    pub created_files: Value,
    pub operator_uid: u32,
    pub recorded_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedReceipt {
    payload: FileRecoveryReceipt,
    signature: String,
}

#[derive(Debug, Serialize)]
pub struct FilePublicationInspection {
    pub snapshot_hash: String,
    pub snapshot: InvocationSnapshot,
    pub publication: PublicationIntent,
    pub state: PublicationState,
    pub recovery: Option<FileRecoveryReceipt>,
}

fn intent(
    inspection: &InvocationInspection,
    publication_id: Uuid,
) -> Result<PublicationIntent, String> {
    let effect = inspection.recovery.as_ref().ok_or("invocation has no signed effects")?.effects.iter()
        .find(|effect| matches!(effect.identity, EffectIdentity::FilePublication { publication_id: id, .. } if id == publication_id))
        .ok_or("publication is not in the authenticated invocation")?;
    let intent: PublicationIntent =
        serde_json::from_value(effect.details.clone()).map_err(|e| e.to_string())?;
    if intent.publication_id != publication_id {
        return Err("publication identity mismatch".into());
    }
    Ok(intent)
}

fn receipt_path(
    project: &Path,
    scope: &str,
    id: Uuid,
    publication: Uuid,
) -> Result<PathBuf, String> {
    let hash = digest_json(&serde_json::json!([
        "file-publication-v1",
        scope,
        id,
        publication
    ]))?;
    Ok(project.join(".symbiont/invocations").join(format!(
        "{}.publication.json",
        hash.trim_start_matches("sha256:")
    )))
}

fn signing_bytes(receipt: &FileRecoveryReceipt) -> Result<Vec<u8>, String> {
    let mut bytes = b"symbi-file-publication-recovery:v1\n".to_vec();
    bytes.extend(serde_json::to_vec(receipt).map_err(|e| e.to_string())?);
    Ok(bytes)
}

fn snapshot_hash(snapshot: &InvocationSnapshot) -> Result<String, String> {
    digest_json(&serde_json::to_value(snapshot).map_err(|e| e.to_string())?)
}

fn read_receipt(
    project: &Path,
    inspection: &InvocationInspection,
    publication: &PublicationIntent,
) -> Result<Option<FileRecoveryReceipt>, String> {
    let path = receipt_path(
        project,
        &inspection.snapshot.scope,
        inspection.snapshot.id,
        publication.publication_id,
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
        return Err("oversized file recovery receipt".into());
    }
    let signed: SignedReceipt = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let key: [u8; 32] = hex::decode(&inspection.snapshot.audit.public_key)
        .map_err(|e| e.to_string())?
        .try_into()
        .map_err(|_| "invalid original audit key")?;
    VerifyingKey::from_bytes(&key)
        .map_err(|e| e.to_string())?
        .verify_strict(
            &signing_bytes(&signed.payload)?,
            &Signature::from_slice(&hex::decode(&signed.signature).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    if signed.payload.version != 1
        || snapshot_hash(&signed.payload.snapshot)? != inspection.snapshot_hash
        || serde_json::to_value(&signed.payload.publication).map_err(|e| e.to_string())?
            != serde_json::to_value(publication).map_err(|e| e.to_string())?
    {
        return Err("file recovery receipt differs from original evidence".into());
    }
    Ok(Some(signed.payload))
}

/// Inspect one exact file publication from an inactive invocation. This API is
/// operator authority: it opens only the destination bound by the signed intent.
pub fn inspect_file_publication(
    project: &Path,
    scope: &str,
    id: Uuid,
    publication_id: Uuid,
) -> Result<FilePublicationInspection, String> {
    let mut claim = reconciliation::locked_claim(project, scope, id)?;
    let inspection = reconciliation::capture(project, &mut claim, scope, id)?;
    let publication = intent(&inspection, publication_id)?;
    let state = publication.inspect()?;
    let recovery = read_receipt(project, &inspection, &publication)?;
    Ok(FilePublicationInspection {
        snapshot_hash: inspection.snapshot_hash,
        snapshot: inspection.snapshot,
        publication,
        state,
        recovery,
    })
}

/// Finish only the recorded candidate, or acknowledge that its exact inode and
/// content are already published. This does not complete or replay the run.
pub fn recover_file_publication(
    project: &Path,
    scope: &str,
    id: Uuid,
    publication_id: Uuid,
    reviewed_snapshot: &str,
) -> Result<FileRecoveryReceipt, String> {
    let mut claim = reconciliation::locked_claim(project, scope, id)?;
    let inspection = reconciliation::capture(project, &mut claim, scope, id)?;
    if reviewed_snapshot != inspection.snapshot_hash {
        return Err("invocation evidence changed since review".into());
    }
    let publication = intent(&inspection, publication_id)?;
    if let Some(receipt) = read_receipt(project, &inspection, &publication)? {
        if publication.inspect()? != PublicationState::Published {
            return Err("previously recovered output changed; no file altered".into());
        }
        return Ok(receipt);
    }
    if inspection.status == "recorded" {
        return Err("invocation already has a recorded result".into());
    }
    if reconciliation::read_resolution(
        project,
        &mut claim,
        scope,
        id,
        &inspection.snapshot.request_hash,
    )?
    .is_some()
    {
        return Err(
            "invocation already has an operator resolution; no new file recovery permitted".into(),
        );
    }
    let parent = project.join(".symbiont/governed");
    private_directory(&parent)?;
    std::fs::symlink_metadata(parent.join("audit-signing.key"))
        .map_err(|_| "original signing key is unavailable")?;
    let key = load_key(&parent)?;
    if hex::encode(key.verifying_key().to_bytes()) != inspection.snapshot.audit.public_key {
        return Err("current signing key differs from the original invocation key".into());
    }
    let store = project.join(".symbiont/invocations");
    let _guard = store_lock(&store)?;
    let current = reconciliation::capture(project, &mut claim, scope, id)?;
    if current.snapshot_hash != inspection.snapshot_hash {
        return Err("invocation evidence changed during recovery".into());
    }
    // Reserve store capacity before the filesystem effect. A receipt failure
    // after publication can be retried by inspecting the same inode and hash.
    let receipt_allowance = serde_json::to_vec(&serde_json::json!({
        "snapshot":inspection.snapshot, "publication":publication,
        "created_files":[{"path":publication.path,"bytes":publication.bytes,"sha256":publication.sha256}]
    }))
    .map_err(|e| e.to_string())?
    .len() as u64
        + 4096;
    if receipt_allowance > MAX_RECORD_BYTES {
        return Err("file recovery receipt would exceed its bound".into());
    }
    capacity(&store, 1, receipt_allowance)?;
    let created_files = publication.recover()?;
    // SAFETY: geteuid has no preconditions and identifies the local service user.
    let payload = FileRecoveryReceipt {
        version: 1,
        snapshot: inspection.snapshot,
        publication,
        created_files,
        operator_uid: unsafe { libc::geteuid() },
        recorded_at: chrono::Utc::now(),
    };
    let signature = hex::encode(key.sign(&signing_bytes(&payload)?).to_bytes());
    let encoded =
        serde_json::to_vec(&SignedReceipt { payload, signature }).map_err(|e| e.to_string())?;
    if encoded.len() as u64 > receipt_allowance {
        return Err("file recovered but receipt exceeds its bound".into());
    }
    let mut temporary = tempfile::NamedTempFile::new_in(&store).map_err(|e| e.to_string())?;
    temporary
        .write_all(&encoded)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    temporary
        .persist_noclobber(receipt_path(project, scope, id, publication_id)?)
        .map_err(|e| e.to_string())?;
    File::open(&store)
        .and_then(|file| file.sync_all())
        .map_err(|e| e.to_string())?;
    read_receipt(project, &current, &intent(&current, publication_id)?)?
        .ok_or_else(|| "file recovered but recovery receipt is unconfirmed".into())
}
