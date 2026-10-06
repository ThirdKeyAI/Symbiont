use super::evaluation::{Evaluator, TrialReference};
use super::*;
use crate::reasoning::protected_journal::load_key;
use ed25519_dalek::{Signature, Signer, SigningKey};
use serde::de::DeserializeOwned;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedDocument {
    payload: serde_json::Value,
    signature: String,
}

/// Offline authenticity verification with an independently supplied public key.
/// A valid signature is not approval, activation or evidence of task quality.
pub fn verify_document(
    bytes: &[u8],
    kind: &str,
    public_key: &[u8; 32],
) -> Result<serde_json::Value, String> {
    if !["state", "candidate", "evaluation", "approval"].contains(&kind)
        || bytes.len() as u64 > MAX_DOCUMENT_BYTES
    {
        return Err("unsupported document kind or oversized document".into());
    }
    let document: SignedDocument = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let signature =
        Signature::from_slice(&hex::decode(&document.signature).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    ed25519_dalek::VerifyingKey::from_bytes(public_key)
        .map_err(|e| e.to_string())?
        .verify_strict(&Store::signing_bytes(kind, &document.payload)?, &signature)
        .map_err(|_| "improvement document signature is invalid")?;
    if document
        .payload
        .get("schema_version")
        .and_then(|v| v.as_u64())
        != Some(u64::from(SCHEMA_VERSION))
    {
        return Err("unsupported improvement document version".into());
    }
    Ok(document.payload)
}

/// Holding the store serializes administrative changes. Drop it before running
/// an agent; a PinnedImprovement owns an immutable copy of its admitted version.
pub struct Store {
    project: PathBuf,
    directory: PathBuf,
    _lock: File,
    signer: SigningKey,
    state: WorkflowState,
    failed: bool,
}

fn read_bytes(path: &Path, private: bool) -> Result<Vec<u8>, String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let before = file.metadata().map_err(|e| e.to_string())?;
    if !before.is_file()
        || before.len() > MAX_DOCUMENT_BYTES
        || before.nlink() != 1
        || (private && (before.uid() != uid() || before.mode() & 0o077 != 0))
    {
        return Err(
            "expected a bounded, singly linked regular file with appropriate ownership".into(),
        );
    }
    let mut bytes = Vec::new();
    (&file)
        .take(MAX_DOCUMENT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let after = file.metadata().map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_DOCUMENT_BYTES
        || after.len() != before.len()
        || after.mtime() != before.mtime()
        || after.mtime_nsec() != before.mtime_nsec()
        || after.ctime() != before.ctime()
        || after.ctime_nsec() != before.ctime_nsec()
    {
        return Err("document changed during read or exceeds its bound".into());
    }
    Ok(bytes)
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    serde_json::from_slice(&read_bytes(path, false)?).map_err(|e| e.to_string())
}

fn uid() -> u32 {
    // SAFETY: geteuid has no pointer arguments or memory-safety preconditions.
    unsafe { libc::geteuid() }
}

fn directory(path: &Path, create: bool, private: bool) -> Result<(), String> {
    if create {
        match std::fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_dir()
        || meta.uid() != uid()
        || meta.mode() & if private { 0o077 } else { 0o022 } != 0
    {
        return Err("improvement directories must be unlinked, operator-owned and protected from other writers".into());
    }
    Ok(())
}

fn open_directory(
    project: &Path,
    workflow: &str,
    create: bool,
) -> Result<(PathBuf, PathBuf, File), String> {
    identifier(workflow)?;
    let project = project.canonicalize().map_err(|e| e.to_string())?;
    directory(&project.join(".symbiont"), create, false)?;
    let parent = project.join(".symbiont/improvements");
    directory(&parent, create, true)?;
    let root = parent.join(workflow);
    directory(&root, create, true)?;
    directory(&root.join("objects"), create, true)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(root.join(".lock"))
        .map_err(|e| e.to_string())?;
    let meta = lock.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.uid() != uid() || meta.mode() & 0o077 != 0 || meta.nlink() != 1 {
        return Err("unsafe improvement store lock".into());
    }
    loop {
        // SAFETY: the descriptor remains live for the lifetime of the Store.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            break;
        }
        // flock is interruptible. Only real contention means the store is busy.
        if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        return Err("improvement store is busy; retry the operation".into());
    }
    Ok((project, root, lock))
}

/// Fingerprint the governed deployment inputs, including directory additions.
/// Contents and credentials are never copied into candidate artifacts.
fn deployment(project: &Path) -> Result<BTreeMap<String, String>, String> {
    fn visit(
        project: &Path,
        path: &Path,
        out: &mut BTreeMap<String, String>,
        total: &mut usize,
    ) -> Result<(), String> {
        if out.len() >= 1024 {
            return Err("deployment fingerprint exceeds 1024 entries".into());
        }
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.to_string()),
        };
        let name = path
            .strip_prefix(project)
            .map_err(|e| e.to_string())?
            .to_str()
            .ok_or("deployment paths must be UTF-8")?
            .to_string();
        if metadata.is_dir() {
            out.insert(format!("{name}/"), sha256(b"directory"));
            for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
                visit(
                    project,
                    &entry.map_err(|e| e.to_string())?.path(),
                    out,
                    total,
                )?;
            }
        } else if metadata.is_file() {
            let bytes = read_bytes(path, false)?;
            *total += bytes.len();
            if *total > 16 * 1024 * 1024 {
                return Err("deployment inputs exceed 16 MiB".into());
            }
            out.insert(name, sha256(&bytes));
        } else {
            return Err("deployment inputs must not contain links or special files".into());
        }
        Ok(())
    }
    let mut result = BTreeMap::new();
    let mut total = 0;
    for root in [
        "symbiont.toml",
        "toolclad.toml",
        "mcp-config.toml",
        "scope",
        "policies",
        "tools",
    ] {
        visit(project, &project.join(root), &mut result, &mut total)?;
    }
    Ok(result)
}

