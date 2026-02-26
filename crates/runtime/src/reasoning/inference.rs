//! Unified inference provider trait
//!
//! Defines the `InferenceProvider` trait that abstracts over cloud LLM APIs
//! and local SLM runners, adding tool calling and structured output support
//! on top of the existing `LlmClient` and `SlmRunner`.

use crate::reasoning::conversation::Conversation;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A tool definition that can be provided to an inference call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// Tool name (must match the name the LLM will use to call it).
    pub name: String,
    /// Human-readable description of what the tool does.
    pub description: String,
    /// JSON Schema describing the tool's parameters.
    pub parameters: serde_json::Value,
}

/// A tool call request returned by the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRequest {
    /// Unique identifier for this tool call.
    pub id: String,
    /// Name of the tool to invoke.
    pub name: String,
    /// JSON-encoded arguments for the tool.
    pub arguments: String,
}

/// The reason the model stopped generating.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    /// Model produced a complete response.
    Stop,
    /// Model wants to call one or more tools.
    ToolCalls,
    /// Generation was truncated due to max_tokens.
    MaxTokens,
    /// Generation was truncated due to content filter.
    ContentFilter,
}

/// Desired response format from the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ResponseFormat {
    /// Free-form text response.
    #[serde(rename = "text")]
    Text,
    /// JSON object response (model is instructed to return valid JSON).
    #[serde(rename = "json_object")]
    JsonObject,
    /// JSON response conforming to a specific schema.
    #[serde(rename = "json_schema")]
    JsonSchema {
        /// The JSON schema the response must conform to.
        schema: serde_json::Value,
        /// Optional name for the schema (used in API calls).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
}

/// Token usage information.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    /// Tokens in the prompt/input.
    pub prompt_tokens: u32,
    /// Tokens in the completion/output.
    pub completion_tokens: u32,
    /// Total tokens used.
    pub total_tokens: u32,
}

/// Options for an inference call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceOptions {
    /// Maximum tokens to generate.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// Sampling temperature (0.0 = deterministic, 1.0 = creative).
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    /// Tool definitions available for this call.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_definitions: Vec<ToolDefinition>,
    /// Desired response format.
    #[serde(default = "default_response_format")]
    pub response_format: ResponseFormat,
    /// Optional model override (provider decides default otherwise).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Additional provider-specific parameters.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub extra: HashMap<String, serde_json::Value>,
}

fn default_max_tokens() -> u32 {
    4096
}

fn default_temperature() -> f32 {
    0.3
}

fn default_response_format() -> ResponseFormat {
    ResponseFormat::Text
}

impl Default for InferenceOptions {
    fn default() -> Self {
        Self {
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
            tool_definitions: Vec::new(),
            response_format: ResponseFormat::Text,
            model: None,
            extra: HashMap::new(),
        }
    }
}

/// Response from an inference call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceResponse {
    /// Text content of the response.
    pub content: String,
    /// Tool calls requested by the model (empty if none).
    pub tool_calls: Vec<ToolCallRequest>,
    /// Why the model stopped generating.
    pub finish_reason: FinishReason,
    /// Token usage statistics.
    pub usage: Usage,
    /// The model ID that actually served the request.
    pub model: String,
}

impl InferenceResponse {
    /// Returns true if the model requested tool calls.
    pub fn has_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }
}

/// Errors that can occur during inference.
#[derive(Debug, thiserror::Error)]
pub enum InferenceError {
    #[error("Provider error: {0}")]
    Provider(String),

    #[error("Rate limited, retry after {retry_after_ms}ms")]
    RateLimited { retry_after_ms: u64 },

    #[error("Context window exceeded: {0} tokens requested, {1} available")]
    ContextOverflow(usize, usize),

    #[error("Model not available: {0}")]
    ModelUnavailable(String),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("Timeout after {0:?}")]
    Timeout(std::time::Duration),

    #[error("Response parse error: {0}")]
    ParseError(String),
}

/// Unified trait for inference providers (cloud LLMs and local SLMs).
///
/// Wraps existing `LlmClient` and `SlmRunner` to add:
/// - Multi-turn conversation support
/// - Tool calling
/// - Structured output (response_format)
/// - Token usage tracking
#[async_trait]
pub trait InferenceProvider: Send + Sync {
    /// Run inference on a conversation with the given options.
    async fn complete(
        &self,
        conversation: &Conversation,
        options: &InferenceOptions,
    ) -> Result<InferenceResponse, InferenceError>;

    /// Get the provider's name for logging and routing.
    fn provider_name(&self) -> &str;

    /// Get the default model ID for this provider.
    fn default_model(&self) -> &str;

    /// Check if this provider supports tool calling natively.
    fn supports_native_tools(&self) -> bool;

    /// Check if this provider supports structured output natively.
    fn supports_structured_output(&self) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inference_options_default() {
        let opts = InferenceOptions::default();
        assert_eq!(opts.max_tokens, 4096);
        assert!((opts.temperature - 0.3).abs() < f32::EPSILON);
        assert!(opts.tool_definitions.is_empty());
        assert!(matches!(opts.response_format, ResponseFormat::Text));
    }

    #[test]
    fn test_tool_definition_serde() {
        let tool = ToolDefinition {
            name: "web_search".into(),
            description: "Search the web".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                },
                "required": ["query"]
            }),
        };
        let json = serde_json::to_string(&tool).unwrap();
        let restored: ToolDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.name, "web_search");
    }

    #[test]
    fn test_response_format_serde() {
        let text = ResponseFormat::Text;
        let json = serde_json::to_string(&text).unwrap();
        assert!(json.contains("text"));

        let schema = ResponseFormat::JsonSchema {
            schema: serde_json::json!({"type": "object"}),
            name: Some("MySchema".into()),
        };
        let json = serde_json::to_string(&schema).unwrap();
        assert!(json.contains("json_schema"));
        assert!(json.contains("MySchema"));
    }

    #[test]
    fn test_inference_response_has_tool_calls() {
        let resp = InferenceResponse {
            content: String::new(),
            tool_calls: vec![ToolCallRequest {
                id: "tc_1".into(),
                name: "search".into(),
                arguments: "{}".into(),
            }],
            finish_reason: FinishReason::ToolCalls,
            usage: Usage::default(),
            model: "test".into(),
        };
        assert!(resp.has_tool_calls());

        let resp_no_tools = InferenceResponse {
            content: "Hello".into(),
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
            usage: Usage::default(),
            model: "test".into(),
        };
        assert!(!resp_no_tools.has_tool_calls());
    }

    #[test]
    fn test_finish_reason_serde() {
        let json = serde_json::to_string(&FinishReason::ToolCalls).unwrap();
        assert_eq!(json, "\"tool_calls\"");
        let restored: FinishReason = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, FinishReason::ToolCalls);
    }
}
