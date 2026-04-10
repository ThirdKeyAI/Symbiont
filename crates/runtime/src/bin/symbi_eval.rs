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
    /// Sampling temperature. Eval workloads should pin this to 0.0 for
    /// reproducibility; the runtime default of 0.3 is wrong for benchmarks.
    #[serde(default = "default_temperature")]
    temperature: f32,
    /// Optional per-task sandbox spec. Populated when the task declares
    /// non-mock tool kinds (fs_read/fs_write/fs_list/shell). If any such
    /// tool exists but `sandbox` is None, defaults are used.
    #[serde(default)]
    sandbox: Option<EvalSandbox>,
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
    /// Only consulted when `kind == "mock"`.
    #[serde(default = "default_mock_response")]
    mock_response: String,
    /// How the executor should handle this tool:
    ///   "mock"     — return `mock_response` verbatim (default)
    ///   "fs_read"  — read a file inside the task's scratch dir
    ///   "fs_write" — write a file inside the scratch dir
    ///   "fs_list"  — list dir contents inside the scratch dir
    ///   "shell"    — run `bash -c <command>` inside the scratch dir
    #[serde(default = "default_kind")]
    kind: String,
}

fn default_kind() -> String {
    "mock".to_string()
}

#[derive(Debug, Deserialize, Clone, Default)]
struct EvalSandbox {
    #[serde(default)]
    seed_files: std::collections::HashMap<String, String>,
    #[serde(default)]
    setup_commands: Vec<String>,
    #[serde(default = "default_command_timeout_seconds")]
    command_timeout_seconds: u64,
    #[serde(default = "default_max_output_bytes")]
    max_output_bytes: usize,
}

