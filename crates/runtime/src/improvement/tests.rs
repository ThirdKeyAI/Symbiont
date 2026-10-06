use super::*;
use evaluation::{Evaluator, ExactAnswerEvaluator, TrialEvidence};

fn suite() -> AcceptanceSuite {
    AcceptanceSuite {
        schema_version: 1,
        cases: vec![AcceptanceCase {
            id: "case".into(),
            input: "input".into(),
            expected_output: "expected".into(),
        }],
        minimum_passed: 1,
        maximum_tokens_per_case: 100,
        maximum_denied_calls: 0,
    }
}
fn proposal() -> Proposal {
    Proposal {
        instructions: "Collect evidence before escalation.".into(),
        rationale: "Reduce incomplete handoffs".into(),
        evidence: vec![],
    }
}
fn environment() -> ExecutionEnvironment {
    ExecutionEnvironment {
        runtime_version: "1".into(),
        provider: "fixture".into(),
        model: "fixture".into(),
        provider_configuration: sha256(b"provider configuration"),
        loop_config_sha256: sha256(b"configuration"),
        system_prompt_sha256: sha256(b"system"),
    }
}
fn candidate() -> Candidate {
    Candidate {
        schema_version: 1,
        workflow: "claims".into(),
        agent: "claims".into(),
        source_sha256: sha256(b"agent"),
        deployment_sha256: sha256(b"deployment"),
        suite_sha256: digest(&suite()).unwrap(),
        parent: None,
        proposal: proposal(),
    }
}
fn trial(c: &Candidate) -> TrialEvidence {
    TrialEvidence {
        case_id: "case".into(),
        binding: RunBinding {
            schema_version: 1,
            workflow: "claims".into(),
            candidate: digest(c).unwrap(),
            approval: None,
            mode: SelectionMode::Trial,
            input_sha256: sha256(b"input"),
            environment: environment(),
        },
        run_id: uuid::Uuid::new_v4(),
        journal_sha256: sha256(b"journal"),
        output: "expected".into(),
        tokens: 13,
        denied_calls: 0,
    }
}

#[test]
fn evaluation_requires_complete_matched_evidence_and_independent_resource_gates() {
    let candidate = candidate();
    let mut trials = vec![trial(&candidate)];
    let evaluate = |t: &[TrialEvidence]| ExactAnswerEvaluator.evaluate(&candidate, &suite(), t);
    assert!(evaluate(&trials).unwrap().accepted);
    trials[0].tokens = 101;
    assert!(!evaluate(&trials).unwrap().accepted);
    trials[0].tokens = 13;
    trials[0].denied_calls = 1;
    assert!(!evaluate(&trials).unwrap().accepted);
    trials[0].denied_calls = 0;
    trials[0].output = "wrong".into();
    assert!(!evaluate(&trials).unwrap().accepted);
    trials[0].binding.input_sha256 = sha256(b"other input");
    assert!(evaluate(&trials).is_err());
    trials[0] = trial(&candidate);
    trials[0].binding.candidate = sha256(b"another candidate");
    assert!(evaluate(&trials).is_err());
    trials[0] = trial(&candidate);
    trials[0].binding.mode = SelectionMode::Approved;
    assert!(evaluate(&trials).is_err());
    assert!(evaluate(&[]).is_err());
}

#[test]
fn schemas_reject_candidate_authority_and_ambiguous_suites() {
    let mut value = serde_json::to_value(proposal()).unwrap();
    value["permissions"] = serde_json::json!(["execute_payment"]);
    assert!(serde_json::from_value::<Proposal>(value).is_err());
    let mut s = suite();
    s.cases.push(s.cases[0].clone());
    assert!(s.validate().is_err());
    let mut s = suite();
    s.minimum_passed = 0;
    assert!(s.validate().is_err());
    let mut s = suite();
    s.schema_version = 2;
    assert!(s.validate().is_err());
    assert!(identifier("../workflow").is_err());
    assert!(hash_valid("../../other").is_err());
}

#[cfg(unix)]
mod storage {
    use super::*;
    use crate::{
        reasoning::{
            loop_types::{
                JournalEntry, JournalWriter, LoopConfig, LoopEvent, ProposedAction,
                TerminationReason,
            },
            protected_journal::ProtectedJournal,
            run_audit::RunAuditReference,
        },
        types::AgentId,
    };
    use std::{
        collections::HashMap,
        os::unix::fs::{symlink, PermissionsExt},
        path::Path,
        time::Duration,
    };

    fn init(root: &Path) -> Store {
        Store::initialize(root, "claims", "claims", "agent claims {}", suite()).unwrap()
    }
    async fn signed_trial(
        root: &Path,
        candidate: &str,
        answer: &str,
    ) -> evaluation::TrialReference {
        signed_trial_accounted(root, candidate, answer, false).await
    }

