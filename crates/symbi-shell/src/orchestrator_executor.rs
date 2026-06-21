use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use symbi_runtime::reasoning::circuit_breaker::CircuitBreakerRegistry;
use symbi_runtime::reasoning::executor::ActionExecutor;
use symbi_runtime::reasoning::inference::ToolDefinition;
use symbi_runtime::reasoning::loop_types::{LoopConfig, Observation, ProposedAction};

use crate::validation;
use crate::validation::constraints::ProjectConstraints;

/// Action executor for the orchestrator agent.
///
/// Handles tool calls for artifact validation and agent management.
/// The policy gate has already approved each action before it reaches here.
pub struct OrchestratorExecutor {
    constraints: Arc<ProjectConstraints>,
    engine: Arc<repl_core::ReplEngine>,
    bridge: Arc<repl_core::RuntimeBridge>,
    cards: Arc<tokio::sync::RwLock<Vec<crate::agents::AgentCard>>>,
    /// When false (the default), the `shell` tool is neither advertised nor
    /// executable. Enabled only via the shell's `--allow-shell` flag.
    allow_shell: bool,
}

impl OrchestratorExecutor {
    pub fn new(
        constraints: Arc<ProjectConstraints>,
        engine: Arc<repl_core::ReplEngine>,
        bridge: Arc<repl_core::RuntimeBridge>,
        cards: Arc<tokio::sync::RwLock<Vec<crate::agents::AgentCard>>>,
        allow_shell: bool,
    ) -> Self {
        Self {
            constraints,
            engine,
            bridge,
            cards,
            allow_shell,
        }
    }
}

#[async_trait]
impl ActionExecutor for OrchestratorExecutor {
    async fn execute_actions(
        &self,
        actions: &[ProposedAction],
        _config: &LoopConfig,
        _circuit_breakers: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        let mut observations = Vec::new();

        for action in actions {
            match action {
                ProposedAction::ToolCall {
                    call_id,
                    name,
                    arguments,
                } => {
                    let result = self.handle_tool_call(name, arguments).await;
                    let is_error = result.is_err();
                    observations.push(Observation {
                        source: name.clone(),
                        content: result.unwrap_or_else(|e| format!("Error: {}", e)),
                        is_error,
                        call_id: Some(call_id.clone()),
                        metadata: HashMap::new(),
                    });
                }
                ProposedAction::Respond { .. }
                | ProposedAction::Terminate { .. }
                | ProposedAction::Delegate { .. } => {
                    // These are handled by the loop runner, not the executor
                }
            }
        }

        observations
    }

    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        let mut defs = vec![
            ToolDefinition {
                name: "list_agents".to_string(),
                description: "List all running agents with their state".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
            ToolDefinition {
                name: "validate_dsl".to_string(),
                description: "Validate a Symbiont DSL artifact against project constraints. Use this before presenting generated DSL to the user.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "dsl_code": {
                            "type": "string",
                            "description": "The DSL code to validate"
                        }
                    },
                    "required": ["dsl_code"]
                }),
            },
            ToolDefinition {
                name: "validate_cedar".to_string(),
                description: "Validate a Cedar policy against project constraints. Use this before presenting generated policies to the user.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "cedar_policy": {
                            "type": "string",
                            "description": "The Cedar policy text to validate"
                        }
                    },
                    "required": ["cedar_policy"]
                }),
            },
            ToolDefinition {
                name: "validate_toolclad".to_string(),
                description: "Validate a ToolClad TOML manifest against project constraints. Use this before presenting generated manifests to the user.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "toml_manifest": {
                            "type": "string",
                            "description": "The ToolClad TOML manifest to validate"
                        }
                    },
                    "required": ["toml_manifest"]
                }),
            },
            ToolDefinition {
                name: "save_artifact".to_string(),
                description: "Save a validated artifact to disk. Only call this AFTER the user explicitly approves the artifact. The artifact must have been validated first.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "filename": {
                            "type": "string",
                            "description": "Filename to save as (e.g. 'agents/monitor.dsl', 'policies/api_access.cedar', 'tools/healthcheck.clad.toml')"
                        },
                        "content": {
                            "type": "string",
                            "description": "The artifact content to save"
                        },
                        "artifact_type": {
                            "type": "string",
                            "enum": ["dsl", "cedar", "toolclad"],
                            "description": "Type of artifact"
                        }
                    },
                    "required": ["filename", "content", "artifact_type"]
                }),
            },
        ];

        let fleet = match self.cards.try_read() {
            Ok(cards) if !cards.is_empty() => cards
                .iter()
                .map(|c| format!("  - {}: {}", c.name, c.description))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => "  (no agents loaded)".to_string(),
        };
        defs.push(ToolDefinition {
            name: "delegate".to_string(),
            description: format!(
                "Delegate a task to a loaded agent and return its reply. Available agents:\n{fleet}",
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent": { "type": "string", "description": "Name of the agent to delegate to" },
                    "task":  { "type": "string", "description": "The task/message for the agent" }
                },
                "required": ["agent", "task"]
            }),
        });

        defs.push(ToolDefinition {
            name: "read_file".to_string(),
            description: "Read a text file from the current project. Paths are relative to the project root; absolute paths and '..' are rejected. Large files are truncated.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Project-relative path to the file (e.g. 'agents/monitor.dsl')"
                    }
                },
                "required": ["path"]
            }),
        });

        defs.push(ToolDefinition {
            name: "search".to_string(),
            description: "Recursively search text files in the project for lines containing a query string. Returns matches as 'path:lineno: line'. Skips binary files, target/, and .git/. Paths are relative to the project root; absolute paths and '..' are rejected.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Substring to search for"
                    },
                    "path": {
                        "type": "string",
                        "description": "Optional project-relative directory to search under (default '.')"
                    }
                },
                "required": ["query"]
            }),
        });

        defs.push(ToolDefinition {
            name: "edit_file".to_string(),
            description: "Create or overwrite a text file in the project (requires human approval). Paths are project-relative; absolute paths and '..' are rejected.".to_string(),
            parameters: serde_json::json!({
                "type":"object",
                "properties":{
                    "path":{"type":"string","description":"Project-relative file path"},
                    "content":{"type":"string","description":"Full new file contents"}
                },
                "required":["path","content"]
            }),
        });

        if self.allow_shell {
            defs.push(ToolDefinition {
                name: "shell".to_string(),
                description: "Run a shell command in the project root and return its output (requires human approval; enabled via --allow-shell).".to_string(),
                parameters: serde_json::json!({
                    "type":"object",
                    "properties":{"command":{"type":"string","description":"The shell command to run"}},
                    "required":["command"]
                }),
            });
        }

        defs
    }
}