fn default_command_timeout_seconds() -> u64 {
    60
}
fn default_max_output_bytes() -> usize {
    65536
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
fn default_temperature() -> f32 {
    0.0
}

/// Replace any character outside `[a-zA-Z0-9_-]` with `_`.
///
/// Required for Anthropic-backed providers, which enforce that regex on
/// tool names. BFCL ships dotted module names like `math.factorial` that
/// would otherwise be rejected upstream.
fn sanitize_tool_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
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
// RealSandbox — per-task scratch dir + filesystem/shell tool implementations.
//
// Mirrors the Python `symbiont_eval.sandbox.Sandbox` class in symbiont-eval.
// Path-resolves caller-supplied relative paths inside the scratch root and
// rejects `..`-escape attempts. Runs shell commands via `bash -c` with a
// hard timeout and captured+truncated stdout/stderr.
//
// ISOLATION IS NOT SECURITY. The agent still has network access and can
// read arbitrary host paths via `shell` (e.g. `cat /etc/passwd`). For
// untrusted models, run the whole process inside Docker.
// ---------------------------------------------------------------------------

use std::path::{Path as StdPath, PathBuf};

struct RealSandbox {
    scratch_dir: PathBuf,
    command_timeout: Duration,
    max_output_bytes: usize,
    // Kept alive so the tempdir isn't removed until we explicitly drop it.
    _tempdir_guard: Option<tempfile::TempDir>,
}

impl RealSandbox {
    fn new(cfg: &EvalSandbox) -> std::io::Result<Self> {
        let td = tempfile::Builder::new().prefix("sbx-").tempdir()?;
        let scratch_dir = td.path().to_path_buf();
        Ok(Self {
            scratch_dir,
            command_timeout: Duration::from_secs(cfg.command_timeout_seconds),
            max_output_bytes: cfg.max_output_bytes,
            _tempdir_guard: Some(td),
        })
    }

    /// Write seed files and run setup commands. Errors during individual
    /// commands are captured but do not abort the sequence — the agent can
    /// still run afterwards even if some setup commands failed.
    async fn run_setup(&self, cfg: &EvalSandbox) -> Vec<String> {
        let mut log = Vec::new();

        for (rel, content) in &cfg.seed_files {
            match self.resolve(rel) {
                Ok(path) => {
                    if let Some(parent) = path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if let Err(e) = std::fs::write(&path, content) {
                        log.push(format!("seed {}: {}", rel, e));
                    } else {
                        log.push(format!("seed {}: ok", rel));
                    }
                }
                Err(e) => log.push(format!("seed {}: {}", rel, e)),
            }
        }

        for cmd in &cfg.setup_commands {
            let r = self.shell(cmd).await;
            log.push(format!(
                "setup `{}`: {}",
                cmd,
                if r.ok { "ok" } else { r.error.as_str() }
            ));
        }

        log
    }

    fn resolve(&self, rel: &str) -> Result<PathBuf, String> {
        if rel.is_empty() || rel == "." {
            return Ok(self.scratch_dir.clone());
        }
        let p = StdPath::new(rel);
        if p.is_absolute() {
            return Err(format!("absolute paths not allowed: {}", rel));
        }
        let candidate = self.scratch_dir.join(p);
        // Manual normalization: resolve any `..` segments without requiring
        // the target to exist (std::fs::canonicalize would fail for not-yet-
        // written files).
        let mut normalized = PathBuf::new();
        for component in candidate.components() {
            match component {
                std::path::Component::ParentDir => {
                    if !normalized.pop() {
                        return Err(format!("path escapes sandbox root: {}", rel));
                    }
                }
                std::path::Component::CurDir => {}
                other => normalized.push(other.as_os_str()),
            }
        }
        if !normalized.starts_with(&self.scratch_dir) {
            return Err(format!("path escapes sandbox root: {}", rel));
        }
        Ok(normalized)
    }

    fn truncate(&self, mut s: String) -> String {
        if s.len() <= self.max_output_bytes {
            return s;
        }
        let extra = s.len() - self.max_output_bytes;
        s.truncate(self.max_output_bytes);
        s.push_str(&format!("\n... [truncated {} bytes]", extra));
        s
    }

    fn fs_read(&self, args: &serde_json::Value) -> SandboxCallResult {
        let rel = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return SandboxCallResult::err("fs_read requires 'path'"),
        };
        let path = match self.resolve(rel) {
            Ok(p) => p,
            Err(e) => return SandboxCallResult::err(&e),
        };
        if !path.exists() {
            return SandboxCallResult::err(&format!("no such file: {}", rel));
        }
        if !path.is_file() {
            return SandboxCallResult::err(&format!("not a file: {}", rel));
        }
        match std::fs::read_to_string(&path) {
            Ok(content) => SandboxCallResult::ok(self.truncate(content)),
            Err(e) => SandboxCallResult::err(&format!("read failed: {}", e)),
        }
    }

    fn fs_write(&self, args: &serde_json::Value) -> SandboxCallResult {
        let rel = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return SandboxCallResult::err("fs_write requires 'path'"),
        };
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let path = match self.resolve(rel) {
            Ok(p) => p,
            Err(e) => return SandboxCallResult::err(&e),
        };
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return SandboxCallResult::err(&format!("mkdir failed: {}", e));
            }
        }
        match std::fs::write(&path, content) {
            Ok(()) => SandboxCallResult::ok(format!(
                "wrote {} bytes to {}",
                content.len(),
                rel
            )),
            Err(e) => SandboxCallResult::err(&format!("write failed: {}", e)),
        }
    }

    fn fs_list(&self, args: &serde_json::Value) -> SandboxCallResult {
        let rel = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let path = match self.resolve(rel) {
            Ok(p) => p,
            Err(e) => return SandboxCallResult::err(&e),
        };
        if !path.exists() {
            return SandboxCallResult::err(&format!("no such directory: {}", rel));
        }
        if !path.is_dir() {
            return SandboxCallResult::err(&format!("not a directory: {}", rel));
        }
        let mut entries: Vec<String> = Vec::new();
        let read_dir = match std::fs::read_dir(&path) {
            Ok(r) => r,
            Err(e) => return SandboxCallResult::err(&format!("readdir failed: {}", e)),
        };
        for entry in read_dir.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let suffix = if entry.path().is_dir() { "/" } else { "" };
            entries.push(format!("{}{}", name, suffix));
        }
        entries.sort();
        SandboxCallResult::ok(entries.join("\n"))
    }

    async fn shell(&self, command: &str) -> SandboxCallResult {
        if command.is_empty() {
            return SandboxCallResult::err("empty command");
        }
        let fut = tokio::process::Command::new("bash")
            .arg("-c")
            .arg(command)
            .current_dir(&self.scratch_dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output();

        match tokio::time::timeout(self.command_timeout, fut).await {
            Ok(Ok(output)) => {
                let mut combined = String::from_utf8_lossy(&output.stdout).to_string();
                if !output.stderr.is_empty() {
                    combined.push_str("\n--- stderr ---\n");
                    combined.push_str(&String::from_utf8_lossy(&output.stderr));
                }
                let combined = self.truncate(combined);
                if output.status.success() {
                    SandboxCallResult::ok(combined)
                } else {
                    SandboxCallResult {
                        ok: false,
                        content: combined,
                        error: format!("exit {}", output.status.code().unwrap_or(-1)),
                    }
                }
            }
            Ok(Err(e)) => SandboxCallResult::err(&format!("subprocess error: {}", e)),
            Err(_) => SandboxCallResult::err(&format!(
                "command timed out after {}s",
                self.command_timeout.as_secs()
            )),
        }
    }
}

struct SandboxCallResult {
    ok: bool,
    content: String,
    error: String,
}

impl SandboxCallResult {
    fn ok(content: String) -> Self {
        Self { ok: true, content, error: String::new() }
    }
    fn err(msg: &str) -> Self {
        Self { ok: false, content: String::new(), error: msg.to_string() }
    }
    fn into_observation_content(self) -> String {
        if self.ok {
            if self.content.is_empty() {
                "[empty]".to_string()
            } else {
                self.content
            }
        } else if self.content.is_empty() {
            format!("[error] {}", self.error)
        } else {
            format!("[error] {}\n{}", self.error, self.content)
        }
    }
}

// ---------------------------------------------------------------------------
// HybridToolExecutor — dispatches tool calls to either the canned
// mock_response path or the RealSandbox based on `EvalTool.kind`.
// Records every call so the harness can inspect them post-run.
// ---------------------------------------------------------------------------