    async fn signed_trial_accounted(
        root: &Path,
        candidate: &str,
        answer: &str,
        uncertain: bool,
    ) -> evaluation::TrialReference {
        let id = AgentId::new();
        let run_id = uuid::Uuid::new_v4();
        let journal =
            ProtectedJournal::create_run(&root.join(".symbiont/governed"), id, run_id).unwrap();
        let binding = RunBinding {
            schema_version: 1,
            workflow: "claims".into(),
            candidate: candidate.into(),
            approval: None,
            mode: SelectionMode::Trial,
            input_sha256: sha256(b"input"),
            environment: environment(),
        };
        let mut budget = crate::reasoning::budget::SharedBudget::new(100).snapshot();
        if uncertain {
            budget.uncertain_tokens = 17;
            budget.available_tokens -= 17;
        }
        let events = vec![
            LoopEvent::Started {
                agent_id: id,
                config: Box::new(LoopConfig::default()),
                execution_context: HashMap::from([(
                    "improvement".into(),
                    serde_json::to_value(binding).unwrap(),
                )]),
            },
            LoopEvent::ReasoningComplete {
                iteration: 0,
                actions: vec![ProposedAction::Respond {
                    // Proposed text may differ from the final post-policy output.
                    content: "expected".into(),
                }],
                usage: Default::default(),
            },
            LoopEvent::BudgetUpdated { budget },
            LoopEvent::ImprovementOutput {
                output: answer.into(),
            },
            LoopEvent::Terminated {
                reason: TerminationReason::Completed,
                iterations: 1,
                total_usage: Default::default(),
                duration: Duration::ZERO,
            },
        ];
        for (sequence, event) in events.into_iter().enumerate() {
            journal
                .append(JournalEntry {
                    sequence: sequence as u64,
                    timestamp: chrono::Utc::now(),
                    agent_id: id,
                    iteration: 0,
                    event,
                })
                .await
                .unwrap();
        }
        evaluation::TrialReference {
            case_id: "case".into(),
            audit: RunAuditReference {
                run_id,
                path: journal.path().into(),
                public_key: hex::encode(journal.public_key()),
            },
        }
    }

    #[tokio::test]
    async fn complete_lifecycle_binds_approval_rolls_back_and_keeps_pinned_versions() {
        let root = tempfile::tempdir().unwrap();
        let mut store = init(root.path());
        assert!(Store::open(root.path(), "claims").is_err());
        assert!(store.pin("claims", "agent claims {}", None).is_err());
        let first = store.propose(proposal()).unwrap();
        let reference = signed_trial(root.path(), &first, "expected").await;
        let (report, summary) = store
            .evaluate(
                &first,
                std::slice::from_ref(&reference),
                &ExactAnswerEvaluator,
            )
            .unwrap();
        assert!(summary.accepted);
        let approval = store.approve(&first, &report, "Reviewed evidence").unwrap();
        store.activate(&first, &approval, None, false).unwrap();
        let pinned = store.pin("claims", "agent claims {}", None).unwrap();
        assert_eq!(pinned.candidate_id(), first);
        let second = store.propose(proposal()).unwrap();
        assert_ne!(first, second);
        assert!(store
            .activate(&second, &approval, Some(&first), false)
            .is_err());
        assert!(store
            .evaluate(&second, &[reference], &ExactAnswerEvaluator)
            .is_err());
        let reference = signed_trial(root.path(), &second, "expected").await;
        let (report, _) = store
            .evaluate(&second, &[reference], &ExactAnswerEvaluator)
            .unwrap();
        let approval2 = store
            .approve(&second, &report, "Reviewed replacement")
            .unwrap();
        assert!(store.activate(&second, &approval2, None, false).is_err());
        store
            .activate(&second, &approval2, Some(&first), false)
            .unwrap();
        assert_eq!(pinned.candidate_id(), first);
        store
            .activate(&first, &approval, Some(&second), true)
            .unwrap();
        store.set_enabled(false).unwrap();
        assert!(store.pin("claims", "agent claims {}", None).is_err());
        assert_eq!(pinned.candidate_id(), first);
        let exported = store.export_document("state", None).unwrap();
        let key = hex::decode(store.public_key()).unwrap().try_into().unwrap();
        verify_document(&serde_json::to_vec(&exported).unwrap(), "state", &key).unwrap();
        assert!(
            verify_document(&serde_json::to_vec(&exported).unwrap(), "candidate", &key).is_err()
        );
        drop(store);
        let store = Store::open(root.path(), "claims").unwrap();
        assert!(!store.state().enabled);
        assert_eq!(store.state().active.as_deref(), Some(first.as_str()));
    }

