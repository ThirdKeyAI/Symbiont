//! Claude Code adapter for the CLI executor
//!
//! Implements `AiCliAdapter` for Anthropic's Claude Code CLI tool,
//! using `--print --output-format json` for non-interactive operation.

use std::collections::HashMap;
use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::cli_executor::adapter::{AiCliAdapter, CodeGenRequest, CodeGenResult};
use crate::cli_executor::executor::StdinStrategy;
use crate::sandbox::ExecutionResult;

/// Adapter for Anthropic's Claude Code CLI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeCodeAdapter {
    /// Path or name of the Claude Code executable.
    pub executable_path: String,
    /// Maximum number of agentic turns before stopping. This is the *primary*
    /// cooperative bound for managed runs — Claude Code exits cleanly with its
    /// JSON result when the limit is hit (vs. the wall-clock timeout, which is a
    /// hard backstop).
    pub max_turns: Option<u32>,
    /// Model to use (e.g. "claude-sonnet-4-5-20250929").
    pub model: Option<String>,
    /// Tools explicitly allowed for this invocation.
    pub allowed_tools: Vec<String>,
    /// Tools explicitly disallowed for this invocation.
    pub disallowed_tools: Vec<String>,

    // ── Mode B (governed subprocess) wiring ───────────────────────────────
    /// Local plugin directories to load (`--plugin-dir`, repeatable). For Mode B
    /// this points at the symbi-claude-code plugin so its hooks fire and defer.
    #[serde(default)]
    pub plugin_dirs: Vec<String>,
    /// MCP config passed via `--mcp-config` (a file path or inline JSON).
    #[serde(default)]
    pub mcp_config: Option<String>,
    /// Emit `--strict-mcp-config` so only `mcp_config` is used.
    #[serde(default)]
    pub strict_mcp_config: bool,
    /// Emit `--bare` to skip auto-discovery (plugins/MCP load only via flags).
    #[serde(default)]
    pub bare: bool,
    /// Permission mode for unattended runs (`--permission-mode`, e.g. "dontAsk").
    #[serde(default)]
    pub permission_mode: Option<String>,
    /// Extra system prompt appended via `--append-system-prompt`.
    #[serde(default)]
    pub append_system_prompt: Option<String>,
    /// Emit `--output-format stream-json` instead of `json`.
    ///
    /// The child then writes one JSON event per line as it works, so a caller
    /// with a line sink can record its tool calls while the run is still in
    /// flight. `parse_output` folds the stream back into the same shape the
    /// `json` format produces, so consumers see no difference.
    #[serde(default)]
    pub stream_json: bool,

    // ── Mode B env handshake (emitted via non_interactive_env) ────────────
    /// When true, set `SYMBIONT_MANAGED=true` so the plugin defers enforcement.
    #[serde(default)]
    pub managed: bool,
    /// Correlation id exported as `SYMBIONT_SESSION_ID`.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Token budget exported as `SYMBIONT_BUDGET_TOKENS` (awareness only today).
    #[serde(default)]
    pub budget_tokens: Option<u64>,
    /// Time budget (seconds) exported as `SYMBIONT_BUDGET_TIMEOUT`.
    #[serde(default)]
    pub budget_timeout_secs: Option<u64>,
    /// Project dir exported as `CLAUDE_PROJECT_DIR` so hooks find `.symbiont/`.
    #[serde(default)]
    pub project_dir: Option<String>,
}

/// Pull the terminal `result` event out of a `stream-json` stdout stream.
///
/// The stream is newline-delimited JSON: `system` events, then `assistant` /
/// `user` turns, then exactly one `{"type":"result", ...}` carrying the final
/// text, token usage and turn count. That last event is the same object the
/// `json` output format emits on its own, so lifting it out keeps every
/// consumer of `parsed_output` working across both formats.
///
/// Returns `None` when no terminal event is present — a killed or timed-out
/// child never writes one, and reporting that honestly is better than
/// synthesising a result the run never reached. Scans from the end so a run
/// whose output was truncated mid-stream still finds the newest complete
/// event, and tolerates unparseable lines rather than giving up on the stream.
fn terminal_result_event(stdout: &str) -> Option<serde_json::Value> {
    stdout.lines().rev().find_map(|line| {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let value = serde_json::from_str::<serde_json::Value>(line).ok()?;
        (value.get("type").and_then(|t| t.as_str()) == Some("result")).then_some(value)
    })
}

