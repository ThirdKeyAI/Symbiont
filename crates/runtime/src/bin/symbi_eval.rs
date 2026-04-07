//! `symbi-eval` — JSON-in / JSON-out runner for the ORGA reasoning loop.
//!
//! Reads a single eval task as JSON (from stdin or `--task-file`), runs it
//! through `ReasoningLoopRunner` with a real cloud inference provider, then
//! emits the loop result and full journal to stdout as JSON.
//!
//! Designed to be invoked from the symbiont-eval Python harness via
//! `subprocess`.  Tools are mocked from the task definition; this exercises
//! the actual ORGA loop, journal, policy gate, and circuit breakers without
//! requiring a running symbiont server.
//!
//! ## Inference provider
//!
//! Uses `CloudInferenceProvider::from_env()`, which auto-detects from one of:
//!   * `OPENAI_API_KEY` (+ optional `OPENAI_BASE_URL`, `CHAT_MODEL`)
//!   * `OPENROUTER_API_KEY`
//!   * `ANTHROPIC_API_KEY`
//!
//! For local Ollama, set:
//!   ```bash
//!   export OPENAI_API_KEY=ollama
//!   export OPENAI_BASE_URL=http://localhost:11434/v1
//!   export CHAT_MODEL=gemma4:latest
//!   ```
//!
//! Build with `--features cloud-llm`.

#![cfg(feature = "cloud-llm")]

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use symbi_runtime::reasoning::circuit_breaker::CircuitBreakerRegistry;
use symbi_runtime::reasoning::conversation::{Conversation, ConversationMessage};
use symbi_runtime::reasoning::executor::ActionExecutor;
use symbi_runtime::reasoning::inference::{InferenceProvider, ToolDefinition};
use symbi_runtime::reasoning::loop_types::{
    BufferedJournal, JournalEntry, LoopConfig, LoopResult, Observation, ProposedAction,
};
use symbi_runtime::reasoning::policy_bridge::DefaultPolicyGate;
use symbi_runtime::reasoning::providers::cloud::CloudInferenceProvider;
use symbi_runtime::reasoning::reasoning_loop::ReasoningLoopRunner;
use symbi_runtime::types::AgentId;

// ---------------------------------------------------------------------------
// Task input format
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct EvalTask {
    /// Task identifier (echoed in output for correlation).
    task_id: String,
    /// System prompt for the agent.
    system: String,
    /// Initial user message. Either this or `conversation` must be set.
    #[serde(default)]
    prompt: Option<String>,
    /// Multi-turn user messages prepended to the conversation. Mutually
    /// exclusive with `prompt` (if both are provided, `conversation` wins).
    #[serde(default)]
    conversation: Vec<ConversationTurn>,
    /// Tool definitions exposed to the model. Each carries a `mock_response`
    /// the executor returns verbatim when the tool is called.
    #[serde(default)]
    tools: Vec<EvalTool>,
    /// Maximum reasoning loop iterations.
    #[serde(default = "default_max_iterations")]
    max_iterations: u32,
    /// Maximum total tokens before forced termination.
    #[serde(default = "default_max_tokens")]
    max_total_tokens: u32,
    /// Wall-clock timeout for the entire loop, in seconds.
    #[serde(default = "default_timeout_seconds")]
    timeout_seconds: u64,
}

#[derive(Debug, Deserialize)]
struct ConversationTurn {
    /// Always "user" today (assistant turns are produced by the loop itself).
    role: String,
    content: String,
}

#[derive(Debug, Deserialize, Clone)]
struct EvalTool {
    name: String,
    #[serde(default)]
    description: String,
    /// JSON schema for the tool parameters (passed through to the provider).
    parameters: serde_json::Value,
    /// Canned response returned by the executor whenever this tool is called.
    #[serde(default = "default_mock_response")]
    mock_response: String,
}

fn default_max_iterations() -> u32 {
    10
}
fn default_max_tokens() -> u32 {
    8000
}
fn default_timeout_seconds() -> u64 {
    60
}
fn default_mock_response() -> String {
    "{}".to_string()
}

