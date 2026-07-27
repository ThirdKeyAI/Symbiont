//! Coordinator action executor.
//!
//! Maps the coordinator's tool calls to in-process [`RuntimeApiProvider`]
//! method invocations — no HTTP round-trip.

#[cfg(feature = "http-api")]
use std::sync::Arc;

#[cfg(feature = "http-api")]
use async_trait::async_trait;

#[cfg(feature = "http-api")]
use crate::reasoning::circuit_breaker::CircuitBreakerRegistry;
#[cfg(feature = "http-api")]
use crate::reasoning::executor::ActionExecutor;
#[cfg(feature = "http-api")]
use crate::reasoning::inference::ToolDefinition;
#[cfg(feature = "http-api")]
use crate::reasoning::loop_types::{LoopConfig, Observation, ProposedAction};
#[cfg(feature = "http-api")]
use crate::reasoning::phases::DELEGATE_TOOL_NAME;
#[cfg(feature = "http-api")]
use crate::types::AgentId;

#[cfg(feature = "http-api")]
use super::traits::RuntimeApiProvider;

/// An [`ActionExecutor`] that dispatches coordinator tool calls to the
/// [`RuntimeApiProvider`] in-process.
#[cfg(feature = "http-api")]
pub struct CoordinatorExecutor {
    provider: Arc<dyn RuntimeApiProvider>,
}

#[cfg(feature = "http-api")]
impl CoordinatorExecutor {
    pub fn new(provider: Arc<dyn RuntimeApiProvider>) -> Self {
        Self { provider }
    }

    /// Tool definitions offered to the model. `agent_names` are the delegation
    /// registry's keys; when non-empty a `delegate` tool is advertised listing
    /// them, so the model names a target that actually resolves.
    pub fn tool_definitions(agent_names: &[String]) -> Vec<ToolDefinition> {
        let mut defs = vec![
            ToolDefinition {
                name: "list_agents".into(),
                description: "List all agents in the fleet with their current status. Includes external agents with their heartbeat state, last result, and recent events.".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
            ToolDefinition {
                name: "agent_status".into(),
                description: "Get the detailed status of a specific agent by ID. For external agents, includes metadata, last result, recent events, and execution mode.".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "agent_id": {
                            "type": "string",
                            "description": "The UUID of the agent to query."
                        }
                    },
                    "required": ["agent_id"]
                }),
            },
            ToolDefinition {
                name: "query_metrics".into(),
                description: "Get current system metrics (CPU, memory, uptime, etc.).".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
            ToolDefinition {
                name: "list_schedules".into(),
                description: "List all scheduled jobs with their summaries.".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
            ToolDefinition {
                name: "scheduler_health".into(),
                description: "Get scheduler health and run statistics.".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
            ToolDefinition {
                name: "system_health".into(),
                description: "Get overall system health status.".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
        ];

        if !agent_names.is_empty() {
            defs.push(ToolDefinition {
                name: DELEGATE_TOOL_NAME.to_string(),
                description: format!(
                    "Delegate a task to a loaded agent and return its reply. \
                     Available agents: {}",
                    agent_names.join(", ")
                ),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "agent": { "type": "string", "description": "Name of the agent to delegate to" },
                        "task": { "type": "string", "description": "The task/message for that agent" }
                    },
                    "required": ["agent", "task"]
                }),
            });
        }
        defs
    }

    async fn dispatch_tool(&self, name: &str, arguments: &str) -> Result<String, String> {
        match name {
            "list_agents" => {
                let agent_ids = self
                    .provider
                    .list_agents()
                    .await
                    .map_err(|e| e.to_string())?;

                let mut agents = Vec::new();
                for id in &agent_ids {
                    match self.provider.get_agent_status(*id).await {
                        Ok(status) => agents.push(serde_json::to_value(status).unwrap()),
                        Err(e) => {
                            agents.push(serde_json::json!({
                                "agent_id": id.0.to_string(),
                                "error": e.to_string()
                            }));
                        }
                    }
                }
                serde_json::to_string_pretty(&agents).map_err(|e| e.to_string())
            }

            "agent_status" => {
                let args: serde_json::Value =
                    serde_json::from_str(arguments).map_err(|e| e.to_string())?;
                let agent_id_str = args["agent_id"].as_str().ok_or("missing agent_id")?;
                let uuid = uuid::Uuid::parse_str(agent_id_str).map_err(|e| e.to_string())?;
                let status = self
                    .provider
                    .get_agent_status(AgentId(uuid))
                    .await
                    .map_err(|e| e.to_string())?;
                serde_json::to_string_pretty(&status).map_err(|e| e.to_string())
            }

            "query_metrics" => {
                let metrics = self
                    .provider
                    .get_metrics()
                    .await
                    .map_err(|e| e.to_string())?;
                serde_json::to_string_pretty(&metrics).map_err(|e| e.to_string())
            }

            "list_schedules" => {
                let schedules = self
                    .provider
                    .list_schedules()
                    .await
                    .map_err(|e| e.to_string())?;
                serde_json::to_string_pretty(&schedules).map_err(|e| e.to_string())
            }

            "scheduler_health" => {
                let health = self
                    .provider
                    .get_scheduler_health()
                    .await
                    .map_err(|e| e.to_string())?;
                serde_json::to_string_pretty(&health).map_err(|e| e.to_string())
            }

            "system_health" => {
                let health = self
                    .provider
                    .get_system_health()
                    .await
                    .map_err(|e| e.to_string())?;
                serde_json::to_string_pretty(&health).map_err(|e| e.to_string())
            }

            // A well-formed `delegate` call is converted to a `Delegate` action
            // in `produce_output` and never reaches dispatch. This arm is
            // reached only for malformed arguments, so it reports the error
            // without ever invoking delegation — the executor must never hold
            // or invoke a `DelegationExecutor` (the sub-loop's own executor
            // *is* a `CoordinatorExecutor`, which would create an Arc cycle).
            DELEGATE_TOOL_NAME => Err(
                "delegate requires string fields 'agent' (a loaded agent name) and 'task' \
                 (the message for that agent)"
                    .to_string(),
            ),

            _ => Err(format!("Unknown tool: {}", name)),
        }
    }
}

