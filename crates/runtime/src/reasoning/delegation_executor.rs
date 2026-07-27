//! `SubLoopDelegationExecutor` — v1 in-process delegation. Runs a target agent
//! as a bounded sub-loop that reuses the parent runner's shared deps + the
//! target's system prompt. Per-agent toolsets are a follow-up (see the design).

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use async_trait::async_trait;

use crate::reasoning::circuit_breaker::CircuitBreakerRegistry;
use crate::reasoning::context_manager::ContextManager;
use crate::reasoning::conversation::{Conversation, ConversationMessage};
use crate::reasoning::delegation::{DelegationContext, DelegationError, DelegationExecutor};
use crate::reasoning::executor::ActionExecutor;
use crate::reasoning::inference::InferenceProvider;
use crate::reasoning::loop_types::{JournalWriter, LoopConfig, TerminationReason};
use crate::reasoning::policy_bridge::ReasoningPolicyGate;
use crate::reasoning::reasoning_loop::ReasoningLoopRunner;
use crate::types::AgentId;

/// Namespace for delegated-agent ids. Stable across processes so the same target
/// always presents the same principal to the policy gate and the journal.
const DELEGATE_ID_NAMESPACE: uuid::Uuid = uuid::Uuid::from_bytes([
    0x9f, 0x1a, 0x4b, 0x2c, 0x7d, 0x3e, 0x4f, 0x50, 0x8a, 0x6b, 0x1c, 0x2d, 0x3e, 0x4f, 0x50, 0x61,
]);

/// The `AgentId` a delegated sub-loop runs under, derived from the target's name.
///
/// Deterministic on purpose: the same target yields the same id every time, so
/// delegated actions are attributable and a Cedar policy can name the principal.
pub fn delegated_agent_id(target: &str) -> AgentId {
    AgentId(uuid::Uuid::new_v5(
        &DELEGATE_ID_NAMESPACE,
        target.as_bytes(),
    ))
}

/// Runs delegated targets as bounded sub-loops sharing the parent's deps.
pub struct SubLoopDelegationExecutor {
    provider: Arc<dyn InferenceProvider>,
    executor: Arc<dyn ActionExecutor>,
    policy_gate: Arc<dyn ReasoningPolicyGate>,
    context_manager: Arc<dyn ContextManager>,
    circuit_breakers: Arc<CircuitBreakerRegistry>,
    journal: Arc<dyn JournalWriter>,
    /// Agent name -> system prompt for that agent.
    registry: HashMap<String, String>,
    max_depth: u32,
    /// Self-reference so a sub-loop can itself delegate (B -> C).
    self_ref: Weak<SubLoopDelegationExecutor>,
    /// Cumulative tokens spent by delegated sub-loops, so callers can report
    /// true cost instead of only the parent's own inference.
    delegated_tokens: std::sync::atomic::AtomicU32,
}

impl SubLoopDelegationExecutor {
    /// Total tokens spent by sub-loops run through this executor.
    pub fn delegated_token_usage(&self) -> u32 {
        self.delegated_tokens
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        provider: Arc<dyn InferenceProvider>,
        executor: Arc<dyn ActionExecutor>,
        policy_gate: Arc<dyn ReasoningPolicyGate>,
        context_manager: Arc<dyn ContextManager>,
        circuit_breakers: Arc<CircuitBreakerRegistry>,
        journal: Arc<dyn JournalWriter>,
        registry: HashMap<String, String>,
        max_depth: u32,
    ) -> Arc<Self> {
        Arc::new_cyclic(|weak| SubLoopDelegationExecutor {
            provider,
            executor,
            policy_gate,
            context_manager,
            circuit_breakers,
            journal,
            registry,
            max_depth,
            self_ref: weak.clone(),
            delegated_tokens: std::sync::atomic::AtomicU32::new(0),
        })
    }
}

