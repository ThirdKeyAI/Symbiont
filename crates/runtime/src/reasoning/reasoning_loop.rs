//! Reasoning loop driver
//!
//! The main entry point for running an observe-reason-gate-act loop.
//! This module wires together the typestate phases, context management,
//! circuit breakers, and journal writing into a single `run()` function.

use std::sync::Arc;

use crate::reasoning::circuit_breaker::CircuitBreakerRegistry;
use crate::reasoning::context_manager::ContextManager;
use crate::reasoning::conversation::Conversation;
use crate::reasoning::executor::ActionExecutor;
use crate::reasoning::inference::InferenceProvider;
use crate::reasoning::loop_types::*;
use crate::reasoning::phases::{AgentLoop, LoopContinuation, Reasoning};
use crate::reasoning::policy_bridge::ReasoningPolicyGate;
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
        let mut current_loop = AgentLoop::<Reasoning>::new(state, config);

        loop {
            // Phase 1: Reasoning
            let policy_phase = match current_loop
                .produce_output(self.provider.as_ref(), self.context_manager.as_ref())
                .await
            {
                Ok(phase) => phase,
                Err(termination) => return termination.into_result(),
            };

            // Phase 2: Policy Check
            let dispatch_phase = match policy_phase.check_policy(self.policy_gate.as_ref()).await {
                Ok(phase) => phase,
                Err(termination) => return termination.into_result(),
            };

            // Emit policy evaluation journal event
            let (action_count, denied_count) = dispatch_phase.policy_summary();
            let _ = self
                .journal
                .append(JournalEntry {
                    sequence: self.journal.next_sequence().await,
                    timestamp: chrono::Utc::now(),
                    agent_id: dispatch_phase.state.agent_id,
                    iteration: dispatch_phase.state.iteration,
                    event: LoopEvent::PolicyEvaluated {
                        iteration: dispatch_phase.state.iteration,
                        action_count,
                        denied_count,
                    },
                })
                .await;

            // Phase 3: Tool Dispatching
            let observe_phase = match dispatch_phase
                .dispatch_tools(self.executor.as_ref(), self.circuit_breakers.as_ref())
                .await
            {
                Ok(phase) => phase,
                Err(termination) => return termination.into_result(),
            };

            // Phase 4: Observation
            match observe_phase.observe_results() {
                LoopContinuation::Continue(reasoning_loop) => {
                    current_loop = *reasoning_loop;
                }
                LoopContinuation::Complete(result) => {
                    // Emit termination event
                    let _ = self.emit_termination_event(&result).await;
                    return result;
                }
            }
        }
    }

    async fn emit_termination_event(&self, result: &LoopResult) -> Result<(), JournalError> {
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
                agent_id: crate::types::AgentId::new(), // placeholder, use real ID in production
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
}
