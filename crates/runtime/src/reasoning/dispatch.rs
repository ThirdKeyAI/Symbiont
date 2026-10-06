//! Shared authorization and audited dispatch for explicit tool calls.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use super::circuit_breaker::CircuitBreakerRegistry;
use super::executor::ActionExecutor;
use super::loop_types::{
    JournalEntry, JournalError, JournalWriter, LoopConfig, LoopDecision, LoopEvent, LoopState,
    Observation, ProposedAction,
};
use super::policy_bridge::ReasoningPolicyGate;
use super::prepared::AuthorizedAction;

/// Prepare every replacement before reevaluating it. Used by both ORGA and
/// explicit callers so modifications cannot evade validation or approval.
pub(crate) async fn authorize_action(
    original: &ProposedAction,
    state: &LoopState,
    config: &LoopConfig,
    executor: &dyn ActionExecutor,
    gate: &dyn ReasoningPolicyGate,
) -> Result<AuthorizedAction, String> {
    let remaining = config
        .timeout
        .saturating_sub(state.elapsed().to_std().unwrap_or(Duration::ZERO));
    if remaining.is_zero() {
        return Err("authorization deadline exhausted".into());
    }
    tokio::time::timeout(remaining, async {
        let mut action = original.clone();
        for _ in 0..4 {
            let prepared = executor.prepare_action(&action, config)?;
            prepared.check_source_policy(state)?;
            let approval = gate.approve_prepared(&prepared, state, config).await?;
            let mut policy_state = state.clone();
            policy_state.trusted_context.insert(
                "has_human_approval".into(),
                serde_json::json!(approval.is_some()),
            );
            policy_state.trusted_context.insert(
                "approved_fingerprint".into(),
                serde_json::json!(approval.as_ref().map(|_| prepared.fingerprint()).unwrap_or("")),
            );
            match gate.evaluate_prepared(&state.agent_id, &prepared, &policy_state).await {
                LoopDecision::Allow => return AuthorizedAction::issue(prepared, state, config, approval),
                LoopDecision::Deny { reason } => return Err(reason),
                LoopDecision::Modify { modified_action, .. } => {
                    if action_call_id(original) != action_call_id(&modified_action)
                        || std::mem::discriminant(original) != std::mem::discriminant(modified_action.as_ref())
                    {
                        return Err("policy modification changed the originating call identity or action kind".into());
                    }
                    action = *modified_action;
                }
            }
        }
        Err("policy modifications did not converge to an authorization".into())
    }).await.map_err(|_| "authorization timed out".to_string())?
}

fn action_call_id(action: &ProposedAction) -> Option<&str> {
    match action {
        ProposedAction::ToolCall { call_id, .. } | ProposedAction::Delegate { call_id, .. } => {
            Some(call_id)
        }
        _ => None,
    }
}

/// A dispatcher for non-inference entry points such as DSL tool calls and
/// context pre-fetch. The journal checkpoint must succeed before any effect.
pub struct GovernedToolDispatcher<'a> {
    pub executor: &'a dyn ActionExecutor,
    pub gate: &'a dyn ReasoningPolicyGate,
    pub journal: &'a dyn JournalWriter,
    pub circuit_breakers: &'a CircuitBreakerRegistry,
}

impl GovernedToolDispatcher<'_> {
    pub async fn dispatch(
        &self,
        actions: &[ProposedAction],
        state: &LoopState,
        config: &LoopConfig,
    ) -> Result<Vec<Observation>, JournalError> {
        let started = Instant::now();
        let valid_ids = valid_call_identities(actions)
            && actions
                .iter()
                .all(|action| matches!(action, ProposedAction::ToolCall { .. }));
        let mut grants = Vec::new();
        let mut observations = Vec::new();
        let mut denied_calls = Vec::new();
        for action in actions {
            let authorized = if valid_ids {
                authorize_action(action, state, config, self.executor, self.gate).await
            } else {
                Err("tool batch requires unique, nonempty call identities and tool actions".into())
            };
            match authorized {
                Ok(grant) => grants.push(grant),
                Err(reason) => {
                    denied_calls.push(serde_json::json!({"action": action, "reason": reason}));
                    let mut obs = action_error(action, reason);
                    obs.metadata
                        .insert("error_type".into(), "policy_denied".into());
                    observations.push(obs);
                }
            }
        }
        self.append(
            state,
            LoopEvent::PolicyEvaluated {
                iteration: state.iteration,
                action_count: actions.len(),
                denied_count: denied_calls.len(),
                approved_calls: grants.iter().map(AuthorizedAction::audit_context).collect(),
                denied_calls,
            },
        )
        .await?;

        let mut executable = Vec::new();
        for grant in grants {
            match grant.check_binding(state, config) {
                Ok(()) => executable.push(grant),
                Err(reason) => observations.push(action_error(grant.action(), reason)),
            }
        }
        observations.extend(
            execute_tool_grants(
                executable,
                config,
                self.executor,
                self.circuit_breakers,
                Some(self.journal),
            )
            .await?,
        );
        self.append(
            state,
            LoopEvent::ToolBatchCompleted {
                iteration: state.iteration,
                observations: observations.clone(),
                duration: started.elapsed(),
            },
        )
        .await?;
        Ok(observations)
    }

    async fn append(&self, state: &LoopState, event: LoopEvent) -> Result<(), JournalError> {
        self.journal
            .append(JournalEntry {
                sequence: self.journal.next_sequence().await,
                timestamp: chrono::Utc::now(),
                agent_id: state.agent_id,
                iteration: state.iteration,
                event,
            })
            .await
    }
}

