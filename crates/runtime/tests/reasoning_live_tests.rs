//! Live integration tests for the reasoning loop.
//!
//! Uses a real LLM via OpenRouter for inference, with controlled mocks for
//! executor, policy, and circuit breakers. This validates that the loop
//! correctly orchestrates real LLM responses through each phase.
//!
//! Requires OPENROUTER_API_KEY (+ optionally OPENROUTER_MODEL) in the
//! environment. All tests skip gracefully when no key is set.
//!
//! Run:
//!   OPENROUTER_API_KEY="..." OPENROUTER_MODEL="google/gemini-2.0-flash-001" \
//!     cargo test -j2 -p symbi-runtime --features http-input --test reasoning_live_tests -- --nocapture

#![cfg(feature = "http-input")]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use symbi_runtime::reasoning::circuit_breaker::{CircuitBreakerConfig, CircuitBreakerRegistry};
use symbi_runtime::reasoning::context_manager::DefaultContextManager;
use symbi_runtime::reasoning::conversation::{Conversation, ConversationMessage};
use symbi_runtime::reasoning::executor::ActionExecutor;
use symbi_runtime::reasoning::inference::{InferenceProvider, ToolDefinition};
use symbi_runtime::reasoning::loop_types::{
    BufferedJournal, LoopConfig, LoopDecision, LoopEvent, LoopState, Observation, ProposedAction,
    TerminationReason,
};
use symbi_runtime::reasoning::policy_bridge::{DefaultPolicyGate, ReasoningPolicyGate};
use symbi_runtime::reasoning::providers::cloud::CloudInferenceProvider;
use symbi_runtime::reasoning::reasoning_loop::ReasoningLoopRunner;
use symbi_runtime::types::AgentId;

// ---------------------------------------------------------------------------
// Shared infrastructure
// ---------------------------------------------------------------------------

/// Mock executor that returns canned results by tool name.
/// Unknown tools get error observations.
struct MockToolExecutor {
    canned: HashMap<String, String>,
}

impl MockToolExecutor {
    fn new(canned: HashMap<String, String>) -> Self {
        Self { canned }
    }
}

#[async_trait]
impl ActionExecutor for MockToolExecutor {
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
                // Check circuit breaker first
                if let Err(err) = circuit_breakers.check(name).await {
                    observations.push(Observation::tool_error(
                        call_id.clone(),
                        format!(
                            "Tool '{}' circuit is open: {}. The tool endpoint has been failing and is temporarily disabled.",
                            name, err
                        ),
                    ));
                    circuit_breakers.record_failure(name).await;
                    continue;
                }

                if let Some(result) = self.canned.get(name.as_str()) {
                    observations.push(Observation::tool_result(call_id.clone(), result.clone()));
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

/// Policy gate that denies specific tool names, allowing everything else.
struct DenyToolGate {
    denied_tools: HashMap<String, String>,
}

impl DenyToolGate {
    fn new(denied_tools: HashMap<String, String>) -> Self {
        Self { denied_tools }
    }
}

#[async_trait]
impl ReasoningPolicyGate for DenyToolGate {
    async fn evaluate_action(
        &self,
        _agent_id: &AgentId,
        action: &ProposedAction,
        _state: &LoopState,
    ) -> LoopDecision {
        if let ProposedAction::ToolCall { name, .. } = action {
            if let Some(reason) = self.denied_tools.get(name.as_str()) {
                return LoopDecision::Deny {
                    reason: reason.clone(),
                };
            }
        }
        LoopDecision::Allow
    }
}

/// Creates a `ReasoningLoopRunner` with a real cloud provider and the given
/// mock components. Returns `None` (skipping the test) if no API key is set.
fn make_live_runner(
    executor: Arc<dyn ActionExecutor>,
    policy_gate: Arc<dyn ReasoningPolicyGate>,
    circuit_breakers: Arc<CircuitBreakerRegistry>,
    journal: Arc<BufferedJournal>,
) -> Option<ReasoningLoopRunner> {
    let provider = CloudInferenceProvider::from_env()?;

    eprintln!(
        "Using provider: {} model: {}",
        provider.provider_name(),
        provider.default_model()
    );

    Some(ReasoningLoopRunner {
        provider: Arc::new(provider),
        policy_gate,
        executor,
        context_manager: Arc::new(DefaultContextManager::default()),
        circuit_breakers,
        journal,
        knowledge_bridge: None,
    })
}

// ---------------------------------------------------------------------------
// Tool definition helpers
// ---------------------------------------------------------------------------

fn weather_tool_def() -> ToolDefinition {
    ToolDefinition {
        name: "get_weather".into(),
        description:
            "Get current weather for a city. Returns temperature, conditions, and humidity.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "city": {
                    "type": "string",
                    "description": "City name"
                }
            },
            "required": ["city"]
        }),
    }
}

