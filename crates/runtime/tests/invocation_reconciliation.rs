#![cfg(unix)]

use serde_json::json;
use std::{io::Write, os::unix::fs::PermissionsExt, path::Path};
use symbi_runtime::{
    reasoning::{
        invocation::{
            open_invocation, reconciliation::*, ExistingInvocation, Invocation, OpenInvocation,
        },
        loop_types::{JournalEntry, LoopConfig, LoopEvent, TerminationReason},
    },
    types::AgentId,
};
use uuid::Uuid;

async fn claim(
    project: &Path,
    scope: &str,
    id: Uuid,
    request: &serde_json::Value,
    agent: AgentId,
) -> Box<Invocation> {
    let OpenInvocation::Fresh(owner) = open_invocation(project, scope, id, request, agent)
        .await
        .unwrap()
    else {
        panic!("expected fresh claim")
    };
    owner
}

async fn event(owner: &Invocation, event: LoopEvent) {
    owner
        .journal()
        .append(JournalEntry {
            sequence: 0,
            timestamp: chrono::Utc::now(),
            agent_id: owner.agent_id(),
            iteration: 0,
            event,
        })
        .await
        .unwrap();
}

async fn start(owner: &Invocation) {
    event(
        owner,
        LoopEvent::Started {
            agent_id: owner.agent_id(),
            config: Box::new(LoopConfig::default()),
            execution_context: Default::default(),
        },
    )
    .await;
}

fn review(inspection: &InvocationInspection) -> ResolutionReview {
    ResolutionReview {
        snapshot_hash: inspection.snapshot_hash.clone(),
        outcome: ResolutionOutcome::Failed,
        rationale: "Inspected the original effect and confirmed all workers stopped.".into(),
        evidence: vec![ResolutionEvidence {
            reference: "operator/worker-and-effect-observation.json".into(),
            sha256: "a".repeat(64),
        }],
        effects_stopped: true,
    }
}

