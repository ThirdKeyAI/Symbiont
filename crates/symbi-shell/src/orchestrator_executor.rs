use async_trait::async_trait;
use std::sync::Arc;
use symbi_runtime::reasoning::circuit_breaker::CircuitBreakerRegistry;
use symbi_runtime::reasoning::executor::ActionExecutor;
use symbi_runtime::reasoning::inference::ToolDefinition;
use symbi_runtime::reasoning::loop_types::{LoopConfig, Observation, ProposedAction};

use crate::sandbox_tools::SandboxTools;
use symbi_runtime::reasoning::prepared::{AuthorizedAction, PreparedAction};
use symbi_runtime::types::AgentId;

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
    sandbox_tools: SandboxTools,
    instance_id: uuid::Uuid,
    execution_context: std::collections::HashMap<String, serde_json::Value>,
    /// When `Some`, only tools whose name is in the set are advertised and
    /// executable. `None` (default) advertises the full orchestrator tool set.
    /// Fleet agents pass `Some(manifest_tools − {delegate})`; the orchestrator
    /// uses `None`.
    allowed_tools: Option<std::collections::HashSet<String>>,
}

impl OrchestratorExecutor {
    pub fn new(
        constraints: Arc<ProjectConstraints>,
        engine: Arc<repl_core::ReplEngine>,
        bridge: Arc<repl_core::RuntimeBridge>,
        cards: Arc<tokio::sync::RwLock<Vec<crate::agents::AgentCard>>>,
        allow_shell: bool,
    ) -> Self {
        let sandbox_tools =
            SandboxTools::new(&bridge.reasoning_context().project_root, allow_shell);
        Self {
            sandbox_tools,
            instance_id: uuid::Uuid::new_v4(),
            execution_context: Default::default(),
            constraints,
            engine,
            bridge,
            cards,
            allowed_tools: None,
        }
    }

    /// Restrict this executor to a specific set of tool names (fleet agents).
    pub fn with_allowed_tools(mut self, tools: std::collections::HashSet<String>) -> Self {
        self.allowed_tools = Some(tools);
        self
    }

    /// Select the retained canonical declaration before validating any project default.
    pub fn with_definition(
        mut self,
        definition: &dsl::ConversationalAgent,
        allow_shell: bool,
    ) -> Self {
        let settings = definition.settings();
        let boundary = self
            .bridge
            .reasoning_context()
            .project_root
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|project| {
                symbi_runtime::sandbox::command::CommandBoundary::load_for_agent(project, settings)
            });
        self.sandbox_tools = SandboxTools::with_boundary(boundary, allow_shell);
        let source_hash = symbi_runtime::reasoning::prepared::digest_json(&serde_json::json!(
            definition.source()
        ))
        .expect("literal source is JSON serializable");
        let declaration_hash = symbi_runtime::reasoning::prepared::digest_json(&serde_json::json!(
            settings.agent_source
        ))
        .expect("literal declaration is JSON serializable");
        self.execution_context.insert(
            "agent_definition".into(),
            serde_json::json!({
                "mode":"canonical_orga_conversation", "name":settings.agent_name,
                "source_hash":source_hash, "declaration_hash":declaration_hash,
                "sandbox_tier":settings.sandbox_tier, "timeout_seconds":settings.timeout_seconds,
                "policy_semantics": if definition.policy().is_empty() { "external_cedar" } else { "inline_effect_policy_v1_and_external_cedar" },
            }),
        );
        self
    }

    /// Whether a tool is permitted by this executor's static allow-list.
    /// `None` allow-list permits everything (orchestrator).
    fn tool_allowed(&self, name: &str) -> bool {
        self.allowed_tools
            .as_ref()
            .map(|set| set.contains(name))
            .unwrap_or(true)
    }
}

#[async_trait]
impl ActionExecutor for OrchestratorExecutor {
    fn execution_context(&self) -> std::collections::HashMap<String, serde_json::Value> {
        self.execution_context.clone()
    }
    fn validate_configuration(&self) -> Result<(), String> {
        self.sandbox_tools.validate_configuration()
    }