impl OrchestratorExecutor {
    async fn handle_tool_call(&self, name: &str, arguments: &str) -> Result<String, String> {
        let args: serde_json::Value =
            serde_json::from_str(arguments).map_err(|e| format!("Invalid arguments: {}", e))?;

        match name {
            "list_agents" => {
                let agents = self.engine.evaluator().list_agents().await;
                if agents.is_empty() {
                    Ok("No agents currently running.".to_string())
                } else {
                    let mut out = String::from("Running agents:\n");
                    for agent in &agents {
                        out.push_str(&format!(
                            "  {} — {} ({:?})\n",
                            &agent.id.to_string()[..8],
                            agent.definition.name,
                            agent.state
                        ));
                    }
                    Ok(out)
                }
            }
            "validate_dsl" => {
                let code = args
                    .get("dsl_code")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing dsl_code argument")?;
                let issues =
                    validation::dsl_validator::validate_dsl(code, &self.constraints.constraints)
                        .map_err(|e| format!("Validation error: {}", e))?;

                if issues.is_empty() {
                    Ok("DSL validation passed — no issues found.".to_string())
                } else {
                    let mut out = String::from("DSL validation issues:\n");
                    for issue in &issues {
                        out.push_str(&format!("  [{:?}] {}\n", issue.severity, issue.message));
                    }
                    Ok(out)
                }
            }
            "validate_cedar" => {
                let policy = args
                    .get("cedar_policy")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing cedar_policy argument")?;
                let issues = validation::cedar_validator::validate_cedar(
                    policy,
                    &self.constraints.constraints.cedar,
                )
                .map_err(|e| format!("Validation error: {}", e))?;

                if issues.is_empty() {
                    Ok("Cedar policy validation passed — no issues found.".to_string())
                } else {
                    let mut out = String::from("Cedar policy validation issues:\n");
                    for issue in &issues {
                        out.push_str(&format!("  [{:?}] {}\n", issue.severity, issue.message));
                    }
                    Ok(out)
                }
            }
            "validate_toolclad" => {
                let manifest = args
                    .get("toml_manifest")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing toml_manifest argument")?;
                let issues = validation::toolclad_validator::validate_toolclad(
                    manifest,
                    &self.constraints.constraints.toolclad,
                )
                .map_err(|e| format!("Validation error: {}", e))?;

                if issues.is_empty() {
                    Ok("ToolClad validation passed — no issues found.".to_string())
                } else {
                    let mut out = String::from("ToolClad validation issues:\n");
                    for issue in &issues {
                        out.push_str(&format!("  [{:?}] {}\n", issue.severity, issue.message));
                    }
                    Ok(out)
                }
            }
            "save_artifact" => {
                let filename = args
                    .get("filename")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing filename argument")?;
                let content = args
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing content argument")?;
                let artifact_type = args
                    .get("artifact_type")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing artifact_type argument")?;

                // Re-validate before saving (defense in depth)
                let issues = match artifact_type {
                    "dsl" => validation::dsl_validator::validate_dsl(
                        content,
                        &self.constraints.constraints,
                    ),
                    "cedar" => validation::cedar_validator::validate_cedar(
                        content,
                        &self.constraints.constraints.cedar,
                    ),
                    "toolclad" => validation::toolclad_validator::validate_toolclad(
                        content,
                        &self.constraints.constraints.toolclad,
                    ),
                    _ => return Err(format!("Unknown artifact type: {}", artifact_type)),
                }
                .map_err(|e| format!("Re-validation error: {}", e))?;

                let errors: Vec<_> = issues
                    .iter()
                    .filter(|i| i.severity == validation::dsl_validator::Severity::Error)
                    .collect();
                if !errors.is_empty() {
                    let mut out = String::from("Cannot save — validation errors:\n");
                    for issue in errors {
                        out.push_str(&format!("  [Error] {}\n", issue.message));
                    }
                    return Err(out);
                }

                // Sanitize filename — prevent path traversal
                let sanitized = std::path::Path::new(filename);
                if sanitized.is_absolute()
                    || sanitized
                        .components()
                        .any(|c| matches!(c, std::path::Component::ParentDir))
                {
                    return Err("Invalid filename — no absolute paths or .. allowed".to_string());
                }

                // Ensure parent directory exists
                if let Some(parent) = sanitized.parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent)
                            .map_err(|e| format!("Failed to create directory: {}", e))?;
                    }
                }