// ---------------------------------------------------------------------------
// Result output format
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct EvalOutput {
    task_id: String,
    output: String,
    iterations: u32,
    total_usage: UsageOut,
    termination_reason: serde_json::Value,
    duration_ms: u128,
    journal_entries: Vec<JournalEntry>,
    /// Tool calls observed by the executor (name + arguments + mock response),
    /// in the order they happened. Convenient for the harness's
    /// `tool_sequence` scorer.
    tool_calls: Vec<ToolCallRecord>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct UsageOut {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

#[derive(Debug, Serialize, Clone)]
struct ToolCallRecord {
    name: String,
    arguments: String,
    response: String,
}

// ---------------------------------------------------------------------------
// Mock executor — looks up tool name in the task's tool list and returns
// the canned mock_response. Records each call into a shared Vec for the
// harness to consume.
// ---------------------------------------------------------------------------

struct MockToolExecutor {
    tools: Vec<EvalTool>,
    /// Recorded tool calls in dispatch order.
    recorded: Arc<tokio::sync::Mutex<Vec<ToolCallRecord>>>,
}

impl MockToolExecutor {
    fn new(tools: Vec<EvalTool>) -> Self {
        Self {
            tools,
            recorded: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }

    fn recorded_handle(&self) -> Arc<tokio::sync::Mutex<Vec<ToolCallRecord>>> {
        self.recorded.clone()
    }

    fn lookup(&self, name: &str) -> Option<&EvalTool> {
        self.tools.iter().find(|t| t.name == name)
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
                arguments,
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

                match self.lookup(name) {
                    Some(tool) => {
                        let response = tool.mock_response.clone();
                        self.recorded.lock().await.push(ToolCallRecord {
                            name: name.clone(),
                            arguments: arguments.clone(),
                            response: response.clone(),
                        });
                        observations.push(Observation::tool_result(call_id.clone(), response));
                        circuit_breakers.record_success(name).await;
                    }
                    None => {
                        observations.push(Observation::tool_error(
                            call_id.clone(),
                            format!("Unknown tool: {}", name),
                        ));
                        circuit_breakers.record_failure(name).await;
                    }
                }
            }
        }

        observations
    }

    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .iter()
            .map(|t| ToolDefinition {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.parameters.clone(),
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Read task JSON. Either --task-file <path> or stdin.
    let args: Vec<String> = std::env::args().collect();
    let task_json = if let Some(idx) = args.iter().position(|a| a == "--task-file") {
        let path = args
            .get(idx + 1)
            .ok_or("--task-file requires a path argument")?;
        std::fs::read_to_string(path)?
    } else {
        use std::io::Read;
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        s
    };

    let task: EvalTask = serde_json::from_str(&task_json)
        .map_err(|e| format!("Failed to parse task JSON: {}", e))?;

    // Build inference provider from environment.
    let provider = CloudInferenceProvider::from_env().ok_or_else(|| {
        "No LLM provider configured. Set OPENAI_API_KEY (+ OPENAI_BASE_URL for Ollama), \
         OPENROUTER_API_KEY, or ANTHROPIC_API_KEY"
            .to_string()
    })?;

    // Build executor with mock tools.
    let executor = Arc::new(MockToolExecutor::new(task.tools.clone()));
    let recorded = executor.recorded_handle();

    // Build runner.
    let journal = Arc::new(BufferedJournal::new(1000));
    let circuit_breakers = Arc::new(CircuitBreakerRegistry::default());

    let runner = ReasoningLoopRunner::builder()
        .provider(Arc::new(provider) as Arc<dyn InferenceProvider>)
        .executor(executor as Arc<dyn ActionExecutor>)
        .policy_gate(Arc::new(DefaultPolicyGate::permissive()))
        .circuit_breakers(circuit_breakers)
        .journal(journal.clone())
        .build();

    // Build conversation.
    let mut conv = Conversation::with_system(task.system.clone());
    if !task.conversation.is_empty() {
        for turn in &task.conversation {
            match turn.role.as_str() {
                "user" => conv.push(ConversationMessage::user(turn.content.clone())),
                other => {
                    return Err(format!(
                        "Unsupported conversation role '{}': only 'user' is supported",
                        other
                    )
                    .into());
                }
            }
        }
    } else if let Some(prompt) = &task.prompt {
        conv.push(ConversationMessage::user(prompt.clone()));
    } else {
        return Err("Task must have either `prompt` or `conversation`".into());
    }

    // Build loop config from task.
    let tool_definitions: Vec<ToolDefinition> = task
        .tools
        .iter()
        .map(|t| ToolDefinition {
            name: t.name.clone(),
            description: t.description.clone(),
            parameters: t.parameters.clone(),
        })
        .collect();

    let config = LoopConfig {
        max_iterations: task.max_iterations,
        max_total_tokens: task.max_total_tokens,
        timeout: Duration::from_secs(task.timeout_seconds),
        tool_definitions,
        ..Default::default()
    };

    // Run the loop.
    let started = std::time::Instant::now();
    let result: LoopResult = runner.run(AgentId::new(), conv, config).await;
    let duration_ms = started.elapsed().as_millis();

    // Drain journal & recorded tool calls.
    let entries = journal.entries().await;
    let tool_calls = recorded.lock().await.clone();

    // Serialize termination_reason via serde_json so the harness gets a
    // tagged enum representation it can parse without coupling to Rust types.
    let termination_reason_json = serde_json::to_value(&result.termination_reason)?;

    let out = EvalOutput {
        task_id: task.task_id,
        output: result.output,
        iterations: result.iterations,
        total_usage: UsageOut {
            prompt_tokens: result.total_usage.prompt_tokens,
            completion_tokens: result.total_usage.completion_tokens,
            total_tokens: result.total_usage.total_tokens,
        },
        termination_reason: termination_reason_json,
        duration_ms,
        journal_entries: entries,
        tool_calls,
        error: None,
    };

    println!("{}", serde_json::to_string(&out)?);
    Ok(())
}