#[cfg(all(test, feature = "http-api"))]
mod delegate_tool_tests {
    use super::*;

    #[test]
    fn delegate_tool_is_advertised_with_the_configured_agent_names() {
        let defs =
            CoordinatorExecutor::tool_definitions(&["reviewer".to_string(), "planner".to_string()]);
        let d = defs
            .iter()
            .find(|t| t.name == "delegate")
            .expect("delegate tool must be advertised when agents are loaded");
        // The model picks the target from this description, so it must list the
        // exact registry keys that will resolve.
        assert!(d.description.contains("reviewer"));
        assert!(d.description.contains("planner"));
        let required = d.parameters["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "agent"));
        assert!(required.iter().any(|v| v == "task"));
    }

    #[test]
    fn action_executor_impl_advertises_tools_to_sub_loops() {
        // A delegated sub-loop is built with a default LoopConfig, so the runner
        // auto-populates its tool list from `ActionExecutor::tool_definitions`.
        // Without that impl the sub-loop is offered nothing and can only reply
        // from its prompt. Naming the impl here means deleting it fails to
        // compile; the forwarded set is asserted below.
        let _guard: fn(&CoordinatorExecutor) -> Vec<ToolDefinition> =
            <CoordinatorExecutor as crate::reasoning::executor::ActionExecutor>::tool_definitions;

        let forwarded = CoordinatorExecutor::tool_definitions(&[]);
        assert!(forwarded.iter().any(|t| t.name == "list_agents"));
        assert!(forwarded.iter().any(|t| t.name == "system_health"));
        assert!(
            forwarded.iter().all(|t| t.name != "delegate"),
            "a sub-loop has no target registry, so it must not be offered delegate"
        );
    }

    #[test]
    fn delegate_tool_is_not_advertised_without_agents() {
        // Offering a tool with no possible target would invite guaranteed failures.
        let defs = CoordinatorExecutor::tool_definitions(&[]);
        assert!(defs.iter().all(|t| t.name != "delegate"));
        // The other tools are unaffected.
        assert!(defs.iter().any(|t| t.name == "list_agents"));
    }
}

#[cfg(feature = "http-api")]
#[async_trait]
impl ActionExecutor for CoordinatorExecutor {
    async fn execute_actions(
        &self,
        actions: &[ProposedAction],
        _config: &LoopConfig,
        _circuit_breakers: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        let mut observations = Vec::new();

        for action in actions {
            if let ProposedAction::ToolCall {
                call_id,
                name,
                arguments,
            } = action
            {
                match self.dispatch_tool(name, arguments).await {
                    Ok(result) => {
                        observations.push(
                            Observation::tool_result(name.clone(), result)
                                .with_call_id(call_id.clone()),
                        );
                    }
                    Err(err) => {
                        observations.push(
                            Observation::tool_error(name.clone(), err)
                                .with_call_id(call_id.clone()),
                        );
                    }
                }
            }
        }

        observations
    }

    /// Tools this executor can dispatch, for runners that don't set the list
    /// themselves. A delegated sub-loop is built with a default `LoopConfig`, so
    /// without this it was offered nothing and could only reply from its prompt.
    ///
    /// The `delegate` tool is deliberately excluded: it is advertised by the
    /// parent (which holds the delegation handle and the registry of targets),
    /// and offering it here would let a sub-agent start a further hop with no
    /// registry to resolve against.
    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        Self::tool_definitions(&[])
    }
}