                std::fs::write(sanitized, content)
                    .map_err(|e| format!("Failed to write file: {}", e))?;

                Ok(format!("Saved {} artifact to {}", artifact_type, filename))
            }
            "delegate" => {
                let agent = args
                    .get("agent")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing agent argument")?;
                let task = args
                    .get("task")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing task argument")?;
                match self.bridge.delegate(agent, task).await {
                    Ok(reply) => Ok(reply),
                    Err(e) => {
                        let loaded = match self.cards.try_read() {
                            Ok(cards) => cards
                                .iter()
                                .map(|c| c.name.clone())
                                .collect::<Vec<_>>()
                                .join(", "),
                            _ => String::new(),
                        };
                        Err(format!(
                            "delegation to '{agent}' failed: {e}. Loaded agents: {loaded}"
                        ))
                    }
                }
            }
            "read_file" => {
                let path = args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing path argument")?;
                read_repo_file(path)
            }
            "search" => {
                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing query argument")?;
                if query.is_empty() {
                    return Err("query must not be empty".to_string());
                }
                let base = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
                search_repo(query, base)
            }
            "edit_file" => {
                let path = args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing path argument")?;
                let content = args
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing content argument")?;
                let target = sanitize_repo_write_path(path)?;
                std::fs::write(&target, content)
                    .map_err(|e| format!("Failed to write {path}: {e}"))?;
                Ok(format!("Wrote {} bytes to {path}", content.len()))
            }
            "shell" => {
                if !self.allow_shell {
                    return Err(
                        "shell tool is disabled (start symbi-shell with --allow-shell to enable)"
                            .to_string(),
                    );
                }
                let command = args
                    .get("command")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing command argument")?;
                let root = std::env::current_dir()
                    .map_err(|e| format!("Cannot resolve project root: {e}"))?;
                let output = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(command)
                    .current_dir(&root)
                    .output()
                    .map_err(|e| format!("Failed to run command: {e}"))?;
                let mut out = String::new();
                out.push_str(&String::from_utf8_lossy(&output.stdout));
                let err = String::from_utf8_lossy(&output.stderr);
                if !err.is_empty() {
                    out.push_str("\n[stderr]\n");
                    out.push_str(&err);
                }
                const SHELL_MAX: usize = 16 * 1024;
                if out.len() > SHELL_MAX {
                    out.truncate(SHELL_MAX);
                    out.push_str("\n[truncated]");
                }
                Ok(format!(
                    "[exit {}]\n{}",
                    output.status.code().unwrap_or(-1),
                    out
                ))
            }
            _ => Err(format!("Unknown tool: {}", name)),
        }
    }
}