impl Store {
    pub fn initialize(
        project: &Path,
        workflow: &str,
        agent: &str,
        source: &str,
        suite: AcceptanceSuite,
    ) -> Result<Self, String> {
        suite.validate()?;
        text(agent, 256)?;
        text(source, 1024 * 1024)?;
        let (project, root, lock) = open_directory(project, workflow, true)?;
        if std::fs::symlink_metadata(root.join("state.json")).is_ok() {
            return Err(
                "workflow already exists; initialization never replaces its acceptance criteria"
                    .into(),
            );
        }
        let signer = load_key(&root)?;
        let state = WorkflowState {
            schema_version: SCHEMA_VERSION,
            workflow: workflow.into(),
            enabled: true,
            agent: agent.into(),
            source_sha256: sha256(source.as_bytes()),
            deployment: deployment(&project)?,
            suite,
            active: None,
            activations: vec![],
            controls: vec![ControlChange {
                enabled: true,
                operator_uid: uid(),
                recorded_at: chrono::Utc::now(),
            }],
        };
        let store = Self {
            project,
            directory: root,
            _lock: lock,
            signer,
            state,
            failed: false,
        };
        store.save_state()?;
        Ok(store)
    }

    pub fn open(project: &Path, workflow: &str) -> Result<Self, String> {
        let (project, root, lock) = open_directory(project, workflow, false)?;
        // Inspection must never manufacture a replacement trust root.
        read_bytes(&root.join("audit-signing.key"), true)?;
        let signer = load_key(&root)?;
        let state: WorkflowState = Self::read_signed(&root.join("state.json"), "state", &signer)?;
        if state.schema_version != SCHEMA_VERSION || state.workflow != workflow {
            return Err("invalid workflow state identity".into());
        }
        state.suite.validate()?;
        if state.controls.is_empty()
            || state.controls.len() > 1024
            || state.controls.last().map(|c| c.enabled) != Some(state.enabled)
            || state.activations.len() > 1024
            || state.active != state.activations.last().map(|a| a.candidate.clone())
        {
            return Err("invalid active workflow history".into());
        }
        Ok(Self {
            project,
            directory: root,
            _lock: lock,
            signer,
            state,
            failed: false,
        })
    }

