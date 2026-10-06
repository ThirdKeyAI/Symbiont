//! Evaluators receive completed evidence; they cannot activate a candidate.

use super::*;
use crate::reasoning::{
    loop_types::{LoopEvent, TerminationReason},
    run_audit::RunAuditReference,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialReference {
    pub case_id: String,
    pub audit: RunAuditReference,
}

/// Verified execution evidence for a domain-specific evaluator. Construction is
/// restricted to the signed-journal reader; caller-supplied scores are not evidence.
#[derive(Debug)]
pub struct TrialEvidence {
    pub(crate) case_id: String,
    pub(crate) binding: RunBinding,
    pub(crate) run_id: uuid::Uuid,
    pub(crate) journal_sha256: String,
    pub(crate) output: String,
    pub(crate) tokens: u32,
    pub(crate) denied_calls: u32,
}

impl TrialEvidence {
    pub fn case_id(&self) -> &str {
        &self.case_id
    }
    pub fn output(&self) -> &str {
        &self.output
    }
    pub fn binding(&self) -> &RunBinding {
        &self.binding
    }
    pub fn tokens(&self) -> u32 {
        self.tokens
    }
    pub fn denied_calls(&self) -> u32 {
        self.denied_calls
    }
}

/// Host-registered evaluator interface. Implementations are trusted operator code,
/// never paths or plugins supplied by an improvement candidate.
pub trait Evaluator {
    fn evaluate(
        &self,
        candidate: &Candidate,
        suite: &AcceptanceSuite,
        trials: &[TrialEvidence],
    ) -> Result<EvaluationReport, String>;
}

pub struct ExactAnswerEvaluator;

impl Evaluator for ExactAnswerEvaluator {
    fn evaluate(
        &self,
        candidate: &Candidate,
        suite: &AcceptanceSuite,
        trials: &[TrialEvidence],
    ) -> Result<EvaluationReport, String> {
        suite.validate()?;
        let candidate_id = digest(candidate)?;
        let suite_id = digest(suite)?;
        if candidate.suite_sha256 != suite_id || trials.len() != suite.cases.len() {
            return Err(
                "evaluation requires the frozen suite and exactly one trial for every case".into(),
            );
        }
        let environment = trials
            .first()
            .ok_or("missing trial evidence")?
            .binding
            .environment
            .clone();
        let mut seen = BTreeSet::new();
        let mut runs = BTreeSet::new();
        let mut cases = Vec::new();
        for trial in trials {
            let case = suite
                .cases
                .iter()
                .find(|c| c.id == trial.case_id)
                .ok_or("unknown trial case")?;
            if !seen.insert(&trial.case_id)
                || !runs.insert(trial.run_id)
                || trial.binding.schema_version != SCHEMA_VERSION
                || trial.binding.mode != SelectionMode::Trial
                || trial.binding.approval.is_some()
                || trial.binding.workflow != candidate.workflow
                || trial.binding.candidate != candidate_id
                || trial.binding.input_sha256 != sha256(case.input.as_bytes())
                || trial.binding.environment != environment
            {
                return Err(
                    "trial identity, input, candidate or execution environment does not match"
                        .into(),
                );
            }
            cases.push(CaseResult {
                id: case.id.clone(),
                run_id: trial.run_id,
                journal_sha256: trial.journal_sha256.clone(),
                answer_matches: trial.output == case.expected_output,
                tokens: trial.tokens,
                denied_calls: trial.denied_calls,
            });
        }
        cases.sort_by(|a, b| a.id.cmp(&b.id));
        let accepted = cases.iter().filter(|c| c.answer_matches).count() >= suite.minimum_passed
            && cases
                .iter()
                .all(|c| c.tokens <= suite.maximum_tokens_per_case)
            && cases.iter().map(|c| u64::from(c.denied_calls)).sum::<u64>()
                <= u64::from(suite.maximum_denied_calls);
        Ok(EvaluationReport {
            schema_version: SCHEMA_VERSION,
            evaluator: "exact-answer:v1".into(),
            candidate: candidate_id,
            suite_sha256: suite_id,
            environment,
            cases,
            accepted,
        })
    }
}

#[cfg(unix)]
pub(crate) fn load_trial(
    reference: &TrialReference,
    expected_key: &[u8; 32],
) -> Result<TrialEvidence, String> {
    use crate::reasoning::protected_journal::ProtectedJournal;
    if reference.audit.public_key != hex::encode(expected_key) {
        return Err("trial is not signed by the configured project audit key".into());
    }
    let prefix = ProtectedJournal::verify_run_prefix(
        &reference.audit.path,
        expected_key,
        reference.audit.run_id,
    )
    .map_err(|e| e.to_string())?;
    if prefix.unverified_tail_bytes != 0 {
        return Err("incomplete trial journal".into());
    }
    let Some(LoopEvent::Started {
        execution_context, ..
    }) = prefix.entries.first().map(|e| &e.event)
    else {
        return Err("trial lacks its signed startup binding".into());
    };
    let binding: RunBinding = serde_json::from_value(
        execution_context
            .get("improvement")
            .cloned()
            .ok_or("trial lacks an improvement binding")?,
    )
    .map_err(|e| e.to_string())?;
    let Some(LoopEvent::Terminated {
        reason: TerminationReason::Completed,
        total_usage,
        ..
    }) = prefix.entries.last().map(|e| &e.event)
    else {
        return Err("trial did not complete successfully".into());
    };
    // Grade the runtime's actual post-policy result, not a proposed response
    // which could have been denied or replaced by a policy gate.
    let n = prefix.entries.len();
    let Some(LoopEvent::ImprovementOutput { output }) = n
        .checked_sub(2)
        .and_then(|i| prefix.entries.get(i))
        .map(|e| &e.event)
    else {
        return Err("trial lacks final output evidence".into());
    };
    let Some(LoopEvent::BudgetUpdated { budget }) = n
        .checked_sub(3)
        .and_then(|i| prefix.entries.get(i))
        .map(|e| &e.event)
    else {
        return Err("trial lacks final resource accounting".into());
    };
    if budget.exceeded
        || budget.uncertain_tokens != 0
        || budget.reserved_tokens != 0
        || budget.usage.total_tokens != total_usage.total_tokens
    {
        return Err("trial has exceeded, uncertain or inconsistent token accounting".into());
    }
    let mut denied_calls = 0u32;
    for entry in &prefix.entries {
        if let LoopEvent::PolicyEvaluated { denied_count, .. } = &entry.event {
            denied_calls = denied_calls
                .checked_add(u32::try_from(*denied_count).map_err(|_| "denial count overflow")?)
                .ok_or("denial count overflow")?;
        }
    }
    let trial = TrialEvidence {
        case_id: reference.case_id.clone(),
        binding,
        run_id: reference.audit.run_id,
        journal_sha256: prefix.snapshot_sha256.clone(),
        output: output.clone(),
        tokens: total_usage.total_tokens,
        denied_calls,
    };
    let report = crate::reasoning::recovery::classify(prefix, reference.audit.run_id)
        .map_err(|e| e.to_string())?;
    if report.requires_reconciliation {
        return Err("trial has unresolved effects".into());
    }
    Ok(trial)
}
