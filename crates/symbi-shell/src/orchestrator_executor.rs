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
}

impl OrchestratorExecutor {
    pub fn new(constraints: Arc<ProjectConstraints>, engine: Arc<repl_core::ReplEngine>) -> Self {
        Self {
            constraints,
            engine,
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
        vec![
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
        ]
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
            _ => Err(format!("Unknown tool: {}", name)),
        }
    }
}