    pub fn state(&self) -> &WorkflowState {
        &self.state
    }
    pub fn public_key(&self) -> String {
        hex::encode(self.signer.verifying_key().to_bytes())
    }

    fn signing_bytes(kind: &str, payload: &serde_json::Value) -> Result<Vec<u8>, String> {
        Ok(format!(
            "symbi-improvement:{SCHEMA_VERSION}:{kind}:{}",
            digest(payload)?
        )
        .into_bytes())
    }

    fn read_signed<T: DeserializeOwned>(
        path: &Path,
        kind: &str,
        key: &SigningKey,
    ) -> Result<T, String> {
        serde_json::from_value(verify_document(
            &read_bytes(path, true)?,
            kind,
            &key.verifying_key().to_bytes(),
        )?)
        .map_err(|e| e.to_string())
    }

    pub fn export_document(
        &self,
        kind: &str,
        id: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let path = match kind {
            "state" if id.is_none() => self.directory.join("state.json"),
            "candidate" | "evaluation" | "approval" => {
                self.object_path(kind, id.ok_or("object digest is required")?)?
            }
            _ => return Err("unsupported document kind or unexpected digest".into()),
        };
        let bytes = read_bytes(&path, true)?;
        let value = verify_document(&bytes, kind, &self.signer.verifying_key().to_bytes())?;
        if let Some(expected) = id {
            if digest(&value)? != expected {
                return Err("object digest mismatch".into());
            }
        }
        serde_json::from_slice(&bytes).map_err(|e| e.to_string())
    }

    fn write_signed<T: Serialize>(
        &self,
        path: &Path,
        kind: &str,
        value: &T,
        replace: bool,
    ) -> Result<(), String> {
        let payload = serde_json::to_value(value).map_err(|e| e.to_string())?;
        let signature = hex::encode(
            self.signer
                .sign(&Self::signing_bytes(kind, &payload)?)
                .to_bytes(),
        );
        let bytes = serde_json::to_vec_pretty(&SignedDocument { payload, signature })
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
            return Err("signed document exceeds size bound".into());
        }
        let parent = path.parent().ok_or("missing document parent")?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        temporary
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        temporary
            .write_all(&bytes)
            .and_then(|_| temporary.as_file().sync_all())
            .map_err(|e| e.to_string())?;
        if replace {
            temporary.persist(path).map_err(|e| e.to_string())?;
        } else {
            match temporary.persist_noclobber(path) {
                Ok(_) => {}
                Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if read_bytes(path, true)? != bytes {
                        return Err(
                            "immutable object already exists with different contents".into()
                        );
                    }
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())
    }

