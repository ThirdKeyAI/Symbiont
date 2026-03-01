//! Composio tool executor.
//!
//! An [`ActionExecutor`] that dispatches tool calls to a Composio MCP
//! endpoint via JSON-RPC over the [`SseTransport`].

#[cfg(all(feature = "http-api", feature = "composio"))]
use std::sync::Arc;

#[cfg(all(feature = "http-api", feature = "composio"))]
use async_trait::async_trait;

#[cfg(all(feature = "http-api", feature = "composio"))]
use crate::integrations::composio::transport::SseTransport;
#[cfg(all(feature = "http-api", feature = "composio"))]
use crate::integrations::composio::ComposioError;
#[cfg(all(feature = "http-api", feature = "composio"))]
use crate::reasoning::circuit_breaker::CircuitBreakerRegistry;
#[cfg(all(feature = "http-api", feature = "composio"))]
use crate::reasoning::executor::ActionExecutor;
#[cfg(all(feature = "http-api", feature = "composio"))]
use crate::reasoning::inference::ToolDefinition;
#[cfg(all(feature = "http-api", feature = "composio"))]
use crate::reasoning::loop_types::{LoopConfig, Observation, ProposedAction};

/// An [`ActionExecutor`] that dispatches tool calls to Composio via JSON-RPC.
#[cfg(all(feature = "http-api", feature = "composio"))]
pub struct ComposioToolExecutor {
    transport: Arc<SseTransport>,
    tool_definitions: Vec<ToolDefinition>,
}

#[cfg(all(feature = "http-api", feature = "composio"))]
impl ComposioToolExecutor {
    /// Discover available tools from the Composio MCP endpoint and return a
    /// new executor ready to dispatch calls.
    pub async fn discover(transport: Arc<SseTransport>) -> Result<Self, ComposioError> {
        let result = transport
            .request("tools/list", serde_json::json!({}))
            .await?;

        let tools_value = result.get("tools").cloned().unwrap_or(result.clone());
        let raw_tools: Vec<serde_json::Value> =
            serde_json::from_value(tools_value).map_err(|e| ComposioError::TransportError {
                reason: format!("failed to parse tools/list response: {}", e),
            })?;

        let tool_definitions = raw_tools
            .into_iter()
            .map(|t| {
                let name = t["name"].as_str().unwrap_or("unknown").to_string();
                let description = t["description"].as_str().unwrap_or("").to_string();
                let parameters = t
                    .get("inputSchema")
                    .or_else(|| t.get("parameters"))
                    .cloned()
                    .unwrap_or(serde_json::json!({
                        "type": "object",
                        "properties": {},
                        "required": []
                    }));
                ToolDefinition {
                    name,
                    description,
                    parameters,
                }
            })
            .collect();

        Ok(Self {
            transport,
            tool_definitions,
        })
    }

    /// Return the tool definitions discovered from Composio.
    pub fn tool_definitions(&self) -> &[ToolDefinition] {
        &self.tool_definitions
    }

    /// Call a single tool on the Composio MCP endpoint.
    async fn call_tool(&self, name: &str, arguments: &str) -> Result<String, String> {
        let args: serde_json::Value =
            serde_json::from_str(arguments).unwrap_or(serde_json::json!({}));

        let params = serde_json::json!({
            "name": name,
            "arguments": args,
        });

        let result = self
            .transport
            .request("tools/call", params)
            .await
            .map_err(|e| e.to_string())?;

        // MCP tools/call returns { content: [{ type: "text", text: "..." }] }
        if let Some(content) = result.get("content") {
            if let Some(arr) = content.as_array() {
                let texts: Vec<&str> = arr
                    .iter()
                    .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
                    .collect();
                if !texts.is_empty() {
                    return Ok(texts.join("\n"));
                }
            }
        }

        // Fallback: return raw JSON
        Ok(serde_json::to_string_pretty(&result).unwrap_or_default())
    }
}

#[cfg(all(feature = "http-api", feature = "composio"))]
#[async_trait]
impl ActionExecutor for ComposioToolExecutor {
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
                match self.call_tool(name, arguments).await {
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

#[cfg(test)]
#[cfg(all(feature = "http-api", feature = "composio"))]
mod tests {
    use super::*;

    #[test]
    fn test_tool_definition_parsing() {
        let raw = serde_json::json!([
            {
                "name": "TWITTER_CREATE_TWEET",
                "description": "Post a tweet to Twitter/X",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "text": { "type": "string", "description": "Tweet text" }
                    },
                    "required": ["text"]
                }
            }
        ]);

        let tools: Vec<serde_json::Value> = serde_json::from_value(raw).unwrap();
        let defs: Vec<ToolDefinition> = tools
            .into_iter()
            .map(|t| {
                let name = t["name"].as_str().unwrap_or("unknown").to_string();
                let description = t["description"].as_str().unwrap_or("").to_string();
                let parameters = t
                    .get("inputSchema")
                    .or_else(|| t.get("parameters"))
                    .cloned()
                    .unwrap_or(serde_json::json!({}));
                ToolDefinition {
                    name,
                    description,
                    parameters,
                }
            })
            .collect();

        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "TWITTER_CREATE_TWEET");
        assert!(defs[0].parameters["properties"]["text"].is_object());
    }

    #[test]
    fn test_mcp_content_extraction() {
        let result = serde_json::json!({
            "content": [
                { "type": "text", "text": "Tweet posted successfully" }
            ]
        });

        if let Some(content) = result.get("content") {
            if let Some(arr) = content.as_array() {
                let texts: Vec<&str> = arr
                    .iter()
                    .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
                    .collect();
                assert_eq!(texts, vec!["Tweet posted successfully"]);
            }
        }
    }
}
