//! Reasoning loop driver
//!
//! The main entry point for running an observe-reason-gate-act loop.
//! This module wires together the typestate phases, context management,
//! circuit breakers, and journal writing into a single `run()` function.

use std::sync::Arc;

use crate::reasoning::circuit_breaker::CircuitBreakerRegistry;
use crate::reasoning::context_manager::{ContextManager, DefaultContextManager};
use crate::reasoning::conversation::Conversation;
use crate::reasoning::executor::ActionExecutor;
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
}

/// Builder for `ReasoningLoopRunner` with typestate enforcement.
///
/// Only `provider` and `executor` are required. All other fields have
/// sensible defaults. Call order doesn't matter.
///
/// ```ignore
/// let runner = ReasoningLoopRunner::builder()
///     .provider(my_provider)
///     .executor(my_executor)
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
}

impl ReasoningLoopRunner {
    /// Create a new builder with sensible defaults.
    pub fn builder() -> ReasoningLoopRunnerBuilder<(), ()> {
        ReasoningLoopRunnerBuilder {
            provider: (),
            executor: (),
            policy_gate: None,
            context_manager: None,
            circuit_breakers: None,
            journal: None,
            knowledge_bridge: None,
        }
    }
}

// Methods available regardless of typestate
impl<P, E> ReasoningLoopRunnerBuilder<P, E> {
    /// Set a custom policy gate. Default: `DefaultPolicyGate::permissive()`.
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

    /// Set a custom journal writer. Default: `BufferedJournal::new(1000)`.
    pub fn journal(mut self, journal: Arc<dyn JournalWriter>) -> Self {
        self.journal = Some(journal);
        self
    }

    /// Set a knowledge bridge. Default: `None`.
    pub fn knowledge_bridge(mut self, bridge: Arc<KnowledgeBridge>) -> Self {
        self.knowledge_bridge = Some(bridge);
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
                .unwrap_or_else(|| Arc::new(DefaultPolicyGate::permissive())),
            context_manager: self
                .context_manager
                .unwrap_or_else(|| Arc::new(DefaultContextManager::default())),
            circuit_breakers: self
                .circuit_breakers
                .unwrap_or_else(|| Arc::new(CircuitBreakerRegistry::default())),
            journal: self
                .journal
                .unwrap_or_else(|| Arc::new(BufferedJournal::new(1000))),
            knowledge_bridge: self.knowledge_bridge,
        }
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
        let state = LoopState::new(agent_id, conversation);

        // Add knowledge tool definitions if bridge is present
        let mut config = config;
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

        // Emit loop started event
        let start_event = LoopEvent::Started {
            agent_id: state.agent_id,
            config: config.clone(),
        };
        let _ = self
            .journal
            .append(JournalEntry {
                sequence: self.journal.next_sequence().await,
                timestamp: chrono::Utc::now(),
                agent_id: state.agent_id,
                iteration: 0,
                event: start_event,
            })
            .await;

        // Wrap the entire loop in a timeout
        let timeout = config.timeout;
        match tokio::time::timeout(timeout, self.run_inner(state, config)).await {
            Ok(result) => result,
            Err(_) => {
                tracing::warn!("Reasoning loop timed out after {:?}", timeout);
                LoopResult {
                    output: String::new(),
                    iterations: 0,
                    total_usage: crate::reasoning::inference::Usage::default(),
                    termination_reason: TerminationReason::Timeout,
                    duration: timeout,
                    conversation: Conversation::new(),
                }
            }
        }
    }

    async fn run_inner(&self, state: LoopState, config: LoopConfig) -> LoopResult {
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
                .produce_output(self.provider.as_ref(), self.context_manager.as_ref())
                .await
            {
                Ok(phase) => phase,
                Err(termination) => return termination.into_result(),
            };

            // Emit ReasoningComplete: captures the raw LLM output BEFORE policy check
            // so crash recovery can replay from journal without re-calling the LLM
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
            let _ = self
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
                .await;

            // Phase 2: Policy Check
            let dispatch_phase = match policy_phase.check_policy(self.policy_gate.as_ref()).await {
                Ok(phase) => phase,
                Err(termination) => return termination.into_result(),
            };

            // Emit PolicyEvaluated journal event
            let (action_count, denied_count) = dispatch_phase.policy_summary();
            let _ = self
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
                    },
                })
                .await;

            // Phase 3: Tool Dispatching (uses effective_executor which handles knowledge tools)
            let dispatch_start = std::time::Instant::now();
            let observe_phase = match dispatch_phase
                .dispatch_tools(effective_executor.as_ref(), self.circuit_breakers.as_ref())
                .await
            {
                Ok(phase) => phase,
                Err(termination) => return termination.into_result(),
            };
            let dispatch_duration = dispatch_start.elapsed();

            // Emit ToolsDispatched journal event
            let observation_count = observe_phase.observation_count();
            let _ = self
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
                .await;

            // Phase 4: Observation
            // Emit ObservationsCollected before consuming observe_phase
            let obs_iteration = observe_phase.state.iteration;
            let obs_count = observe_phase.observation_count();
            let _ = self
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
                .await;

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

                    // Emit termination event
                    let _ = self.emit_termination_event(agent_id, &result).await;
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
            policy_gate: Arc::new(DefaultPolicyGate::permissive()),
            executor: Arc::new(DefaultActionExecutor::default()),
            context_manager: Arc::new(DefaultContextManager::default()),
            circuit_breakers: Arc::new(CircuitBreakerRegistry::default()),
            journal: Arc::new(BufferedJournal::new(1000)),
            knowledge_bridge: None,
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
            policy_gate: Arc::new(DefaultPolicyGate::permissive()),
            executor: Arc::new(ToolfulExecutor),
            context_manager: Arc::new(DefaultContextManager::default()),
            circuit_breakers: Arc::new(CircuitBreakerRegistry::default()),
            journal: Arc::new(BufferedJournal::new(1000)),
            knowledge_bridge: None,
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
    async fn test_builder_minimal() {
        let provider: Arc<dyn InferenceProvider> =
            Arc::new(MockProvider::new(vec![InferenceResponse {
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
            .provider(provider)
            .executor(executor)
            .build();

        let conv = Conversation::with_system("builder test");
        let result = runner
            .run(AgentId::new(), conv, LoopConfig::default())
            .await;

        assert!(matches!(
            result.termination_reason,
            TerminationReason::Completed
        ));
        assert_eq!(result.output, "Built with builder.");
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