#[async_trait]
impl DelegationExecutor for SubLoopDelegationExecutor {
    async fn delegate(
        &self,
        target: &str,
        message: &str,
        ctx: DelegationContext,
    ) -> Result<String, DelegationError> {
        // Resolve first, then enforce guards BEFORE running anything.
        let system_prompt = self
            .registry
            .get(target)
            .ok_or_else(|| DelegationError::UnknownTarget(target.to_string()))?;
        if ctx.chain.iter().any(|a| a == target) {
            return Err(DelegationError::Cycle(target.to_string()));
        }
        if ctx.depth >= self.max_depth {
            return Err(DelegationError::DepthExceeded(self.max_depth));
        }

        // Build the target's conversation.
        let mut conversation = Conversation::with_system(system_prompt);
        conversation.push(ConversationMessage::user(message));

        // Sub-runner reuses the parent's shared deps + this executor for nested
        // delegation. Propagate depth+1 and chain+[target] so nested delegations
        // stay bounded and cycle-checked.
        let sub = ReasoningLoopRunner {
            provider: self.provider.clone(),
            executor: self.executor.clone(),
            policy_gate: self.policy_gate.clone(),
            context_manager: self.context_manager.clone(),
            circuit_breakers: self.circuit_breakers.clone(),
            journal: self.journal.clone(),
            knowledge_bridge: None,
            delegation: self
                .self_ref
                .upgrade()
                .map(|arc| arc as Arc<dyn DelegationExecutor>),
        };

        let mut chain = ctx.chain.clone();
        chain.push(target.to_string());
        // Each hop inherits the parent's configured ceiling, not its remaining budget, so total spend across hops is bounded in practice only by the parent run's wall-clock `timeout` and by sub-loops currently having no tools; true remaining-budget deduction is the follow-up if either changes.
        let config = LoopConfig {
            delegation_depth: ctx.depth + 1,
            delegation_chain: chain,
            max_delegation_depth: self.max_depth,
            max_iterations: ctx.max_iterations,
            max_total_tokens: ctx.max_total_tokens,
            timeout: ctx.timeout,
            ..Default::default()
        };

        // Run under an id derived from the target's name rather than a fresh
        // random one, so a delegated action is attributable: policy decisions and
        // journal entries for the same target always carry the same principal,
        // and it can be named in a Cedar policy. A random id per call made
        // delegated work impossible to attribute or to write policy against.
        let result = sub
            .run(delegated_agent_id(target), conversation, config)
            .await;

        self.delegated_tokens.fetch_add(
            result.total_usage.total_tokens,
            std::sync::atomic::Ordering::Relaxed,
        );

        match result.termination_reason {
            // The sub-agent finished on its own terms. An explicitly empty
            // response (e.g. a deliberate `Respond`/`Terminate` with no
            // content) is a legitimate outcome the sub-agent chose, not a
            // failure to run -- the loop's own "no-progress turn" detection
            // already converts genuinely stuck/empty turns to `Error` before
            // `Completed` is ever reached, so by the time we get here an
            // empty `Completed` output means the target meant to say
            // nothing, not that delegation failed.
            TerminationReason::Completed => Ok(result.output),
            TerminationReason::PolicyDenial { reason } => Err(DelegationError::Failed(format!(
                "target '{}' was denied by policy: {}",
                target, reason
            ))),
            TerminationReason::Error { message } => Err(DelegationError::Failed(format!(
                "target '{}' errored: {}",
                target, message
            ))),
            other => Err(DelegationError::Failed(format!(
                "target '{}' did not complete: {:?}",
                target, other
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reasoning::context_manager::DefaultContextManager;
    use crate::reasoning::executor::DefaultActionExecutor;
    use crate::reasoning::inference::{
        FinishReason, InferenceError, InferenceOptions, InferenceResponse, ToolCallRequest, Usage,
    };
    use crate::reasoning::loop_types::BufferedJournal;
    use crate::reasoning::policy_bridge::DefaultPolicyGate;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Minimal mock inference provider that always returns a fixed assistant
    /// response, so a sub-loop terminates deterministically in one turn.
    /// Mirrors the shape of `reasoning_loop.rs`'s own `MockProvider` test.
    /// Counts invocations so rejection tests can prove the target never ran.
    struct MockProvider {
        response: String,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl InferenceProvider for MockProvider {
        async fn complete(
            &self,
            _conversation: &Conversation,
            _options: &InferenceOptions,
        ) -> Result<InferenceResponse, InferenceError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(InferenceResponse {
                content: self.response.clone(),
                tool_calls: vec![],
                finish_reason: FinishReason::Stop,
                usage: Usage {
                    prompt_tokens: 10,
                    completion_tokens: 5,
                    total_tokens: 15,
                },
                model: "mock".into(),
            })
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

    /// A provider that always proposes a tool call, so a sub-loop never
    /// reaches a `Respond`/`Terminate` action and instead runs out its
    /// iteration budget. Used to force a non-`Completed` termination
    /// (`MaxIterations`) so the failure-mapping in `delegate()` can be
    /// exercised deterministically and cheaply.
    struct LoopingToolCallProvider;

    #[async_trait]
    impl InferenceProvider for LoopingToolCallProvider {
        async fn complete(
            &self,
            _conversation: &Conversation,
            _options: &InferenceOptions,
        ) -> Result<InferenceResponse, InferenceError> {
            Ok(InferenceResponse {
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
            })
        }

        fn provider_name(&self) -> &str {
            "looping-mock"
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

    /// Builds a delegation executor plus the shared call counter for its
    /// mock provider, so tests can assert the target ran (or didn't).
    fn build_test_delegation(
        registry: HashMap<String, String>,
    ) -> (Arc<SubLoopDelegationExecutor>, Arc<AtomicUsize>) {
        build_test_delegation_with_max(registry, 3)
    }

    fn build_test_delegation_with_max(
        registry: HashMap<String, String>,
        max_depth: u32,
    ) -> (Arc<SubLoopDelegationExecutor>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let provider: Arc<dyn InferenceProvider> = Arc::new(MockProvider {
            response: "sub-loop response".to_string(),
            calls: calls.clone(),
        });
        let executor = build_test_delegation_with_provider(registry, provider, max_depth);
        (executor, calls)
    }

    fn build_test_delegation_with_provider(
        registry: HashMap<String, String>,
        provider: Arc<dyn InferenceProvider>,
        max_depth: u32,
    ) -> Arc<SubLoopDelegationExecutor> {
        SubLoopDelegationExecutor::new(
            provider,
            Arc::new(DefaultActionExecutor::default()),
            Arc::new(DefaultPolicyGate::permissive_for_dev_only()),
            Arc::new(DefaultContextManager::default()),
            Arc::new(CircuitBreakerRegistry::default()),
            Arc::new(BufferedJournal::new(1000)),
            registry,
            max_depth,
        )
    }

    #[test]
    fn delegated_agent_id_is_stable_per_target() {
        // Attribution depends on this: the same target must always present the
        // same principal, and two targets must not collide.
        let a = super::delegated_agent_id("reviewer");
        let b = super::delegated_agent_id("reviewer");
        let c = super::delegated_agent_id("planner");
        assert_eq!(a, b, "the same target must yield the same id");
        assert_ne!(a, c, "different targets must not share an id");
    }

    #[tokio::test]
    async fn cycle_is_rejected_before_running() {
        let registry = HashMap::from([("b".to_string(), "You are b".to_string())]);
        let (d, calls) = build_test_delegation(registry);
        let err = d
            .delegate(
                "b",
                "hi",
                DelegationContext {
                    depth: 0,
                    chain: vec!["b".into()],
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert_eq!(err, DelegationError::Cycle("b".into()));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "target must not run when rejected for a cycle"
        );
    }

    #[tokio::test]
    async fn depth_limit_is_enforced_before_running() {
        let registry = HashMap::from([("b".to_string(), "You are b".to_string())]);
        let (d, calls) = build_test_delegation_with_max(registry, 2);
        let err = d
            .delegate(
                "b",
                "hi",
                DelegationContext {
                    depth: 2,
                    chain: vec![],
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert_eq!(err, DelegationError::DepthExceeded(2));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "target must not run when rejected for depth"
        );
    }

    #[tokio::test]
    async fn unknown_target_is_rejected() {
        let (d, calls) = build_test_delegation(HashMap::new());
        let err = d
            .delegate("ghost", "hi", DelegationContext::default())
            .await
            .unwrap_err();
        assert_eq!(err, DelegationError::UnknownTarget("ghost".into()));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "target must not run when rejected as unknown"
        );
    }

    #[tokio::test]
    async fn known_target_runs_subloop_and_returns_output() {
        let registry = HashMap::from([("b".to_string(), "You are b".to_string())]);
        let (d, calls) = build_test_delegation(registry);
        let out = d
            .delegate("b", "please summarize", DelegationContext::default())
            .await
            .unwrap();
        assert_eq!(out, "sub-loop response");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn non_completed_subloop_is_an_honest_error_not_empty_ok() {
        // A sub-loop that never reaches Respond/Terminate (it keeps calling
        // tools until it exhausts its iteration budget, i.e. terminates with
        // `TerminationReason::MaxIterations`) must surface as
        // `DelegationError::Failed`, never as `Ok("")`. Before the fix,
        // `delegate()` unconditionally returned `Ok(result.output)`, which is
        // `Ok("")` for every non-`Completed` termination -- a fabricated
        // success the caller would read as an empty-but-successful
        // delegation instead of an honest failure.
        let registry = HashMap::from([("b".to_string(), "You are b".to_string())]);
        let d = build_test_delegation_with_provider(registry, Arc::new(LoopingToolCallProvider), 3);
        let err = d
            .delegate("b", "please summarize", DelegationContext::default())
            .await
            .unwrap_err();
        assert!(
            matches!(err, DelegationError::Failed(_)),
            "expected DelegationError::Failed for a non-completing sub-loop, got {:?}",
            err
        );
    }
}
