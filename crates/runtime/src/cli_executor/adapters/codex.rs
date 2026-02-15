//! Codex CLI adapter for the CLI executor
//!
//! Implements `AiCliAdapter` for OpenAI's Codex CLI tool,
//! using `exec --full-auto --json <prompt>` for non-interactive operation.

use std::collections::HashMap;
use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::cli_executor::adapter::{AiCliAdapter, CodeGenRequest, CodeGenResult};
use crate::cli_executor::executor::StdinStrategy;
use crate::sandbox::ExecutionResult;

/// Approval mode for Codex CLI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CodexApprovalMode {
    /// Automatically apply all changes without confirmation.
    FullAuto,
    /// Suggest changes but don't apply automatically.
    Suggest,
}

impl Default for CodexApprovalMode {
    fn default() -> Self {
        Self::FullAuto
    }
}

/// Adapter for OpenAI's Codex CLI tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexAdapter {
    /// Path or name of the Codex executable.
    pub executable_path: String,
    /// Model to use (e.g. "o4-mini", "o3").
    pub model: Option<String>,
    /// Approval mode (FullAuto or Suggest).
    pub approval_mode: CodexApprovalMode,
    /// Additional CLI arguments passed through to Codex.
    pub extra_args: Vec<String>,
}

impl Default for CodexAdapter {
    fn default() -> Self {
        Self {
            executable_path: "codex".to_string(),
            model: None,
            approval_mode: CodexApprovalMode::FullAuto,
            extra_args: Vec::new(),
        }
    }
}

#[async_trait]
impl AiCliAdapter for CodexAdapter {
    fn name(&self) -> &str {
        "codex"
    }

    fn executable(&self) -> &str {
        &self.executable_path
    }

    fn build_args(&self, request: &CodeGenRequest) -> Vec<String> {
        let mut args = vec!["exec".to_string()];

        match self.approval_mode {
            CodexApprovalMode::FullAuto => args.push("--full-auto".to_string()),
            CodexApprovalMode::Suggest => args.push("--suggest".to_string()),
        }

        args.push("--json".to_string());

        // Request-level model takes precedence over adapter default
        let model = request.model.as_ref().or(self.model.as_ref());
        if let Some(m) = model {
            args.push("--model".to_string());
            args.push(m.clone());
        }

        // Append extra args
        args.extend(self.extra_args.iter().cloned());

        // The prompt is the final positional argument
        args.push(request.prompt.clone());

        args
    }

    fn non_interactive_env(&self) -> HashMap<String, String> {
        // Codex `exec` subcommand is non-interactive;
        // base executor already sets CI=true etc.
        HashMap::new()
    }

    fn stdin_strategy(&self) -> StdinStrategy {
        // `exec` subcommand is non-interactive
        StdinStrategy::CloseImmediately
    }

    fn parse_output(&self, _request: &CodeGenRequest, result: ExecutionResult) -> CodeGenResult {
        // Codex --json emits newline-delimited JSON events.
        // Collect all events into a JSON array; look for file write events.
        let mut events: Vec<serde_json::Value> = Vec::new();
        let mut files_modified: Vec<PathBuf> = Vec::new();

        for line in result.stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(event) = serde_json::from_str::<serde_json::Value>(line) {
                // Look for file write events to populate files_modified
                if let Some(path) = extract_file_path(&event) {
                    if !files_modified.contains(&path) {
                        files_modified.push(path);
                    }
                }
                events.push(event);
            }
        }

        let parsed_output = if events.is_empty() {
            None
        } else {
            Some(serde_json::Value::Array(events))
        };

        let success = result.success;

        CodeGenResult {
            success,
            execution: result,
            parsed_output,
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
            .map_err(|e| anyhow::anyhow!("Codex not found at '{}': {}", self.executable_path, e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "Codex health check failed (exit {}): {}",
                output.status.code().unwrap_or(-1),
                stderr
            );
        }

        Ok(())
    }
}

