use super::*;
use crate::scheduler::job_store::JobStore;
use crate::types::{AgentId, Priority, ResourceLimits, SecurityTier};

fn private_root() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    root
}

fn job() -> CronJobDefinition {
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
    let mut job =
        CronJobDefinition::new("fixture".into(), "* * * * * *".into(), "UTC".into(), config);
    job.next_run = Some(Utc::now() - chrono::Duration::seconds(5));
    job
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn timer_intent_and_clock_commit_once_across_connections() {
    let root = private_root();
    let path = root.path().join("cron.db");
    let a = std::sync::Arc::new(SqliteJobStore::open(&path).unwrap());
    let b = std::sync::Arc::new(SqliteJobStore::open(&path).unwrap());
    let job = job();
    a.save_job(&job).await.unwrap();
    let occurrence = Occurrence::timer(job.clone()).unwrap();
    let next = Some(Utc::now() + chrono::Duration::hours(1));
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let attempt = |store: std::sync::Arc<SqliteJobStore>| {
        let barrier = barrier.clone();
        let occurrence = occurrence.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            store.prepare_occurrence(&occurrence, next, 10).await
        })
    };
    let (first, second) = tokio::join!(attempt(a.clone()), attempt(b.clone()));
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(
        first.unwrap().unwrap().occurrence.identity.id,
        second.unwrap().unwrap().occurrence.identity.id
    );
    let stored = b.get_job(job.job_id).await.unwrap().unwrap();
    assert_eq!(stored.run_count, 1);
    assert_eq!(stored.next_run, next);
    let history = a.get_run_history(job.job_id, 10).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].status, JobRunStatus::Pending);
    assert_eq!(a.pending_occurrences().await.unwrap().len(), 1);
}

#[tokio::test]
async fn failed_intent_insert_rolls_back_clock_and_history() {
    let store = SqliteJobStore::open_in_memory().unwrap();
    let job = job();
    store.save_job(&job).await.unwrap();
    store.conn.lock().await.execute_batch("CREATE TRIGGER refuse_intent BEFORE INSERT ON cron_occurrences BEGIN SELECT RAISE(ABORT,'injected persistence failure'); END;").unwrap();
    assert!(store
        .prepare_occurrence(&Occurrence::timer(job.clone()).unwrap(), None, 10)
        .await
        .is_err());
    let stored = store.get_job(job.job_id).await.unwrap().unwrap();
    assert_eq!(stored.next_run, job.next_run);
    assert_eq!(stored.run_count, 0);
    assert!(stored.enabled);
    assert!(store
        .get_run_history(job.job_id, 10)
        .await
        .unwrap()
        .is_empty());
    assert!(store.pending_occurrences().await.unwrap().is_empty());
}

#[tokio::test]
async fn fingerprint_ignores_mutable_counters_but_binds_source_and_caller() {
    let store = SqliteJobStore::open_in_memory().unwrap();
    let job = job();
    store.save_job(&job).await.unwrap();
    let original = Occurrence::timer(job).unwrap();
    store
        .prepare_occurrence(&original, None, 10)
        .await
        .unwrap()
        .unwrap();
    let mut changed = original.clone();
    changed.job.run_count += 1;
    changed.job.failure_count += 1;
    changed.job.updated_at = Utc::now();
    changed.created_at = Utc::now();
    assert_eq!(
        original.fingerprint().unwrap(),
        changed.fingerprint().unwrap()
    );
    let retry = store
        .prepare_occurrence(&changed, None, 10)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retry.occurrence.created_at, original.created_at);
    changed.job.agent_config.dsl_source.push_str("// altered\n");
    assert!(matches!(
        store.prepare_occurrence(&changed, None, 10).await,
        Err(JobStoreError::InvocationConflict)
    ));
    changed = original.clone();
    changed.identity.context = json!({"caller":"different"});
    assert!(matches!(
        store.prepare_occurrence(&changed, None, 10).await,
        Err(JobStoreError::InvocationConflict)
    ));
}