fn claim_path(project: &Path) -> std::path::PathBuf {
    std::fs::read_dir(project.join(".symbiont/invocations"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .unwrap()
}

#[tokio::test]
async fn inactive_unknown_run_resolves_without_changing_evidence_or_granting_replay() {
    let root = tempfile::tempdir().unwrap();
    let id = Uuid::new_v4();
    let request = json!({"task":"effect"});
    let agent = AgentId::new();
    let owner = claim(root.path(), "fixture", id, &request, agent).await;
    start(&owner).await;
    event(
        &owner,
        LoopEvent::ToolDispatchStarted {
            dispatch_id: Uuid::new_v4(),
            run_key: "run".into(),
            call_id: "call".into(),
            call_fingerprint: "hash".into(),
            tool_name: "effect".into(),
        },
    )
    .await;
    assert!(inspect_invocation(root.path(), "fixture", id)
        .unwrap_err()
        .contains("still owned"));
    let journal = owner.audit().path.clone();
    drop(owner);
    let path = claim_path(root.path());
    let before = std::fs::read(&path).unwrap();
    let audit_before = std::fs::read(&journal).unwrap();
    let inspection = inspect_invocation(root.path(), "fixture", id).unwrap();
    assert!(
        inspection
            .recovery
            .as_ref()
            .unwrap()
            .requires_reconciliation
    );
    assert_eq!(inspection.recovery.as_ref().unwrap().effects.len(), 1);
    let review = review(&inspection);
    let receipt = reconcile_invocation(root.path(), "fixture", id, review.clone()).unwrap();
    assert_eq!(receipt.review, review);
    assert_eq!(std::fs::read(path).unwrap(), before);
    assert_eq!(std::fs::read(journal).unwrap(), audit_before);
    assert_eq!(
        reconcile_invocation(root.path(), "fixture", id, review.clone())
            .unwrap()
            .recorded_at,
        receipt.recorded_at
    );
    let OpenInvocation::Existing(ExistingInvocation::Reconciled { resolution, .. }) =
        open_invocation(root.path(), "fixture", id, &request, agent)
            .await
            .unwrap()
    else {
        panic!("resolved ID granted replay or lost its receipt")
    };
    assert_eq!(resolution.review, review);
    assert!(open_invocation(
        root.path(),
        "fixture",
        id,
        &json!({"task":"changed"}),
        agent
    )
    .await
    .is_err());
    let mut changed = review;
    changed.outcome = ResolutionOutcome::Completed;
    assert!(reconcile_invocation(root.path(), "fixture", id, changed).is_err());
    assert_eq!(
        inspect_invocation(root.path(), "fixture", id)
            .unwrap()
            .status,
        "reconciled"
    );
}

#[tokio::test]
async fn torn_claim_and_journal_tails_are_retained_and_stale_reviews_refused() {
    let root = tempfile::tempdir().unwrap();
    let id = Uuid::new_v4();
    let agent = AgentId::new();
    let request = json!({});
    let owner = claim(root.path(), "fixture", id, &request, agent).await;
    start(&owner).await;
    let journal = owner.audit().path.clone();
    drop(owner);
    let stale = review(&inspect_invocation(root.path(), "fixture", id).unwrap());
    let path = claim_path(root.path());
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{\"kind\":\"res")
        .unwrap();
    std::fs::OpenOptions::new()
        .append(true)
        .open(&journal)
        .unwrap()
        .write_all(b"{\"payload\":")
        .unwrap();
    assert!(reconcile_invocation(root.path(), "fixture", id, stale)
        .unwrap_err()
        .contains("changed since review"));
    let inspection = inspect_invocation(root.path(), "fixture", id).unwrap();
    assert!(inspection.recovery.as_ref().unwrap().unverified_tail_bytes > 0);
    let before = std::fs::read(&path).unwrap();
    reconcile_invocation(root.path(), "fixture", id, review(&inspection)).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(matches!(
        open_invocation(root.path(), "fixture", id, &request, agent)
            .await
            .unwrap(),
        OpenInvocation::Existing(ExistingInvocation::Reconciled { .. })
    ));
    assert!(
        open_invocation(root.path(), "fixture", id, &json!({"changed":true}), agent)
            .await
            .is_err()
    );
    std::fs::OpenOptions::new()
        .append(true)
        .open(journal)
        .unwrap()
        .write_all(b"more")
        .unwrap();
    assert!(open_invocation(root.path(), "fixture", id, &request, agent)
        .await
        .is_err());
}

#[tokio::test]
async fn forged_receipt_and_removed_claim_never_create_another_owner() {
    let root = tempfile::tempdir().unwrap();
    let id = Uuid::new_v4();
    let agent = AgentId::new();
    let request = json!({});
    let owner = claim(root.path(), "fixture", id, &request, agent).await;
    start(&owner).await;
    drop(owner);
    let inspection = inspect_invocation(root.path(), "fixture", id).unwrap();
    let mut invalid = review(&inspection);
    invalid.effects_stopped = false;
    assert!(reconcile_invocation(root.path(), "fixture", id, invalid).is_err());
    reconcile_invocation(root.path(), "fixture", id, review(&inspection)).unwrap();
    let receipt = std::fs::read_dir(root.path().join(".symbiont/invocations"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.to_string_lossy().ends_with(".resolved.json"))
        .unwrap();
    let mut signed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&receipt).unwrap()).unwrap();
    signed["payload"]["review"]["outcome"] = json!("completed");
    std::fs::write(&receipt, serde_json::to_vec(&signed).unwrap()).unwrap();
    assert!(open_invocation(root.path(), "fixture", id, &request, agent)
        .await
        .is_err());
    std::fs::remove_file(claim_path(root.path())).unwrap();
    assert!(open_invocation(root.path(), "fixture", id, &request, agent)
        .await
        .is_err());
    std::fs::remove_file(&receipt).unwrap();
    std::os::unix::fs::symlink(root.path().join("missing-receipt"), receipt).unwrap();
    assert!(open_invocation(root.path(), "fixture", id, &request, agent)
        .await
        .is_err());
}

#[tokio::test]
async fn recorded_results_and_invalid_signed_records_cannot_be_overridden() {
    let root = tempfile::tempdir().unwrap();
    let id = Uuid::new_v4();
    let agent = AgentId::new();
    let request = json!({});
    let owner = claim(root.path(), "fixture", id, &request, agent).await;
    start(&owner).await;
    let journal = owner.audit().path.clone();
    event(
        &owner,
        LoopEvent::Terminated {
            reason: TerminationReason::Completed,
            iterations: 1,
            total_usage: Default::default(),
            duration: Default::default(),
        },
    )
    .await;
    assert!(matches!(
        owner.finish(json!({"result":10})).await.unwrap(),
        ExistingInvocation::Recorded { .. }
    ));
    let inspection = inspect_invocation(root.path(), "fixture", id).unwrap();
    assert_eq!(inspection.status, "recorded");
    assert!(
        reconcile_invocation(root.path(), "fixture", id, review(&inspection))
            .unwrap_err()
            .contains("verified recorded result")
    );
    std::fs::OpenOptions::new()
        .append(true)
        .open(journal)
        .unwrap()
        .write_all(b"{}\n")
        .unwrap();
    assert!(inspect_invocation(root.path(), "fixture", id).is_err());
}

#[tokio::test]
async fn empty_startup_journal_and_read_review_path_limits_are_explicit() {
    let root = tempfile::tempdir().unwrap();
    let id = Uuid::new_v4();
    let agent = AgentId::new();
    let request = json!({});
    drop(claim(root.path(), "fixture", id, &request, agent).await);
    let inspection = inspect_invocation(root.path(), "fixture", id).unwrap();
    assert!(inspection.recovery.is_none());
    let review = review(&inspection);
    let path = root.path().join("review.json");
    std::fs::write(&path, serde_json::to_vec(&review).unwrap()).unwrap();
    assert_eq!(read_review(&path).unwrap(), review);
    let link = root.path().join("linked.json");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert!(read_review(&link).is_err());
    reconcile_invocation(root.path(), "fixture", id, review).unwrap();
    assert!(matches!(
        open_invocation(root.path(), "fixture", id, &request, agent)
            .await
            .unwrap(),
        OpenInvocation::Existing(ExistingInvocation::Reconciled { .. })
    ));
    std::fs::set_permissions(
        claim_path(root.path()),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(inspect_invocation(root.path(), "fixture", id).is_err());
}

#[cfg(feature = "cron")]
#[tokio::test]
async fn cron_reconciliation_preserves_history_and_requires_explicit_resume() {
    use symbi_runtime::types::{
        AgentConfig, ExecutionMode, Priority, ResourceLimits, SecurityTier,
    };
    use symbi_runtime::{
        scheduler::{invocations::InvocationIdentity, occurrences::Occurrence},
        CronJobDefinition, CronJobStatus, JobRunStatus, JobStore, SqliteJobStore,
    };
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let id = Uuid::new_v4();
    let config = AgentConfig {
        id: AgentId::new(),
        name: "fixture".into(),
        dsl_source: "agent fixture() {}".into(),
        execution_mode: ExecutionMode::Ephemeral,
        security_tier: SecurityTier::Tier1,
        resource_limits: ResourceLimits::default(),
        capabilities: vec![],
        policies: vec![],
        metadata: Default::default(),
        priority: Priority::Normal,
    };
    let job = CronJobDefinition::new(
        "fixture".into(),
        "0 0 0 1 1 * 2099".into(),
        "UTC".into(),
        config,
    );
    let store = SqliteJobStore::open(&root.path().join("cron.db")).unwrap();
    store.bind_project(root.path()).await.unwrap();
    store.save_job(&job).await.unwrap();
    let occurrence = Occurrence::manual(
        job.clone(),
        InvocationIdentity {
            id,
            context: json!({"caller":"fixture"}),
        },
    );
    store
        .prepare_occurrence(&occurrence, None, 10)
        .await
        .unwrap()
        .unwrap();
    let (config, input, identity) = occurrence.admission();
    let request = json!({"version":1,"context":identity.context,"target":config,"input":input});
    let owner = claim(
        root.path(),
        "scheduler:v1",
        id,
        &request,
        job.agent_config.id,
    )
    .await;
    start(&owner).await;
    store
        .occurrence_running(&occurrence, owner.audit())
        .await
        .unwrap();
    store
        .finish_occurrence(
            &occurrence,
            None,
            Some(owner.audit()),
            Some("lost effect"),
            true,
        )
        .await
        .unwrap();
    drop(owner);
    assert!(store
        .job_has_unresolved_occurrence(job.job_id)
        .await
        .unwrap());
    assert!(store.reconcile_occurrence(root.path(), id).await.is_err());
    let inspection = inspect_invocation(root.path(), "scheduler:v1", id).unwrap();
    reconcile_invocation(root.path(), "scheduler:v1", id, review(&inspection)).unwrap();
    assert!(store.reconcile_occurrence(root.path(), id).await.unwrap());
    assert!(!store.reconcile_occurrence(root.path(), id).await.unwrap());
    let row = &store.get_run_history(job.job_id, 10).await.unwrap()[0];
    assert_eq!(row.status, JobRunStatus::Reconciled);
    assert!(row.execution.is_none());
    assert_eq!(row.error.as_deref(), Some("lost effect"));
    assert!(row.resolution.is_some());
    let paused = store.get_job(job.job_id).await.unwrap().unwrap();
    assert_eq!(paused.status, CronJobStatus::Paused);
    assert!(!paused.enabled);
    assert!(!store
        .job_has_unresolved_occurrence(job.job_id)
        .await
        .unwrap());
    assert!(matches!(
        open_invocation(
            root.path(),
            "scheduler:v1",
            id,
            &request,
            job.agent_config.id
        )
        .await
        .unwrap(),
        OpenInvocation::Existing(ExistingInvocation::Reconciled { .. })
    ));
}