/// Maximum number of bytes `read_file` will return before truncating.
const READ_FILE_MAX_BYTES: usize = 64 * 1024;

/// Maximum number of matches `search` will collect before noting truncation.
const SEARCH_MAX_MATCHES: usize = 100;

/// Resolve a project-relative path and confirm it stays inside the repo root.
///
/// First rejects absolute paths and any `..` component lexically (cheap), then
/// canonicalizes BOTH the repo root and the candidate and verifies the candidate
/// is contained in the root. Canonicalization resolves symlinks, so a symlinked
/// file/dir inside the tree cannot be used to escape it (the lexical check alone
/// does not catch this). The target must exist — these tools only read.
fn sanitize_repo_path(path: &str) -> Result<std::path::PathBuf, String> {
    let p = std::path::Path::new(path);
    if p.is_absolute()
        || p.components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("Invalid path — no absolute paths or .. allowed".to_string());
    }
    let root = std::env::current_dir()
        .and_then(|d| d.canonicalize())
        .map_err(|e| format!("Cannot resolve project root: {e}"))?;
    let canonical = root
        .join(p)
        .canonicalize()
        .map_err(|e| format!("Cannot resolve path '{path}': {e}"))?;
    if !canonical.starts_with(&root) {
        return Err("Invalid path — resolves outside the project root".to_string());
    }
    Ok(canonical)
}

/// Resolve a writable repo-relative path: lexical reject of absolute/`..`, then
/// ensure the parent dir exists and canonicalizes to inside the repo root (so a
/// symlinked directory cannot redirect the write outside the tree).
fn sanitize_repo_write_path(path: &str) -> Result<std::path::PathBuf, String> {
    let p = std::path::Path::new(path);
    if p.is_absolute()
        || p.components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("Invalid path — no absolute paths or .. allowed".to_string());
    }
    let file_name = p
        .file_name()
        .ok_or_else(|| "Invalid path — no file name".to_string())?;
    let root = std::env::current_dir()
        .and_then(|d| d.canonicalize())
        .map_err(|e| format!("Cannot resolve project root: {e}"))?;
    let target = root.join(p);
    let parent = target.parent().ok_or_else(|| "Invalid path".to_string())?;
    std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create directory: {e}"))?;
    let parent_canon = parent
        .canonicalize()
        .map_err(|e| format!("Cannot resolve path '{path}': {e}"))?;
    if !parent_canon.starts_with(&root) {
        return Err("Invalid path — resolves outside the project root".to_string());
    }
    let target = parent_canon.join(file_name);
    // The parent is contained, but the final component itself could be an
    // existing symlink pointing outside the root — writing through it would
    // follow the link and escape. Reject any symlinked final component.
    if let Ok(meta) = std::fs::symlink_metadata(&target) {
        if meta.file_type().is_symlink() {
            return Err("Invalid path — target is a symlink".to_string());
        }
    }
    Ok(target)
}

/// Read a UTF-8 text file rooted in the project, capping output at
/// `READ_FILE_MAX_BYTES` (truncating with a note when larger).
fn read_repo_file(path: &str) -> Result<String, String> {
    let sanitized = sanitize_repo_path(path)?;
    let bytes = std::fs::read(&sanitized).map_err(|e| format!("Failed to read {path}: {e}"))?;

    let truncated = bytes.len() > READ_FILE_MAX_BYTES;
    let slice = if truncated {
        &bytes[..READ_FILE_MAX_BYTES]
    } else {
        &bytes[..]
    };

    // Reject binary content rather than emitting garbage.
    let content = String::from_utf8_lossy(slice);
    if content.contains('\u{0}') {
        return Err(format!("{path} appears to be a binary file"));
    }

    if truncated {
        Ok(format!(
            "{content}\n\n[truncated — file is {} bytes, showing first {} bytes]",
            bytes.len(),
            READ_FILE_MAX_BYTES
        ))
    } else {
        Ok(content.into_owned())
    }
}