#[tokio::test]
async fn missing_history_prevents_terminal_transition_and_duplicate_failure_count() {
    let store = SqliteJobStore::open_in_memory().unwrap();
    let job = job();
    store.save_job(&job).await.unwrap();
    let occurrence = Occurrence::timer(job.clone()).unwrap();
    store
        .prepare_occurrence(&occurrence, None, 10)
        .await
        .unwrap();
    store
        .conn
        .lock()
        .await
        .execute("UPDATE job_run_log SET status='succeeded'", [])
        .unwrap();
    assert!(store
        .finish_occurrence(&occurrence, None, None, Some("unknown"), true)
        .await
        .is_err());
    assert_eq!(
        store.pending_occurrences().await.unwrap()[0].state,
        "prepared"
    );
    assert_eq!(
        store
            .get_job(job.job_id)
            .await
            .unwrap()
            .unwrap()
            .failure_count,
        0
    );
    store
        .conn
        .lock()
        .await
        .execute("UPDATE job_run_log SET status='pending'", [])
        .unwrap();
    assert!(store
        .finish_occurrence(&occurrence, None, None, Some("unknown"), true)
        .await
        .unwrap());
    assert!(!store
        .finish_occurrence(&occurrence, None, None, Some("unknown"), true)
        .await
        .unwrap());
    assert_eq!(
        store
            .get_job(job.job_id)
            .await
            .unwrap()
            .unwrap()
            .failure_count,
        1
    );
    assert!(store
        .job_has_unresolved_occurrence(job.job_id)
        .await
        .unwrap());
}

#[tokio::test]
async fn legacy_unfinished_run_remains_an_unresolved_barrier() {
    let root = private_root();
    let path = root.path().join("cron.db");
    let store = SqliteJobStore::open(&path).unwrap();
    let mut job = job();
    store.save_job(&job).await.unwrap();
    let record = JobRunRecord {
        resolution: None,
        run_id: Uuid::new_v4(),
        job_id: job.job_id,
        agent_id: job.agent_config.id,
        started_at: Utc::now(),
        completed_at: None,
        status: JobRunStatus::Running,
        error: None,
        execution_time_ms: None,
        execution: None,
        admission_audit: None,
    };
    store.save_run_record(&record).await.unwrap();
    drop(store);
    let store = SqliteJobStore::open(&path).unwrap();
    assert_eq!(
        store.get_run_history(job.job_id, 10).await.unwrap()[0].status,
        JobRunStatus::Unresolved
    );
    assert!(store
        .job_has_unresolved_occurrence(job.job_id)
        .await
        .unwrap());
    assert_eq!(
        store.get_job(job.job_id).await.unwrap().unwrap().status,
        CronJobStatus::DeadLetter
    );
    // Updating mutable job state cannot erase the retained uncertainty barrier.
    job.updated_at = Utc::now();
    store.save_job(&job).await.unwrap();
    assert!(store.get_due_jobs(Utc::now()).await.unwrap().is_empty());
    assert!(store
        .prepare_occurrence(&Occurrence::timer(job).unwrap(), None, 10)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn store_owner_cannot_be_rebound_to_another_project() {
    let a = private_root();
    let b = private_root();
    let path = a.path().join("cron.db");
    let store = SqliteJobStore::open(&path).unwrap();
    store.bind_project(a.path()).await.unwrap();
    drop(store);
    let store = SqliteJobStore::open(&path).unwrap();
    assert!(store
        .bind_project(b.path())
        .await
        .unwrap_err()
        .to_string()
        .contains("different execution project"));
    store.bind_project(a.path()).await.unwrap();
}

#[test]
fn linked_or_public_store_authority_is_refused() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let root = private_root();
    let path = root.path().join("cron.db");
    let target = root.path().join("target");
    std::fs::write(&target, b"original").unwrap();
    symlink(&target, &path).unwrap();
    assert!(SqliteJobStore::open(&path).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"original");
    std::fs::remove_file(&path).unwrap();
    let store = SqliteJobStore::open(&path).unwrap();
    drop(store);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(SqliteJobStore::open(&path).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    symlink(&target, root.path().join("cron.db-wal")).unwrap();
    assert!(SqliteJobStore::open(&path).is_err());
}

#[tokio::test]
async fn recovery_pages_reach_intents_beyond_the_first_live_owners() {
    let store = SqliteJobStore::open_in_memory().unwrap();
    let mut job = job();
    job.max_concurrent = 64;
    store.save_job(&job).await.unwrap();
    for _ in 0..33 {
        let occurrence = Occurrence::manual(
            job.clone(),
            InvocationIdentity {
                id: Uuid::new_v4(),
                context: json!({"caller":"fixture"}),
            },
        );
        store
            .prepare_occurrence(&occurrence, None, 64)
            .await
            .unwrap()
            .unwrap();
    }
    let first = store.pending_occurrences_after(None).await.unwrap();
    assert_eq!(first.len(), 32);
    let second = store
        .pending_occurrences_after(Some(first.last().unwrap().occurrence.identity.id))
        .await
        .unwrap();
    assert_eq!(second.len(), 1);
    assert!(!first
        .iter()
        .any(|p| p.occurrence.identity.id == second[0].occurrence.identity.id));
    assert_eq!(
        store.pending_occurrences_after(None).await.unwrap().len(),
        32
    );
}
