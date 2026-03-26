//! Offline integration tests for the reasoning loop ORGA cycle.
//!
//! Uses fully mocked inference providers (no real LLM needed) to exercise
//! the Observe-Reason-Gate-Act cycle deterministically. These tests do not
//! require any API keys or network access.
//!
//! Run:
//!   cargo test -j2 -p symbi-runtime --test reasoning_orga_tests -- --nocapture

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use symbi_runtime::reasoning::circuit_breaker::CircuitBreakerRegistry;
use symbi_runtime::reasoning::conversation::{Conversation, ConversationMessage};
use symbi_runtime::reasoning::executor::ActionExecutor;
use symbi_runtime::reasoning::inference::{
    FinishReason, InferenceError, InferenceOptions, InferenceProvider, InferenceResponse,
    ToolCallRequest, ToolDefinition, Usage,
};
use symbi_runtime::reasoning::loop_types::{
    BufferedJournal, LoopConfig, LoopEvent, Observation, ProposedAction, TerminationReason,
};
use symbi_runtime::reasoning::policy_bridge::DefaultPolicyGate;
use symbi_runtime::reasoning::reasoning_loop::ReasoningLoopRunner;
use symbi_runtime::types::AgentId;

// ---------------------------------------------------------------------------
// Mock inference provider for deterministic ORGA testing
// ---------------------------------------------------------------------------

/// A mock provider that returns a tool call on the first invocation, then
/// a text completion on the second invocation. This simulates a complete
/// ORGA cycle without any LLM network calls.
struct OrgaMockProvider {
    call_count: AtomicUsize,
}

impl OrgaMockProvider {
    fn new() -> Self {
        Self {
            call_count: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl InferenceProvider for OrgaMockProvider {
    async fn complete(
        &self,
        _conversation: &Conversation,
        _options: &InferenceOptions,
    ) -> Result<InferenceResponse, InferenceError> {
        let count = self.call_count.fetch_add(1, Ordering::SeqCst);

        match count {
            // First call: propose a tool call
            0 => Ok(InferenceResponse {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call_001".into(),
                    name: "test_tool".into(),
                    arguments: r#"{"input": "hello"}"#.into(),
                }],
                finish_reason: FinishReason::ToolCalls,
                usage: Usage {
                    prompt_tokens: 50,
                    completion_tokens: 20,
                    total_tokens: 70,
                },
                model: "mock-model".into(),
            }),
            // Second call: return final text response
            _ => Ok(InferenceResponse {
                content: "done".into(),
                tool_calls: vec![],
                finish_reason: FinishReason::Stop,
                usage: Usage {
                    prompt_tokens: 80,
                    completion_tokens: 5,
                    total_tokens: 85,
                },
                model: "mock-model".into(),
            }),
        }
    }

    fn provider_name(&self) -> &str {
        "orga-mock"
    }

    fn default_model(&self) -> &str {
        "mock-model"
    }

    fn supports_native_tools(&self) -> bool {
        true
    }

    fn supports_structured_output(&self) -> bool {
        false
    }
}

/// Mock executor that returns a canned result for the `test_tool` tool.
struct OrgaMockExecutor;

#[async_trait]
impl ActionExecutor for OrgaMockExecutor {
    async fn execute_actions(
        &self,
        actions: &[ProposedAction],
        _config: &LoopConfig,
        circuit_breakers: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        let mut observations = Vec::new();

        for action in actions {
            if let ProposedAction::ToolCall {
                call_id,
                name,
                arguments: _,
            } = action
            {
                if let Err(err) = circuit_breakers.check(name).await {
                    observations.push(Observation::tool_error(
                        call_id.clone(),
                        format!("Circuit open for '{}': {}", name, err),
                    ));
                    circuit_breakers.record_failure(name).await;
                    continue;
                }

                if name == "test_tool" {
                    observations.push(Observation::tool_result(
                        call_id.clone(),
                        r#"{"status": "ok", "result": "tool executed successfully"}"#.to_string(),
                    ));
                    circuit_breakers.record_success(name).await;
                } else {
                    observations.push(Observation::tool_error(
                        call_id.clone(),
                        format!("Unknown tool: {}", name),
                    ));
                    circuit_breakers.record_failure(name).await;
                }
            }
        }

        observations
    }
}

fn test_tool_def() -> ToolDefinition {
    ToolDefinition {
        name: "test_tool".into(),
        description: "A test tool that processes input.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "input": {
                    "type": "string",
                    "description": "Input string to process"
                }
            },
            "required": ["input"]
        }),
    }
}