    async fn execute_actions(
        &self,
        actions: &[ProposedAction],
        _config: &LoopConfig,
        _circuit_breakers: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        actions
            .iter()
            .filter_map(|action| match action {
                ProposedAction::ToolCall { call_id, name, .. } => Some(
                    Observation::tool_error(
                        name.clone(),
                        "shell tools require a prepared authorization grant",
                    )
                    .with_call_id(call_id.clone()),
                ),
                _ => None,
            })
            .collect()
    }

    fn prepare_action(
        &self,
        action: &ProposedAction,
        config: &LoopConfig,
    ) -> Result<PreparedAction, String> {
        let ProposedAction::ToolCall {
            name, arguments, ..
        } = action
        else {
            return PreparedAction::new(action.clone(), None);
        };
        if !self.tool_allowed(name) {
            return Err(format!("tool '{name}' is not available to this agent"));
        }
        if arguments.len() > 65536 {
            return Err("shell tool arguments exceed 65536 bytes".into());
        }
        if name == "save_artifact" {
            self.validate_artifact(
                &serde_json::from_str(arguments).map_err(|error| error.to_string())?,
            )?;
        }
        let prepared = if SandboxTools::handles(name) {
            self.sandbox_tools.prepare(action, config)?
        } else {
            symbi_runtime::reasoning::executor::prepare_registered_action(
                action,
                config,
                &self.tool_definitions(),
            )?
        };
        let mut resolved = prepared.policy_context()["resolved"].clone();
        resolved["shell_executor"] = serde_json::json!(self.instance_id);
        if let Some(definition) = self.execution_context.get("agent_definition") {
            resolved["agent_definition"] = definition.clone();
        }
        resolved["constraints_hash"] =
            serde_json::json!(symbi_runtime::reasoning::prepared::digest_json(
                &serde_json::to_value(self.constraints.as_ref())
                    .map_err(|error| error.to_string())?
            )?);
        prepared.with_resolved(resolved)
    }

    async fn execute_authorized(
        &self,
        actions: Vec<AuthorizedAction>,
        config: &LoopConfig,
        circuit_breakers: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        let mut observations = Vec::new();
        for grant in actions {
            let ProposedAction::ToolCall {
                call_id,
                name,
                arguments,
            } = grant.action().clone()
            else {
                continue;
            };
            if !self.tool_allowed(&name)
                || grant.prepared().policy_context()["resolved"]["shell_executor"]
                    != serde_json::json!(self.instance_id)
            {
                observations.push(
                    Observation::tool_error(
                        name,
                        "authorization belongs to another shell executor or tool scope",
                    )
                    .with_call_id(call_id),
                );
                continue;
            }
            if SandboxTools::handles(&name) {
                let worker_hash =
                    grant.prepared().policy_context()["resolved"]["worker_program_hash"].clone();
                let mut results = self
                    .sandbox_tools
                    .execute_authorized(vec![grant], config, circuit_breakers)
                    .await;
                if name != "shell" {
                    // The fixed implementation is already bound by the prepared
                    // contract. Keep observations focused on the actual effect.
                    for observation in &mut results {
                        if let Ok(mut value) =
                            serde_json::from_str::<serde_json::Value>(&observation.content)
                        {
                            if let Some(object) = value.as_object_mut() {
                                object.remove("command");
                                object.insert("worker_program_hash".into(), worker_hash.clone());
                                observation.content = value.to_string();
                            }
                        }
                    }
                }
                observations.extend(results);
                continue;
            }
            let result = async {
                grant.check_live()?;
                let current = self.prepare_action(grant.action(), config)?;
                if !current.matches_authorized_call(grant.prepared())? {
                    return Err("shell tool contract changed after authorization".into());
                }
                let remaining = grant
                    .deadline()
                    .saturating_duration_since(std::time::Instant::now());
                let principal = grant.principal();
                grant.into_prepared()?;
                tokio::time::timeout(
                    remaining,
                    self.handle_control_call(&name, &arguments, principal),
                )
                .await
                .map_err(|_| "shell control call timed out".to_owned())?
            }
            .await;
            observations.push(match result {
                Ok(content) => Observation::tool_result(name, content).with_call_id(call_id),
                Err(error) => Observation::tool_error(name, error).with_call_id(call_id),
            });
        }
        observations
    }

