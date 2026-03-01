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

    /// Return the tool definitions the coordinator agent should advertise.
    pub fn tool_definitions() -> Vec<ToolDefinition> {
        vec![
            ToolDefinition {
                name: "list_agents".into(),
                description: "List all agents in the fleet with their current status.".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
            ToolDefinition {
                name: "agent_status".into(),
                description: "Get the detailed status of a specific agent by ID.".into(),
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
        ]
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

            _ => Err(format!("Unknown tool: {}", name)),
        }
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
}
