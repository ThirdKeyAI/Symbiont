#![cfg(target_os = "linux")]

use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::PathBuf,
};
use symbi_runtime::{
    reasoning::{
        effect_journal::ToolEffect,
        invocation::{
            file_recovery::*, open_invocation, reconciliation::*, ExistingInvocation, Invocation,
            OpenInvocation,
        },
        loop_types::{JournalEntry, LoopConfig, LoopEvent},
    },
    sandbox::files::{PublicationIntent, PublicationState},
    types::AgentId,
};
use uuid::Uuid;

struct Fixture {
    root: tempfile::TempDir,
    id: Uuid,
    owner: Option<Box<Invocation>>,
    intent: PublicationIntent,
    journal: PathBuf,
}

impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let id = Uuid::new_v4();
        let OpenInvocation::Fresh(owner) = open_invocation(
            root.path(),
            "publication-fixture",
            id,
            &json!({}),
            AgentId::new(),
        )
        .await
        .unwrap() else {
            panic!("expected fresh fixture");
        };
        let parent = root.path().join("data");
        fs::create_dir(&parent).unwrap();
        let publication_id = Uuid::new_v4();
        let candidate_name = format!(".symbi-publish-{publication_id}");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(parent.join(&candidate_name))
            .unwrap();
        file.write_all(b"verified-output").unwrap();
        file.sync_all().unwrap();
        let candidate = file.metadata().unwrap();
        let directory = fs::metadata(&parent).unwrap();
        let intent = PublicationIntent {
            publication_id,
            path: "report.txt".into(),
            parent_path: parent,
            parent_identity: (directory.dev(), directory.ino()),
            candidate_name,
            candidate_identity: (candidate.dev(), candidate.ino()),
            target_name: "report.txt".into(),
            bytes: 15,
            sha256: hex::encode(Sha256::digest(b"verified-output")),
        };
        for event in [
            LoopEvent::Started {
                agent_id: owner.agent_id(),
                config: Box::new(LoopConfig::default()),
                execution_context: Default::default(),
            },
            LoopEvent::ToolDispatchStarted {
                dispatch_id: Uuid::new_v4(),
                run_key: "fixture".into(),
                call_id: "call".into(),
                call_fingerprint: "fingerprint".into(),
                tool_name: "write_report".into(),
            },
            LoopEvent::ToolEffect {
                run_key: "fixture".into(),
                call_fingerprint: "fingerprint".into(),
                effect: ToolEffect::FilePublicationPrepared {
                    intent: intent.clone(),
                },
            },
        ] {
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
        let journal = owner.audit().path.clone();
        Self {
            root,
            id,
            owner: Some(owner),
            intent,
            journal,
        }
    }
    fn release(&mut self) {
        self.owner.take();
    }
    fn inspect(&self) -> FilePublicationInspection {
        inspect_file_publication(
            self.root.path(),
            "publication-fixture",
            self.id,
            self.intent.publication_id,
        )
        .unwrap()
    }
    fn recover(&self, hash: &str) -> Result<FileRecoveryReceipt, String> {
        recover_file_publication(
            self.root.path(),
            "publication-fixture",
            self.id,
            self.intent.publication_id,
            hash,
        )
    }
    fn candidate(&self) -> PathBuf {
        self.intent.parent_path.join(&self.intent.candidate_name)
    }
    fn target(&self) -> PathBuf {
        self.intent.parent_path.join(&self.intent.target_name)
    }
}

#[tokio::test]
async fn pending_candidate_recovers_once_without_changing_original_evidence_or_replaying() {
    let mut fixture = Fixture::new().await;
    assert!(inspect_file_publication(
        fixture.root.path(),
        "publication-fixture",
        fixture.id,
        fixture.intent.publication_id
    )
    .unwrap_err()
    .contains("still owned"));
    fixture.release();
    let before = fs::read(&fixture.journal).unwrap();
    let inspection = fixture.inspect();
    assert_eq!(inspection.state, PublicationState::ReadyToPublish);
    let receipt = fixture.recover(&inspection.snapshot_hash).unwrap();
    assert_eq!(fs::read(fixture.target()).unwrap(), b"verified-output");
    assert!(!fixture.candidate().exists());
    assert_eq!(fixture.inspect().state, PublicationState::Published);
    assert_eq!(
        fixture
            .recover(&inspection.snapshot_hash)
            .unwrap()
            .recorded_at,
        receipt.recorded_at
    );
    assert_eq!(fs::read(&fixture.journal).unwrap(), before);
    assert!(matches!(
        open_invocation(
            fixture.root.path(),
            "publication-fixture",
            fixture.id,
            &json!({}),
            AgentId::new()
        )
        .await
        .unwrap(),
        OpenInvocation::Existing(ExistingInvocation::Unresolved { .. })
    ));
}