/// Correlate actual results and enforce the remaining authorization deadline
/// for both ORGA and explicit-tool dispatch.
pub(crate) async fn execute_tool_grants(
    mut executable: Vec<AuthorizedAction>,
    config: &LoopConfig,
    executor: &dyn ActionExecutor,
    circuit_breakers: &CircuitBreakerRegistry,
    journal: Option<&dyn JournalWriter>,
) -> Result<Vec<Observation>, JournalError> {
    let mut observations = Vec::new();
    // Preserve call identities when a backend completes out of order,
    // omits results, or returns malformed/duplicate correlations.
    let bindings: HashMap<_, _> = executable
        .iter()
        .map(|grant| {
            (
                action_call_id(grant.action()).unwrap().to_owned(),
                (
                    grant.action().clone(),
                    grant.prepared().fingerprint().to_owned(),
                ),
            )
        })
        .collect();
    let expected: Vec<_> = executable
        .iter()
        .map(|grant| grant.action().clone())
        .collect();
    let mut dispatches = HashMap::new();
    let mut effect_outcomes = HashMap::new();
    if !executable.is_empty() {
        let deadline = executable
            .iter()
            .map(AuthorizedAction::deadline)
            .min()
            .unwrap();
        let (sender, receiver) = tokio::sync::mpsc::channel(32);
        if let Some(writer) = journal {
            for grant in &mut executable {
                let dispatch_id = uuid::Uuid::new_v4();
                let call_id = action_call_id(grant.action()).unwrap();
                let ProposedAction::ToolCall { name, .. } = grant.action() else {
                    return Err(JournalError::WriteFailed("non-tool dispatch grant".into()));
                };
                writer
                    .append(JournalEntry {
                        sequence: writer.next_sequence().await,
                        timestamp: chrono::Utc::now(),
                        agent_id: grant.principal(),
                        iteration: grant.iteration(),
                        event: LoopEvent::ToolDispatchStarted {
                            dispatch_id,
                            run_key: grant.run_key().into(),
                            call_id: call_id.into(),
                            call_fingerprint: grant.prepared().fingerprint().into(),
                            tool_name: name.clone(),
                        },
                    })
                    .await?;
                dispatches.insert(
                    call_id.to_owned(),
                    (dispatch_id, grant.principal(), grant.iteration()),
                );
                grant
                    .attach_worker_origin(writer.audit_reference(), dispatch_id)
                    .map_err(JournalError::WriteFailed)?;
                grant.attach_effect_journal(sender.clone());
                effect_outcomes.insert(
                    action_call_id(grant.action()).unwrap().to_owned(),
                    grant.effect_journal().unwrap(),
                );
            }
        }
        drop(sender);
        let execution = executor.execute_authorized(executable, config, circuit_breakers);
        tokio::pin!(execution);
        let results =
            tokio::time::timeout(deadline.saturating_duration_since(Instant::now()), async {
                if let Some(writer) = journal {
                    tokio::select! {
                        biased;
                        result = super::effect_journal::serve(writer, receiver) => {
                            result?;
                            Ok(execution.await)
                        }
                        results = &mut execution => Ok(results),
                    }
                } else {
                    Ok(execution.await)
                }
            })
            .await;
        match results {
            Ok(Err(error)) => return Err(error),
            Ok(Ok(results)) => {
                let expected_ids: HashSet<_> = expected.iter().filter_map(action_call_id).collect();
                let mut by_id = HashMap::new();
                let mut invalid = false;
                for observation in results {
                    match observation.call_id.clone() {
                        Some(id) if expected_ids.contains(id.as_str()) => {
                            invalid |= by_id.insert(id, observation).is_some();
                        }
                        _ => invalid = true,
                    }
                }
                for action in &expected {
                    let id = action_call_id(action).unwrap();
                    observations.push(if invalid {
                        action_error(action, "executor returned invalid result correlations")
                    } else {
                        by_id.remove(id).unwrap_or_else(|| {
                            action_error(action, "executor returned no result for this call")
                        })
                    });
                }
            }
            Err(_) => observations.extend(expected.iter().map(|action| {
                action_error(action, "tool dispatch timed out; completion is unconfirmed")
            })),
        }
    }
    for obs in &mut observations {
        let unconfirmed = obs.is_error
            || obs
                .call_id
                .as_ref()
                .and_then(|id| effect_outcomes.get(id))
                .is_some_and(super::effect_journal::EffectJournal::has_unconfirmed_effect);
        if unconfirmed {
            if !obs.is_error {
                obs.content = format!("Tool returned with unconfirmed effects: {}", obs.content);
            }
            obs.mark_unconfirmed_effect();
        } else {
            // Do not trust outcome metadata returned by a backend.
            obs.metadata
                .insert("effect_outcome".into(), "result_recorded".into());
        }
        if let Some((
            ProposedAction::ToolCall {
                name, arguments, ..
            },
            fingerprint,
        )) = obs.call_id.as_ref().and_then(|id| bindings.get(id))
        {
            obs.metadata.insert("authorized_tool".into(), name.clone());
            obs.metadata
                .insert("authorized_arguments".into(), arguments.clone());
            obs.metadata
                .insert("call_fingerprint".into(), fingerprint.clone());
        }
        if let (Some(writer), Some((dispatch_id, agent_id, iteration))) = (
            journal,
            obs.call_id.as_ref().and_then(|id| dispatches.get(id)),
        ) {
            let observation_hash = super::prepared::digest_json(
                &serde_json::to_value(&*obs)
                    .map_err(|error| JournalError::WriteFailed(error.to_string()))?,
            )
            .map_err(JournalError::WriteFailed)?;
            writer
                .append(JournalEntry {
                    sequence: writer.next_sequence().await,
                    timestamp: chrono::Utc::now(),
                    agent_id: *agent_id,
                    iteration: *iteration,
                    event: LoopEvent::ToolDispatchFinished {
                        dispatch_id: *dispatch_id,
                        observation_hash,
                        is_error: obs.is_error,
                    },
                })
                .await?;
        }
    }
    Ok(observations)
}