    fn cancel_run(&self, run: &str, deadline: std::time::Instant) {
        self.sandbox_tools.executor.cancel_run(run, deadline);
    }

    async fn close_run(&self, run: &str, deadline: std::time::Instant) -> Result<(), String> {
        self.sandbox_tools.executor.close_run(run, deadline).await
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

        defs.extend(self.sandbox_tools.definitions());
        for definition in &mut defs {
            definition.parameters["additionalProperties"] = serde_json::json!(false);
        }

        defs.retain(|d| self.tool_allowed(&d.name));
        defs
    }
}

impl OrchestratorExecutor {
    fn validate_artifact(&self, args: &serde_json::Value) -> Result<(), String> {
        let content = args
            .get("content")
            .and_then(|value| value.as_str())
            .ok_or("Missing content argument")?;
        let artifact_type = args
            .get("artifact_type")
            .and_then(|value| value.as_str())
            .ok_or("Missing artifact_type argument")?;
        // Re-validate before saving (defense in depth)
        let issues = match artifact_type {
            "dsl" => {
                validation::dsl_validator::validate_dsl(content, &self.constraints.constraints)
            }
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

        Ok(())
    }

    #[cfg(test)]
    async fn handle_tool_call(&self, name: &str, arguments: &str) -> Result<String, String> {
        self.handle_control_call(name, arguments, AgentId::new())
            .await
    }

    async fn handle_control_call(
        &self,
        name: &str,
        arguments: &str,
        principal: AgentId,
    ) -> Result<String, String> {
        if !self.tool_allowed(name) {
            return Err(format!("tool '{name}' is not available to this agent"));
        }
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
            "delegate" => {
                let agent = args
                    .get("agent")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing agent argument")?;
                let task = args
                    .get("task")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing task argument")?;
                let mut context = self.bridge.reasoning_context();
                context.sender_agent_id = Some(principal);
                match repl_core::dsl::agent_composition::governed_ask(&context, agent, task, None)
                    .await
                {
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
            _ => Err(format!("Unknown tool: {}", name)),
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

    async fn executor_with_agent(project: &std::path::Path) -> OrchestratorExecutor {
        executor_with_shell(false, project).await
    }

    async fn executor_with_shell(allow: bool, project: &std::path::Path) -> OrchestratorExecutor {
        let bridge = Arc::new(
            repl_core::RuntimeBridge::new_permissive_for_dev()
                .with_project_root(project)
                .unwrap(),
        );
        bridge.set_inference_provider(Arc::new(MockProvider));
        bridge
            .register_agent("worker", "You are worker.", vec![])
            .await;
        let cards = Arc::new(tokio::sync::RwLock::new(vec![crate::agents::AgentCard {
            name: "worker".into(),
            description: "does work".into(),
            tools: vec![],
        }]));
        let engine = Arc::new(repl_core::ReplEngine::new(Arc::clone(&bridge)));
        let constraints = Arc::new(ProjectConstraints::default());
        OrchestratorExecutor::new(constraints, engine, bridge, cards, allow)
    }

    #[tokio::test]
    async fn delegate_success_returns_agent_reply() {
        let project = tempfile::tempdir().unwrap();
        let exec = executor_with_agent(project.path()).await;
        let out = exec
            .handle_tool_call("delegate", "{\"agent\":\"worker\",\"task\":\"do it\"}")
            .await
            .unwrap();
        assert_eq!(out, "delegated-reply");
    }

    #[tokio::test]
    async fn delegate_unknown_agent_is_recoverable_error() {
        let project = tempfile::tempdir().unwrap();
        let exec = executor_with_agent(project.path()).await;
        let err = exec
            .handle_tool_call("delegate", "{\"agent\":\"ghost\",\"task\":\"x\"}")
            .await
            .unwrap_err();
        assert!(err.contains("ghost"));
        assert!(err.contains("worker"), "should list the loaded fleet");
    }

    #[tokio::test]
    async fn delegate_tool_is_listed_with_fleet() {
        let project = tempfile::tempdir().unwrap();
        let exec = executor_with_agent(project.path()).await;
        let defs = exec.tool_definitions();
        let d = defs.iter().find(|d| d.name == "delegate").unwrap();
        assert!(d.description.contains("worker"));
    }

    #[tokio::test]
    async fn read_file_and_search_are_listed() {
        let project = tempfile::tempdir().unwrap();
        let exec = executor_with_agent(project.path()).await;
        let defs = exec.tool_definitions();
        assert!(defs.iter().any(|d| d.name == "read_file"));
        assert!(defs.iter().any(|d| d.name == "search"));
    }

    #[tokio::test]
    async fn scoped_executor_lists_only_allowed_tools() {
        let project = tempfile::tempdir().unwrap();
        let exec = executor_with_agent(project.path())
            .await
            .with_allowed_tools(
                ["read_file", "search"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            );
        let defs = exec.tool_definitions();
        let names: Vec<_> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"search"));
        assert!(!names.contains(&"edit_file"));
        assert!(!names.contains(&"delegate"));
        assert!(!names.contains(&"validate_dsl"));
    }

    #[tokio::test]
    async fn scoped_executor_refuses_out_of_set_tool() {
        let project = tempfile::tempdir().unwrap();
        let exec = executor_with_agent(project.path())
            .await
            .with_allowed_tools(["read_file"].iter().map(|s| s.to_string()).collect());
        let err = exec
            .handle_tool_call("delegate", "{\"agent\":\"worker\",\"task\":\"x\"}")
            .await
            .unwrap_err();
        assert!(err.contains("not available"));
    }

    #[tokio::test]
    async fn unscoped_executor_still_lists_full_set() {
        let project = tempfile::tempdir().unwrap();
        let exec = executor_with_agent(project.path()).await; // allowed_tools = None
        let defs = exec.tool_definitions();
        assert!(defs.iter().any(|d| d.name == "delegate"));
        assert!(defs.iter().any(|d| d.name == "validate_dsl"));
    }

    #[tokio::test]
    async fn shell_disabled_by_default() {
        let project = tempfile::tempdir().unwrap();
        let exec = executor_with_shell(false, project.path()).await;
        let defs = exec.tool_definitions();
        assert!(
            !defs.iter().any(|d| d.name == "shell"),
            "shell must not be listed when allow_shell=false"
        );
        let res = exec
            .handle_tool_call("shell", "{\"command\":\"echo hi\"}")
            .await;
        assert!(res.is_err(), "shell has no direct host handler");
    }

    #[tokio::test]
    async fn enabling_shell_does_not_enable_direct_host_dispatch() {
        let project = tempfile::tempdir().unwrap();
        let exec = executor_with_shell(true, project.path()).await;
        let defs = exec.tool_definitions();
        assert!(
            defs.iter().any(|d| d.name == "shell"),
            "shell must be listed when allow_shell=true"
        );
        let action = ProposedAction::ToolCall {
            call_id: "unapproved".into(),
            name: "shell".into(),
            arguments: serde_json::json!({"command":"echo hi"}).to_string(),
        };
        let results = exec
            .execute_actions(
                &[action],
                &LoopConfig::default(),
                &CircuitBreakerRegistry::default(),
            )
            .await;
        assert_eq!(results.len(), 1);
        assert!(results[0].is_error);
        assert!(results[0].content.contains("authorization grant"));
    }
}
