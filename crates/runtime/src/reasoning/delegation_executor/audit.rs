//! Protected child journals and retained child ownership for governed delegation.
use super::*;
use crate::reasoning::{
    loop_types::{JournalEntry, JournalError, LoopEvent, LoopResult, Observation, ProposedAction},
    prepared::AuthorizedAction,
};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio_util::sync::CancellationToken;

pub(super) struct ChildRun {
    pub(super) cancellation: CancellationToken,
    task: tokio::task::JoinHandle<Result<(), String>>,
}

pub(super) async fn join_children(children: Vec<ChildRun>) -> Result<(), String> {
    let mut failure = None;
    for child in children {
        match child.task.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                failure.get_or_insert(error);
            }
            Err(error) => {
                failure.get_or_insert(error.to_string());
            }
        }
    }
    failure.map_or(Ok(()), Err)
}

/// Distinguish a durable child outcome from a loop that merely returned after
/// failing its required journal. Never manufacture completion from a prefix.
struct ChildJournal {
    inner: Arc<dyn JournalWriter>,
    terminal: AtomicBool,
    failed: AtomicBool,
}

#[async_trait]
impl JournalWriter for ChildJournal {
    fn audit_reference(&self) -> Option<crate::reasoning::run_audit::RunAuditReference> {
        self.inner.audit_reference()
    }
    async fn next_sequence(&self) -> u64 {
        self.inner.next_sequence().await
    }

    async fn append(&self, entry: JournalEntry) -> Result<(), JournalError> {
        let terminal = matches!(entry.event, LoopEvent::Terminated { .. });
        if let Err(error) = self.inner.append(entry).await {
            self.failed.store(true, Ordering::SeqCst);
            return Err(error);
        }
        if terminal {
            self.terminal.store(true, Ordering::SeqCst);
        }
        Ok(())
    }
}

/// Preserve the exact parent/source linkage in both the child's startup record
/// and its subsequent prepared-call bindings.
struct LinkedExecutor {
    inner: Arc<dyn ActionExecutor>,
    link: serde_json::Value,
    definition: Option<serde_json::Value>,
}

#[async_trait]
impl ActionExecutor for LinkedExecutor {
    fn execution_context(&self) -> HashMap<String, serde_json::Value> {
        let mut context = self.inner.execution_context();
        context.insert("delegation".into(), self.link.clone());
        if let Some(definition) = &self.definition {
            context.insert("agent_definition".into(), definition.clone());
        }
        context
    }
    fn validate_configuration(&self) -> Result<(), String> {
        self.inner.validate_configuration()
    }
    fn prepare_action(
        &self,
        action: &ProposedAction,
        config: &LoopConfig,
    ) -> Result<crate::reasoning::prepared::PreparedAction, String> {
        self.inner.prepare_action(action, config)
    }
    fn tool_definitions(&self) -> Vec<crate::reasoning::inference::ToolDefinition> {
        self.inner.tool_definitions()
    }
    fn cancel_run(&self, run: &str, deadline: std::time::Instant) {
        self.inner.cancel_run(run, deadline);
    }
    async fn close_run(&self, run: &str, deadline: std::time::Instant) -> Result<(), String> {
        self.inner.close_run(run, deadline).await
    }
    async fn execute_actions(
        &self,
        actions: &[ProposedAction],
        config: &LoopConfig,
        breakers: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        self.inner.execute_actions(actions, config, breakers).await
    }
    async fn execute_authorized(
        &self,
        actions: Vec<AuthorizedAction>,
        config: &LoopConfig,
        breakers: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        self.inner
            .execute_authorized(actions, config, breakers)
            .await
    }
}

impl SubLoopDelegationExecutor {
    pub(super) fn take_children(&self, run: &str) -> Vec<ChildRun> {
        self.children
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(run)
            .unwrap_or_default()
    }

