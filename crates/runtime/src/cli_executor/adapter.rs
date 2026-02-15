//! AI CLI adapter trait and request/response types
//!
//! Defines the `AiCliAdapter` trait that per-tool adapters implement to
//! translate a `CodeGenRequest` into CLI arguments, environment variables,
//! and stdin handling, then parse output back into a `CodeGenResult`.

use std::collections::HashMap;
use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::sandbox::ExecutionResult;

use super::executor::StdinStrategy;

/// Request describing what code generation work to perform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeGenRequest {
    /// The prompt or task description to send to the AI CLI tool.
    pub prompt: String,
    /// Working directory for the tool to operate in.
    pub working_dir: PathBuf,
    /// Specific files the tool should focus on.
    pub target_files: Vec<PathBuf>,
    /// Optional system-level context or instructions.
    pub system_context: Option<String>,
    /// Optional model override (e.g. "claude-sonnet-4-5-20250929").
    pub model: Option<String>,
    /// Adapter-specific key-value options.
    pub options: HashMap<String, String>,
}

/// Result of an AI CLI tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeGenResult {
    /// Whether the tool completed successfully.
    pub success: bool,
    /// Raw execution result (exit code, stdout, stderr, timing).
    pub execution: ExecutionResult,
    /// Structured output parsed from stdout, if available.
    pub parsed_output: Option<serde_json::Value>,
    /// Files that were modified by the tool.
    pub files_modified: Vec<PathBuf>,
    /// Name of the adapter that produced this result.
    pub adapter_name: String,
}

/// Trait that per-tool adapters implement to integrate AI CLI tools.
///
/// Each adapter knows how to build CLI arguments, set environment
/// variables for non-interactive mode, choose a stdin strategy, and
/// parse the tool's output into structured results.
#[async_trait]
pub trait AiCliAdapter: Send + Sync {
    /// Human-readable name of this adapter (e.g. "claude-code").
    fn name(&self) -> &str;

    /// Path or name of the executable to invoke.
    fn executable(&self) -> &str;

    /// Build command-line arguments for the given request.
    fn build_args(&self, request: &CodeGenRequest) -> Vec<String>;

    /// Environment variables that force non-interactive mode.
    fn non_interactive_env(&self) -> HashMap<String, String>;

    /// Stdin handling strategy for this tool.
    fn stdin_strategy(&self) -> StdinStrategy;

    /// Parse raw execution output into a structured `CodeGenResult`.
    fn parse_output(&self, request: &CodeGenRequest, result: ExecutionResult) -> CodeGenResult;

    /// Verify the tool is installed and reachable.
    async fn health_check(&self) -> Result<(), anyhow::Error>;
}