/// Extract a file path from a Codex JSON event.
///
/// Codex events may contain file paths in several forms:
/// - `{"type": "file_write", "path": "..."}`
/// - `{"type": "patch", "path": "..."}`
/// - `{"path": "...", "action": "write"}`
fn extract_file_path(event: &serde_json::Value) -> Option<PathBuf> {
    let path_str = event.get("path").and_then(|v| v.as_str())?;

    let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let action = event.get("action").and_then(|v| v.as_str()).unwrap_or("");

    if matches!(event_type, "file_write" | "patch" | "write")
        || matches!(action, "write" | "patch" | "create")
    {
        Some(PathBuf::from(path_str))
    } else {
        None
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
        let adapter = CodexAdapter::default();
        let request = sample_request();
        let args = adapter.build_args(&request);

        assert_eq!(args[0], "exec");
        assert_eq!(args[1], "--full-auto");
        assert_eq!(args[2], "--json");
        assert_eq!(*args.last().unwrap(), "Fix the bug in main.rs");
    }

    #[test]
    fn test_build_args_suggest_mode() {
        let adapter = CodexAdapter {
            approval_mode: CodexApprovalMode::Suggest,
            ..Default::default()
        };
        let request = sample_request();
        let args = adapter.build_args(&request);

        assert_eq!(args[0], "exec");
        assert_eq!(args[1], "--suggest");
        assert!(!args.contains(&"--full-auto".to_string()));
    }

    #[test]
    fn test_build_args_model_override_from_request() {
        let adapter = CodexAdapter {
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
        let adapter = CodexAdapter {
            model: Some("adapter-model".to_string()),
            ..Default::default()
        };
        let request = sample_request();
        let args = adapter.build_args(&request);

        let idx = args.iter().position(|a| a == "--model").unwrap();
        assert_eq!(args[idx + 1], "adapter-model");
    }

    #[test]
    fn test_build_args_with_extra_args() {
        let adapter = CodexAdapter {
            extra_args: vec!["--verbose".to_string()],
            ..Default::default()
        };
        let request = sample_request();
        let args = adapter.build_args(&request);

        assert!(args.contains(&"--verbose".to_string()));
    }

    #[test]
    fn test_parse_output_valid_ndjson() {
        let adapter = CodexAdapter::default();
        let request = sample_request();

        let stdout = [
            r#"{"type":"file_write","path":"src/main.rs","content":"fn main() {}"}"#,
            r#"{"type":"message","text":"Done!"}"#,
            r#"{"type":"patch","path":"src/lib.rs","diff":"..."}"#,
        ]
        .join("\n");

        let result = ExecutionResult {
            exit_code: 0,
            stdout,
            stderr: String::new(),
            execution_time_ms: 3000,
            success: true,
            stdout_truncated: false,
            stderr_truncated: false,
        };

        let codegen = adapter.parse_output(&request, result);

        assert!(codegen.success);
        assert!(codegen.parsed_output.is_some());
        let events = codegen.parsed_output.unwrap();
        assert_eq!(events.as_array().unwrap().len(), 3);
        assert_eq!(codegen.files_modified.len(), 2);
        assert_eq!(codegen.files_modified[0], PathBuf::from("src/main.rs"));
        assert_eq!(codegen.files_modified[1], PathBuf::from("src/lib.rs"));
        assert_eq!(codegen.adapter_name, "codex");
    }

    #[test]
    fn test_parse_output_non_json() {
        let adapter = CodexAdapter::default();
        let request = sample_request();

        let result = ExecutionResult {
            exit_code: 0,
            stdout: "Not valid JSON output from codex\nJust plain text".to_string(),
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
    fn test_parse_output_empty() {
        let adapter = CodexAdapter::default();
        let request = sample_request();

        let result = ExecutionResult {
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
            execution_time_ms: 100,
            success: true,
            stdout_truncated: false,
            stderr_truncated: false,
        };

        let codegen = adapter.parse_output(&request, result);

        assert!(codegen.success);
        assert!(codegen.parsed_output.is_none());
        assert!(codegen.files_modified.is_empty());
    }

    #[test]
    fn test_parse_output_failure() {
        let adapter = CodexAdapter::default();
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
    fn test_parse_output_deduplicates_files() {
        let adapter = CodexAdapter::default();
        let request = sample_request();

        let stdout = [
            r#"{"type":"file_write","path":"src/main.rs","content":"v1"}"#,
            r#"{"type":"file_write","path":"src/main.rs","content":"v2"}"#,
        ]
        .join("\n");

        let result = ExecutionResult {
            exit_code: 0,
            stdout,
            stderr: String::new(),
            execution_time_ms: 1000,
            success: true,
            stdout_truncated: false,
            stderr_truncated: false,
        };

        let codegen = adapter.parse_output(&request, result);
        assert_eq!(codegen.files_modified.len(), 1);
    }

    #[test]
    fn test_stdin_strategy_is_close_immediately() {
        let adapter = CodexAdapter::default();
        assert!(matches!(
            adapter.stdin_strategy(),
            StdinStrategy::CloseImmediately
        ));
    }

    #[test]
    fn test_non_interactive_env() {
        let adapter = CodexAdapter::default();
        let env = adapter.non_interactive_env();
        // Codex doesn't need extra env vars beyond base executor defaults
        assert!(env.is_empty());
    }

    #[tokio::test]
    #[ignore] // Requires Codex to be installed
    async fn test_health_check() {
        let adapter = CodexAdapter::default();
        let result = adapter.health_check().await;
        let _ = result;
    }
}
