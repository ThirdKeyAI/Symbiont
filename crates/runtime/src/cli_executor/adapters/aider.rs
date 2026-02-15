//! Aider adapter for the CLI executor
//!
//! Implements `AiCliAdapter` for the Aider CLI tool,
//! using `--yes-always --no-auto-commits --message <prompt>` for non-interactive operation.

use std::collections::HashMap;
use std::path::PathBuf;

use async_trait::async_trait;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::cli_executor::adapter::{AiCliAdapter, CodeGenRequest, CodeGenResult};
use crate::cli_executor::executor::StdinStrategy;
use crate::sandbox::ExecutionResult;

/// Adapter for the Aider CLI tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiderAdapter {
    /// Path or name of the Aider executable.
    pub executable_path: String,
    /// Model to use (e.g. "gpt-4o", "claude-sonnet-4-20250514").
    pub model: Option<String>,
    /// Whether Aider should auto-commit changes (default false).
    pub auto_commits: bool,
    /// Additional CLI arguments passed through to Aider.
    pub extra_args: Vec<String>,
}

impl Default for AiderAdapter {
    fn default() -> Self {
        Self {
            executable_path: "aider".to_string(),
            model: None,
            auto_commits: false,
            extra_args: Vec::new(),
        }
    }
}

#[async_trait]
impl AiCliAdapter for AiderAdapter {
    fn name(&self) -> &str {
        "aider"
    }

    fn executable(&self) -> &str {
        &self.executable_path
    }

    fn build_args(&self, request: &CodeGenRequest) -> Vec<String> {
        let mut args = vec!["--yes-always".to_string()];

        if !self.auto_commits {
            args.push("--no-auto-commits".to_string());
        }

        // Request-level model takes precedence over adapter default
        let model = request.model.as_ref().or(self.model.as_ref());
        if let Some(m) = model {
            args.push("--model".to_string());
            args.push(m.clone());
        }

        args.push("--message".to_string());
        args.push(request.prompt.clone());

        // Append extra args
        args.extend(self.extra_args.iter().cloned());

        // Target files as trailing positional arguments
        for file in &request.target_files {
            args.push(file.display().to_string());
        }

        args
    }

    fn non_interactive_env(&self) -> HashMap<String, String> {
        let mut env = HashMap::new();
        env.insert("AIDER_YES_ALWAYS".to_string(), "1".to_string());
        env
    }

    fn stdin_strategy(&self) -> StdinStrategy {
        // --yes-always prevents interactive prompts
        StdinStrategy::CloseImmediately
    }

    fn parse_output(&self, _request: &CodeGenRequest, result: ExecutionResult) -> CodeGenResult {
        // Aider outputs unstructured text. Scan for "Applied edit to <file>" lines.
        let re = Regex::new(r"Applied edit to (.+)").unwrap();
        let files_modified: Vec<PathBuf> = re
            .captures_iter(&result.stdout)
            .filter_map(|cap| cap.get(1).map(|m| PathBuf::from(m.as_str().trim())))
            .collect();

        let success = result.success;

        CodeGenResult {
            success,
            execution: result,
            parsed_output: None, // Aider doesn't produce structured JSON
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
            .map_err(|e| anyhow::anyhow!("Aider not found at '{}': {}", self.executable_path, e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "Aider health check failed (exit {}): {}",
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
        let adapter = AiderAdapter::default();
        let request = sample_request();
        let args = adapter.build_args(&request);

        assert_eq!(args[0], "--yes-always");
        assert_eq!(args[1], "--no-auto-commits");
        let msg_idx = args.iter().position(|a| a == "--message").unwrap();
        assert_eq!(args[msg_idx + 1], "Fix the bug in main.rs");
        // Target file is last
        assert_eq!(*args.last().unwrap(), "src/main.rs");
    }

    #[test]
    fn test_build_args_model_override_from_request() {
        let adapter = AiderAdapter {
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
        let adapter = AiderAdapter {
            model: Some("adapter-model".to_string()),
            ..Default::default()
        };
        let request = sample_request();
        let args = adapter.build_args(&request);

        let idx = args.iter().position(|a| a == "--model").unwrap();
        assert_eq!(args[idx + 1], "adapter-model");
    }

    #[test]
    fn test_build_args_with_target_files() {
        let adapter = AiderAdapter::default();
        let mut request = sample_request();
        request.target_files = vec![PathBuf::from("src/main.rs"), PathBuf::from("src/lib.rs")];
        let args = adapter.build_args(&request);

        // Target files should be the last two args
        let len = args.len();
        assert_eq!(args[len - 2], "src/main.rs");
        assert_eq!(args[len - 1], "src/lib.rs");
    }

    #[test]
    fn test_build_args_with_auto_commits() {
        let adapter = AiderAdapter {
            auto_commits: true,
            ..Default::default()
        };
        let request = sample_request();
        let args = adapter.build_args(&request);

        assert!(!args.contains(&"--no-auto-commits".to_string()));
    }

    #[test]
    fn test_build_args_with_extra_args() {
        let adapter = AiderAdapter {
            extra_args: vec!["--no-git".to_string(), "--dark-mode".to_string()],
            ..Default::default()
        };
        let request = sample_request();
        let args = adapter.build_args(&request);

        assert!(args.contains(&"--no-git".to_string()));
        assert!(args.contains(&"--dark-mode".to_string()));
    }

    #[test]
    fn test_parse_output_with_applied_edits() {
        let adapter = AiderAdapter::default();
        let request = sample_request();

        let result = ExecutionResult {
            exit_code: 0,
            stdout: [
                "Aider v0.50.0",
                "Model: gpt-4o",
                "Applied edit to src/main.rs",
                "Applied edit to src/lib.rs",
                "Tokens: 1234 sent, 567 received",
            ]
            .join("\n"),
            stderr: String::new(),
            execution_time_ms: 5000,
            success: true,
            stdout_truncated: false,
            stderr_truncated: false,
        };

        let codegen = adapter.parse_output(&request, result);

        assert!(codegen.success);
        assert!(codegen.parsed_output.is_none());
        assert_eq!(codegen.files_modified.len(), 2);
        assert_eq!(codegen.files_modified[0], PathBuf::from("src/main.rs"));
        assert_eq!(codegen.files_modified[1], PathBuf::from("src/lib.rs"));
        assert_eq!(codegen.adapter_name, "aider");
    }

    #[test]
    fn test_parse_output_no_edits() {
        let adapter = AiderAdapter::default();
        let request = sample_request();

        let result = ExecutionResult {
            exit_code: 0,
            stdout: "Aider v0.50.0\nNo changes made.\n".to_string(),
            stderr: String::new(),
            execution_time_ms: 2000,
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
        let adapter = AiderAdapter::default();
        let request = sample_request();

        let result = ExecutionResult {
            exit_code: 1,
            stdout: String::new(),
            stderr: "Error: API key not set".to_string(),
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
        let adapter = AiderAdapter::default();
        assert!(matches!(
            adapter.stdin_strategy(),
            StdinStrategy::CloseImmediately
        ));
    }

    #[test]
    fn test_non_interactive_env() {
        let adapter = AiderAdapter::default();
        let env = adapter.non_interactive_env();
        assert_eq!(env.get("AIDER_YES_ALWAYS"), Some(&"1".to_string()));
    }

    #[tokio::test]
    #[ignore] // Requires Aider to be installed
    async fn test_health_check() {
        let adapter = AiderAdapter::default();
        let result = adapter.health_check().await;
        let _ = result;
    }
}
