//! Read-only classification of authenticated run evidence. No replay authority
//! is derived from a missing receipt, an error, or an incomplete signed prefix.

use super::{
    effect_journal::ToolEffect,
    loop_types::{JournalEntry, JournalError, LoopEvent, TerminationReason},
    protected_journal::{ProtectedJournal, VerifiedPrefix},
};
use crate::types::AgentId;
use serde::Serialize;
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path};
use uuid::Uuid;

const MAX_EFFECTS: usize = 32_768;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EffectIdentity {
    FilePublication {
        run_key: String,
        call_fingerprint: String,
        publication_id: Uuid,
    },
    ToolDispatch {
        dispatch_id: Uuid,
    },
    NetworkRequest {
        run_key: String,
        call_fingerprint: String,
        request_id: String,
    },
    Delegation {
        run_key: String,
        call_id: String,
    },
    Inference {
        request_id: String,
    },
    DirectInference {
        call_id: String,
    },
    SharedInference {
        reservation_id: Uuid,
    },
    ResponseDelivery {
        fingerprint: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectOutcome {
    /// The runtime recorded a non-error result or complete response. This is
    /// not independent proof of an external service's state or exactly-once work.
    ResultRecorded,
    /// No finish or an error result: effects may already have occurred.
    Unknown,
}

#[derive(Debug, Serialize)]
pub struct RecoveredEffect {
    pub identity: EffectIdentity,
    pub start_sequence: u64,
    pub finish_sequence: Option<u64>,
    pub outcome: EffectOutcome,
    pub details: Value,
}

#[derive(Debug, Serialize)]
pub struct RecoveryReport {
    pub run_id: Uuid,
    pub agent_id: AgentId,
    pub verified_records: usize,
    pub verified_bytes: u64,
    pub unverified_tail_bytes: u64,
    pub journal_complete: bool,
    pub terminal_reason: Option<TerminationReason>,
    pub requires_reconciliation: bool,
    pub untracked_tool_approvals: usize,
    pub effects: Vec<RecoveredEffect>,
    pub warnings: Vec<String>,
    pub recovered_budget: Option<super::budget::journal::RecoveredBudget>,
}

/// Verify the expected invocation and key, then classify only signed records.
/// Invalid complete records fail; an unterminated tail is counted, never trusted.
pub fn inspect_run(
    path: &Path,
    public_key: &[u8; 32],
    run_id: Uuid,
) -> Result<RecoveryReport, JournalError> {
    classify(
        ProtectedJournal::verify_run_prefix(path, public_key, run_id)?,
        run_id,
    )
}

#[derive(Default)]
struct Effects(BTreeMap<EffectIdentity, RecoveredEffect>);

impl Effects {
    fn start(
        &mut self,
        identity: EffectIdentity,
        entry: &JournalEntry,
        details: Value,
    ) -> Result<(), JournalError> {
        if self.0.len() >= MAX_EFFECTS || self.0.contains_key(&identity) {
            return Err(failure(
                "duplicate effect start or recovery effect limit exceeded",
            ));
        }
        self.0.insert(
            identity.clone(),
            RecoveredEffect {
                identity,
                start_sequence: entry.sequence,
                finish_sequence: None,
                outcome: EffectOutcome::Unknown,
                details,
            },
        );
        Ok(())
    }

    fn finish(
        &mut self,
        identity: EffectIdentity,
        entry: &JournalEntry,
        confirmed: bool,
    ) -> Result<(), JournalError> {
        let effect = self
            .0
            .get_mut(&identity)
            .ok_or_else(|| failure("effect finish has no matching start"))?;
        if effect.finish_sequence.replace(entry.sequence).is_some() {
            return Err(failure("duplicate effect finish"));
        }
        if confirmed {
            effect.outcome = EffectOutcome::ResultRecorded;
        }
        Ok(())
    }
}

pub(crate) fn classify(
    prefix: VerifiedPrefix,
    run_id: Uuid,
) -> Result<RecoveryReport, JournalError> {
    let agent_id = prefix
        .entries
        .first()
        .ok_or_else(|| failure("no authenticated records"))?
        .agent_id;
    let mut effects = Effects::default();
    let mut started = false;
    let mut terminal_reason = None;
    let mut approvals = BTreeMap::<(u32, String), usize>::new();
    for entry in &prefix.entries {
        if terminal_reason.is_some() {
            return Err(failure("signed records follow run termination"));
        }
        match &entry.event {
            LoopEvent::Started {
                agent_id: principal,
                ..
            } => {
                if started || entry.sequence != 0 || *principal != agent_id {
                    return Err(failure("invalid run start record"));
                }
                started = true;
            }
            LoopEvent::Terminated { reason, .. } => terminal_reason = Some(reason.clone()),
            LoopEvent::PolicyEvaluated { approved_calls, .. } => {
                for call in approved_calls {
                    if call["action"].get("ToolCall").is_some() {
                        let fingerprint = call["fingerprint"]
                            .as_str()
                            .ok_or_else(|| failure("tool approval lacks fingerprint"))?;
                        *approvals
                            .entry((entry.iteration, fingerprint.into()))
                            .or_default() += 1;
                    }
                }
            }
            LoopEvent::ToolDispatchStarted {
                dispatch_id,
                run_key,
                call_id,
                call_fingerprint,
                tool_name,
            } => {
                if let Some(count) = approvals.get_mut(&(entry.iteration, call_fingerprint.clone()))
                {
                    *count = count.saturating_sub(1);
                }
                effects.start(EffectIdentity::ToolDispatch { dispatch_id: *dispatch_id }, entry,
                    json!({"run_key": run_key, "call_id": call_id, "call_fingerprint": call_fingerprint, "tool_name": tool_name}))?;
            }
            LoopEvent::ToolDispatchFinished {
                dispatch_id,
                is_error,
                ..
            } => {
                effects.finish(
                    EffectIdentity::ToolDispatch {
                        dispatch_id: *dispatch_id,
                    },
                    entry,
                    !is_error,
                )?;
            }
            LoopEvent::ToolEffect {
                run_key,
                call_fingerprint,
                effect,
            } => match effect {
                ToolEffect::FilePublicationPrepared { intent } => {
                    effects.start(
                        EffectIdentity::FilePublication {
                            run_key: run_key.clone(),
                            call_fingerprint: call_fingerprint.clone(),
                            publication_id: intent.publication_id,
                        },
                        entry,
                        serde_json::to_value(intent)
                            .map_err(|error| failure(&error.to_string()))?,
                    )?;
                }
                ToolEffect::FilePublicationFinished {
                    publication_id,
                    confirmed,
                    error,
                } => {
                    effects.finish(
                        EffectIdentity::FilePublication {
                            run_key: run_key.clone(),
                            call_fingerprint: call_fingerprint.clone(),
                            publication_id: *publication_id,
                        },
                        entry,
                        *confirmed && error.is_none(),
                    )?;
                }
                ToolEffect::NetworkRequestStarted {
                    request_id,
                    method,
                    url,
                    request_hash,
                    ..
                } => {
                    effects.start(
                        EffectIdentity::NetworkRequest {
                            run_key: run_key.clone(),
                            call_fingerprint: call_fingerprint.clone(),
                            request_id: request_id.clone(),
                        },
                        entry,
                        json!({"method": method, "url": url, "request_hash": request_hash}),
                    )?;
                }
                ToolEffect::NetworkRequestFinished {
                    request_id,
                    status,
                    response_hash,
                    error,
                    ..
                } => {
                    effects.finish(
                        EffectIdentity::NetworkRequest {
                            run_key: run_key.clone(),
                            call_fingerprint: call_fingerprint.clone(),
                            request_id: request_id.clone(),
                        },
                        entry,
                        status.is_some() && response_hash.is_some() && error.is_none(),
                    )?;
                }
            },
            LoopEvent::DelegationStarted {
                run_key,
                call_id,
                call_fingerprint,
                target,
                audit,
                ..
            } => {
                effects.start(
                    EffectIdentity::Delegation {
                        run_key: run_key.clone(),
                        call_id: call_id.clone(),
                    },
                    entry,
                    json!({"target": target, "call_fingerprint": call_fingerprint, "audit": audit}),
                )?;
            }
            LoopEvent::DelegationFinished {
                run_key,
                call_id,
                reason,
                ..
            } => {
                effects.finish(
                    EffectIdentity::Delegation {
                        run_key: run_key.clone(),
                        call_id: call_id.clone(),
                    },
                    entry,
                    matches!(reason, TerminationReason::Completed),
                )?;
            }
            LoopEvent::InferenceRequested {
                request_id,
                endpoint,
                model,
                request_hash,
                ..
            } => {
                effects.start(
                    EffectIdentity::Inference {
                        request_id: request_id.clone(),
                    },
                    entry,
                    json!({"endpoint": endpoint, "model": model, "request_hash": request_hash}),
                )?;
            }
            LoopEvent::BudgetReservationStarted { reservation } => {
                effects.start(EffectIdentity::SharedInference {reservation_id:reservation.id}, entry,
                    json!({"root_id":reservation.root_id,"scope":reservation.ancestors.first().map(|s|s.scope),
                        "provider":reservation.provider,"model":reservation.model,"request_hash":reservation.request_hash,
                        "input_tokens":reservation.input_tokens,"output_tokens":reservation.output_tokens,
                        "agent_id":reservation.agent_id,"audit":reservation.audit}))?;
            }
            LoopEvent::BudgetReservationFinished { reservation_id, .. } => {
                // Settlement resolves the reservation, so the effect is
                // recorded whatever the accounting says. The outcome field
                // reports whether token usage was *confirmed*, which is a
                // different question from whether the effect concluded:
                // a settled reservation with unknown or exceeded usage is
                // still settled. Only a reservation with no settlement at
                // all -- a crash between reserve and settle -- is unresolved
                // and should force reconciliation.
                effects.finish(
                    EffectIdentity::SharedInference {
                        reservation_id: *reservation_id,
                    },
                    entry,
                    true,
                )?;
            }
            LoopEvent::InferenceCompleted { request_id, .. } => {
                effects.finish(
                    EffectIdentity::Inference {
                        request_id: request_id.clone(),
                    },
                    entry,
                    true,
                )?;
            }
            LoopEvent::DirectInferenceRequested {
                call_id,
                operation,
                provider,
                model,
                request_hash,
                ..
            } => {
                effects.start(EffectIdentity::DirectInference { call_id: call_id.clone() }, entry,
                    json!({"operation": operation, "provider": provider, "model": model, "request_hash": request_hash}))?;
            }
            LoopEvent::DirectInferenceFinished {
                call_id,
                reason,
                response_hash,
                ..
            } => {
                effects.finish(
                    EffectIdentity::DirectInference {
                        call_id: call_id.clone(),
                    },
                    entry,
                    matches!(reason, TerminationReason::Completed) && response_hash.is_some(),
                )?;
            }
            LoopEvent::ResponseDeliveryStarted {
                fingerprint,
                request_hash,
                ..
            } => {
                effects.start(
                    EffectIdentity::ResponseDelivery {
                        fingerprint: fingerprint.clone(),
                    },
                    entry,
                    json!({"request_hash": request_hash}),
                )?;
            }
            LoopEvent::ResponseDeliveryFinished {
                fingerprint,
                confirmed,
                ..
            } => {
                effects.finish(
                    EffectIdentity::ResponseDelivery {
                        fingerprint: fingerprint.clone(),
                    },
                    entry,
                    *confirmed,
                )?;
            }
            _ => {}
        }
    }
    let untracked_tool_approvals = approvals.values().sum();
    let journal_complete =
        started && terminal_reason.is_some() && prefix.unverified_tail_bytes == 0;
    let mut effects: Vec<_> = effects.0.into_values().collect();
    effects.sort_by_key(|effect| effect.start_sequence);
    let requires_reconciliation = !journal_complete
        || untracked_tool_approvals != 0
        || effects
            .iter()
            .any(|effect| effect.outcome == EffectOutcome::Unknown);
    let mut warnings = Vec::new();
    if !started {
        warnings.push("No signed run start record.".into());
    }
    if terminal_reason.is_none() {
        warnings
            .push("No signed terminal record; the run may still be active or interrupted.".into());
    }
    if prefix.unverified_tail_bytes != 0 {
        warnings.push("The final fragment is unauthenticated. It may reflect interruption or tampering; the file was preserved.".into());
    }
    if untracked_tool_approvals != 0 {
        warnings.push("Some tool approvals lack dispatch lifecycle records; do not infer that their effects did not occur.".into());
    }
    Ok(RecoveryReport {
        run_id,
        agent_id,
        verified_records: prefix.entries.len(),
        verified_bytes: prefix.verified_bytes,
        unverified_tail_bytes: prefix.unverified_tail_bytes,
        journal_complete,
        terminal_reason,
        requires_reconciliation,
        untracked_tool_approvals,
        effects,
        warnings,
        recovered_budget: super::budget::journal::recover(&prefix.entries)
            .map_err(|error| failure(&error))?,
    })
}

fn failure(message: &str) -> JournalError {
    JournalError::WriteFailed(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reasoning::loop_types::{JournalWriter, LoopConfig};
    use std::io::Write;

    async fn journal(events: Vec<LoopEvent>) -> (tempfile::TempDir, ProtectedJournal, Uuid) {
        let root = tempfile::tempdir().unwrap();
        let run_id = Uuid::new_v4();
        let agent_id = AgentId::new();
        let writer =
            ProtectedJournal::create_run(&root.path().join("audit"), agent_id, run_id).unwrap();
        for event in std::iter::once(LoopEvent::Started {
            agent_id,
            config: Box::new(LoopConfig::default()),
            execution_context: Default::default(),
        })
        .chain(events)
        {
            writer
                .append(JournalEntry {
                    sequence: 0,
                    timestamp: chrono::Utc::now(),
                    agent_id,
                    iteration: 0,
                    event,
                })
                .await
                .unwrap();
        }
        (root, writer, run_id)
    }

    fn start(dispatch_id: Uuid) -> LoopEvent {
        LoopEvent::ToolDispatchStarted {
            dispatch_id,
            run_key: "run".into(),
            call_id: "call".into(),
            call_fingerprint: "fingerprint".into(),
            tool_name: "fixture".into(),
        }
    }

    fn terminal() -> LoopEvent {
        LoopEvent::Terminated {
            reason: TerminationReason::Completed,
            iterations: 1,
            total_usage: Default::default(),
            duration: Default::default(),
        }
    }

    #[tokio::test]
    async fn finished_errors_and_unfinished_effects_remain_unknown_even_with_terminal_record() {
        for result in [None, Some(true), Some(false)] {
            let id = Uuid::new_v4();
            let mut events = vec![start(id)];
            if let Some(is_error) = result {
                events.push(LoopEvent::ToolDispatchFinished {
                    dispatch_id: id,
                    observation_hash: "hash".into(),
                    is_error,
                });
            }
            events.push(terminal());
            let (_root, writer, run_id) = journal(events).await;
            let report = inspect_run(writer.path(), &writer.public_key(), run_id).unwrap();
            assert!(report.journal_complete);
            assert_eq!(report.requires_reconciliation, result != Some(false));
            assert_eq!(
                report.effects[0].outcome == EffectOutcome::ResultRecorded,
                result == Some(false)
            );
        }
    }

    #[tokio::test]
    async fn prefix_inspection_preserves_partial_evidence_but_rejects_corruption_and_substitution()
    {
        let (root, writer, run_id) = journal(vec![start(Uuid::new_v4())]).await;
        let original = std::fs::read(writer.path()).unwrap();
        let copy = root.path().join("interrupted.jsonl");
        let mut truncated = original.clone();
        truncated.extend_from_slice(b"{\"payload\":");
        std::fs::write(&copy, &truncated).unwrap();
        let report = inspect_run(&copy, &writer.public_key(), run_id).unwrap();
        assert!(!report.journal_complete && report.requires_reconciliation);
        assert_eq!(report.verified_bytes, original.len() as u64);
        assert_eq!(report.unverified_tail_bytes, 11);
        assert_eq!(report.effects[0].outcome, EffectOutcome::Unknown);
        assert_eq!(std::fs::read(&copy).unwrap(), truncated);
        assert!(ProtectedJournal::verify_run(&copy, &writer.public_key(), run_id).is_err());
        assert!(inspect_run(&copy, &writer.public_key(), Uuid::new_v4()).is_err());
        let other = ed25519_dalek::SigningKey::from_bytes(&[9; 32]);
        assert!(inspect_run(&copy, &other.verifying_key().to_bytes(), run_id).is_err());
        std::fs::OpenOptions::new()
            .append(true)
            .open(&copy)
            .unwrap()
            .write_all(b"\n")
            .unwrap();
        assert!(inspect_run(&copy, &writer.public_key(), run_id).is_err());
        let link = root.path().join("link.jsonl");
        std::os::unix::fs::symlink(writer.path(), &link).unwrap();
        assert!(inspect_run(&link, &writer.public_key(), run_id).is_err());
    }

    #[tokio::test]
    async fn missing_nested_receipts_legacy_gaps_and_invalid_lifecycles_are_not_clean() {
        let network = LoopEvent::ToolEffect {
            run_key: "run".into(),
            call_fingerprint: "fingerprint".into(),
            effect: ToolEffect::NetworkRequestStarted {
                request_id: "request".into(),
                method: "POST".into(),
                url: "https://example.test/effect".into(),
                request_hash: "hash".into(),
                request_bytes: 1,
            },
        };
        let (_root, writer, run_id) = journal(vec![network, terminal()]).await;
        let report = inspect_run(writer.path(), &writer.public_key(), run_id).unwrap();
        assert!(report.journal_complete && report.requires_reconciliation);
        assert_eq!(report.effects[0].outcome, EffectOutcome::Unknown);
        let legacy = LoopEvent::PolicyEvaluated {
            iteration: 0,
            action_count: 1,
            denied_count: 0,
            approved_calls: vec![json!({"action": {"ToolCall": {}}, "fingerprint": "legacy"})],
            denied_calls: vec![],
        };
        let (_root, writer, run_id) = journal(vec![legacy, terminal()]).await;
        let report = inspect_run(writer.path(), &writer.public_key(), run_id).unwrap();
        assert_eq!(report.untracked_tool_approvals, 1);
        assert!(report.requires_reconciliation);
        let id = Uuid::new_v4();
        for events in [
            vec![start(id), start(id)],
            vec![LoopEvent::ToolDispatchFinished {
                dispatch_id: id,
                observation_hash: "hash".into(),
                is_error: false,
            }],
            vec![terminal(), start(id)],
        ] {
            let (_root, writer, run_id) = journal(events).await;
            assert!(inspect_run(writer.path(), &writer.public_key(), run_id).is_err());
        }
    }
}