fn make_loop_config(max_iterations: u32, tool_definitions: Vec<ToolDefinition>) -> LoopConfig {
    LoopConfig {
        max_iterations,
        max_total_tokens: 8000,
        timeout: Duration::from_secs(30),
        tool_definitions,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Exercises the full Observe-Reason-Gate-Act cycle:
///
/// 1. Provider returns a tool call proposal for "test_tool"
/// 2. Policy gate (permissive) approves it
/// 3. Executor runs the tool (mock) and returns a result observation
/// 4. Loop observes the result and feeds it back to the provider
/// 5. Provider returns a final text response "done"
/// 6. Loop completes with TerminationReason::Completed
#[tokio::test]
async fn test_full_orga_cycle() {
    let journal = Arc::new(BufferedJournal::new(100));
    let circuit_breakers = Arc::new(CircuitBreakerRegistry::default());

    let runner = ReasoningLoopRunner::builder()
        .provider(Arc::new(OrgaMockProvider::new()) as Arc<dyn InferenceProvider>)
        .executor(Arc::new(OrgaMockExecutor) as Arc<dyn ActionExecutor>)
        .policy_gate(Arc::new(DefaultPolicyGate::permissive()))
        .circuit_breakers(circuit_breakers)
        .journal(journal.clone())
        .build();

    let mut conv = Conversation::with_system(
        "You are a test assistant. Use the test_tool when asked, then respond with 'done'.",
    );
    conv.push(ConversationMessage::user("Please run the test tool."));

    let config = make_loop_config(5, vec![test_tool_def()]);

    let result = runner.run(AgentId::new(), conv, config).await;

    eprintln!("Output: {}", result.output);
    eprintln!("Iterations: {}", result.iterations);
    eprintln!("Termination: {:?}", result.termination_reason);

    // The loop should complete successfully
    assert!(
        matches!(result.termination_reason, TerminationReason::Completed),
        "Expected Completed, got {:?}",
        result.termination_reason
    );

    // Exactly 2 iterations: tool call + final text response
    assert_eq!(
        result.iterations, 2,
        "Expected exactly 2 iterations (tool call + final response), got {}",
        result.iterations
    );

    // Output should be the final text from the mock provider
    assert_eq!(
        result.output, "done",
        "Expected output 'done', got '{}'",
        result.output
    );

    // Verify journal recorded the full lifecycle
    let entries = journal.entries().await;
    eprintln!("Journal entries: {}", entries.len());
    for entry in &entries {
        eprintln!(
            "  seq={} iter={} event={:?}",
            entry.sequence,
            entry.iteration,
            std::mem::discriminant(&entry.event)
        );
    }

    // Must have Started event
    assert!(
        matches!(&entries[0].event, LoopEvent::Started { .. }),
        "Expected first entry to be Started, got {:?}",
        entries[0].event
    );

    // Must have Terminated event
    assert!(
        matches!(&entries.last().unwrap().event, LoopEvent::Terminated { .. }),
        "Expected last entry to be Terminated, got {:?}",
        entries.last().unwrap().event
    );

    // Must have at least one ReasoningComplete event (tool call phase)
    let has_reasoning = entries
        .iter()
        .any(|e| matches!(&e.event, LoopEvent::ReasoningComplete { .. }));
    assert!(
        has_reasoning,
        "Expected at least one ReasoningComplete journal entry"
    );

    // Must have at least one ToolsDispatched event
    let has_dispatch = entries
        .iter()
        .any(|e| matches!(&e.event, LoopEvent::ToolsDispatched { .. }));
    assert!(
        has_dispatch,
        "Expected at least one ToolsDispatched journal entry"
    );

    // Sequence numbers must be monotonically increasing
    for window in entries.windows(2) {
        assert!(
            window[1].sequence > window[0].sequence,
            "Sequence numbers must be monotonically increasing: {} -> {}",
            window[0].sequence,
            window[1].sequence
        );
    }
}