    pub(super) async fn delegate_protected(
        &self,
        grant: &AuthorizedAction,
        ctx: DelegationContext,
        parent: Option<&dyn JournalWriter>,
        project: &Result<std::path::PathBuf, String>,
    ) -> Result<String, DelegationError> {
        let parent =
            parent.ok_or_else(|| DelegationError::Audit("parent journal is unavailable".into()))?;
        let ProposedAction::Delegate {
            call_id,
            target,
            message,
        } = grant.action()
        else {
            return Err(DelegationError::Failed(
                "expected a delegation grant".into(),
            ));
        };
        let selected = match &self.registered {
            Some(registry) => {
                let selected = registry.resolve(target).map_err(DelegationError::Failed)?;
                let frozen = grant
                    .prepared()
                    .backend::<Arc<registry::RegisteredTarget>>()
                    .ok_or_else(|| {
                        DelegationError::Failed(
                            "registered delegation requires a frozen source grant".into(),
                        )
                    })?;
                if !Arc::ptr_eq(frozen, &selected)
                    || selected.agent.settings().agent_name != *target
                {
                    return Err(DelegationError::Failed(
                        "delegation source changed after authorization".into(),
                    ));
                }
                Some(selected)
            }
            None => None,
        };
        let prompt = match &selected {
            Some(selected) => &selected.prompt,
            None => self
                .registry
                .get(target)
                .ok_or_else(|| DelegationError::UnknownTarget(target.clone()))?,
        };
        if ctx.chain.iter().any(|name| {
            name == target
                || self.registered.as_ref().is_some_and(|registry| {
                    registry
                        .resolve(name)
                        .is_ok_and(|previous| previous.agent.settings().agent_name == *target)
                })
        }) {
            return Err(DelegationError::Cycle(target.clone()));
        }
        if ctx.depth >= self.max_depth {
            return Err(DelegationError::DepthExceeded(self.max_depth));
        }
        let project = project
            .as_ref()
            .map_err(|error| DelegationError::Audit(error.clone()))?;
        let agent_id = delegated_agent_id(target);
        let (writer, audit) = crate::reasoning::run_audit::open_run_journal(project, agent_id)
            .await
            .map_err(DelegationError::Audit)?;
        grant.check_live().map_err(DelegationError::Failed)?;
        let start = LoopEvent::DelegationStarted {
            run_key: grant.run_key().into(),
            call_id: call_id.clone(),
            call_fingerprint: grant.prepared().fingerprint().into(),
            target: target.clone(),
            child_agent_id: agent_id,
            audit: audit.clone(),
            system_prompt_hash: hex::encode(Sha256::digest(prompt.as_bytes())),
            message_hash: hex::encode(Sha256::digest(message.as_bytes())),
        };
        let mut link = serde_json::to_value(&start)
            .map_err(|error| DelegationError::Audit(error.to_string()))?
            .get("DelegationStarted")
            .cloned()
            .expect("serialized delegation event");
        link["parent_agent_id"] = serde_json::json!(grant.principal());
        if let Some(reference) = parent.audit_reference() {
            link["parent_audit"] = serde_json::json!(reference);
        }
        append_parent(parent, grant, start).await?;
        grant.check_live().map_err(DelegationError::Failed)?;

        let journal = Arc::new(ChildJournal {
            inner: writer,
            terminal: AtomicBool::new(false),
            failed: AtomicBool::new(false),
        });
        let mut conversation = Conversation::with_system(prompt);
        conversation.push(ConversationMessage::user(message));
        let mut chain = ctx.chain;
        chain.push(target.clone());
        let mut config = LoopConfig {
            delegation_depth: ctx.depth + 1,
            delegation_chain: chain,
            max_delegation_depth: self.max_depth,
            max_iterations: ctx.max_iterations,
            max_total_tokens: ctx.max_total_tokens,
            shared_budget: ctx
                .shared_budget
                .as_ref()
                .map(|budget| budget.child(ctx.max_total_tokens))
                .transpose()
                .map_err(DelegationError::Failed)?,
            timeout: ctx.timeout.min(
                grant
                    .deadline()
                    .saturating_duration_since(std::time::Instant::now()),
            ),
            ..Default::default()
        };
        let mut executor: Arc<dyn ActionExecutor> = Arc::new(LinkedExecutor {
            inner: self.executor.clone(),
            link,
            definition: selected.as_ref().map(|target| target.metadata.clone()),
        });
        if let Some(selected) = &selected {
            config.timeout = config.timeout.min(std::time::Duration::from_secs(
                selected
                    .agent
                    .settings()
                    .timeout_seconds
                    .unwrap_or(120)
                    .min(120),
            ));
            config.max_output_tokens = config.max_output_tokens.min(4096);
            executor = Arc::new(crate::reasoning::source_policy::SourcePolicyExecutor::new(
                executor,
                selected.agent.policy().clone(),
            ));
        }
        let owner = self
            .self_ref
            .upgrade()
            .ok_or_else(|| DelegationError::Failed("delegation owner is unavailable".into()))?;
        let sub = ReasoningLoopRunner {
            provider: self.provider.clone(),
            executor,
            policy_gate: self.policy_gate.clone(),
            context_manager: self.context_manager.clone(),
            circuit_breakers: self.circuit_breakers.clone(),
            journal: journal.clone(),
            knowledge_bridge: None,
            delegation: Some(owner.clone()),
        };
        let cancellation = CancellationToken::new();
        let _cancel_on_drop = cancellation.clone().drop_guard();
        let child_cancel = cancellation.clone();
        let (reply, response) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let result = sub
                .run_cancellable(agent_id, conversation, config, child_cancel)
                .await;
            owner
                .delegated_tokens
                .fetch_add(result.total_usage.total_tokens, Ordering::Relaxed);
            if journal.failed.load(Ordering::SeqCst) || !journal.terminal.load(Ordering::SeqCst) {
                let error = "child did not persist a complete required journal".to_string();
                tracing::error!(%agent_id, %error, "Required child audit failed");
                let _ = reply.send(Err(error.clone()));
                return Err(error);
            }
            let _ = reply.send(Ok(result));
            Ok(())
        });
        self.children
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(grant.run_key().into())
            .or_default()
            .push(ChildRun { cancellation, task });

        let result: LoopResult = response
            .await
            .map_err(|error| DelegationError::Audit(error.to_string()))?
            .map_err(DelegationError::Audit)?;
        append_parent(
            parent,
            grant,
            LoopEvent::DelegationFinished {
                run_key: grant.run_key().into(),
                call_id: call_id.clone(),
                audit,
                reason: result.termination_reason.clone(),
                output_hash: hex::encode(Sha256::digest(result.output.as_bytes())),
            },
        )
        .await?;
        match result.termination_reason {
            TerminationReason::Completed => Ok(result.output),
            reason => Err(DelegationError::Unconfirmed(format!(
                "target '{target}' did not complete: {reason:?}"
            ))),
        }
    }
}