pub(crate) fn valid_call_identities(actions: &[ProposedAction]) -> bool {
    let mut seen = HashSet::new();
    actions
        .iter()
        .filter_map(action_call_id)
        .all(|id| !id.is_empty() && seen.insert(id))
}

fn action_error(action: &ProposedAction, reason: impl Into<String>) -> Observation {
    match action {
        ProposedAction::ToolCall { call_id, name, .. } => {
            Observation::tool_error(name, reason).with_call_id(call_id)
        }
        _ => Observation::policy_denial(reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reasoning::conversation::Conversation;
    use crate::reasoning::inference::ToolDefinition;
    use crate::reasoning::loop_types::BufferedJournal;
    use crate::reasoning::policy_bridge::DefaultPolicyGate;
    use crate::types::AgentId;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CorrelatedExecutor;
    #[async_trait::async_trait]
    impl ActionExecutor for CorrelatedExecutor {
        fn tool_definitions(&self) -> Vec<ToolDefinition> {
            vec![ToolDefinition {
                name: "fixture".into(),
                description: "Correlation fixture".into(),
                parameters: serde_json::json!({"type":"object", "properties":{}}),
            }]
        }
        async fn execute_actions(
            &self,
            actions: &[ProposedAction],
            _: &LoopConfig,
            _: &CircuitBreakerRegistry,
        ) -> Vec<Observation> {
            actions
                .iter()
                .rev()
                .filter_map(|action| match action {
                    ProposedAction::ToolCall { call_id, .. } if call_id == "missing" => None,
                    ProposedAction::ToolCall { call_id, name, .. } => {
                        Some(Observation::tool_result(name, call_id).with_call_id(call_id))
                    }
                    _ => None,
                })
                .collect()
        }
        async fn execute_authorized(
            &self,
            actions: Vec<AuthorizedAction>,
            config: &LoopConfig,
            circuit_breakers: &CircuitBreakerRegistry,
        ) -> Vec<Observation> {
            self.execute_actions(
                &actions
                    .into_iter()
                    .map(|action| action.action().clone())
                    .collect::<Vec<_>>(),
                config,
                circuit_breakers,
            )
            .await
        }
    }

    #[tokio::test]
    async fn results_are_correlated_and_missing_or_duplicate_calls_fail() {
        let executor = CorrelatedExecutor;
        let gate = DefaultPolicyGate::permissive_for_dev_only();
        let journal = BufferedJournal::new(100);
        let circuit_breakers = CircuitBreakerRegistry::default();
        let dispatcher = GovernedToolDispatcher {
            executor: &executor,
            gate: &gate,
            journal: &journal,
            circuit_breakers: &circuit_breakers,
        };
        let state = LoopState::new(AgentId::new(), Conversation::new());
        let config = LoopConfig {
            tool_definitions: executor.tool_definitions(),
            ..Default::default()
        };
        let actions: Vec<_> = ["first", "missing", "last"]
            .into_iter()
            .map(|id| ProposedAction::ToolCall {
                call_id: id.into(),
                name: "fixture".into(),
                arguments: "{}".into(),
            })
            .collect();
        let results = dispatcher
            .dispatch(&actions, &state, &config)
            .await
            .unwrap();
        assert_eq!(results[0].content, "first");
        assert!(results[1].is_error && results[1].content.contains("no result"));
        assert_eq!(results[2].content, "last");
        let results = dispatcher
            .dispatch(&[actions[0].clone(), actions[0].clone()], &state, &config)
            .await
            .unwrap();
        assert!(results
            .iter()
            .all(|obs| obs.is_error && obs.content.contains("unique")));
        assert!(matches!(
            journal.entries().await.last().unwrap().event,
            LoopEvent::ToolBatchCompleted { .. }
        ));
    }

    struct FailingJournal {
        writes: AtomicUsize,
        fail_at: usize,
    }
    #[async_trait::async_trait]
    impl JournalWriter for FailingJournal {
        async fn append(&self, _: JournalEntry) -> Result<(), JournalError> {
            if self.writes.fetch_add(1, Ordering::SeqCst) == self.fail_at {
                Err(JournalError::WriteFailed("fixture failure".into()))
            } else {
                Ok(())
            }
        }
        async fn next_sequence(&self) -> u64 {
            0
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn audit_checkpoints_precede_real_effects_and_required_approval_cannot_be_waived() {
        use crate::toolclad::executor::ToolCladExecutor;
        for (approval, fail_at, expect_effect, expect_journal_error) in [
            (false, 0, false, true),
            (false, 1, false, true),
            (false, 2, true, true),
            (false, 3, true, true),
            (false, usize::MAX, true, false),
            (true, usize::MAX, false, false),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let marker = dir.path().join("effect");
            let mut manifest: crate::toolclad::manifest::Manifest = toml::from_str(
                r#"
[tool]
name = "fixture"
version = "1"
binary = "/usr/bin/touch"
description = "Journal checkpoint fixture"
[command]
template = "/usr/bin/touch"
[output]
format = "text"
"#,
            )
            .unwrap();
            manifest.command.template = Some(format!("/usr/bin/touch {}", marker.display()));
            manifest.tool.human_approval = approval;
            let executor = ToolCladExecutor::new(vec![("fixture".into(), manifest)])
                .with_development_host_execution();
            let gate = DefaultPolicyGate::permissive_for_dev_only();
            let journal = FailingJournal {
                writes: AtomicUsize::new(0),
                fail_at,
            };
            let circuit_breakers = CircuitBreakerRegistry::default();
            let config = LoopConfig {
                tool_definitions: executor.tool_definitions(),
                ..Default::default()
            };
            let state = LoopState::new(AgentId::new(), Conversation::new());
            let result = GovernedToolDispatcher {
                executor: &executor,
                gate: &gate,
                journal: &journal,
                circuit_breakers: &circuit_breakers,
            }
            .dispatch(
                &[ProposedAction::ToolCall {
                    call_id: "fixture".into(),
                    name: "fixture".into(),
                    arguments: "{}".into(),
                }],
                &state,
                &config,
            )
            .await;
            assert_eq!(result.is_err(), expect_journal_error, "{result:?}");
            assert_eq!(marker.exists(), expect_effect);
            if approval {
                assert!(result.unwrap()[0].is_error);
            }
        }
    }
}