/// Recursively search text files under a project-rooted directory for lines
/// containing `query`. Skips `target/`, `.git/`, and binary files. Caps at
/// `SEARCH_MAX_MATCHES`.
fn search_repo(query: &str, base: &str) -> Result<String, String> {
    let root = sanitize_repo_path(base)?;
    if !root.exists() {
        return Err(format!("Search path does not exist: {base}"));
    }

    let mut matches: Vec<String> = Vec::new();
    let mut truncated = false;
    // `root` is the canonical repo-rooted base; keep it for per-entry containment
    // checks so a symlink cannot walk us out of the tree.
    let mut stack: Vec<std::path::PathBuf> = vec![root.clone()];

    while let Some(dir) = stack.pop() {
        if truncated {
            break;
        }
        // Walk both files and directories; a plain file as the base is handled
        // by reading it directly.
        let entries = if dir.is_dir() {
            match std::fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => continue,
            }
        } else {
            // `dir` is actually a file (e.g. base pointed at a file).
            scan_file_for_matches(&dir, query, &mut matches, &mut truncated);
            continue;
        };

        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            // Skip noisy / large generated trees.
            if name == "target" || name == ".git" || name == "node_modules" {
                continue;
            }
            // Authoritative containment: canonicalize the entry (resolving any
            // symlink) and confirm it still lives inside the repo root. Anything
            // that resolves outside — or can't be resolved — is skipped, so a
            // symlinked file or directory cannot escape the search scope.
            let real = match entry.path().canonicalize() {
                Ok(r) if r.starts_with(&root) => r,
                _ => continue,
            };
            if real.is_dir() {
                stack.push(real);
            } else if real.is_file() {
                scan_file_for_matches(&real, query, &mut matches, &mut truncated);
                if truncated {
                    break;
                }
            }
        }
    }

    if matches.is_empty() {
        return Ok(format!("No matches for '{query}'."));
    }

    let mut out = format!("Found {} match(es) for '{query}':\n", matches.len());
    out.push_str(&matches.join("\n"));
    if truncated {
        out.push_str(&format!(
            "\n\n[truncated — stopped at {SEARCH_MAX_MATCHES} matches]"
        ));
    }
    Ok(out)
}