    fn save_state(&self) -> Result<(), String> {
        self.write_signed(
            &self.directory.join("state.json"),
            "state",
            &self.state,
            true,
        )
    }
    fn object_path(&self, kind: &str, id: &str) -> Result<PathBuf, String> {
        hash_valid(id)?;
        Ok(self
            .directory
            .join("objects")
            .join(format!("{id}.{kind}.json")))
    }
    fn put<T: Serialize>(&self, kind: &str, value: &T) -> Result<String, String> {
        let id = digest(value)?;
        if std::fs::read_dir(self.directory.join("objects"))
            .map_err(|e| e.to_string())?
            .take(4096)
            .count()
            >= 4096
        {
            return Err("workflow object limit reached".into());
        }
        self.write_signed(&self.object_path(kind, &id)?, kind, value, false)?;
        Ok(id)
    }
    fn get<T: DeserializeOwned + Serialize>(&self, kind: &str, id: &str) -> Result<T, String> {
        let value = Self::read_signed(&self.object_path(kind, id)?, kind, &self.signer)?;
        if digest(&value)? != id {
            return Err("improvement content digest mismatch".into());
        }
        Ok(value)
    }
    fn enabled(&self) -> Result<(), String> {
        self.usable()?;
        if self.state.enabled {
            Ok(())
        } else {
            Err("improvements are disabled for this workflow".into())
        }
    }
    fn check_deployment(&self) -> Result<(), String> {
        if deployment(&self.project)? != self.state.deployment {
            return Err("deployment inputs changed; initialize and evaluate a new workflow".into());
        }
        Ok(())
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<(), String> {
        self.usable()?;
        if self.state.enabled == enabled {
            return Ok(());
        }
        if self.state.controls.len() >= 1024 {
            return Err("workflow control history limit reached".into());
        }
        let mut next = self.state.clone();
        next.enabled = enabled;
        next.controls.push(ControlChange {
            enabled,
            operator_uid: uid(),
            recorded_at: chrono::Utc::now(),
        });
        self.commit_state(next)
    }

    fn usable(&self) -> Result<(), String> {
        if self.failed {
            Err("state publication failed; reopen and inspect the workflow before further operations".into())
        } else {
            Ok(())
        }
    }

    fn commit_state(&mut self, next: WorkflowState) -> Result<(), String> {
        if let Err(error) =
            self.write_signed(&self.directory.join("state.json"), "state", &next, true)
        {
            // Rename might have succeeded before a sync failure. This handle
            // must not authorize from either an old or merely in-memory state.
            self.failed = true;
            return Err(error);
        }
        self.state = next;
        Ok(())
    }

    pub fn propose(&self, proposal: Proposal) -> Result<String, String> {
        self.enabled()?;
        self.check_deployment()?;
        proposal.validate()?;
        self.put(
            "candidate",
            &Candidate {
                schema_version: SCHEMA_VERSION,
                workflow: self.state.workflow.clone(),
                agent: self.state.agent.clone(),
                source_sha256: self.state.source_sha256.clone(),
                deployment_sha256: digest(&self.state.deployment)?,
                suite_sha256: digest(&self.state.suite)?,
                parent: self.state.active.clone(),
                proposal,
            },
        )
    }
    pub fn candidate(&self, id: &str) -> Result<Candidate, String> {
        let c: Candidate = self.get("candidate", id)?;
        if c.schema_version != SCHEMA_VERSION
            || c.workflow != self.state.workflow
            || c.agent != self.state.agent
            || c.source_sha256 != self.state.source_sha256
            || c.deployment_sha256 != digest(&self.state.deployment)?
            || c.suite_sha256 != digest(&self.state.suite)?
        {
            return Err("candidate does not belong to this workflow contract".into());
        }
        c.proposal.validate()?;
        Ok(c)
    }
    pub fn evaluate(
        &self,
        id: &str,
        references: &[TrialReference],
        evaluator: &dyn Evaluator,
    ) -> Result<(String, EvaluationReport), String> {
        self.enabled()?;
        self.check_deployment()?;
        let candidate = self.candidate(id)?;
        if references.len() != self.state.suite.cases.len() {
            return Err("one trial per acceptance case is required".into());
        }
        let audit_directory = self.project.join(".symbiont/governed");
        read_bytes(&audit_directory.join("audit-signing.key"), true)?;
        let key = load_key(&audit_directory)?.verifying_key().to_bytes();
        let trials = references
            .iter()
            .map(|r| evaluation::load_trial(r, &key))
            .collect::<Result<Vec<_>, _>>()?;
        // Always check identity, coverage and ceilings independently of custom scoring.
        let baseline =
            evaluation::ExactAnswerEvaluator.evaluate(&candidate, &self.state.suite, &trials)?;
        let report = evaluator.evaluate(&candidate, &self.state.suite, &trials)?;
        if report.schema_version != SCHEMA_VERSION
            || report.candidate != id
            || report.suite_sha256 != digest(&self.state.suite)?
            || report.environment != baseline.environment
            || report.cases != baseline.cases
        {
            return Err("evaluator returned a report for a different contract".into());
        }
        text(&report.evaluator, 256)?;
        // First-release promotion uses the independently specified exact-answer
        // contract. Custom evaluators may strengthen it, but cannot waive a gate.
        if report.accepted && !baseline.accepted {
            return Err("evaluator attempted to waive an acceptance gate".into());
        }
        let hash = self.put("evaluation", &report)?;
        Ok((hash, report))
    }
    pub fn evaluation(&self, id: &str) -> Result<EvaluationReport, String> {
        self.get("evaluation", id)
    }

    pub fn approve(
        &self,
        candidate: &str,
        evaluation: &str,
        rationale: &str,
    ) -> Result<String, String> {
        self.enabled()?;
        self.check_deployment()?;
        self.candidate(candidate)?;
        text(rationale, 4096)?;
        let report = self.evaluation(evaluation)?;
        if !report.accepted
            || report.candidate != candidate
            || report.suite_sha256 != digest(&self.state.suite)?
        {
            return Err(
                "approval requires a passing evaluation of this exact candidate and suite".into(),
            );
        }
        self.put(
            "approval",
            &Approval {
                schema_version: SCHEMA_VERSION,
                workflow: self.state.workflow.clone(),
                candidate: candidate.into(),
                evaluation: evaluation.into(),
                rationale: rationale.into(),
                operator_uid: uid(),
                recorded_at: chrono::Utc::now(),
            },
        )
    }
    fn approved(&self, candidate: &str, approval: &str) -> Result<EvaluationReport, String> {
        self.candidate(candidate)?;
        let a: Approval = self.get("approval", approval)?;
        if a.schema_version != SCHEMA_VERSION
            || a.workflow != self.state.workflow
            || a.candidate != candidate
        {
            return Err("approval is not bound to the selected candidate".into());
        }
        let report = self.evaluation(&a.evaluation)?;
        if !report.accepted
            || report.candidate != candidate
            || report.suite_sha256 != digest(&self.state.suite)?
        {
            return Err("approved evaluation no longer validates this candidate".into());
        }
        Ok(report)
    }
    pub fn activate(
        &mut self,
        candidate: &str,
        approval: &str,
        expected: Option<&str>,
        rollback: bool,
    ) -> Result<(), String> {
        self.enabled()?;
        self.check_deployment()?;
        self.approved(candidate, approval)?;
        if self.state.active.as_deref() != expected {
            return Err("active version changed; inspect and review before retrying".into());
        }
        if self.state.active.as_deref() == Some(candidate) {
            return Err("candidate is already active".into());
        }
        if self.state.activations.len() >= 1024 {
            return Err("activation history limit reached".into());
        }
        if rollback
            && !self
                .state
                .activations
                .iter()
                .any(|a| a.candidate == candidate)
        {
            return Err("rollback target must have been activated previously".into());
        }
        if !rollback && self.candidate(candidate)?.parent.as_deref() != expected {
            return Err("candidate was proposed against a different active version".into());
        }
        let mut next = self.state.clone();
        next.activations.push(Activation {
            candidate: candidate.into(),
            approval: approval.into(),
            previous: self.state.active.clone(),
            rollback,
            operator_uid: uid(),
            recorded_at: chrono::Utc::now(),
        });
        next.active = Some(candidate.into());
        self.commit_state(next)
    }

    pub fn pin(
        &self,
        agent: &str,
        source: &str,
        trial: Option<&str>,
    ) -> Result<PinnedImprovement, String> {
        self.enabled()?;
        self.check_deployment()?;
        if agent != self.state.agent || sha256(source.as_bytes()) != self.state.source_sha256 {
            return Err("agent source changed or does not match the workflow".into());
        }
        let id = trial
            .or(self.state.active.as_deref())
            .ok_or("workflow has no approved active version")?;
        let candidate = self.candidate(id)?;
        let (approval, environment, mode) = if trial.is_some() {
            (None, None, SelectionMode::Trial)
        } else {
            let activation = self.state.activations.last().ok_or("missing activation")?;
            let report = self.approved(id, &activation.approval)?;
            (
                Some(activation.approval.clone()),
                Some(report.environment),
                SelectionMode::Approved,
            )
        };
        Ok(PinnedImprovement::new(
            id.into(),
            candidate,
            approval,
            environment,
            mode,
        ))
    }
}
