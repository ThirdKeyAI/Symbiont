//! End-to-end: a model-issued `delegate` tool call runs a target agent as a
//! sub-loop and returns its output to the parent, which then answers normally.
//! This exercises the whole path — tool call -> Delegate conversion -> policy
//! gate -> dispatch -> sub-loop -> correlated tool_result -> next turn.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use symbi_runtime::reasoning::circuit_breaker::CircuitBreakerRegistry;
use symbi_runtime::reasoning::context_manager::DefaultContextManager;
use symbi_runtime::reasoning::conversation::{Conversation, ConversationMessage, MessageRole};
use symbi_runtime::reasoning::delegation_executor::SubLoopDelegationExecutor;
use symbi_runtime::reasoning::executor::DefaultActionExecutor;
use symbi_runtime::reasoning::inference::{
    FinishReason, InferenceError, InferenceOptions, InferenceProvider, InferenceResponse,
    ToolCallRequest, Usage,
};
use symbi_runtime::reasoning::loop_types::{BufferedJournal, LoopConfig};
use symbi_runtime::reasoning::policy_bridge::DefaultPolicyGate;
use symbi_runtime::reasoning::reasoning_loop::ReasoningLoopRunner;
use symbi_runtime::types::AgentId;

/// Serves canned responses in order: parent turn 1 (delegate tool call),
/// sub-loop turn (the delegated answer), parent turn 2 (final text).
struct ScriptedProvider {
    responses: Mutex<Vec<InferenceResponse>>,
}

impl ScriptedProvider {
    fn new(responses: Vec<InferenceResponse>) -> Self {
        Self {
            responses: Mutex::new(responses),
        }
    }
}

fn text(content: &str) -> InferenceResponse {
    InferenceResponse {
        content: content.to_string(),
        tool_calls: vec![],
        finish_reason: FinishReason::Stop,
        usage: Usage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
        },
        model: "mock".into(),
    }
}

fn delegate_call(call_id: &str, agent: &str, task: &str) -> InferenceResponse {
    InferenceResponse {
        content: String::new(),
        tool_calls: vec![ToolCallRequest {
            id: call_id.to_string(),
            name: "delegate".to_string(),
            arguments: format!(r#"{{"agent":"{agent}","task":"{task}"}}"#),
        }],
        finish_reason: FinishReason::ToolCalls,
        usage: Usage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
        },
        model: "mock".into(),
    }
}