/// Scan a single file for `query`, appending `path:lineno: line` matches.
/// Skips files that look binary. Sets `truncated` once the global cap is hit.
fn scan_file_for_matches(
    path: &std::path::Path,
    query: &str,
    matches: &mut Vec<String>,
    truncated: &mut bool,
) {
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    // Heuristic binary skip: NUL byte in the first chunk.
    let probe = &bytes[..bytes.len().min(8192)];
    if probe.contains(&0) {
        return;
    }
    let content = String::from_utf8_lossy(&bytes);
    let display = path.display();
    for (idx, line) in content.lines().enumerate() {
        if line.contains(query) {
            if matches.len() >= SEARCH_MAX_MATCHES {
                *truncated = true;
                return;
            }
            matches.push(format!("{display}:{}: {}", idx + 1, line.trim_end()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use symbi_runtime::reasoning::conversation::Conversation;
    use symbi_runtime::reasoning::inference::{
        FinishReason, InferenceError, InferenceOptions, InferenceProvider, InferenceResponse, Usage,
    };

    struct MockProvider;
    #[async_trait]
    impl InferenceProvider for MockProvider {
        async fn complete(
            &self,
            _c: &Conversation,
            _o: &InferenceOptions,
        ) -> Result<InferenceResponse, InferenceError> {
            Ok(InferenceResponse {
                content: "delegated-reply".to_string(),
                tool_calls: vec![],
                finish_reason: FinishReason::Stop,
                usage: Usage::default(),
                model: "mock".to_string(),
            })
        }
        fn provider_name(&self) -> &str {
            "mock"
        }
        fn default_model(&self) -> &str {
            "mock"
        }
        fn supports_native_tools(&self) -> bool {
            false
        }
        fn supports_structured_output(&self) -> bool {
            false
        }
    }

    async fn executor_with_agent() -> OrchestratorExecutor {
        executor_with_shell(false).await
    }

    async fn executor_with_shell(allow: bool) -> OrchestratorExecutor {
        let bridge = Arc::new(repl_core::RuntimeBridge::new_permissive_for_dev());
        bridge.set_inference_provider(Arc::new(MockProvider));
        bridge
            .register_agent("worker", "You are worker.", vec![])
            .await;
        let cards = Arc::new(tokio::sync::RwLock::new(vec![crate::agents::AgentCard {
            name: "worker".into(),
            description: "does work".into(),
        }]));
        let engine = Arc::new(repl_core::ReplEngine::new(Arc::clone(&bridge)));
        let constraints = Arc::new(ProjectConstraints::default());
        OrchestratorExecutor::new(constraints, engine, bridge, cards, allow)
    }

    #[tokio::test]
    async fn delegate_success_returns_agent_reply() {
        let exec = executor_with_agent().await;
        let out = exec
            .handle_tool_call("delegate", "{\"agent\":\"worker\",\"task\":\"do it\"}")
            .await
            .unwrap();
        assert_eq!(out, "delegated-reply");
    }

    #[tokio::test]
    async fn delegate_unknown_agent_is_recoverable_error() {
        let exec = executor_with_agent().await;
        let err = exec
            .handle_tool_call("delegate", "{\"agent\":\"ghost\",\"task\":\"x\"}")
            .await
            .unwrap_err();
        assert!(err.contains("ghost"));
        assert!(err.contains("worker"), "should list the loaded fleet");
    }

    #[tokio::test]
    async fn delegate_tool_is_listed_with_fleet() {
        let exec = executor_with_agent().await;
        let defs = exec.tool_definitions();
        let d = defs.iter().find(|d| d.name == "delegate").unwrap();
        assert!(d.description.contains("worker"));
    }

    #[tokio::test]
    async fn read_file_and_search_are_listed() {
        let exec = executor_with_agent().await;
        let defs = exec.tool_definitions();
        assert!(defs.iter().any(|d| d.name == "read_file"));
        assert!(defs.iter().any(|d| d.name == "search"));
    }

    // Helper: run a closure with CWD temporarily set to `dir`. The
    // read_file/search tools are repo-relative, so tests pin CWD to a temp
    // dir. Serialized via a mutex because CWD is process-global.
    fn with_cwd<T>(dir: &std::path::Path, f: impl FnOnce() -> T) -> T {
        use std::sync::Mutex;
        static CWD_LOCK: Mutex<()> = Mutex::new(());
        let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir).unwrap();
        let result = f();
        std::env::set_current_dir(prev).unwrap();
        result
    }

    #[test]
    fn read_file_reads_repo_relative_file() {
        let tmp = std::env::temp_dir().join(format!("symbi_rf_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(tmp.join("sub")).unwrap();
        std::fs::write(tmp.join("sub/note.txt"), "hello world\nsecond line").unwrap();

        let out = with_cwd(&tmp, || read_repo_file("sub/note.txt")).unwrap();
        assert!(out.contains("hello world"));
        assert!(out.contains("second line"));

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn read_file_rejects_traversal_and_absolute() {
        assert!(read_repo_file("../etc/passwd").is_err());
        assert!(read_repo_file("/etc/passwd").is_err());
        assert!(read_repo_file("a/../../b").is_err());
    }

    #[test]
    fn read_file_truncates_oversize() {
        let tmp = std::env::temp_dir().join(format!("symbi_rf_big_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let big = "x".repeat(READ_FILE_MAX_BYTES + 5000);
        std::fs::write(tmp.join("big.txt"), &big).unwrap();

        let out = with_cwd(&tmp, || read_repo_file("big.txt")).unwrap();
        assert!(out.contains("[truncated"));
        assert!(out.len() < big.len());

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn search_finds_known_string() {
        let tmp = std::env::temp_dir().join(format!("symbi_search_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(tmp.join("nested")).unwrap();
        std::fs::write(tmp.join("a.txt"), "needle here\nnope").unwrap();
        std::fs::write(tmp.join("nested/b.txt"), "another needle line").unwrap();

        let out = with_cwd(&tmp, || search_repo("needle", ".")).unwrap();
        assert!(out.contains("a.txt"));
        assert!(out.contains("b.txt"));
        assert!(out.contains("needle here"));

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn search_respects_cap() {
        let tmp = std::env::temp_dir().join(format!("symbi_search_cap_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let lines: String = (0..SEARCH_MAX_MATCHES + 50).map(|_| "match\n").collect();
        std::fs::write(tmp.join("many.txt"), lines).unwrap();

        let out = with_cwd(&tmp, || search_repo("match", ".")).unwrap();
        assert!(out.contains("[truncated"));

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn search_rejects_traversal_and_absolute() {
        assert!(search_repo("x", "../etc").is_err());
        assert!(search_repo("x", "/etc").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn read_file_rejects_symlink_escape() {
        // A symlink inside the repo pointing at a file OUTSIDE it must not be
        // readable — the lexical `..` check can't catch this; canonicalization does.
        let base = std::env::temp_dir().join(format!("symbi_sym_{}", uuid::Uuid::new_v4()));
        let outside = std::env::temp_dir().join(format!("symbi_secret_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "TOP SECRET").unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), base.join("link.txt")).unwrap();

        let res = with_cwd(&base, || read_repo_file("link.txt"));
        assert!(
            res.is_err(),
            "symlink escaping the repo root must be rejected"
        );

        std::fs::remove_dir_all(&base).ok();
        std::fs::remove_dir_all(&outside).ok();
    }

    #[cfg(unix)]
    #[test]
    fn search_does_not_follow_symlinked_dir_escape() {
        // A symlinked directory inside the repo pointing OUTSIDE must not be walked.
        let base = std::env::temp_dir().join(format!("symbi_symd_{}", uuid::Uuid::new_v4()));
        let outside = std::env::temp_dir().join(format!("symbi_secretd_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("leak.txt"), "needle-secret").unwrap();
        std::os::unix::fs::symlink(&outside, base.join("escape")).unwrap();

        let out = with_cwd(&base, || search_repo("needle-secret", ".")).unwrap();
        // The output echoes the query string, so assert on the match verdict:
        // a followed symlink would report "Found ..."; a correctly-skipped one
        // reports "No matches".
        assert!(
            out.contains("No matches"),
            "search must not follow symlinked dirs out of the repo; got: {out}"
        );

        std::fs::remove_dir_all(&base).ok();
        std::fs::remove_dir_all(&outside).ok();
    }

    #[test]
    fn edit_file_writes_within_root() {
        let tmp = std::env::temp_dir().join(format!("symbi_ef_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();

        let target = with_cwd(&tmp, || sanitize_repo_write_path("sub/new.txt")).unwrap();
        std::fs::write(&target, "fresh content").unwrap();
        assert!(target.starts_with(tmp.canonicalize().unwrap()));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "fresh content");

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn edit_file_rejects_traversal_and_absolute() {
        assert!(sanitize_repo_write_path("../x").is_err());
        assert!(sanitize_repo_write_path("/etc/x").is_err());
        assert!(sanitize_repo_write_path("a/../../b").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn edit_file_rejects_symlinked_target() {
        // An existing symlink as the final component must not be written through —
        // it could point outside the repo root even though the parent is contained.
        let tmp = std::env::temp_dir().join(format!("symbi_wsym_{}", uuid::Uuid::new_v4()));
        let outside = std::env::temp_dir().join(format!("symbi_wsecret_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("target.txt"), "original").unwrap();
        std::os::unix::fs::symlink(outside.join("target.txt"), tmp.join("link.txt")).unwrap();

        let res = with_cwd(&tmp, || sanitize_repo_write_path("link.txt"));
        assert!(
            res.is_err(),
            "writing through a symlinked target must be rejected"
        );
        // The outside file must be untouched.
        assert_eq!(
            std::fs::read_to_string(outside.join("target.txt")).unwrap(),
            "original"
        );

        std::fs::remove_dir_all(&tmp).ok();
        std::fs::remove_dir_all(&outside).ok();
    }

    #[tokio::test]
    async fn shell_disabled_by_default() {
        let exec = executor_with_shell(false).await;
        let defs = exec.tool_definitions();
        assert!(
            !defs.iter().any(|d| d.name == "shell"),
            "shell must not be listed when allow_shell=false"
        );
        let res = exec
            .handle_tool_call("shell", "{\"command\":\"echo hi\"}")
            .await;
        assert!(res.is_err(), "shell must error when disabled");
    }

    #[tokio::test]
    async fn shell_enabled_runs_command() {
        let exec = executor_with_shell(true).await;
        let defs = exec.tool_definitions();
        assert!(
            defs.iter().any(|d| d.name == "shell"),
            "shell must be listed when allow_shell=true"
        );
        let out = exec
            .handle_tool_call("shell", "{\"command\":\"echo hi\"}")
            .await
            .unwrap();
        assert!(
            out.contains("hi"),
            "shell output should contain 'hi': {out}"
        );
    }
}