impl Default for ClaudeCodeAdapter {
    fn default() -> Self {
        Self {
            executable_path: "claude".to_string(),
            max_turns: None,
            model: None,
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            plugin_dirs: Vec::new(),
            mcp_config: None,
            strict_mcp_config: false,
            bare: false,
            permission_mode: None,
            append_system_prompt: None,
            stream_json: false,
            managed: false,
            session_id: None,
            budget_tokens: None,
            budget_timeout_secs: None,
            project_dir: None,
        }
    }
}

#[async_trait]
impl AiCliAdapter for ClaudeCodeAdapter {
    fn name(&self) -> &str {
        "claude-code"
    }

    fn executable(&self) -> &str {
        &self.executable_path
    }

    fn build_args(&self, request: &CodeGenRequest) -> Vec<String> {
        let mut args = vec!["--print".to_string(), "--output-format".to_string()];
        if self.stream_json {
            args.push("stream-json".to_string());
            // stream-json in --print mode emits nothing without --verbose.
            args.push("--verbose".to_string());
        } else {
            args.push("json".to_string());
        }

        if self.bare {
            args.push("--bare".to_string());
        }

        for dir in &self.plugin_dirs {
            args.push("--plugin-dir".to_string());
            args.push(dir.clone());
        }

        if let Some(ref cfg) = self.mcp_config {
            args.push("--mcp-config".to_string());
            args.push(cfg.clone());
        }
        if self.strict_mcp_config {
            args.push("--strict-mcp-config".to_string());
        }

        if let Some(ref mode) = self.permission_mode {
            args.push("--permission-mode".to_string());
            args.push(mode.clone());
        }

        if let Some(turns) = self.max_turns {
            args.push("--max-turns".to_string());
            args.push(turns.to_string());
        }

        // Request-level model takes precedence over adapter default
        let model = request.model.as_ref().or(self.model.as_ref());
        if let Some(m) = model {
            args.push("--model".to_string());
            args.push(m.clone());
        }

        if !self.allowed_tools.is_empty() {
            args.push("--allowedTools".to_string());
            args.push(self.allowed_tools.join(","));
        }

        if !self.disallowed_tools.is_empty() {
            args.push("--disallowedTools".to_string());
            args.push(self.disallowed_tools.join(","));
        }

        if let Some(ref ctx) = request.system_context {
            args.push("--system-prompt".to_string());
            args.push(ctx.clone());
        }

        if let Some(ref extra) = self.append_system_prompt {
            args.push("--append-system-prompt".to_string());
            args.push(extra.clone());
        }

        // The prompt is the final positional argument
        args.push(request.prompt.clone());

        args
    }

    fn non_interactive_env(&self) -> HashMap<String, String> {
        let mut env = HashMap::new();
        env.insert("CI".to_string(), "true".to_string());

        // Mode B handshake. These are injected here (the executor merges
        // adapter env *unfiltered*, unlike the caller-env ENV_ALLOWLIST) so the
        // symbi-claude-code plugin detects managed mode and defers enforcement
        // to the outer Gate.
        if self.managed {
            env.insert("SYMBIONT_MANAGED".to_string(), "true".to_string());
        }
        if let Some(ref id) = self.session_id {
            env.insert("SYMBIONT_SESSION_ID".to_string(), id.clone());
        }
        if let Some(tokens) = self.budget_tokens {
            env.insert("SYMBIONT_BUDGET_TOKENS".to_string(), tokens.to_string());
        }
        if let Some(secs) = self.budget_timeout_secs {
            env.insert("SYMBIONT_BUDGET_TIMEOUT".to_string(), secs.to_string());
        }
        if let Some(ref dir) = self.project_dir {
            env.insert("CLAUDE_PROJECT_DIR".to_string(), dir.clone());
        }

        env
    }

    fn stdin_strategy(&self) -> StdinStrategy {
        // --print mode doesn't read stdin
        StdinStrategy::CloseImmediately
    }