#[tokio::test]
async fn crash_after_rename_is_acknowledged_without_republishing() {
    let mut fixture = Fixture::new().await;
    fs::rename(fixture.candidate(), fixture.target()).unwrap();
    fixture.release();
    let inode = fs::metadata(fixture.target()).unwrap().ino();
    let inspection = fixture.inspect();
    assert_eq!(inspection.state, PublicationState::Published);
    fixture.recover(&inspection.snapshot_hash).unwrap();
    assert_eq!(fs::metadata(fixture.target()).unwrap().ino(), inode);
    assert!(fixture.inspect().recovery.is_some());
}

#[tokio::test]
async fn changed_candidates_competitors_links_and_missing_files_never_publish() {
    for case in [
        "content",
        "inode",
        "competitor",
        "symlink",
        "hardlink",
        "parent",
        "missing",
    ] {
        let mut fixture = Fixture::new().await;
        fixture.release();
        let hash = fixture.inspect().snapshot_hash;
        match case {
            "content" => fs::write(fixture.candidate(), b"changed-output!").unwrap(),
            "inode" => {
                fs::rename(fixture.candidate(), fixture.intent.parent_path.join("old")).unwrap();
                fs::write(fixture.candidate(), b"verified-output").unwrap();
            }
            "competitor" => fs::write(fixture.target(), b"competing-output").unwrap(),
            "symlink" => std::os::unix::fs::symlink(fixture.candidate(), fixture.target()).unwrap(),
            "hardlink" => fs::hard_link(
                fixture.candidate(),
                fixture.intent.parent_path.join("alias"),
            )
            .unwrap(),
            "parent" => {
                fs::rename(
                    &fixture.intent.parent_path,
                    fixture.root.path().join("old-data"),
                )
                .unwrap();
                fs::create_dir(&fixture.intent.parent_path).unwrap();
            }
            "missing" => fs::remove_file(fixture.candidate()).unwrap(),
            _ => unreachable!(),
        }
        assert!(fixture.recover(&hash).is_err(), "accepted {case}");
        if case == "competitor" {
            assert_eq!(fs::read(fixture.target()).unwrap(), b"competing-output");
        }
    }
}

#[tokio::test]
async fn stale_snapshot_and_unknown_publication_cannot_authorize_recovery() {
    let mut fixture = Fixture::new().await;
    fixture.release();
    let hash = fixture.inspect().snapshot_hash;
    assert!(recover_file_publication(
        fixture.root.path(),
        "publication-fixture",
        fixture.id,
        Uuid::new_v4(),
        &hash
    )
    .is_err());
    fs::OpenOptions::new()
        .append(true)
        .open(&fixture.journal)
        .unwrap()
        .write_all(b"{\"unfinished\":")
        .unwrap();
    assert!(fixture
        .recover(&hash)
        .unwrap_err()
        .contains("changed since review"));
    assert!(!fixture.target().exists());
}

#[tokio::test]
async fn forged_receipt_and_later_output_changes_do_not_grant_republication() {
    let mut fixture = Fixture::new().await;
    fixture.release();
    let hash = fixture.inspect().snapshot_hash;
    fixture.recover(&hash).unwrap();
    fs::write(fixture.target(), b"later-user-edit").unwrap();
    assert!(fixture.recover(&hash).is_err());
    assert_eq!(fs::read(fixture.target()).unwrap(), b"later-user-edit");
    let path = fs::read_dir(fixture.root.path().join(".symbiont/invocations"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.to_string_lossy().ends_with(".publication.json"))
        .unwrap();
    let mut receipt: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    receipt["signature"] = json!("00".repeat(64));
    fs::write(path, serde_json::to_vec(&receipt).unwrap()).unwrap();
    assert!(fixture.recover(&hash).is_err());
}

#[tokio::test]
async fn prior_operator_resolution_blocks_new_publication() {
    let mut fixture = Fixture::new().await;
    fixture.release();
    let hash = fixture.inspect().snapshot_hash;
    reconcile_invocation(
        fixture.root.path(),
        "publication-fixture",
        fixture.id,
        ResolutionReview {
            snapshot_hash: hash.clone(),
            outcome: ResolutionOutcome::Failed,
            rationale: "Fixture cleanup verified; retain the unpublished candidate.".into(),
            evidence: vec![ResolutionEvidence {
                reference: "fixture-evidence".into(),
                sha256: "a".repeat(64),
            }],
            effects_stopped: true,
        },
    )
    .unwrap();
    assert!(fixture
        .recover(&hash)
        .unwrap_err()
        .contains("operator resolution"));
    assert!(!fixture.target().exists());
}
