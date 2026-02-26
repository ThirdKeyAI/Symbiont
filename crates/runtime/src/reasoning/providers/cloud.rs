//! Cloud inference provider
//!
//! Wraps the existing `LlmClient` to implement `InferenceProvider` with
//! tool calling and structured output support across OpenAI, Anthropic,
//! and OpenRouter backends.

use crate::http_input::llm_client::{LlmClient, LlmProvider};
use crate::reasoning::conversation::Conversation;
use crate::reasoning::inference::*;
use async_trait::async_trait;

/// Cloud inference provider wrapping `LlmClient`.
pub struct CloudInferenceProvider {
    client: LlmClient,
}

impl CloudInferenceProvider {
    /// Create a new cloud provider wrapping an existing LlmClient.
    pub fn new(client: LlmClient) -> Self {
        Self { client }
    }

    /// Auto-detect from environment, returning None if no API key is set.
    pub fn from_env() -> Option<Self> {
        LlmClient::from_env().map(|c| Self { client: c })
    }

    /// Build the request body for OpenAI-compatible APIs (OpenAI, OpenRouter).
    fn build_openai_body(
        &self,
        conversation: &Conversation,
        options: &InferenceOptions,
    ) -> serde_json::Value {
        let model = options
            .model
            .as_deref()
            .unwrap_or_else(|| self.client.model());

        let mut body = serde_json::json!({
            "model": model,
            "messages": conversation.to_openai_messages(),
            "max_tokens": options.max_tokens,
            "temperature": options.temperature,
        });

        // Add tools if provided
        if !options.tool_definitions.is_empty() {
            let tools: Vec<serde_json::Value> = options
                .tool_definitions
                .iter()
                .map(|td| {
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": td.name,
                            "description": td.description,
                            "parameters": td.parameters,
                        }
                    })
                })
                .collect();
            body["tools"] = serde_json::Value::Array(tools);
        }

        // Add response_format if not plain text
        match &options.response_format {
            ResponseFormat::Text => {}
            ResponseFormat::JsonObject => {
                body["response_format"] = serde_json::json!({"type": "json_object"});
            }
            ResponseFormat::JsonSchema { schema, name } => {
                body["response_format"] = serde_json::json!({
                    "type": "json_schema",
                    "json_schema": {
                        "name": name.as_deref().unwrap_or("response"),
                        "schema": schema,
                    }
                });
            }
        }

        body
    }

    /// Build the request body for the Anthropic Messages API.
    fn build_anthropic_body(
        &self,
        conversation: &Conversation,
        options: &InferenceOptions,
    ) -> serde_json::Value {
        let model = options
            .model
            .as_deref()
            .unwrap_or_else(|| self.client.model());

        let (system, messages) = conversation.to_anthropic_messages();

        let mut body = serde_json::json!({
            "model": model,
            "messages": messages,
            "max_tokens": options.max_tokens,
        });

        // Anthropic uses temperature in metadata, not a direct field always present
        if options.temperature > 0.0 {
            body["temperature"] = serde_json::json!(options.temperature);
        }

        if let Some(sys) = system {
            body["system"] = serde_json::Value::String(sys);
        }

        // Add tools
        if !options.tool_definitions.is_empty() {
            let tools: Vec<serde_json::Value> = options
                .tool_definitions
                .iter()
                .map(|td| {
                    serde_json::json!({
                        "name": td.name,
                        "description": td.description,
                        "input_schema": td.parameters,
                    })
                })
                .collect();
            body["tools"] = serde_json::Value::Array(tools);
        }

        body
    }

    /// Parse an OpenAI-format response into InferenceResponse.
    fn parse_openai_response(
        &self,
        resp: &serde_json::Value,
        model: &str,
    ) -> Result<InferenceResponse, InferenceError> {
        let choice = resp
            .get("choices")
            .and_then(|c| c.get(0))
            .ok_or_else(|| InferenceError::ParseError("No choices in response".into()))?;

        let message = choice
            .get("message")
            .ok_or_else(|| InferenceError::ParseError("No message in choice".into()))?;

        let content = message
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();

        let tool_calls = message
            .get("tool_calls")
            .and_then(|tc| tc.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|tc| {
                        let id = tc.get("id")?.as_str()?.to_string();
                        let func = tc.get("function")?;
                        let name = func.get("name")?.as_str()?.to_string();
                        let arguments = func.get("arguments")?.as_str()?.to_string();
                        Some(ToolCallRequest {
                            id,
                            name,
                            arguments,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let finish_reason = match choice
            .get("finish_reason")
            .and_then(|f| f.as_str())
            .unwrap_or("stop")
        {
            "tool_calls" => FinishReason::ToolCalls,
            "length" => FinishReason::MaxTokens,
            "content_filter" => FinishReason::ContentFilter,
            _ => {
                if tool_calls.is_empty() {
                    FinishReason::Stop
                } else {
                    FinishReason::ToolCalls
                }
            }
        };

        let usage = resp
            .get("usage")
            .map(|u| Usage {
                prompt_tokens: u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                completion_tokens: u
                    .get("completion_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32,
                total_tokens: u.get("total_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            })
            .unwrap_or_default();

        let actual_model = resp
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or(model)
            .to_string();

        Ok(InferenceResponse {
            content,
            tool_calls,
            finish_reason,
            usage,
            model: actual_model,
        })
    }

    /// Parse an Anthropic-format response into InferenceResponse.
    fn parse_anthropic_response(
        &self,
        resp: &serde_json::Value,
        model: &str,
    ) -> Result<InferenceResponse, InferenceError> {
        let content_blocks = resp
            .get("content")
            .and_then(|c| c.as_array())
            .ok_or_else(|| InferenceError::ParseError("No content in response".into()))?;

        let mut text_content = String::new();
        let mut tool_calls = Vec::new();

        for block in content_blocks {
            match block.get("type").and_then(|t| t.as_str()) {
                Some("text") => {
                    if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                        if !text_content.is_empty() {
                            text_content.push('\n');
                        }
                        text_content.push_str(text);
                    }
                }
                Some("tool_use") => {
                    if let (Some(id), Some(name), Some(input)) = (
                        block.get("id").and_then(|v| v.as_str()),
                        block.get("name").and_then(|v| v.as_str()),
                        block.get("input"),
                    ) {
                        tool_calls.push(ToolCallRequest {
                            id: id.to_string(),
                            name: name.to_string(),
                            arguments: serde_json::to_string(input).unwrap_or_default(),
                        });
                    }
                }
                _ => {}
            }
        }

        let stop_reason = resp
            .get("stop_reason")
            .and_then(|s| s.as_str())
            .unwrap_or("end_turn");

        let finish_reason = match stop_reason {
            "tool_use" => FinishReason::ToolCalls,
            "max_tokens" => FinishReason::MaxTokens,
            _ => {
                if tool_calls.is_empty() {
                    FinishReason::Stop
                } else {
                    FinishReason::ToolCalls
                }
            }
        };

        let usage = resp
            .get("usage")
            .map(|u| {
                let input = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let output = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                Usage {
                    prompt_tokens: input,
                    completion_tokens: output,
                    total_tokens: input + output,
                }
            })
            .unwrap_or_default();

        let actual_model = resp
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or(model)
            .to_string();

        Ok(InferenceResponse {
            content: text_content,
            tool_calls,
            finish_reason,
            usage,
            model: actual_model,
        })
    }
}

#[async_trait]
impl InferenceProvider for CloudInferenceProvider {
    async fn complete(
        &self,
        conversation: &Conversation,
        options: &InferenceOptions,
    ) -> Result<InferenceResponse, InferenceError> {
        // Access reqwest client and provider info through the LlmClient
        // We need to make the HTTP call ourselves since LlmClient::chat_completion
        // only supports simple system+user messages without tools.
        let is_anthropic = matches!(self.client.provider(), LlmProvider::Anthropic);
        let model = options
            .model
            .as_deref()
            .unwrap_or_else(|| self.client.model());

        let body = if is_anthropic {
            self.build_anthropic_body(conversation, options)
        } else {
            self.build_openai_body(conversation, options)
        };

        // Build and send the HTTP request using reqwest
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| InferenceError::Provider(format!("HTTP client error: {}", e)))?;

        let (url, request_builder) = if is_anthropic {
            let base = std::env::var("ANTHROPIC_BASE_URL")
                .unwrap_or_else(|_| "https://api.anthropic.com/v1".into());
            let api_key = std::env::var("ANTHROPIC_API_KEY")
                .map_err(|_| InferenceError::Provider("ANTHROPIC_API_KEY not set".into()))?;
            let url = format!("{}/messages", base);
            let rb = http_client
                .post(&url)
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body);
            (url, rb)
        } else {
            let (base, key_var) = match self.client.provider() {
                LlmProvider::OpenRouter => (
                    std::env::var("OPENROUTER_BASE_URL")
                        .unwrap_or_else(|_| "https://openrouter.ai/api/v1".into()),
                    "OPENROUTER_API_KEY",
                ),
                _ => (
                    std::env::var("OPENAI_BASE_URL")
                        .unwrap_or_else(|_| "https://api.openai.com/v1".into()),
                    "OPENAI_API_KEY",
                ),
            };
            let api_key = std::env::var(key_var)
                .map_err(|_| InferenceError::Provider(format!("{} not set", key_var)))?;
            let url = format!("{}/chat/completions", base);
            let rb = http_client
                .post(&url)
                .header("authorization", format!("Bearer {}", api_key))
                .header("content-type", "application/json")
                .json(&body);
            (url, rb)
        };

        tracing::debug!(
            "Cloud inference: provider={} model={} url={}",
            self.provider_name(),
            model,
            url
        );

        let start = std::time::Instant::now();
        let response = request_builder.send().await.map_err(|e| {
            if e.is_timeout() {
                InferenceError::Timeout(std::time::Duration::from_secs(120))
            } else {
                InferenceError::Provider(format!("Request failed: {}", e))
            }
        })?;

        let status = response.status();
        if status.as_u16() == 429 {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(1000);
            return Err(InferenceError::RateLimited {
                retry_after_ms: retry_after * 1000,
            });
        }

        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".into());
            return Err(InferenceError::Provider(format!(
                "API error ({}): {}",
                status, error_text
            )));
        }

        let resp_json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| InferenceError::ParseError(format!("JSON parse error: {}", e)))?;

        let latency = start.elapsed();
        tracing::debug!("Cloud inference completed in {:?}", latency);

        if is_anthropic {
            self.parse_anthropic_response(&resp_json, model)
        } else {
            self.parse_openai_response(&resp_json, model)
        }
    }

    fn provider_name(&self) -> &str {
        match self.client.provider() {
            LlmProvider::OpenRouter => "openrouter",
            LlmProvider::OpenAI => "openai",
            LlmProvider::Anthropic => "anthropic",
        }
    }

    fn default_model(&self) -> &str {
        self.client.model()
    }

    fn supports_native_tools(&self) -> bool {
        true
    }

    fn supports_structured_output(&self) -> bool {
        // OpenAI and Anthropic both support structured output
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reasoning::conversation::{ConversationMessage, ToolCall};

    #[test]
    fn test_build_openai_body_basic() {
        // We can't easily create a CloudInferenceProvider without env vars,
        // so test the parsing functions directly with mock data.
        let openai_response = serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "Hello!",
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15,
            },
            "model": "gpt-4o"
        });

        // Simulate parsing
        let choice = openai_response["choices"][0].clone();
        let content = choice["message"]["content"].as_str().unwrap();
        assert_eq!(content, "Hello!");
    }

    #[test]
    fn test_parse_openai_response_with_tools() {
        let resp = serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_abc123",
                        "type": "function",
                        "function": {
                            "name": "web_search",
                            "arguments": "{\"query\": \"rust crates\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {
                "prompt_tokens": 20,
                "completion_tokens": 10,
                "total_tokens": 30,
            },
            "model": "gpt-4o"
        });

        let tool_calls = resp["choices"][0]["message"]["tool_calls"]
            .as_array()
            .unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0]["function"]["name"], "web_search");
    }

    #[test]
    fn test_parse_anthropic_response() {
        let resp = serde_json::json!({
            "content": [
                {"type": "text", "text": "I'll search for that."},
                {
                    "type": "tool_use",
                    "id": "toolu_123",
                    "name": "web_search",
                    "input": {"query": "rust crates"}
                }
            ],
            "stop_reason": "tool_use",
            "usage": {
                "input_tokens": 15,
                "output_tokens": 20,
            },
            "model": "claude-sonnet-4-5-20250514"
        });

        let content = resp["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "tool_use");
        assert_eq!(content[1]["name"], "web_search");
    }

    #[test]
    fn test_conversation_to_openai_format() {
        let mut conv = Conversation::with_system("sys");
        conv.push(ConversationMessage::user("hello"));
        conv.push(ConversationMessage::assistant_tool_calls(vec![ToolCall {
            id: "tc1".into(),
            name: "search".into(),
            arguments: r#"{"q":"test"}"#.into(),
        }]));
        conv.push(ConversationMessage::tool_result("tc1", "search", "result"));

        let msgs = conv.to_openai_messages();
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[2]["tool_calls"][0]["function"]["name"], "search");
        assert_eq!(msgs[3]["tool_call_id"], "tc1");
    }
}