    fn parse_output(&self, _request: &CodeGenRequest, result: ExecutionResult) -> CodeGenResult {
        let parsed = if self.stream_json {
            terminal_result_event(&result.stdout)
        } else {
            serde_json::from_str::<serde_json::Value>(&result.stdout).ok()
        };

        let files_modified = parsed
            .as_ref()
            .and_then(|v| v.get("files_modified"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(PathBuf::from))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let success = result.success;

        CodeGenResult {
            success,
            execution: result,
            parsed_output: parsed,
            files_modified,
            adapter_name: self.name().to_string(),
        }
    }

    async fn health_check(&self) -> Result<(), anyhow::Error> {
        let output = tokio::process::Command::new(&self.executable_path)
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .map_err(|e| {
                anyhow::anyhow!("Claude Code not found at '{}': {}", self.executable_path, e)
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "Claude Code health check failed (exit {}): {}",
                output.status.code().unwrap_or(-1),
                stderr
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> CodeGenRequest {
        CodeGenRequest {
            prompt: "Fix the bug in main.rs".to_string(),
            working_dir: PathBuf::from("/tmp/project"),
            target_files: vec![PathBuf::from("src/main.rs")],
            system_context: None,
            model: None,
            options: HashMap::new(),
        }
    }

    #[test]
    fn test_build_args_basic() {
        let adapter = ClaudeCodeAdapter::default();
        let request = sample_request();
        let args = adapter.build_args(&request);

        assert_eq!(args[0], "--print");
        assert_eq!(args[1], "--output-format");
        assert_eq!(args[2], "json");
        assert_eq!(*args.last().unwrap(), "Fix the bug in main.rs");
    }

    #[test]
    fn test_build_args_with_max_turns() {
        let adapter = ClaudeCodeAdapter {
            max_turns: Some(5),
            ..Default::default()
        };
        let request = sample_request();
        let args = adapter.build_args(&request);

        let idx = args.iter().position(|a| a == "--max-turns").unwrap();
        assert_eq!(args[idx + 1], "5");
    }

    #[test]
    fn test_build_args_model_override_from_request() {
        let adapter = ClaudeCodeAdapter {
            model: Some("default-model".to_string()),
            ..Default::default()
        };
        let mut request = sample_request();
        request.model = Some("request-model".to_string());
        let args = adapter.build_args(&request);

        let idx = args.iter().position(|a| a == "--model").unwrap();
        assert_eq!(args[idx + 1], "request-model");
    }

    #[test]
    fn test_build_args_model_from_adapter() {
        let adapter = ClaudeCodeAdapter {
            model: Some("adapter-model".to_string()),
            ..Default::default()
        };
        let request = sample_request();
        let args = adapter.build_args(&request);

        let idx = args.iter().position(|a| a == "--model").unwrap();
        assert_eq!(args[idx + 1], "adapter-model");
    }

    #[test]
    fn test_build_args_with_system_context() {
        let adapter = ClaudeCodeAdapter::default();
        let mut request = sample_request();
        request.system_context = Some("You are a Rust expert".to_string());
        let args = adapter.build_args(&request);

        let idx = args.iter().position(|a| a == "--system-prompt").unwrap();
        assert_eq!(args[idx + 1], "You are a Rust expert");
    }

    #[test]
    fn test_build_args_with_allowed_tools() {
        let adapter = ClaudeCodeAdapter {
            allowed_tools: vec!["Read".to_string(), "Write".to_string()],
            ..Default::default()
        };
        let request = sample_request();
        let args = adapter.build_args(&request);

        let idx = args.iter().position(|a| a == "--allowedTools").unwrap();
        assert_eq!(args[idx + 1], "Read,Write");
    }

    #[test]
    fn test_parse_output_valid_json() {
        let adapter = ClaudeCodeAdapter::default();
        let request = sample_request();

        let result = ExecutionResult {
            exit_code: 0,
            stdout: r#"{"result":"success","files_modified":["src/main.rs","src/lib.rs"]}"#
                .to_string(),
            stderr: String::new(),
            execution_time_ms: 1000,
            success: true,
            stdout_truncated: false,
            stderr_truncated: false,
        };

        let codegen = adapter.parse_output(&request, result);

        assert!(codegen.success);
        assert!(codegen.parsed_output.is_some());
        assert_eq!(codegen.files_modified.len(), 2);
        assert_eq!(codegen.files_modified[0], PathBuf::from("src/main.rs"));
        assert_eq!(codegen.files_modified[1], PathBuf::from("src/lib.rs"));
        assert_eq!(codegen.adapter_name, "claude-code");
    }

    #[test]
    fn test_parse_output_invalid_json() {
        let adapter = ClaudeCodeAdapter::default();
        let request = sample_request();

        let result = ExecutionResult {
            exit_code: 0,
            stdout: "Not valid JSON output from claude".to_string(),
            stderr: String::new(),
            execution_time_ms: 500,
            success: true,
            stdout_truncated: false,
            stderr_truncated: false,
        };

        let codegen = adapter.parse_output(&request, result);

        // Should degrade gracefully
        assert!(codegen.success);
        assert!(codegen.parsed_output.is_none());
        assert!(codegen.files_modified.is_empty());
    }

    #[test]
    fn test_parse_output_failure() {
        let adapter = ClaudeCodeAdapter::default();
        let request = sample_request();

        let result = ExecutionResult {
            exit_code: 1,
            stdout: String::new(),
            stderr: "Error: something went wrong".to_string(),
            execution_time_ms: 200,
            success: false,
            stdout_truncated: false,
            stderr_truncated: false,
        };

        let codegen = adapter.parse_output(&request, result);
        assert!(!codegen.success);
    }

    #[test]
    fn test_stdin_strategy_is_close_immediately() {
        let adapter = ClaudeCodeAdapter::default();
        assert!(matches!(
            adapter.stdin_strategy(),
            StdinStrategy::CloseImmediately
        ));
    }

    #[test]
    fn test_non_interactive_env() {
        let adapter = ClaudeCodeAdapter::default();
        let env = adapter.non_interactive_env();
        assert_eq!(env.get("CI"), Some(&"true".to_string()));
        // Unmanaged adapter emits no handshake vars.
        assert!(!env.contains_key("SYMBIONT_MANAGED"));
    }

    #[test]
    fn test_build_args_mode_b_flags() {
        let adapter = ClaudeCodeAdapter {
            bare: true,
            plugin_dirs: vec!["/plugins/symbi-claude-code".to_string()],
            mcp_config: Some("/tmp/mcp.json".to_string()),
            strict_mcp_config: true,
            permission_mode: Some("dontAsk".to_string()),
            max_turns: Some(12),
            append_system_prompt: Some("Review rules.".to_string()),
            ..Default::default()
        };
        let args = adapter.build_args(&sample_request());

        assert!(args.contains(&"--bare".to_string()));
        let pd = args.iter().position(|a| a == "--plugin-dir").unwrap();
        assert_eq!(args[pd + 1], "/plugins/symbi-claude-code");
        let mc = args.iter().position(|a| a == "--mcp-config").unwrap();
        assert_eq!(args[mc + 1], "/tmp/mcp.json");
        assert!(args.contains(&"--strict-mcp-config".to_string()));
        let pm = args.iter().position(|a| a == "--permission-mode").unwrap();
        assert_eq!(args[pm + 1], "dontAsk");
        let asp = args
            .iter()
            .position(|a| a == "--append-system-prompt")
            .unwrap();
        assert_eq!(args[asp + 1], "Review rules.");
        // Prompt stays the final positional arg.
        assert_eq!(*args.last().unwrap(), "Fix the bug in main.rs");
    }

    /// `permission_mode` is opt-in: Mode B passes through whatever the agent
    /// declares in its `metadata { ... }` block, and an agent that declares
    /// nothing must not silently inherit `dontAsk` for the whole session.
    #[test]
    fn omits_permission_mode_flag_when_unset() {
        let adapter = ClaudeCodeAdapter {
            permission_mode: None,
            ..Default::default()
        };
        let args = adapter.build_args(&sample_request());
        assert!(
            !args.contains(&"--permission-mode".to_string()),
            "unset permission_mode must omit the flag so the child keeps its \
             own prompting default, got: {args:?}"
        );
        assert!(
            !args.iter().any(|a| a == "dontAsk"),
            "unset permission_mode must never yield dontAsk, got: {args:?}"
        );
    }

    /// stream-json emits nothing in --print mode without --verbose, so the two
    /// flags have to travel together.
    #[test]
    fn stream_json_requests_verbose_output() {
        let adapter = ClaudeCodeAdapter {
            stream_json: true,
            ..Default::default()
        };
        let args = adapter.build_args(&sample_request());
        let of = args.iter().position(|a| a == "--output-format").unwrap();
        assert_eq!(args[of + 1], "stream-json");
        assert!(
            args.contains(&"--verbose".to_string()),
            "stream-json without --verbose produces no events, got: {args:?}"
        );
    }

    #[test]
    fn default_output_format_stays_json() {
        let args = ClaudeCodeAdapter::default().build_args(&sample_request());
        let of = args.iter().position(|a| a == "--output-format").unwrap();
        assert_eq!(args[of + 1], "json");
        assert!(!args.contains(&"--verbose".to_string()));
    }

    /// Shape taken from a real `--output-format stream-json` run: system events,
    /// an assistant turn carrying a tool_use, a user turn carrying the result,
    /// then one terminal `result` event.
    const STREAM_FIXTURE: &str = concat!(
        r#"{"type":"system","subtype":"init","session_id":"s1"}"#,
        "\n",
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/x"}}]}}"#,
        "\n",
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}"#,
        "\n",
        r#"{"type":"result","subtype":"success","is_error":false,"num_turns":2,"result":"done"}"#,
        "\n",
    );

    #[test]
    fn terminal_result_event_is_lifted_from_the_stream() {
        let ev = terminal_result_event(STREAM_FIXTURE).expect("fixture ends with a result event");
        assert_eq!(ev["subtype"], "success");
        assert_eq!(ev["result"], "done");
        assert_eq!(ev["num_turns"], 2);
    }

    /// A killed or timed-out child never writes a terminal event. Reporting
    /// that honestly beats synthesising a result the run never reached.
    #[test]
    fn terminal_result_event_is_none_when_the_run_did_not_finish() {
        let truncated = STREAM_FIXTURE
            .lines()
            .take(2)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(terminal_result_event(&truncated).is_none());
    }

    /// Non-JSON chatter on stdout must not abort the scan.
    #[test]
    fn terminal_result_event_skips_unparseable_lines() {
        let noisy = format!("{STREAM_FIXTURE}\nnot json at all\n");
        let ev = terminal_result_event(&noisy).expect("result event still reachable past noise");
        assert_eq!(ev["subtype"], "success");
    }

    /// parse_output must fold the stream back to the same shape the `json`
    /// format returns, so nothing downstream has to know which was used.
    #[test]
    fn parse_output_normalises_stream_json_to_the_result_event() {
        let adapter = ClaudeCodeAdapter {
            stream_json: true,
            ..Default::default()
        };
        let exec = ExecutionResult {
            exit_code: 0,
            stdout: STREAM_FIXTURE.to_string(),
            stderr: String::new(),
            execution_time_ms: 1000,
            success: true,
            stdout_truncated: false,
            stderr_truncated: false,
        };
        let out = adapter.parse_output(&sample_request(), exec);
        let parsed = out.parsed_output.expect("stream-json must still parse");
        assert_eq!(parsed["result"], "done");
    }

    #[test]
    fn test_mode_b_env_handshake() {
        let adapter = ClaudeCodeAdapter {
            managed: true,
            session_id: Some("sess-123".to_string()),
            budget_tokens: Some(100_000),
            budget_timeout_secs: Some(900),
            project_dir: Some("/work/target".to_string()),
            ..Default::default()
        };
        let env = adapter.non_interactive_env();
        assert_eq!(env.get("SYMBIONT_MANAGED"), Some(&"true".to_string()));
        assert_eq!(
            env.get("SYMBIONT_SESSION_ID"),
            Some(&"sess-123".to_string())
        );
        assert_eq!(
            env.get("SYMBIONT_BUDGET_TOKENS"),
            Some(&"100000".to_string())
        );
        assert_eq!(env.get("SYMBIONT_BUDGET_TIMEOUT"), Some(&"900".to_string()));
        assert_eq!(
            env.get("CLAUDE_PROJECT_DIR"),
            Some(&"/work/target".to_string())
        );
    }

    #[tokio::test]
    #[ignore] // Requires Claude Code to be installed
    async fn test_health_check() {
        let adapter = ClaudeCodeAdapter::default();
        let result = adapter.health_check().await;
        // Only check that it doesn't panic; result depends on installation
        let _ = result;
    }
}