    #[tokio::test]
    async fn failed_foreign_incomplete_and_tampered_trials_cannot_authorize_promotion() {
        let root = tempfile::tempdir().unwrap();
        let store = init(root.path());
        let candidate = store.propose(proposal()).unwrap();
        let trial = signed_trial(root.path(), &candidate, "wrong").await;
        let (evaluation, report) = store
            .evaluate(
                &candidate,
                std::slice::from_ref(&trial),
                &ExactAnswerEvaluator,
            )
            .unwrap();
        assert!(!report.accepted);
        assert!(store.approve(&candidate, &evaluation, "No waiver").is_err());
        let uncertain = signed_trial_accounted(root.path(), &candidate, "expected", true).await;
        assert!(store
            .evaluate(&candidate, &[uncertain], &ExactAnswerEvaluator)
            .unwrap_err()
            .contains("uncertain"));
        let other = tempfile::tempdir().unwrap();
        std::fs::create_dir(other.path().join(".symbiont")).unwrap();
        let foreign = signed_trial(other.path(), &candidate, "expected").await;
        assert!(store
            .evaluate(&candidate, &[foreign], &ExactAnswerEvaluator)
            .is_err());
        let bytes = std::fs::read(&trial.audit.path).unwrap();
        let previous = bytes[..bytes.len() - 1]
            .iter()
            .rposition(|b| *b == b'\n')
            .unwrap()
            + 1;
        std::fs::write(&trial.audit.path, &bytes[..previous]).unwrap();
        assert!(store
            .evaluate(
                &candidate,
                std::slice::from_ref(&trial),
                &ExactAnswerEvaluator
            )
            .is_err());
        std::fs::write(
            &trial.audit.path,
            bytes
                .iter()
                .map(|b| if *b == b'w' { b'v' } else { *b })
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert!(store
            .evaluate(&candidate, &[trial], &ExactAnswerEvaluator)
            .is_err());
    }

    #[test]
    fn refuses_state_tampering_source_drift_new_deployment_files_and_unsafe_paths() {
        let root = tempfile::tempdir().unwrap();
        let store = init(root.path());
        let candidate = store.propose(proposal()).unwrap();
        assert!(store.pin("claims", "changed", Some(&candidate)).is_err());
        std::fs::create_dir(root.path().join("policies")).unwrap();
        assert!(store
            .pin("claims", "agent claims {}", Some(&candidate))
            .is_err());
        std::fs::remove_dir(root.path().join("policies")).unwrap();
        std::fs::create_dir(root.path().join("scope")).unwrap();
        std::fs::write(root.path().join("scope/scope.toml"), "[scope]\n").unwrap();
        assert!(store
            .pin("claims", "agent claims {}", Some(&candidate))
            .is_err());
        std::fs::remove_file(root.path().join("scope/scope.toml")).unwrap();
        std::fs::remove_dir(root.path().join("scope")).unwrap();
        std::fs::write(root.path().join("toolclad.toml"), "[types]\n").unwrap();
        assert!(store
            .pin("claims", "agent claims {}", Some(&candidate))
            .is_err());
        std::fs::remove_file(root.path().join("toolclad.toml")).unwrap();
        assert!(store
            .pin("claims", "agent claims {}", Some(&candidate))
            .is_ok());
        drop(store);
        let path = root.path().join(".symbiont/improvements/claims/state.json");
        let mut value: serde_json::Value = read_json(&path).unwrap();
        value["payload"]["enabled"] = false.into();
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(Store::open(root.path(), "claims").is_err());
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), root.path().join(".symbiont")).unwrap();
        assert!(Store::initialize(root.path(), "claims", "claims", "source", suite()).is_err());
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
        let root = tempfile::tempdir().unwrap();
        let store = init(root.path());
        drop(store);
        let state = root.path().join(".symbiont/improvements/claims/state.json");
        std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(Store::open(root.path(), "claims").is_err());
    }

    #[test]
    fn failed_publication_invalidates_the_handle_until_reopened() {
        let root = tempfile::tempdir().unwrap();
        let mut store = init(root.path());
        let candidate = store.propose(proposal()).unwrap();
        let path = root.path().join(".symbiont/improvements/claims/state.json");
        let saved = path.with_extension("saved");
        std::fs::rename(&path, &saved).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(store.set_enabled(false).is_err());
        assert!(store.state().enabled);
        assert!(store
            .pin("claims", "agent claims {}", Some(&candidate))
            .is_err());
        assert!(store.set_enabled(true).is_err());
        std::fs::remove_dir(&path).unwrap();
        std::fs::rename(saved, path).unwrap();
        drop(store);
        let store = Store::open(root.path(), "claims").unwrap();
        assert!(store
            .pin("claims", "agent claims {}", Some(&candidate))
            .is_ok());
    }
}