fn search_tool_def() -> ToolDefinition {
    ToolDefinition {
        name: "search".into(),
        description: "Search the web for information.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query"
                }
            },
            "required": ["query"]
        }),
    }
}

fn calculator_tool_def() -> ToolDefinition {
    ToolDefinition {
        name: "calculator".into(),
        description: "Evaluate a mathematical expression and return the numeric result.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "expression": {
                    "type": "string",
                    "description": "Math expression to evaluate"
                }
            },
            "required": ["expression"]
        }),
    }
}

fn delete_file_tool_def() -> ToolDefinition {
    ToolDefinition {
        name: "delete_file".into(),
        description: "Delete a file at the given path.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File path to delete"
                }
            },
            "required": ["path"]
        }),
    }
}

fn make_loop_config(max_iterations: u32, tool_definitions: Vec<ToolDefinition>) -> LoopConfig {
    LoopConfig {
        max_iterations,
        max_total_tokens: 8000,
        timeout: Duration::from_secs(60),
        tool_definitions,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Test 1: Multi-turn tool calling
///
/// LLM gets `get_weather` tool, asked "What's the weather in Paris?".
/// Mock executor returns canned weather data. LLM should call tool,
/// receive result, then synthesize a natural-language answer.
///
/// Phases: Reasoning → Policy → Dispatch → Observe → Reasoning → Complete
#[tokio::test]
async fn test_multi_turn_tool_calling() {
    let mut canned = HashMap::new();
    canned.insert(
        "get_weather".into(),
        serde_json::json!({
            "city": "Paris",
            "temperature_celsius": 18,
            "conditions": "partly cloudy",
            "humidity_percent": 65
        })
        .to_string(),
    );

    let journal = Arc::new(BufferedJournal::new(100));
    let circuit_breakers = Arc::new(CircuitBreakerRegistry::default());

    let runner = match make_live_runner(
        Arc::new(MockToolExecutor::new(canned)),
        Arc::new(DefaultPolicyGate::permissive()),
        circuit_breakers,
        journal.clone(),
    ) {
        Some(r) => r,
        None => {
            eprintln!("Skipping: no inference provider API key set");
            return;
        }
    };

    let mut conv = Conversation::with_system(
        "You are a helpful weather assistant. You MUST use the get_weather tool to answer weather questions. After receiving the tool result, summarize the weather in one sentence.",
    );
    conv.push(ConversationMessage::user("What's the weather in Paris?"));

    let config = make_loop_config(5, vec![weather_tool_def()]);

    let result = runner.run(AgentId::new(), conv, config).await;

    eprintln!("Output: {}", result.output);
    eprintln!("Iterations: {}", result.iterations);
    eprintln!("Termination: {:?}", result.termination_reason);

    assert!(
        result.iterations >= 2,
        "Expected at least 2 iterations (tool call + synthesis), got {}",
        result.iterations
    );
    assert!(
        matches!(result.termination_reason, TerminationReason::Completed),
        "Expected Completed, got {:?}",
        result.termination_reason
    );
    assert!(!result.output.is_empty(), "Expected non-empty output");

    // The LLM should mention weather details from the canned response
    let output_lower = result.output.to_lowercase();
    assert!(
        output_lower.contains("paris")
            || output_lower.contains("18")
            || output_lower.contains("cloudy")
            || output_lower.contains("weather"),
        "Expected weather details in output, got: {}",
        result.output
    );
}

/// Test 2: Policy denial feedback loop
///
/// LLM gets `delete_file` tool, asked to clean up temp files. Custom gate
/// denies `delete_file` with a reason. LLM should adapt — acknowledging
/// the constraint instead of retrying the denied action.
///
/// Phases: Reasoning → Policy (deny) → Observe → Reasoning → Complete
#[tokio::test]
async fn test_policy_denial_feedback_loop() {
    let mut denied = HashMap::new();
    denied.insert(
        "delete_file".into(),
        "Destructive operations require approval. The delete_file tool is not permitted.".into(),
    );

    let journal = Arc::new(BufferedJournal::new(100));
    let circuit_breakers = Arc::new(CircuitBreakerRegistry::default());

    let runner = match make_live_runner(
        Arc::new(MockToolExecutor::new(HashMap::new())),
        Arc::new(DenyToolGate::new(denied)),
        circuit_breakers,
        journal.clone(),
    ) {
        Some(r) => r,
        None => {
            eprintln!("Skipping: no inference provider API key set");
            return;
        }
    };

    let mut conv = Conversation::with_system(
        "You are a file management assistant. You have access to a delete_file tool. \
         If a tool call is denied by policy, acknowledge the restriction and explain \
         that you cannot perform the action. Do NOT retry denied actions.",
    );
    conv.push(ConversationMessage::user(
        "Please delete the file /tmp/old_cache.txt to clean up.",
    ));

    let config = make_loop_config(5, vec![delete_file_tool_def()]);

    let result = runner.run(AgentId::new(), conv, config).await;

    eprintln!("Output: {}", result.output);
    eprintln!("Iterations: {}", result.iterations);
    eprintln!("Termination: {:?}", result.termination_reason);

    assert!(
        result.iterations >= 2,
        "Expected at least 2 iterations (denied + adaptation), got {}",
        result.iterations
    );
    assert!(!result.output.is_empty(), "Expected non-empty output");

    // Verify journal captured a PolicyEvaluated event with denials
    let entries = journal.entries().await;
    let has_denial = entries.iter().any(|e| {
        matches!(
            &e.event,
            LoopEvent::PolicyEvaluated { denied_count, .. } if *denied_count > 0
        )
    });
    assert!(
        has_denial,
        "Expected PolicyEvaluated journal entry with denied_count > 0"
    );
}

/// Test 3: Journal event capture
///
/// Multi-turn tool scenario, then verify `BufferedJournal` recorded the full
/// event sequence with monotonically increasing sequence numbers.
#[tokio::test]
async fn test_journal_event_capture() {
    let mut canned = HashMap::new();
    canned.insert(
        "get_weather".into(),
        serde_json::json!({
            "city": "London",
            "temperature_celsius": 12,
            "conditions": "rainy",
            "humidity_percent": 80
        })
        .to_string(),
    );

    let journal = Arc::new(BufferedJournal::new(200));
    let circuit_breakers = Arc::new(CircuitBreakerRegistry::default());

    let runner = match make_live_runner(
        Arc::new(MockToolExecutor::new(canned)),
        Arc::new(DefaultPolicyGate::permissive()),
        circuit_breakers,
        journal.clone(),
    ) {
        Some(r) => r,
        None => {
            eprintln!("Skipping: no inference provider API key set");
            return;
        }
    };

    let mut conv = Conversation::with_system(
        "You are a weather assistant. You MUST use the get_weather tool to answer weather questions. After receiving the tool result, respond with one sentence.",
    );
    conv.push(ConversationMessage::user(
        "What's the weather like in London right now?",
    ));

    let config = make_loop_config(5, vec![weather_tool_def()]);

    let result = runner.run(AgentId::new(), conv, config).await;

    eprintln!("Output: {}", result.output);
    eprintln!("Iterations: {}", result.iterations);
    eprintln!("Termination: {:?}", result.termination_reason);

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

    // Must have entries
    assert!(
        !entries.is_empty(),
        "Expected non-empty journal, got 0 entries"
    );

    // Must start with Started
    assert!(
        matches!(&entries[0].event, LoopEvent::Started { .. }),
        "Expected first entry to be Started, got {:?}",
        entries[0].event
    );

    // Must end with Terminated
    assert!(
        matches!(&entries.last().unwrap().event, LoopEvent::Terminated { .. }),
        "Expected last entry to be Terminated, got {:?}",
        entries.last().unwrap().event
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

/// Test 4: Circuit breaker pre-tripped
///
/// Pre-trip circuit breaker for `search` tool. LLM gets `search` + `calculator`
/// tools. When it tries `search`, gets circuit-open error. Should adapt
/// (use calculator or respond directly).
///
/// Phases: Reasoning → Policy → Dispatch (circuit open) → Observe → Reasoning → Complete
#[tokio::test]
async fn test_circuit_breaker_pre_tripped() {
    let mut canned = HashMap::new();
    canned.insert(
        "calculator".into(),
        serde_json::json!({ "result": 42 }).to_string(),
    );
    // search is deliberately absent from canned — but will hit circuit breaker first anyway

    let circuit_breakers = Arc::new(CircuitBreakerRegistry::new(CircuitBreakerConfig {
        failure_threshold: 2,
        recovery_timeout: Duration::from_secs(300), // won't recover during test
        half_open_max_calls: 1,
    }));

    // Pre-trip the search circuit breaker
    circuit_breakers.record_failure("search").await;
    circuit_breakers.record_failure("search").await;

    let journal = Arc::new(BufferedJournal::new(100));

    let runner = match make_live_runner(
        Arc::new(MockToolExecutor::new(canned)),
        Arc::new(DefaultPolicyGate::permissive()),
        circuit_breakers,
        journal.clone(),
    ) {
        Some(r) => r,
        None => {
            eprintln!("Skipping: no inference provider API key set");
            return;
        }
    };

    let mut conv = Conversation::with_system(
        "You are a helpful assistant with access to search and calculator tools. \
         If a tool call returns an error saying it is unavailable or the circuit is open, \
         do NOT retry that tool. Instead, respond with a text message explaining the situation. \
         IMPORTANT: After any tool error, give a direct text response immediately.",
    );
    conv.push(ConversationMessage::user(
        "Search for the population of France.",
    ));

    let config = make_loop_config(3, vec![search_tool_def(), calculator_tool_def()]);

    let result = runner.run(AgentId::new(), conv, config).await;

    eprintln!("Output: {}", result.output);
    eprintln!("Iterations: {}", result.iterations);
    eprintln!("Termination: {:?}", result.termination_reason);

    // The loop should handle the circuit-open error gracefully.
    // The LLM may complete (adapting after the error) or hit max_iterations.
    // Both are acceptable — the key assertion is that the loop didn't panic
    // and made progress past the circuit breaker error.
    assert!(
        result.iterations >= 2,
        "Expected at least 2 iterations (tool error + adaptation), got {}",
        result.iterations
    );
    assert!(
        matches!(
            result.termination_reason,
            TerminationReason::Completed | TerminationReason::MaxIterations
        ),
        "Expected Completed or MaxIterations, got {:?}",
        result.termination_reason
    );
}

/// Test 5: Max iterations guardrail
///
/// LLM gets tools that encourage repeated calling, with `max_iterations: 2`.
/// Should terminate with `MaxIterations` after 2 iterations.
#[tokio::test]
async fn test_max_iterations_guardrail() {
    let mut canned = HashMap::new();
    canned.insert(
        "get_weather".into(),
        serde_json::json!({
            "city": "Tokyo",
            "temperature_celsius": 25,
            "conditions": "sunny",
            "humidity_percent": 50,
            "note": "For more detail, call get_weather again with the same city."
        })
        .to_string(),
    );

    let journal = Arc::new(BufferedJournal::new(100));
    let circuit_breakers = Arc::new(CircuitBreakerRegistry::default());

    let runner = match make_live_runner(
        Arc::new(MockToolExecutor::new(canned)),
        Arc::new(DefaultPolicyGate::permissive()),
        circuit_breakers,
        journal.clone(),
    ) {
        Some(r) => r,
        None => {
            eprintln!("Skipping: no inference provider API key set");
            return;
        }
    };

    let mut conv = Conversation::with_system(
        "You are a weather assistant. You MUST call the get_weather tool to answer questions. \
         Never respond without calling a tool first.",
    );
    conv.push(ConversationMessage::user("What's the weather in Tokyo?"));

    // Strict limit of 1 iteration — the LLM will call the tool on iteration 1,
    // then the second produce_output check hits the cap before the LLM can respond.
    let config = make_loop_config(1, vec![weather_tool_def()]);

    let result = runner.run(AgentId::new(), conv, config).await;

    eprintln!("Output: {}", result.output);
    eprintln!("Iterations: {}", result.iterations);
    eprintln!("Termination: {:?}", result.termination_reason);

    assert_eq!(
        result.iterations, 1,
        "Expected exactly 1 iteration, got {}",
        result.iterations
    );
    assert!(
        matches!(result.termination_reason, TerminationReason::MaxIterations),
        "Expected MaxIterations, got {:?}",
        result.termination_reason
    );
}