#[async_trait]
impl InferenceProvider for ScriptedProvider {
    async fn complete(
        &self,
        _conversation: &Conversation,
        _options: &InferenceOptions,
    ) -> Result<InferenceResponse, InferenceError> {
        let mut r = self.responses.lock().unwrap();
        if r.is_empty() {
            return Ok(text("no more scripted responses"));
        }
        Ok(r.remove(0))
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

#[tokio::test(flavor = "multi_thread")]
async fn delegate_tool_call_runs_the_target_and_returns_its_output() {
    // The parent's final reply is deliberately text that does NOT contain the
    // sub-agent's answer, so "the output reached the parent" can only be
    // proven by inspecting the tool_result message itself, not by a
    // coincidental substring match against a canned reply.
    let provider: Arc<dyn InferenceProvider> = Arc::new(ScriptedProvider::new(vec![
        delegate_call("toolu_1", "reviewer", "review src/main.rs"),
        text("REVIEWED: looks good"),
        text("Done."),
    ]));

    let registry = HashMap::from([("reviewer".to_string(), "You are reviewer".to_string())]);
    let delegation = SubLoopDelegationExecutor::new(
        provider.clone(),
        Arc::new(DefaultActionExecutor::default()),
        Arc::new(DefaultPolicyGate::permissive_for_dev_only()),
        Arc::new(DefaultContextManager::default()),
        Arc::new(CircuitBreakerRegistry::default()),
        Arc::new(BufferedJournal::new(100)),
        registry,
        3,
    );

    let runner = ReasoningLoopRunner {
        provider: provider.clone(),
        executor: Arc::new(DefaultActionExecutor::default()),
        policy_gate: Arc::new(DefaultPolicyGate::permissive_for_dev_only()),
        context_manager: Arc::new(DefaultContextManager::default()),
        circuit_breakers: Arc::new(CircuitBreakerRegistry::default()),
        journal: Arc::new(BufferedJournal::new(100)),
        knowledge_bridge: None,
        delegation: Some(delegation.clone()),
    };

    let mut conversation = Conversation::with_system("You are the coordinator");
    conversation.push(ConversationMessage::user(
        "Have reviewer review src/main.rs",
    ));

    let result = runner
        .run(AgentId::new(), conversation, LoopConfig::default())
        .await;

    // The parent must reach a normal completion — if the delegation result were
    // emitted as an orphan tool_result the next provider call would fail.
    // `TerminationReason` does not derive `PartialEq`, so assert via `matches!`
    // (mirrors the convention already used throughout reasoning_loop.rs's tests).
    assert!(
        matches!(
            result.termination_reason,
            symbi_runtime::reasoning::loop_types::TerminationReason::Completed
        ),
        "parent loop must survive the turn after a delegation: {:?}",
        result.termination_reason
    );
    // The parent's final scripted reply ("Done.") does not contain the
    // sub-agent's answer, so this can only pass if the parent genuinely
    // reached its own last turn -- not by coincidentally echoing text that
    // happens to appear in a canned response.
    assert_eq!(
        result.output, "Done.",
        "parent should reach its own final answer after processing the delegation result, got: {}",
        result.output
    );

    // Assert directly on the tool_result message correlated to the
    // originating call id, rather than on `Debug`-string containment: a
    // Debug-string check would also pass if the tool_result carried the
    // wrong id or an empty/discarded answer, since "toolu_1" independently
    // appears in the assistant's own tool_calls message regardless of what
    // the tool_result carries. `ConversationMessage`'s fields are public, so
    // this reads them directly via `Conversation::messages()`.
    let tool_result_msg = result
        .conversation
        .messages()
        .iter()
        .find(|m| m.role == MessageRole::Tool && m.tool_call_id.as_deref() == Some("toolu_1"))
        .expect("expected a tool_result message correlated to toolu_1");
    assert!(
        tool_result_msg.content.contains("REVIEWED: looks good"),
        "the delegation tool_result must carry the sub-agent's actual answer, got: {}",
        tool_result_msg.content
    );

    // The sub-loop's tokens are accounted for, not silently spent.
    assert!(
        delegation.delegated_token_usage() > 0,
        "sub-loop token usage must be recorded"
    );
}

/// Counterpart to the happy path: a `delegate` call naming an unregistered
/// target must surface as an honest, correlated `is_error` tool_result -- not
/// vanish, not orphan the tool_use, and not stall the loop. Exercises the
/// same tool-call -> Delegate -> dispatch -> tool_result path with the
/// rejection branch (`DelegationError::UnknownTarget`) instead of a
/// successful sub-loop run.
#[tokio::test(flavor = "multi_thread")]
async fn delegate_tool_call_to_unknown_target_is_an_honest_correlated_error() {
    let provider: Arc<dyn InferenceProvider> = Arc::new(ScriptedProvider::new(vec![
        delegate_call("toolu_2", "ghost", "do something"),
        text("Sorry, I couldn't reach that agent."),
    ]));

    // Empty registry: "ghost" is not a known delegation target.
    let delegation = SubLoopDelegationExecutor::new(
        provider.clone(),
        Arc::new(DefaultActionExecutor::default()),
        Arc::new(DefaultPolicyGate::permissive_for_dev_only()),
        Arc::new(DefaultContextManager::default()),
        Arc::new(CircuitBreakerRegistry::default()),
        Arc::new(BufferedJournal::new(100)),
        HashMap::new(),
        3,
    );

    let runner = ReasoningLoopRunner {
        provider: provider.clone(),
        executor: Arc::new(DefaultActionExecutor::default()),
        policy_gate: Arc::new(DefaultPolicyGate::permissive_for_dev_only()),
        context_manager: Arc::new(DefaultContextManager::default()),
        circuit_breakers: Arc::new(CircuitBreakerRegistry::default()),
        journal: Arc::new(BufferedJournal::new(100)),
        knowledge_bridge: None,
        delegation: Some(delegation.clone()),
    };

    let mut conversation = Conversation::with_system("You are the coordinator");
    conversation.push(ConversationMessage::user("Have ghost do something"));

    let result = runner
        .run(AgentId::new(), conversation, LoopConfig::default())
        .await;

    assert!(
        matches!(
            result.termination_reason,
            symbi_runtime::reasoning::loop_types::TerminationReason::Completed
        ),
        "the loop must survive an honest delegation failure, not stall: {:?}",
        result.termination_reason
    );

    let tool_result_msg = result
        .conversation
        .messages()
        .iter()
        .find(|m| m.role == MessageRole::Tool && m.tool_call_id.as_deref() == Some("toolu_2"))
        .expect("expected a tool_result message correlated to toolu_2");
    assert!(
        tool_result_msg
            .content
            .contains("unknown delegation target"),
        "expected an honest unknown-target error, got: {}",
        tool_result_msg.content
    );

    // Rejected before any sub-loop ran, so no tokens should be attributed.
    assert_eq!(
        delegation.delegated_token_usage(),
        0,
        "an unknown-target rejection must not run (and therefore must not cost) a sub-loop"
    );
}
