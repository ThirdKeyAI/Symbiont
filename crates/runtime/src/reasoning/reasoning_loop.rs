//! Reasoning loop driver
//!
//! The main entry point for running an observe-reason-gate-act loop.
//! This module wires together the typestate phases, context management,
//! circuit breakers, and journal writing into a single `run()` function.

use std::sync::Arc;

use crate::reasoning::circuit_breaker::CircuitBreakerRegistry;
use crate::reasoning::context_manager::{ContextManager, DefaultContextManager};
use crate::reasoning::conversation::Conversation;
use crate::reasoning::executor::{ActionExecutor, ExecutionRunGuard};
use crate::reasoning::inference::InferenceProvider;
use crate::reasoning::knowledge_bridge::KnowledgeBridge;
use crate::reasoning::knowledge_executor::KnowledgeAwareExecutor;
use crate::reasoning::loop_types::*;
use crate::reasoning::phases::{AgentLoop, LoopContinuation, Reasoning};
use crate::reasoning::policy_bridge::{DefaultPolicyGate, ReasoningPolicyGate};
use crate::types::AgentId;

/// Configuration bundle for a reasoning loop run.
pub struct ReasoningLoopRunner {
    /// Inference provider (cloud or SLM).
    pub provider: Arc<dyn InferenceProvider>,
    /// Policy gate (mandatory).
    pub policy_gate: Arc<dyn ReasoningPolicyGate>,
    /// Action executor.
    pub executor: Arc<dyn ActionExecutor>,
    /// Context manager for token budget enforcement.
    pub context_manager: Arc<dyn ContextManager>,
    /// Circuit breaker registry (shared across iterations).
    pub circuit_breakers: Arc<CircuitBreakerRegistry>,
    /// Journal writer for durable execution.
    pub journal: Arc<dyn JournalWriter>,
    /// Optional knowledge bridge for context-aware reasoning.
    pub knowledge_bridge: Option<Arc<KnowledgeBridge>>,
    /// Optional agent-to-agent delegation handle. `None` → an approved
    /// `Delegate` action surfaces an honest error instead of running.
    pub delegation: Option<Arc<dyn crate::reasoning::delegation::DelegationExecutor>>,
}

/// Builder for `ReasoningLoopRunner` with typestate enforcement.
///
/// `provider` and `executor` are required to build. Execution also requires
/// an explicit journal; omission refuses before inference. Call order doesn't matter.
///
/// ```ignore
/// let runner = ReasoningLoopRunner::builder()
///     .provider(my_provider)
///     .executor(my_executor)
///     .journal(protected_run_journal)
///     .build();
/// ```
pub struct ReasoningLoopRunnerBuilder<P, E> {
    provider: P,
    executor: E,
    policy_gate: Option<Arc<dyn ReasoningPolicyGate>>,
    context_manager: Option<Arc<dyn ContextManager>>,
    circuit_breakers: Option<Arc<CircuitBreakerRegistry>>,
    journal: Option<Arc<dyn JournalWriter>>,
    knowledge_bridge: Option<Arc<KnowledgeBridge>>,
    delegation: Option<Arc<dyn crate::reasoning::delegation::DelegationExecutor>>,
}

impl ReasoningLoopRunner {
    /// Create a builder with a denying policy gate and required journal configuration.
    pub fn builder() -> ReasoningLoopRunnerBuilder<(), ()> {
        ReasoningLoopRunnerBuilder {
            provider: (),
            executor: (),
            policy_gate: None,
            context_manager: None,
            circuit_breakers: None,
            journal: None,
            knowledge_bridge: None,
            delegation: None,
        }
    }
}

// Methods available regardless of typestate
impl<P, E> ReasoningLoopRunnerBuilder<P, E> {
    /// Set a custom policy gate. Default: `DefaultPolicyGate::new()` (fail-closed).
    pub fn policy_gate(mut self, gate: Arc<dyn ReasoningPolicyGate>) -> Self {
        self.policy_gate = Some(gate);
        self
    }

    /// Set a custom context manager. Default: `DefaultContextManager::default()`.
    pub fn context_manager(mut self, manager: Arc<dyn ContextManager>) -> Self {
        self.context_manager = Some(manager);
        self
    }

    /// Set a custom circuit breaker registry. Default: `CircuitBreakerRegistry::default()`.
    pub fn circuit_breakers(mut self, registry: Arc<CircuitBreakerRegistry>) -> Self {
        self.circuit_breakers = Some(registry);
        self
    }

    /// Set the required journal writer. Without one, execution fails before inference.
    /// Use `run_audit::open_run_journal` for protected per-invocation storage.
    /// Explicit custom writers remain the embedding application's responsibility.
    pub fn journal(mut self, journal: Arc<dyn JournalWriter>) -> Self {
        self.journal = Some(journal);
        self
    }

    /// Set a knowledge bridge. Default: `None`.
    pub fn knowledge_bridge(mut self, bridge: Arc<KnowledgeBridge>) -> Self {
        self.knowledge_bridge = Some(bridge);
        self
    }

    /// Attach an agent-to-agent delegation handle.
    pub fn delegation(
        mut self,
        delegation: Arc<dyn crate::reasoning::delegation::DelegationExecutor>,
    ) -> Self {
        self.delegation = Some(delegation);
        self
    }
}

// Set provider (transitions from () to Arc<dyn InferenceProvider>)
impl<E> ReasoningLoopRunnerBuilder<(), E> {
    /// Set the inference provider (required).
    pub fn provider(
        self,
        provider: Arc<dyn InferenceProvider>,
    ) -> ReasoningLoopRunnerBuilder<Arc<dyn InferenceProvider>, E> {
        ReasoningLoopRunnerBuilder {
            provider,
            executor: self.executor,
            policy_gate: self.policy_gate,
            context_manager: self.context_manager,
            circuit_breakers: self.circuit_breakers,
            journal: self.journal,
            knowledge_bridge: self.knowledge_bridge,
            delegation: self.delegation,
        }
    }
}

// Set executor (transitions from () to Arc<dyn ActionExecutor>)
impl<P> ReasoningLoopRunnerBuilder<P, ()> {
    /// Set the action executor (required).
    pub fn executor(
        self,
        executor: Arc<dyn ActionExecutor>,
    ) -> ReasoningLoopRunnerBuilder<P, Arc<dyn ActionExecutor>> {
        ReasoningLoopRunnerBuilder {
            provider: self.provider,
            executor,
            policy_gate: self.policy_gate,
            context_manager: self.context_manager,
            circuit_breakers: self.circuit_breakers,
            journal: self.journal,
            knowledge_bridge: self.knowledge_bridge,
            delegation: self.delegation,
        }
    }
}

// build() only available when both provider and executor are set
impl ReasoningLoopRunnerBuilder<Arc<dyn InferenceProvider>, Arc<dyn ActionExecutor>> {
    /// Build the `ReasoningLoopRunner` with defaults for any unset fields.
    pub fn build(self) -> ReasoningLoopRunner {
        ReasoningLoopRunner {
            provider: self.provider,
            executor: self.executor,
            policy_gate: self
                .policy_gate
                .unwrap_or_else(|| Arc::new(DefaultPolicyGate::new())),
            context_manager: self
                .context_manager
                .unwrap_or_else(|| Arc::new(DefaultContextManager::default())),
            circuit_breakers: self
                .circuit_breakers
                .unwrap_or_else(|| Arc::new(CircuitBreakerRegistry::default())),
            journal: self.journal.unwrap_or_else(|| Arc::new(MissingJournal)),
            knowledge_bridge: self.knowledge_bridge,
            delegation: self.delegation,
        }
    }
}