struct HybridToolExecutor {
    tools: Vec<EvalTool>,
    sandbox: Option<Arc<RealSandbox>>,
    recorded: Arc<tokio::sync::Mutex<Vec<ToolCallRecord>>>,
}

impl HybridToolExecutor {
    fn new(tools: Vec<EvalTool>, sandbox: Option<Arc<RealSandbox>>) -> Self {
        Self {
            tools,
            sandbox,
            recorded: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }

    fn recorded_handle(&self) -> Arc<tokio::sync::Mutex<Vec<ToolCallRecord>>> {
        self.recorded.clone()
    }

    fn lookup(&self, name: &str) -> Option<&EvalTool> {
        self.tools.iter().find(|t| t.name == name)
    }

    /// Execute a single tool call. Returns `Ok(content)` on success (mock
    /// or sandbox) or a non-empty string describing the error. Every call
    /// is recorded regardless of outcome.
    async fn execute_one(
        &self,
        tool_name: &str,
        arguments_raw: &str,
    ) -> String {
        let tool = match self.lookup(tool_name) {
            Some(t) => t,
            None => {
                let resp = format!("[error] unknown tool: {}", tool_name);
                self.record(tool_name, arguments_raw, &resp).await;
                return resp;
            }
        };

        // Mock kind: return the canned response verbatim.
        if tool.kind == "mock" {
            let resp = tool.mock_response.clone();
            self.record(tool_name, arguments_raw, &resp).await;
            return resp;
        }

        // Non-mock kind: dispatch through the sandbox.
        let sandbox = match &self.sandbox {
            Some(s) => s.clone(),
            None => {
                let resp = format!(
                    "[error] non-mock tool '{}' called but no sandbox available",
                    tool_name
                );
                self.record(tool_name, arguments_raw, &resp).await;
                return resp;
            }
        };

        // Parse arguments JSON once.
        let args_value: serde_json::Value =
            serde_json::from_str(arguments_raw).unwrap_or(serde_json::json!({}));

        let result = match tool.kind.as_str() {
            "fs_read" => sandbox.fs_read(&args_value),
            "fs_write" => sandbox.fs_write(&args_value),
            "fs_list" => sandbox.fs_list(&args_value),
            "shell" => {
                let cmd = args_value
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                sandbox.shell(&cmd).await
            }
            other => SandboxCallResult::err(&format!("unknown tool kind '{}'", other)),
        };

        let resp = result.into_observation_content();
        self.record(tool_name, arguments_raw, &resp).await;
        resp
    }

    async fn record(&self, name: &str, arguments: &str, response: &str) {
        self.recorded.lock().await.push(ToolCallRecord {
            name: name.to_string(),
            arguments: arguments.to_string(),
            response: response.to_string(),
        });
    }
}

#[async_trait]
impl ActionExecutor for HybridToolExecutor {
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

                let response = self.execute_one(name, arguments).await;
                // Heuristic: treat responses that start with "[error]"
                // as failures for circuit breaker bookkeeping, so repeated
                // misbehavior on the same tool opens the circuit.
                if response.starts_with("[error]") {
                    observations.push(Observation::tool_error(call_id.clone(), response));
                    circuit_breakers.record_failure(name).await;
                } else {
                    observations.push(Observation::tool_result(call_id.clone(), response));
                    circuit_breakers.record_success(name).await;
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

    let mut task: EvalTask = serde_json::from_str(&task_json)
        .map_err(|e| format!("Failed to parse task JSON: {}", e))?;

    // Anthropic enforces tool name regex `^[a-zA-Z0-9_-]{1,128}$`.
    // Sanitize tool names in-place; the harness on the Python side knows
    // to remap recorded calls back to the original names if needed.
    for tool in task.tools.iter_mut() {
        tool.name = sanitize_tool_name(&tool.name);
    }

    // Build inference provider from environment.
    let provider = CloudInferenceProvider::from_env().ok_or_else(|| {
        "No LLM provider configured. Set OPENAI_API_KEY (+ OPENAI_BASE_URL for Ollama), \
         OPENROUTER_API_KEY, or ANTHROPIC_API_KEY"
            .to_string()
    })?;

    // Build the per-task sandbox if the task declares any non-mock tool.
    // Stays None for the common mock-only case (zero overhead).
    let sandbox: Option<Arc<RealSandbox>> = if task
        .tools
        .iter()
        .any(|t| t.kind != "mock")
    {
        let cfg = task.sandbox.clone().unwrap_or_default();
        let sb = RealSandbox::new(&cfg).map_err(|e| {
            format!("failed to create sandbox scratch dir: {}", e)
        })?;
        let sb = Arc::new(sb);
        let setup_log = sb.run_setup(&cfg).await;
        for line in &setup_log {
            eprintln!("[sandbox setup] {}", line);
        }
        Some(sb)
    } else {
        None
    };

    // Build executor with (now-sanitized) tools and the optional sandbox.
    let executor = Arc::new(HybridToolExecutor::new(task.tools.clone(), sandbox));
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
        temperature: task.temperature,
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