async fn append_parent(
    parent: &dyn JournalWriter,
    grant: &AuthorizedAction,
    event: LoopEvent,
) -> Result<(), DelegationError> {
    parent
        .append(JournalEntry {
            sequence: parent.next_sequence().await,
            timestamp: chrono::Utc::now(),
            agent_id: grant.principal(),
            iteration: grant.iteration(),
            event,
        })
        .await
        .map_err(|error| DelegationError::Audit(error.to_string()))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::reasoning::{
        context_manager::DefaultContextManager,
        executor::DefaultActionExecutor,
        inference::{InferenceError, InferenceOptions, InferenceResponse},
        loop_types::LoopState,
        policy_bridge::DefaultPolicyGate,
    };

    struct NoInference;
    #[async_trait]
    impl InferenceProvider for NoInference {
        async fn complete(
            &self,
            _: &Conversation,
            _: &InferenceOptions,
        ) -> Result<InferenceResponse, InferenceError> {
            panic!("child inference before required parent link")
        }
        fn provider_name(&self) -> &str {
            "fixture"
        }
        fn default_model(&self) -> &str {
            "fixture"
        }
        fn supports_native_tools(&self) -> bool {
            false
        }
        fn supports_structured_output(&self) -> bool {
            false
        }
    }
    struct RejectLink;
    #[async_trait]
    impl JournalWriter for RejectLink {
        async fn next_sequence(&self) -> u64 {
            0
        }
        async fn append(&self, entry: JournalEntry) -> Result<(), JournalError> {
            assert!(matches!(entry.event, LoopEvent::DelegationStarted { .. }));
            Err(JournalError::WriteFailed(
                "injected parent link failure".into(),
            ))
        }
    }

    #[tokio::test]
    async fn parent_link_failure_and_missing_authority_prevent_child_inference() {
        let project = tempfile::tempdir().unwrap();
        let executor = Arc::new(DefaultActionExecutor::default());
        let gate = Arc::new(DefaultPolicyGate::permissive_for_dev_only());
        let delegation = SubLoopDelegationExecutor::new_protected(
            Arc::new(NoInference),
            executor.clone(),
            gate.clone(),
            Arc::new(DefaultContextManager::default()),
            Arc::new(CircuitBreakerRegistry::default()),
            Ok(project.path().into()),
            HashMap::from([("reviewer".into(), "fixture child".into())]),
            3,
        );
        assert!(matches!(
            delegation
                .delegate("reviewer", "task", DelegationContext::default())
                .await,
            Err(DelegationError::Audit(_))
        ));
        assert!(!project.path().join(".symbiont/governed").exists());
        let state = LoopState::new(AgentId::new(), Conversation::new());
        let config = LoopConfig::default();
        let grant = crate::reasoning::dispatch::authorize_action(
            &ProposedAction::Delegate {
                call_id: "exact-call".into(),
                target: "reviewer".into(),
                message: "task".into(),
            },
            &state,
            &config,
            executor.as_ref(),
            gate.as_ref(),
        )
        .await
        .unwrap();
        assert!(matches!(
            delegation
                .delegate_authorized(&grant, DelegationContext::default(), None)
                .await,
            Err(DelegationError::Audit(_))
        ));
        assert!(!project.path().join(".symbiont/governed").exists());
        assert!(matches!(
            delegation
                .delegate_authorized(&grant, DelegationContext::default(), Some(&RejectLink))
                .await,
            Err(DelegationError::Audit(_))
        ));
        for entry in std::fs::read_dir(project.path().join(".symbiont/governed")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|ext| ext == "jsonl") {
                assert_eq!(std::fs::metadata(path).unwrap().len(), 0);
            }
        }
        assert!(delegation.children.lock().unwrap().is_empty());
    }
}