/// Missing configuration must not silently select a non-durable writer.
struct MissingJournal;

#[async_trait::async_trait]
impl JournalWriter for MissingJournal {
    async fn append(&self, _: JournalEntry) -> Result<(), JournalError> {
        Err(JournalError::WriteFailed(
            "ReasoningLoopRunner requires an explicit journal; open protected run storage and pass it with .journal(...)".into(),
        ))
    }

    async fn next_sequence(&self) -> u64 {
        0
    }
}

impl ReasoningLoopRunner {
    /// Run the full reasoning loop.
    ///
    /// This is the main entry point. It creates the initial state, then
    /// drives the typestate machine through Reasoning → PolicyCheck →
    /// ToolDispatching → Observing until the loop terminates.
    pub async fn run(
        &self,
        agent_id: AgentId,
        conversation: Conversation,
        config: LoopConfig,
    ) -> LoopResult {
        self.run_cancellable(
            agent_id,
            conversation,
            config,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
    }

    /// Run with explicit cancellation, preserving worker cleanup and terminal audit.
    pub async fn run_cancellable(
        &self,
        agent_id: AgentId,
        conversation: Conversation,
        config: LoopConfig,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> LoopResult {
        let started = std::time::Instant::now();
        let mut state = LoopState::new(agent_id, conversation);
        state.trusted_context = self.executor.execution_context();
        let delegated_cleanup =
            super::delegation::DelegationRunGuard::new(self.delegation.clone(), &state);

        // Add knowledge tool definitions if bridge is present
        let mut config = config;
        let budget = config
            .shared_budget
            .get_or_insert_with(|| super::budget::SharedBudget::new(config.max_total_tokens))
            .clone();
        let identity = budget.snapshot();
        state.trusted_context.insert(
            "budget".into(),
            serde_json::json!({
                "root_id": identity.root_id, "scope": identity.scope, "limit": identity.limit,
            }),
        );
        if let Some(ref bridge) = self.knowledge_bridge {
            config.tool_definitions.extend(bridge.tool_definitions());
        }

        // Auto-populate tool definitions from executor if config has none
        if config.tool_definitions.is_empty() {
            let executor_tools = self.executor.tool_definitions();
            if !executor_tools.is_empty() {
                config.tool_definitions = executor_tools;
            }
        }

        // Apply tool profile filtering (orga-adaptive: tool curation)
        #[cfg(feature = "orga-adaptive")]
        if let Some(ref profile) = config.tool_profile {
            config.tool_definitions = profile.filter_tools(&config.tool_definitions);
        }

        let cleanup = match ExecutionRunGuard::new(self.executor.clone(), &state, &config) {
            Ok(guard) => guard,
            Err(message) => {
                return crate::reasoning::phases::LoopTermination {
                    reason: crate::reasoning::phases::LoopTerminationReason::Error { message },
                    state,
                }
                .into_result()
            }
        };

        // Emit loop started event
        let start_event = LoopEvent::Started {
            agent_id: state.agent_id,
            config: Box::new(config.clone()),
            execution_context: state.trusted_context.clone(),
        };
        if let Err(error) = self
            .journal
            .append(JournalEntry {
                sequence: self.journal.next_sequence().await,
                timestamp: chrono::Utc::now(),
                agent_id: state.agent_id,
                iteration: 0,
                event: start_event,
            })
            .await
        {
            return journal_failure(state, error);
        }

        // Wrap the entire loop in a timeout
        if let Err(error) = budget.attach(&self.journal, agent_id).await {
            return journal_failure(state, JournalError::WriteFailed(error));
        }
        let timeout = config.timeout;
        let execution = tokio::select! {
            biased;
            _ = cancellation.cancelled() => None,
            result = tokio::time::timeout(timeout, self.run_inner(state, config)) => Some(result),
        };
        // A timeout drops the future still holding its inference reservation,
        // and a dropped future cannot await settlement, so settle from out
        // here. The loop itself abandoned this run and recorded a terminal
        // reason for it, so the reservation is resolved.
        //
        // Cancellation is deliberately excluded. There the caller disconnected
        // while work may still be in flight, which is genuinely unresolved;
        // settling it would claim knowledge we do not have.
        if matches!(execution, Some(Err(_))) {
            budget.settle_outstanding().await;
        }
        let mut result = match execution {
            Some(Ok(result)) => result,
            None => LoopResult {
                output: String::new(),
                iterations: 0,
                total_usage: crate::reasoning::inference::Usage::default(),
                budget: None,
                termination_reason: TerminationReason::Error {
                    message: "agent execution cancelled".into(),
                },
                duration: started.elapsed(),
                conversation: Conversation::new(),
            },
            Some(Err(_)) => {
                tracing::warn!("Reasoning loop timed out after {:?}", timeout);
                LoopResult {
                    output: String::new(),
                    iterations: 0,
                    total_usage: crate::reasoning::inference::Usage::default(),
                    budget: None,
                    termination_reason: TerminationReason::Timeout,
                    duration: timeout,
                    conversation: Conversation::new(),
                }
            }
        };
        if let Err(error) = delegated_cleanup.close().await {
            result.output.clear();
            result.termination_reason = TerminationReason::Error {
                message: format!("Required delegated cleanup failed: {error}"),
            };
        }
        if let Err(error) = cleanup.close().await {
            result.output.clear();
            result.termination_reason = TerminationReason::Error {
                message: format!("Required execution finalization failed: {error}"),
            };
        }
        result.duration = started.elapsed();
        let snapshot = budget.snapshot();
        result.total_usage = snapshot.usage.clone();
        result.budget = Some(snapshot);
        if let Err(error) = self.emit_termination_event(agent_id, &result).await {
            result.output.clear();
            result.termination_reason = TerminationReason::Error {
                message: format!("Required journal write failed: {error}"),
            };
        }
        result
    }

    async fn run_inner(&self, state: LoopState, config: LoopConfig) -> LoopResult {
        if let Err(message) = self.executor.validate_configuration() {
            return crate::reasoning::phases::LoopTermination {
                reason: crate::reasoning::phases::LoopTerminationReason::Error { message },
                state,
            }
            .into_result();
        }
        let agent_id = state.agent_id;
        let mut current_loop = AgentLoop::<Reasoning>::new(state, config);

        // Build the effective executor: wrap with KnowledgeAwareExecutor if bridge is present
        let effective_executor: Arc<dyn ActionExecutor> =
            if let Some(ref bridge) = self.knowledge_bridge {
                Arc::new(KnowledgeAwareExecutor::new(
                    self.executor.clone(),
                    bridge.clone(),
                    agent_id,
                ))
            } else {
                self.executor.clone()
            };

        // Pre-hydration: extract and resolve references from task input (orga-adaptive: cold-start context)
        #[cfg(feature = "orga-adaptive")]
        if let Some(ref pre_hydration_config) = current_loop.config.pre_hydration {
            use crate::reasoning::conversation::ConversationMessage;
            use crate::reasoning::pre_hydrate::PreHydrationEngine;

            let engine = PreHydrationEngine::new(pre_hydration_config.clone());

            // Extract task input from last user message
            let task_input = current_loop
                .state
                .conversation
                .messages()
                .iter()
                .rev()
                .find(|m| m.role == crate::reasoning::conversation::MessageRole::User)
                .map(|m| m.content.clone())
                .unwrap_or_default();

            if !task_input.is_empty() {
                let refs = engine.extract_references(&task_input);
                if !refs.is_empty() {
                    let dispatcher = super::dispatch::GovernedToolDispatcher {
                        executor: effective_executor.as_ref(),
                        gate: self.policy_gate.as_ref(),
                        journal: self.journal.as_ref(),
                        circuit_breakers: self.circuit_breakers.as_ref(),
                    };
                    let hydrated = match engine
                        .hydrate(
                            &refs,
                            &dispatcher,
                            &current_loop.state,
                            &current_loop.config,
                        )
                        .await
                    {
                        Ok(hydrated) => hydrated,
                        Err(error) => return journal_failure(current_loop.state, error),
                    };

                    let references_found = refs.len();
                    let references_resolved = hydrated.resolved.len();
                    let references_failed = hydrated.failed.len();
                    let total_tokens = hydrated.total_tokens;

                    let context_text = PreHydrationEngine::format_context(&hydrated);
                    if !context_text.is_empty() {
                        current_loop
                            .state
                            .conversation
                            .push(ConversationMessage::user(context_text));
                    }

                    // Emit pre-hydration event
                    if let Err(error) = self
                        .journal
                        .append(JournalEntry {
                            sequence: self.journal.next_sequence().await,
                            timestamp: chrono::Utc::now(),
                            agent_id,
                            iteration: 0,
                            event: LoopEvent::PreHydrationComplete {
                                references_found,
                                references_resolved,
                                references_failed,
                                total_tokens,
                            },
                        })
                        .await
                    {
                        return journal_failure(current_loop.state, error);
                    }
                }
            }
        }

        loop {
            // Inject knowledge context before reasoning if bridge is present
            if let Some(ref bridge) = self.knowledge_bridge {
                if let Err(e) = bridge
                    .inject_context(&agent_id, &mut current_loop.state.conversation)
                    .await
                {
                    tracing::warn!("Knowledge context injection failed: {}", e);
                }
            }

            // Snapshot usage before reasoning to compute per-step delta
            let usage_before = current_loop.state.total_usage.clone();

            // Phase 1: Reasoning
            let policy_phase = match current_loop
                .produce_output(
                    self.provider.as_ref(),
                    self.context_manager.as_ref(),
                    self.delegation.is_some(),
                    self.journal.as_ref(),
                )
                .await
            {
                Ok(phase) => phase,
                Err(termination) => return termination.into_result(),
            };

            // Retain the proposal before policy checks. Recovery must reconcile
            // action receipts before deciding whether any effect can be repeated.
            let step_usage = crate::reasoning::inference::Usage {
                prompt_tokens: policy_phase
                    .state
                    .total_usage
                    .prompt_tokens
                    .saturating_sub(usage_before.prompt_tokens),
                completion_tokens: policy_phase
                    .state
                    .total_usage
                    .completion_tokens
                    .saturating_sub(usage_before.completion_tokens),
                total_tokens: policy_phase
                    .state
                    .total_usage
                    .total_tokens
                    .saturating_sub(usage_before.total_tokens),
            };
            let proposed_actions = policy_phase.proposed_actions();
            if let Err(error) = self
                .journal
                .append(JournalEntry {
                    sequence: self.journal.next_sequence().await,
                    timestamp: chrono::Utc::now(),
                    agent_id,
                    iteration: policy_phase.state.iteration,
                    event: LoopEvent::ReasoningComplete {
                        iteration: policy_phase.state.iteration,
                        actions: proposed_actions,
                        usage: step_usage,
                    },
                })
                .await
            {
                return journal_failure(policy_phase.state, error);
            }

            // Phase 2: Policy Check
            let dispatch_phase = match policy_phase
                .check_policy(self.policy_gate.as_ref(), effective_executor.as_ref())
                .await
            {
                Ok(phase) => phase,
                Err(termination) => return termination.into_result(),
            };

            // Emit PolicyEvaluated journal event
            let (action_count, denied_count) = dispatch_phase.policy_summary();
            if let Err(error) = self
                .journal
                .append(JournalEntry {
                    sequence: self.journal.next_sequence().await,
                    timestamp: chrono::Utc::now(),
                    agent_id,
                    iteration: dispatch_phase.state.iteration,
                    event: LoopEvent::PolicyEvaluated {
                        iteration: dispatch_phase.state.iteration,
                        action_count,
                        denied_count,
                        approved_calls: dispatch_phase.approved_calls(),
                        denied_calls: dispatch_phase.denied_calls(),
                    },
                })
                .await
            {
                return journal_failure(dispatch_phase.state, error);
            }

            // Phase 3: Tool Dispatching (uses effective_executor which handles knowledge tools)
            let dispatch_start = std::time::Instant::now();
            let observe_phase = match dispatch_phase
                .dispatch_tools_audited(
                    effective_executor.as_ref(),
                    self.circuit_breakers.as_ref(),
                    self.delegation.as_deref(),
                    Some(self.journal.as_ref()),
                )
                .await
            {
                Ok(phase) => phase,
                Err(termination) => return termination.into_result(),
            };
            let dispatch_duration = dispatch_start.elapsed();

            // Emit ToolsDispatched journal event
            let observation_count = observe_phase.observation_count();
            if let Err(error) = self
                .journal
                .append(JournalEntry {
                    sequence: self.journal.next_sequence().await,
                    timestamp: chrono::Utc::now(),
                    agent_id,
                    iteration: observe_phase.state.iteration,
                    event: LoopEvent::ToolsDispatched {
                        iteration: observe_phase.state.iteration,
                        tool_count: observation_count,
                        duration: dispatch_duration,
                    },
                })
                .await
            {
                return journal_failure(observe_phase.state, error);
            }

            if let Err(error) = self
                .journal
                .append(JournalEntry {
                    sequence: self.journal.next_sequence().await,
                    timestamp: chrono::Utc::now(),
                    agent_id,
                    iteration: observe_phase.state.iteration,
                    event: LoopEvent::ToolBatchCompleted {
                        iteration: observe_phase.state.iteration,
                        observations: observe_phase.observations(),
                        duration: dispatch_duration,
                    },
                })
                .await
            {
                return journal_failure(observe_phase.state, error);
            }

            // Phase 4: Observation
            // Emit ObservationsCollected before consuming observe_phase
            let obs_iteration = observe_phase.state.iteration;
            let obs_count = observe_phase.observation_count();
            if let Err(error) = self
                .journal
                .append(JournalEntry {
                    sequence: self.journal.next_sequence().await,
                    timestamp: chrono::Utc::now(),
                    agent_id,
                    iteration: obs_iteration,
                    event: LoopEvent::ObservationsCollected {
                        iteration: obs_iteration,
                        observation_count: obs_count,
                    },
                })
                .await
            {
                return journal_failure(observe_phase.state, error);
            }

            match observe_phase.observe_results() {
                LoopContinuation::Continue(reasoning_loop) => {
                    current_loop = *reasoning_loop;
                }
                LoopContinuation::Complete(result) => {
                    // Persist learnings if bridge is present and auto_persist is enabled
                    if let Some(ref bridge) = self.knowledge_bridge {
                        if let Err(e) = bridge
                            .persist_learnings(&agent_id, &result.conversation)
                            .await
                        {
                            tracing::warn!("Failed to persist learnings: {}", e);
                        }
                    }

                    return result;
                }
            }
        }
    }

    async fn emit_termination_event(
        &self,
        agent_id: AgentId,
        result: &LoopResult,
    ) -> Result<(), JournalError> {
        if let Some(budget) = &result.budget {
            self.journal
                .append(JournalEntry {
                    sequence: self.journal.next_sequence().await,
                    timestamp: chrono::Utc::now(),
                    agent_id,
                    iteration: result.iterations,
                    event: LoopEvent::BudgetUpdated {
                        budget: budget.clone(),
                    },
                })
                .await?;
        }
        let event = LoopEvent::Terminated {
            reason: result.termination_reason.clone(),
            iterations: result.iterations,
            total_usage: result.total_usage.clone(),
            duration: result.duration,
        };
        self.journal
            .append(JournalEntry {
                sequence: self.journal.next_sequence().await,
                timestamp: chrono::Utc::now(),
                agent_id,
                iteration: result.iterations,
                event,
            })
            .await
    }
}

fn journal_failure(state: LoopState, error: JournalError) -> LoopResult {
    crate::reasoning::phases::LoopTermination {
        reason: crate::reasoning::phases::LoopTerminationReason::Error {
            message: format!("Required journal write failed: {error}"),
        },
        state,
    }
    .into_result()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reasoning::circuit_breaker::CircuitBreakerRegistry;
    use crate::reasoning::context_manager::DefaultContextManager;
    use crate::reasoning::conversation::ConversationMessage;
    use crate::reasoning::executor::DefaultActionExecutor;
    use crate::reasoning::inference::*;
    use crate::reasoning::policy_bridge::DefaultPolicyGate;

    /// A mock inference provider for testing the loop.
    struct MockProvider {
        responses: std::sync::Mutex<Vec<InferenceResponse>>,
    }

    impl MockProvider {
        fn new(responses: Vec<InferenceResponse>) -> Self {
            Self {
                responses: std::sync::Mutex::new(responses),
            }
        }
    }

    #[async_trait::async_trait]
    impl InferenceProvider for MockProvider {
        async fn complete(
            &self,
            _conversation: &Conversation,
            _options: &InferenceOptions,
        ) -> Result<InferenceResponse, InferenceError> {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                Ok(InferenceResponse {
                    content: "I'm done.".into(),
                    tool_calls: vec![],
                    finish_reason: FinishReason::Stop,
                    usage: Usage {
                        prompt_tokens: 10,
                        completion_tokens: 5,
                        total_tokens: 15,
                    },
                    model: "mock".into(),
                })
            } else {
                Ok(responses.remove(0))
            }
        }

        fn provider_name(&self) -> &str {
            "mock"
        }
        fn default_model(&self) -> &str {
            "mock-model"
        }
        fn supports_native_tools(&self) -> bool {
            true
        }
        fn supports_structured_output(&self) -> bool {
            true
        }
    }

    fn make_runner(provider: Arc<dyn InferenceProvider>) -> ReasoningLoopRunner {
        ReasoningLoopRunner {
            provider,
            policy_gate: Arc::new(DefaultPolicyGate::permissive_for_dev_only()),
            executor: Arc::new(DefaultActionExecutor::default()),
            context_manager: Arc::new(DefaultContextManager::default()),
            circuit_breakers: Arc::new(CircuitBreakerRegistry::default()),
            journal: Arc::new(BufferedJournal::new(1000)),
            knowledge_bridge: None,
            delegation: None,
        }
    }

    #[tokio::test]
    async fn orga_dispatch_lends_the_current_call_journal() {
        struct EffectProbe;
        #[async_trait::async_trait]
        impl ActionExecutor for EffectProbe {
            fn tool_definitions(&self) -> Vec<ToolDefinition> {
                vec![ToolDefinition {
                    name: "effect_probe".into(),
                    description: "Journal channel fixture".into(),
                    parameters: serde_json::json!({"type":"object","properties":{}}),
                }]
            }
            async fn execute_actions(
                &self,
                _: &[ProposedAction],
                _: &LoopConfig,
                _: &CircuitBreakerRegistry,
            ) -> Vec<Observation> {
                panic!("fixture requires authorization")
            }
            async fn execute_authorized(
                &self,
                grants: Vec<super::super::prepared::AuthorizedAction>,
                _: &LoopConfig,
                _: &CircuitBreakerRegistry,
            ) -> Vec<Observation> {
                let mut results = Vec::new();
                for grant in grants {
                    let audit = grant
                        .effect_journal()
                        .expect("ORGA supplies the invocation journal");
                    audit
                        .append(
                            super::super::effect_journal::ToolEffect::NetworkRequestStarted {
                                request_id: "channel-fixture".into(),
                                method: "GET".into(),
                                url: "https://fixture.invalid/".into(),
                                request_hash: "synthetic-request-hash".into(),
                                request_bytes: 0,
                            },
                        )
                        .await
                        .unwrap();
                    let ProposedAction::ToolCall { call_id, name, .. } = grant.action() else {
                        panic!("tool")
                    };
                    results.push(
                        Observation::tool_result(name, "channel acknowledged")
                            .with_call_id(call_id),
                    );
                }
                results
            }
        }
        let provider = Arc::new(MockProvider::new(vec![InferenceResponse {
            content: String::new(),
            tool_calls: vec![ToolCallRequest {
                id: "channel-call".into(),
                name: "effect_probe".into(),
                arguments: "{}".into(),
            }],
            finish_reason: FinishReason::ToolCalls,
            usage: Usage::default(),
            model: "fixture".into(),
        }]));
        let journal = Arc::new(BufferedJournal::new(100));
        let mut runner = make_runner(provider);
        runner.executor = Arc::new(EffectProbe);
        runner.journal = journal.clone();
        let principal = AgentId::new();
        let result = runner
            .run(principal, Conversation::new(), LoopConfig::default())
            .await;
        assert!(
            matches!(
                result.termination_reason,
                TerminationReason::UnconfirmedEffects
            ),
            "{result:?}"
        );
        let entries = journal.entries().await;
        assert_eq!(
            entries
                .iter()
                .filter(|e| matches!(e.event, LoopEvent::ReasoningComplete { .. }))
                .count(),
            1
        );
        let authorization = entries
            .iter()
            .find_map(|entry| match &entry.event {
                LoopEvent::PolicyEvaluated { approved_calls, .. } if !approved_calls.is_empty() => {
                    Some(approved_calls[0]["fingerprint"].as_str().unwrap())
                }
                _ => None,
            })
            .unwrap();
        let effects: Vec<_> = entries
            .iter()
            .filter_map(|entry| match &entry.event {
                LoopEvent::ToolEffect {
                    call_fingerprint, ..
                } => Some((entry, call_fingerprint)),
                _ => None,
            })
            .collect();
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].0.agent_id, principal);
        assert_eq!(effects[0].1, authorization);
    }

    #[tokio::test]
    async fn required_cleanup_failure_prevents_successful_termination() {
        struct CleanupFailure;
        #[async_trait::async_trait]
        impl ActionExecutor for CleanupFailure {
            async fn execute_actions(
                &self,
                _: &[ProposedAction],
                _: &LoopConfig,
                _: &CircuitBreakerRegistry,
            ) -> Vec<crate::reasoning::loop_types::Observation> {
                Vec::new()
            }
            async fn close_run(&self, _: &str, _: std::time::Instant) -> Result<(), String> {
                Err("synthetic cleanup failure".into())
            }
        }
        let journal = Arc::new(BufferedJournal::new(100));
        let mut runner = make_runner(Arc::new(MockProvider::new(vec![])));
        runner.executor = Arc::new(CleanupFailure);
        runner.journal = journal.clone();
        let result = runner
            .run(AgentId::new(), Conversation::new(), LoopConfig::default())
            .await;
        assert!(result.output.is_empty());
        assert!(
            matches!(&result.termination_reason, TerminationReason::Error { message }
            if message.contains("synthetic cleanup failure"))
        );
        let entries = journal.entries().await;
        let endings: Vec<_> = entries
            .iter()
            .filter(|entry| matches!(entry.event, LoopEvent::Terminated { .. }))
            .collect();
        assert_eq!(endings.len(), 1);
        assert!(matches!(&endings[0].event, LoopEvent::Terminated {
            reason: TerminationReason::Error { message }, .. }
            if message.contains("synthetic cleanup failure")));
    }

    #[cfg(all(unix, feature = "cedar"))]
    fn prepared_fixture(
        dir: &std::path::Path,
        approval: bool,
    ) -> crate::toolclad::manifest::Manifest {
        let mut manifest: crate::toolclad::manifest::Manifest = toml::from_str(
            r#"
[tool]
name = "count_fixture"
version = "1"
binary = "/usr/bin/touch"
description = "Prepared call effect fixture"
[tool.cedar]
resource = "Tool::Fixture"
action = "execute"
[args.count]
position = 1
required = true
type = "integer"
min = 1
max = 5
clamp = true
[command]
template = "/usr/bin/touch {count}"
[output]
format = "text"
"#,
        )
        .unwrap();
        manifest.tool.human_approval = approval;
        manifest.command.template = Some(format!("/usr/bin/touch '{}/{{count}}'", dir.display()));
        manifest
    }

    #[cfg(all(unix, feature = "cedar"))]
    async fn prepared_cedar(
        manifest: &crate::toolclad::manifest::Manifest,
    ) -> Arc<dyn ReasoningPolicyGate> {
        let gate = super::super::cedar_gate::CedarPolicyGate::deny_by_default();
        gate.add_policy(super::super::cedar_gate::CedarPolicy {
            name: "fixture".into(), active: true,
            source: format!("{}\npermit(principal, action == Action::\"respond\", resource);\nforbid(principal, action == Tool::Fixture::Action::\"execute\", resource) when {{ context.invocation.arguments.count != \"5\" }};", crate::toolclad::cedar_gen::generate_policy(manifest).unwrap()),
        }).await;
        Arc::new(gate)
    }

    #[cfg(all(unix, feature = "cedar"))]
    fn prepared_provider(count: &str) -> Arc<dyn InferenceProvider> {
        Arc::new(MockProvider::new(vec![InferenceResponse {
            content: String::new(),
            tool_calls: vec![ToolCallRequest {
                id: "prepared-fixture-call".into(),
                name: "count_fixture".into(),
                arguments: serde_json::json!({"count":count}).to_string(),
            }],
            finish_reason: FinishReason::ToolCalls,
            usage: Usage::default(),
            model: "fixture".into(),
        }]))
    }

    #[cfg(all(unix, feature = "cedar"))]
    #[tokio::test]
    async fn normalized_call_matches_generated_cedar_audit_and_real_effect() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = prepared_fixture(dir.path(), false);
        let journal = Arc::new(BufferedJournal::new(100));
        let mut runner = make_runner(prepared_provider("999"));
        runner.policy_gate = prepared_cedar(&manifest).await;
        runner.executor = Arc::new(
            crate::toolclad::executor::ToolCladExecutor::new(vec![(
                "count_fixture".into(),
                manifest,
            )])
            .with_development_host_execution(),
        );
        runner.journal = journal.clone();
        let result = runner
            .run(
                AgentId::new(),
                Conversation::with_system("Prepared fixture"),
                LoopConfig::default(),
            )
            .await;
        assert!(
            matches!(result.termination_reason, TerminationReason::Completed),
            "{result:?}"
        );
        assert!(
            dir.path().join("5").exists(),
            "normalized action did not execute: {result:?}"
        );
        assert!(!dir.path().join("999").exists());
        let entries = journal.entries().await;
        let call = entries
            .iter()
            .find_map(|entry| match &entry.event {
                LoopEvent::PolicyEvaluated { approved_calls, .. } => approved_calls
                    .iter()
                    .find(|call| call["arguments"]["count"] == "5"),
                _ => None,
            })
            .expect("pre-effect audit must record normalized input");
        assert_eq!(call["contract"]["resource_type"], "Tool::Fixture");
        assert_eq!(call["contract"]["action_id"], "execute");
        assert!(call["contract"]["digest"]
            .as_str()
            .unwrap()
            .starts_with("sha256:"));
        assert!(result
            .conversation
            .messages()
            .iter()
            .any(|message| message.content.contains("toolclad:count_fixture")
                || message.content.contains("output_hash")));
    }

    #[cfg(all(unix, feature = "cedar", feature = "orga-adaptive"))]
    #[tokio::test]
    async fn pre_hydration_uses_prepared_policy_and_audit_before_real_effects() {
        use crate::reasoning::pre_hydrate::{PreHydrationConfig, ReferencePattern};
        for (allow, approval, expected_effect) in [
            (false, false, false),
            (true, false, true),
            (true, true, false),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut manifest = prepared_fixture(dir.path(), approval);
            let count = manifest.args.remove("count").unwrap();
            manifest.args.insert("input".into(), count);
            manifest.command.template = manifest
                .command
                .template
                .map(|template| template.replace("{count}", "{input}"));
            let mut runner = make_runner(Arc::new(MockProvider::new(Vec::new())));
            runner.executor = Arc::new(
                crate::toolclad::executor::ToolCladExecutor::new(vec![(
                    "count_fixture".into(),
                    manifest,
                )])
                .with_development_host_execution(),
            );
            runner.policy_gate = if allow {
                Arc::new(DefaultPolicyGate::permissive_for_dev_only())
            } else {
                Arc::new(DefaultPolicyGate::new())
            };
            let journal = Arc::new(BufferedJournal::new(100));
            runner.journal = journal.clone();
            let config = LoopConfig {
                pre_hydration: Some(PreHydrationConfig {
                    custom_patterns: vec![ReferencePattern {
                        ref_type: "fixture".into(),
                        pattern: "999".into(),
                    }],
                    resolution_tools: [("fixture".into(), "count_fixture".into())].into(),
                    ..Default::default()
                }),
                ..Default::default()
            };
            let mut conversation = Conversation::new();
            conversation.push(ConversationMessage::user("Resolve fixture 999"));
            let result = runner.run(AgentId::new(), conversation, config).await;
            assert_eq!(dir.path().join("5").exists(), expected_effect, "{result:?}");
            assert!(!dir.path().join("999").exists());
            let entries = journal.entries().await;
            let policy_index = entries
                .iter()
                .position(|entry| {
                    matches!(entry.event, LoopEvent::PolicyEvaluated { iteration: 0, .. })
                })
                .unwrap();
            let outcome_index = entries
                .iter()
                .position(|entry| {
                    matches!(
                        entry.event,
                        LoopEvent::ToolBatchCompleted { iteration: 0, .. }
                    )
                })
                .unwrap();
            assert!(policy_index < outcome_index);
            if expected_effect {
                let LoopEvent::PolicyEvaluated { approved_calls, .. } =
                    &entries[policy_index].event
                else {
                    unreachable!()
                };
                assert_eq!(approved_calls[0]["arguments"]["input"], "5");
                assert!(result
                    .conversation
                    .messages()
                    .iter()
                    .filter(|message| message.content.contains("[PRE_HYDRATED_CONTEXT]"))
                    .all(|message| message.role
                        != crate::reasoning::conversation::MessageRole::System));
            }
        }
    }

    #[cfg(all(unix, feature = "cedar"))]
    #[tokio::test]
    async fn manifest_approval_is_independent_of_policy_and_precedes_effects() {
        use crate::escalation::{
            Approver, Decision, EscalationGate, EscalationGateConfig, EscalationQueue, Surface,
        };
        use std::time::Duration;
        for (count, decision, expected_effect) in [
            ("999", Some(true), true),
            ("999", Some(false), false),
            ("999", None, false),
            ("1", Some(true), false),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let manifest = prepared_fixture(dir.path(), true);
            let executor = Arc::new(
                crate::toolclad::executor::ToolCladExecutor::new(vec![(
                    "count_fixture".into(),
                    manifest.clone(),
                )])
                .with_development_host_execution(),
            );
            assert!(executor
                .execute_tool("count_fixture", r#"{"count":"999"}"#)
                .unwrap_err()
                .contains("approval"));
            let mut runner = make_runner(prepared_provider(count));
            runner.executor = executor;
            let agent = AgentId::new();
            let conversation = Conversation::with_system("Approval fixture");
            let result = if let Some(allow) = decision {
                let queue = Arc::new(EscalationQueue::new());
                runner.policy_gate = Arc::new(EscalationGate::new(
                    prepared_cedar(&manifest).await,
                    queue.clone(),
                    EscalationGateConfig {
                        require_approval_tools: Vec::new(),
                        timeout: Duration::from_secs(5),
                    },
                ));
                tokio::time::timeout(Duration::from_secs(10), async {
                    let (result, ()) = tokio::join!(
                        runner.run(agent, conversation, LoopConfig::default()),
                        async {
                            let held = loop {
                                if let Some(held) =
                                    queue.list_pending_async().await.into_iter().next()
                                {
                                    break held;
                                }
                                tokio::time::sleep(Duration::from_millis(5)).await;
                            };
                            assert!(
                                !dir.path().join("5").exists(),
                                "effect occurred before approval"
                            );
                            queue
                                .resolve_async(
                                    &held.id,
                                    if allow {
                                        Decision::Approve { reason: None }
                                    } else {
                                        Decision::Deny {
                                            reason: Some("fixture denied".into()),
                                        }
                                    },
                                    Approver {
                                        surface: Surface::Tui,
                                        id: "fixture-operator".into(),
                                        display: "fixture operator".into(),
                                    },
                                )
                                .await
                                .unwrap();
                            assert!(
                                queue
                                    .resolve_async(
                                        &held.id,
                                        Decision::Approve { reason: None },
                                        Approver {
                                            surface: Surface::Tui,
                                            id: "fixture-operator".into(),
                                            display: "fixture operator".into()
                                        }
                                    )
                                    .await
                                    .is_err(),
                                "approval response was replayed"
                            );
                        }
                    );
                    result
                })
                .await
                .expect("approval fixture did not complete")
            } else {
                // A permissive policy still cannot waive a manifest requirement.
                runner.run(agent, conversation, LoopConfig::default()).await
            };
            assert_eq!(dir.path().join("5").exists(), expected_effect, "{result:?}");
            assert!(
                !dir.path().join("1").exists(),
                "operator approval overrode Cedar denial"
            );
        }
    }

    #[cfg(all(unix, feature = "cedar"))]
    #[tokio::test]
    async fn policy_modification_is_prepared_and_authorized_again() {
        struct RewriteGate;
        #[async_trait::async_trait]
        impl ReasoningPolicyGate for RewriteGate {
            async fn evaluate_action(
                &self,
                _: &AgentId,
                action: &ProposedAction,
                _: &LoopState,
            ) -> LoopDecision {
                match action {
                    ProposedAction::ToolCall {
                        call_id,
                        name,
                        arguments,
                    } if serde_json::from_str::<serde_json::Value>(arguments).unwrap()["count"]
                        == "5" =>
                    {
                        LoopDecision::Modify {
                            modified_action: Box::new(ProposedAction::ToolCall {
                                call_id: call_id.clone(),
                                name: name.clone(),
                                arguments: r#"{"count":"1"}"#.into(),
                            }),
                            reason: "fixture rewrite".into(),
                        }
                    }
                    ProposedAction::ToolCall { .. } => LoopDecision::Deny {
                        reason: "modified call is not authorized".into(),
                    },
                    _ => LoopDecision::Allow,
                }
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let mut runner = make_runner(prepared_provider("999"));
        runner.executor = Arc::new(
            crate::toolclad::executor::ToolCladExecutor::new(vec![(
                "count_fixture".into(),
                prepared_fixture(dir.path(), false),
            )])
            .with_development_host_execution(),
        );
        runner.policy_gate = Arc::new(RewriteGate);
        let result = runner
            .run(
                AgentId::new(),
                Conversation::with_system("Rewrite fixture"),
                LoopConfig::default(),
            )
            .await;
        assert!(!dir.path().join("1").exists());
        assert!(!dir.path().join("5").exists());
        assert!(result
            .conversation
            .messages()
            .iter()
            .any(|message| message.content.contains("modified call is not authorized")));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn journal_failure_stops_dispatch_and_never_reports_success() {
        use std::sync::atomic::{AtomicU64, Ordering};
        struct FailingJournal {
            sequence: AtomicU64,
            fail_at: u64,
            dispatch_checkpoint: AtomicU64,
        }
        #[async_trait::async_trait]
        impl JournalWriter for FailingJournal {
            async fn append(&self, entry: JournalEntry) -> Result<(), JournalError> {
                let checkpoint = self.sequence.fetch_add(1, Ordering::SeqCst);
                if matches!(entry.event, LoopEvent::ToolDispatchStarted { .. }) {
                    self.dispatch_checkpoint.store(checkpoint, Ordering::SeqCst);
                }
                if checkpoint == self.fail_at {
                    Err(JournalError::WriteFailed(
                        "storage fixture refused write".into(),
                    ))
                } else {
                    Ok(())
                }
            }
            async fn next_sequence(&self) -> u64 {
                self.sequence.load(Ordering::SeqCst)
            }
        }
        let manifest: crate::toolclad::manifest::Manifest = toml::from_str(
            r#"
[tool]
name = "touch_fixture"
version = "1"
binary = "/usr/bin/touch"
description = "Journal effect fixture"
[args.path]
position = 1
required = true
type = "string"
[command]
template = "/usr/bin/touch '{path}'"
[output]
format = "text"
"#,
        )
        .unwrap();
        // Discover the successful lifecycle, then interrupt every checkpoint.
        // Dispatch must wait for its durable start; later failures cannot undo
        // the marker and must never turn that effect into a successful result.
        let mut checkpoints = std::collections::VecDeque::from([u64::MAX]);
        let mut dispatch_checkpoint = u64::MAX;
        while let Some(fail_at) = checkpoints.pop_front() {
            let dir = tempfile::tempdir().unwrap();
            let marker = dir.path().join("effect");
            let provider = Arc::new(MockProvider::new(vec![InferenceResponse {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "fixture-call".into(),
                    name: "touch_fixture".into(),
                    arguments: serde_json::json!({"path":marker}).to_string(),
                }],
                finish_reason: FinishReason::ToolCalls,
                usage: Usage::default(),
                model: "fixture".into(),
            }]));
            let mut runner = make_runner(provider);
            runner.executor = Arc::new(
                crate::toolclad::executor::ToolCladExecutor::new(vec![(
                    "touch_fixture".into(),
                    manifest.clone(),
                )])
                .with_development_host_execution(),
            );
            let journal = Arc::new(FailingJournal {
                sequence: AtomicU64::new(0),
                fail_at,
                dispatch_checkpoint: AtomicU64::new(u64::MAX),
            });
            runner.journal = journal.clone();
            let result = runner
                .run(
                    AgentId::new(),
                    Conversation::with_system("Journal fixture"),
                    LoopConfig::default(),
                )
                .await;
            if fail_at == u64::MAX {
                let count = journal.sequence.load(Ordering::SeqCst);
                assert!(count < 32);
                checkpoints.extend(0..count);
                dispatch_checkpoint = journal.dispatch_checkpoint.load(Ordering::SeqCst);
                assert!(dispatch_checkpoint < count);
            }
            assert_eq!(
                marker.exists(),
                fail_at > dispatch_checkpoint,
                "checkpoint {fail_at}"
            );
            if fail_at == u64::MAX {
                assert!(matches!(
                    result.termination_reason,
                    TerminationReason::Completed
                ));
            } else {
                assert!(
                    matches!(result.termination_reason, TerminationReason::Error { ref message } if message.contains("storage fixture refused write")),
                    "checkpoint {fail_at}: {:?}",
                    result.termination_reason
                );
                assert!(result.output.is_empty());
            }
        }
    }

    #[tokio::test]
    async fn test_simple_text_response_terminates() {
        let provider = Arc::new(MockProvider::new(vec![InferenceResponse {
            content: "The answer is 42.".into(),
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
            usage: Usage {
                prompt_tokens: 20,
                completion_tokens: 10,
                total_tokens: 30,
            },
            model: "mock".into(),
        }]));

        let runner = make_runner(provider);
        let mut conv = Conversation::with_system("You are a test agent.");
        conv.push(ConversationMessage::user("What is 6 * 7?"));

        let result = runner
            .run(AgentId::new(), conv, LoopConfig::default())
            .await;

        assert!(matches!(
            result.termination_reason,
            TerminationReason::Completed
        ));
        assert_eq!(result.output, "The answer is 42.");
        assert_eq!(result.iterations, 1);
        assert_eq!(result.total_usage.total_tokens, 30);
    }

    #[tokio::test]
    async fn test_refusal_terminates_with_error_not_empty_respond() {
        // A model refusal (e.g. Anthropic stop_reason=refusal) must not be
        // read as a silent, successful empty completion -- it should
        // terminate the loop distinctly so callers can retry / fail over.
        let provider = Arc::new(MockProvider::new(vec![InferenceResponse {
            content: String::new(),
            tool_calls: vec![],
            finish_reason: FinishReason::Refusal,
            usage: Usage {
                prompt_tokens: 20,
                completion_tokens: 0,
                total_tokens: 20,
            },
            model: "mock".into(),
        }]));

        let runner = make_runner(provider);
        let mut conv = Conversation::with_system("You are a test agent.");
        conv.push(ConversationMessage::user("Do something unsafe."));

        let result = runner
            .run(AgentId::new(), conv, LoopConfig::default())
            .await;

        assert!(
            matches!(result.termination_reason, TerminationReason::Error { .. }),
            "expected Error termination for a refusal, got {:?}",
            result.termination_reason
        );
    }

    #[tokio::test]
    async fn test_no_progress_turn_terminates_with_error_not_empty_respond() {
        // A turn with no tool calls AND no text (e.g. a thinking-only turn)
        // is a no-progress turn. It must not be read as a silent, successful
        // empty completion -- it should terminate the loop distinctly.
        let provider = Arc::new(MockProvider::new(vec![InferenceResponse {
            content: String::new(),
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
            usage: Usage {
                prompt_tokens: 20,
                completion_tokens: 0,
                total_tokens: 20,
            },
            model: "mock".into(),
        }]));

        let runner = make_runner(provider);
        let mut conv = Conversation::with_system("You are a test agent.");
        conv.push(ConversationMessage::user("What is 6 * 7?"));

        let result = runner
            .run(AgentId::new(), conv, LoopConfig::default())
            .await;

        assert!(
            matches!(result.termination_reason, TerminationReason::Error { .. }),
            "expected Error termination for a no-progress turn, got {:?}",
            result.termination_reason
        );
    }

    #[tokio::test]
    async fn test_tool_call_then_response() {
        let provider = Arc::new(MockProvider::new(vec![
            // First response: tool call
            InferenceResponse {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call_1".into(),
                    name: "search".into(),
                    arguments: r#"{"q": "weather"}"#.into(),
                }],
                finish_reason: FinishReason::ToolCalls,
                usage: Usage {
                    prompt_tokens: 20,
                    completion_tokens: 15,
                    total_tokens: 35,
                },
                model: "mock".into(),
            },
            // Second response: final answer
            InferenceResponse {
                content: "The weather is sunny.".into(),
                tool_calls: vec![],
                finish_reason: FinishReason::Stop,
                usage: Usage {
                    prompt_tokens: 40,
                    completion_tokens: 10,
                    total_tokens: 50,
                },
                model: "mock".into(),
            },
        ]));

        let runner = make_runner(provider);
        let mut conv = Conversation::with_system("You are a weather agent.");
        conv.push(ConversationMessage::user("What's the weather?"));

        let result = runner
            .run(AgentId::new(), conv, LoopConfig::default())
            .await;

        assert!(matches!(
            result.termination_reason,
            TerminationReason::Completed
        ));
        assert_eq!(result.output, "The weather is sunny.");
        assert_eq!(result.iterations, 2);
        assert_eq!(result.total_usage.total_tokens, 85);
    }

    #[tokio::test]
    async fn test_max_iterations_termination() {
        // Provider always returns tool calls → loop should hit max_iterations
        let tool_response = || InferenceResponse {
            content: String::new(),
            tool_calls: vec![ToolCallRequest {
                id: "call_1".into(),
                name: "search".into(),
                arguments: "{}".into(),
            }],
            finish_reason: FinishReason::ToolCalls,
            usage: Usage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            },
            model: "mock".into(),
        };
        let provider = Arc::new(MockProvider::new(vec![
            tool_response(),
            tool_response(),
            tool_response(),
        ]));

        let runner = make_runner(provider);
        let conv = Conversation::with_system("Infinite loop test");

        let config = LoopConfig {
            max_iterations: 3,
            ..Default::default()
        };

        let result = runner.run(AgentId::new(), conv, config).await;
        assert!(matches!(
            result.termination_reason,
            TerminationReason::MaxIterations
        ));
        assert_eq!(result.iterations, 3);
    }

    #[tokio::test]
    async fn test_timeout_termination() {
        // Provider that takes forever
        struct SlowProvider;

        #[async_trait::async_trait]
        impl InferenceProvider for SlowProvider {
            async fn complete(
                &self,
                _conv: &Conversation,
                _opts: &InferenceOptions,
            ) -> Result<InferenceResponse, InferenceError> {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                unreachable!()
            }
            fn provider_name(&self) -> &str {
                "slow"
            }
            fn default_model(&self) -> &str {
                "slow"
            }
            fn supports_native_tools(&self) -> bool {
                false
            }
            fn supports_structured_output(&self) -> bool {
                false
            }
        }

        let runner = make_runner(Arc::new(SlowProvider));
        let conv = Conversation::with_system("Timeout test");

        let config = LoopConfig {
            timeout: std::time::Duration::from_millis(100),
            ..Default::default()
        };

        let result = runner.run(AgentId::new(), conv, config).await;
        assert!(matches!(
            result.termination_reason,
            TerminationReason::Timeout
        ));
    }

    #[tokio::test]
    async fn test_policy_denial_fed_back() {
        use crate::reasoning::loop_types::LoopDecision;

        /// A gate that denies the first tool call but allows the second
        struct DenyFirstGate {
            call_count: std::sync::atomic::AtomicU32,
        }

        #[async_trait::async_trait]
        impl ReasoningPolicyGate for DenyFirstGate {
            async fn evaluate_action(
                &self,
                _agent_id: &AgentId,
                action: &ProposedAction,
                _state: &LoopState,
            ) -> LoopDecision {
                if matches!(action, ProposedAction::ToolCall { .. }) {
                    let count = self
                        .call_count
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if count == 0 {
                        return LoopDecision::Deny {
                            reason: "Not authorized for first call".into(),
                        };
                    }
                }
                LoopDecision::Allow
            }
        }

        let provider = Arc::new(MockProvider::new(vec![
            // First: tool call (will be denied)
            InferenceResponse {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "c1".into(),
                    name: "search".into(),
                    arguments: "{}".into(),
                }],
                finish_reason: FinishReason::ToolCalls,
                usage: Usage::default(),
                model: "mock".into(),
            },
            // Second: response after denial
            InferenceResponse {
                content: "I couldn't use the tool.".into(),
                tool_calls: vec![],
                finish_reason: FinishReason::Stop,
                usage: Usage::default(),
                model: "mock".into(),
            },
        ]));

        let runner = ReasoningLoopRunner {
            provider,
            policy_gate: Arc::new(DenyFirstGate {
                call_count: std::sync::atomic::AtomicU32::new(0),
            }),
            executor: Arc::new(DefaultActionExecutor::default()),
            context_manager: Arc::new(DefaultContextManager::default()),
            circuit_breakers: Arc::new(CircuitBreakerRegistry::default()),
            journal: Arc::new(BufferedJournal::new(1000)),
            knowledge_bridge: None,
            delegation: None,
        };

        let conv = Conversation::with_system("test");
        let result = runner
            .run(AgentId::new(), conv, LoopConfig::default())
            .await;

        assert!(matches!(
            result.termination_reason,
            TerminationReason::Completed
        ));
        assert_eq!(result.output, "I couldn't use the tool.");
    }

    #[tokio::test]
    async fn test_runner_auto_populates_tool_definitions_from_executor() {
        use crate::reasoning::inference::ToolDefinition;

        /// An executor that reports tool definitions.
        struct ToolfulExecutor;

        #[async_trait::async_trait]
        impl ActionExecutor for ToolfulExecutor {
            async fn execute_actions(
                &self,
                _actions: &[ProposedAction],
                _config: &LoopConfig,
                _circuit_breakers: &CircuitBreakerRegistry,
            ) -> Vec<Observation> {
                Vec::new()
            }

            fn tool_definitions(&self) -> Vec<ToolDefinition> {
                vec![ToolDefinition {
                    name: "test_tool".into(),
                    description: "A test tool".into(),
                    parameters: serde_json::json!({}),
                }]
            }
        }

        let provider = Arc::new(MockProvider::new(vec![InferenceResponse {
            content: "Done.".into(),
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
            usage: Usage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            },
            model: "mock".into(),
        }]));

        let runner = ReasoningLoopRunner {
            provider,
            policy_gate: Arc::new(DefaultPolicyGate::permissive_for_dev_only()),
            executor: Arc::new(ToolfulExecutor),
            context_manager: Arc::new(DefaultContextManager::default()),
            circuit_breakers: Arc::new(CircuitBreakerRegistry::default()),
            journal: Arc::new(BufferedJournal::new(1000)),
            knowledge_bridge: None,
            delegation: None,
        };

        let config = LoopConfig::default();
        assert!(config.tool_definitions.is_empty());

        let conv = Conversation::with_system("test");
        let result = runner.run(AgentId::new(), conv, config).await;
        assert!(matches!(
            result.termination_reason,
            TerminationReason::Completed
        ));
    }

    #[tokio::test]
    async fn test_builder_requires_explicit_journal_before_inference() {
        let provider = Arc::new(MockProvider::new(vec![InferenceResponse {
            content: "Built with builder.".into(),
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
            usage: Usage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            },
            model: "mock".into(),
        }]));
        let executor: Arc<dyn ActionExecutor> = Arc::new(DefaultActionExecutor::default());

        let runner = ReasoningLoopRunner::builder()
            .provider(provider.clone())
            .executor(executor)
            .build();

        let conv = Conversation::with_system("builder test");
        let result = runner
            .run(AgentId::new(), conv, LoopConfig::default())
            .await;

        assert!(
            matches!(
                result.termination_reason,
                TerminationReason::Error { ref message } if message.contains("requires an explicit journal")
            ),
            "{result:?}"
        );
        assert!(result.output.is_empty());
        assert_eq!(result.iterations, 0);
        assert_eq!(
            provider.responses.lock().unwrap().len(),
            1,
            "provider must not be called"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_builder_with_protected_journal_records_a_complete_run() {
        let project = tempfile::tempdir().unwrap();
        let agent = AgentId::new();
        let (journal, reference) = super::super::run_audit::open_run_journal(project.path(), agent)
            .await
            .unwrap();
        let runner = ReasoningLoopRunner::builder()
            .provider(Arc::new(MockProvider::new(vec![InferenceResponse {
                content: "Recorded response.".into(),
                tool_calls: vec![],
                finish_reason: FinishReason::Stop,
                usage: Usage::default(),
                model: "fixture".into(),
            }])))
            .executor(Arc::new(DefaultActionExecutor::default()))
            .journal(journal)
            .build();
        let result = runner
            .run(agent, Conversation::new(), LoopConfig::default())
            .await;
        assert!(matches!(
            result.termination_reason,
            TerminationReason::Completed
        ));
        assert_eq!(result.output, "Recorded response.");
        let key: [u8; 32] = hex::decode(reference.public_key)
            .unwrap()
            .try_into()
            .unwrap();
        let entries = super::super::protected_journal::ProtectedJournal::verify_run(
            &reference.path,
            &key,
            reference.run_id,
        )
        .unwrap();
        assert!(matches!(entries[0].event, LoopEvent::Started { .. }));
        assert!(matches!(
            entries.last().unwrap().event,
            LoopEvent::Terminated {
                reason: TerminationReason::Completed,
                ..
            }
        ));
        assert!(entries.iter().all(|entry| entry.agent_id == agent));
    }

    #[tokio::test]
    async fn test_builder_with_custom_policy_gate() {
        let provider: Arc<dyn InferenceProvider> = Arc::new(MockProvider::new(vec![
            InferenceResponse {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "c1".into(),
                    name: "blocked_tool".into(),
                    arguments: "{}".into(),
                }],
                finish_reason: FinishReason::ToolCalls,
                usage: Usage::default(),
                model: "mock".into(),
            },
            InferenceResponse {
                content: "Blocked.".into(),
                tool_calls: vec![],
                finish_reason: FinishReason::Stop,
                usage: Usage::default(),
                model: "mock".into(),
            },
        ]));
        let executor: Arc<dyn ActionExecutor> = Arc::new(DefaultActionExecutor::default());

        use crate::reasoning::policy_bridge::ToolFilterPolicyGate;

        let runner = ReasoningLoopRunner::builder()
            .provider(provider)
            .executor(executor)
            .policy_gate(Arc::new(ToolFilterPolicyGate::allow(&["allowed_only"])))
            .journal(Arc::new(BufferedJournal::new(100)))
            .build();

        let conv = Conversation::with_system("policy test");
        let result = runner
            .run(AgentId::new(), conv, LoopConfig::default())
            .await;

        assert!(matches!(
            result.termination_reason,
            TerminationReason::Completed
        ));
    }

    #[test]
    fn test_builder_order_independent() {
        let provider: Arc<dyn InferenceProvider> = Arc::new(MockProvider::new(vec![]));
        let executor: Arc<dyn ActionExecutor> = Arc::new(DefaultActionExecutor::default());

        // executor before provider should also work
        let _runner = ReasoningLoopRunner::builder()
            .executor(executor)
            .provider(provider)
            .build();
        // If this compiles and doesn't panic, the test passes
    }
}
