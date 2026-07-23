//! Cloud inference provider
//!
//! Wraps the existing `LlmClient` to implement `InferenceProvider` with
//! tool calling and structured output support across OpenAI, Anthropic,
//! and OpenRouter backends.

use crate::http_input::llm_client::{LlmClient, LlmProvider};
use crate::reasoning::conversation::Conversation;
use crate::reasoning::inference::*;
use async_trait::async_trait;

/// Map an Anthropic `stop_reason` string to a [`FinishReason`].
///
/// `has_tool_calls` is whether the parsed content produced at least one
/// `tool_use` block. A `"refusal"` (safety-classifier decline; Anthropic
/// returns it with HTTP 200) maps to [`FinishReason::Refusal`] so the loop can
/// fail over rather than reading a refused turn as an empty, successful stop.
fn map_anthropic_stop_reason(stop_reason: &str, has_tool_calls: bool) -> FinishReason {
    match stop_reason {
        "tool_use" => FinishReason::ToolCalls,
        "max_tokens" => FinishReason::MaxTokens,
        "refusal" => FinishReason::Refusal,
        // end_turn / stop_sequence / pause_turn / anything else: a turn that
        // still emitted tool calls is a tool turn; otherwise a plain stop.
        _ if has_tool_calls => FinishReason::ToolCalls,
        _ => FinishReason::Stop,
    }
}

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

    /// Like `from_env`, but also accepts an optional `SecretStore` so API
    /// keys can be sourced from HashiCorp Vault, OpenBao, or the file
    /// backend instead of env vars. See [`LlmClient::from_env_or_secrets`]
    /// for the resolution order and the `*_API_KEY_REF` env vars that
    /// point at secret-store keys. When `store` is `None` or no `*_REF` is
    /// configured, behaviour is identical to `from_env`.
    pub async fn from_env_or_secrets(
        store: Option<std::sync::Arc<dyn crate::secrets::SecretStore + Send + Sync>>,
    ) -> Option<Self> {
        LlmClient::from_env_or_secrets(store)
            .await
            .map(|c| Self { client: c })
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

            // OpenAI Chat Completions tool_choice. Only emit when an
            // explicit choice is supplied; absent the field, OpenAI
            // defaults to "auto".
            if let Some(choice) = &options.tool_choice {
                body["tool_choice"] = match choice {
                    crate::reasoning::inference::ToolChoice::Auto => {
                        serde_json::Value::String("auto".into())
                    }
                    crate::reasoning::inference::ToolChoice::Any => {
                        serde_json::Value::String("required".into())
                    }
                    crate::reasoning::inference::ToolChoice::Tool { name } => {
                        serde_json::json!({
                            "type": "function",
                            "function": {"name": name}
                        })
                    }
                };
            }
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

        // Prompt caching is a prefix match: bytes up to a `cache_control`
        // breakpoint are cached and reused verbatim, and any change
        // anywhere in that prefix invalidates it. Render order for the
        // Anthropic Messages API is tools -> system -> messages, so a
        // single breakpoint on the LAST block of the stable (tools+system)
        // prefix caches everything before it in one shot. We emit at most
        // one breakpoint here (the API caps requests at 4 total): on the
        // system block when a system prompt is present (system renders
        // after tools, so this covers both tools and system), otherwise on
        // the last tool so the tool definitions alone still cache.
        let has_system = system.is_some();

        if let Some(sys) = system {
            body["system"] = serde_json::json!([
                {
                    "type": "text",
                    "text": sys,
                    "cache_control": { "type": "ephemeral" }
                }
            ]);
        }

        // Add tools
        if !options.tool_definitions.is_empty() {
            let mut tools: Vec<serde_json::Value> = options
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

            // No system prompt to carry the breakpoint -- put it on the
            // last tool instead so the tool-definitions prefix still caches.
            if !has_system {
                if let Some(last_tool) = tools.last_mut().and_then(|t| t.as_object_mut()) {
                    last_tool.insert(
                        "cache_control".to_string(),
                        serde_json::json!({"type": "ephemeral"}),
                    );
                }
            }

            body["tools"] = serde_json::Value::Array(tools);

            // Anthropic Messages API tool_choice. We emit it only when
            // an explicit choice is set; absent the field, Anthropic
            // defaults to {"type":"auto"}.
            if let Some(choice) = &options.tool_choice {
                body["tool_choice"] = match choice {
                    crate::reasoning::inference::ToolChoice::Auto => {
                        serde_json::json!({"type": "auto"})
                    }
                    crate::reasoning::inference::ToolChoice::Any => {
                        serde_json::json!({"type": "any"})
                    }
                    crate::reasoning::inference::ToolChoice::Tool { name } => {
                        serde_json::json!({"type": "tool", "name": name})
                    }
                };
            }
        }

        // Forward provider-specific extras (e.g. Anthropic's `output_config`
        // for effort, `thinking`, etc.) after the body is otherwise fully
        // built. This is the enabling mechanism for passing Anthropic-only
        // params without a code change here; `extra` is applied last and so
        // overrides any same-named key set above, by design. Deliberately
        // does NOT hardcode a default `effort` or `thinking` -- this method
        // is shared by every Anthropic-routed call, and a hardcoded default
        // would silently change behavior for callers that don't want it.
        for (k, v) in &options.extra {
            body[k] = v.clone();
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

        let finish_reason = map_anthropic_stop_reason(stop_reason, !tool_calls.is_empty());

        // A turn that produced neither text nor a tool call is a no-progress
        // turn (e.g. a thinking-only / redacted-thinking-only turn, or a
        // block type we don't parse). It's indistinguishable downstream from a
        // deliberate empty stop, so warn to make loop-termination diagnosable.
        if finish_reason == FinishReason::Refusal {
            tracing::warn!(
                "Anthropic response was a refusal (stop_reason=refusal); returning FinishReason::Refusal"
            );
        } else if text_content.is_empty() && tool_calls.is_empty() {
            tracing::warn!(
                "Anthropic response produced no text and no tool calls (stop_reason={}); \
                 the turn made no progress — likely a thinking-only turn or an unparsed block type",
                stop_reason
            );
        }

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

        // Bedrock signs its own requests with SigV4; route through the shared
        // LlmClient Converse path rather than the hand-built HTTP path below.
        // Use the caller-supplied temperature and max_tokens so the reasoning
        // loop's configured values are honoured (rather than fixed defaults).
        #[cfg(feature = "bedrock")]
        if matches!(self.client.provider(), LlmProvider::Bedrock) {
            let (system_opt, messages) = conversation.to_anthropic_messages();
            let system = system_opt.as_deref().unwrap_or("");
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
            let resp_json = self
                .client
                .bedrock_converse(
                    system,
                    &messages,
                    &tools,
                    options.temperature,
                    options.max_tokens,
                )
                .await
                .map_err(|e| InferenceError::Provider(format!("Bedrock Converse error: {e}")))?;
            return self.parse_anthropic_response(&resp_json, model);
        }

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

        // The base URL and API key were resolved once at LlmClient
        // construction (env or secret store) — reuse the cached values
        // instead of re-reading the env on every request.
        let base = self.client.base_url();
        let api_key = self.client.api_key();

        let (url, request_builder) = if is_anthropic {
            let url = format!("{}/messages", base);
            let rb = http_client
                .post(&url)
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body);
            (url, rb)
        } else {
            let url = format!("{}/chat/completions", base);
            let mut rb = http_client
                .post(&url)
                .header("authorization", format!("Bearer {}", api_key))
                .header("content-type", "application/json");
            if matches!(self.client.provider(), LlmProvider::OpenRouter) {
                for (k, v) in crate::http_input::llm_client::openrouter_attribution_headers() {
                    rb = rb.header(k, v);
                }
            }
            let rb = rb.json(&body);
            (url, rb)
        };

        tracing::debug!(
            "Cloud inference: provider={} model={} url={}",
            self.provider_name(),
            model,
            url
        );
        // Debug-level fingerprint of the request body. Useful for
        // diagnosing why an agent terminates early (missing tool_choice,
        // empty messages array, etc.). Enable with RUST_LOG=symbi_runtime=debug.
        tracing::debug!(
            "Cloud request fingerprint: tool_choice={} tools={} system_chars={} msg_count={}",
            body.get("tool_choice")
                .map(|v| v.to_string())
                .unwrap_or_else(|| "<absent>".into()),
            body.get("tools")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0),
            body.get("system")
                .and_then(|v| v.as_str())
                .map(|s| s.len())
                .unwrap_or(0),
            body.get("messages")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0),
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
            tracing::warn!(
                "Cloud API non-success: status={} body={}",
                status,
                error_text.chars().take(400).collect::<String>()
            );
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
        // Debug-level response-shape log. Pairs with the request
        // fingerprint above for diagnosing loop termination.
        if is_anthropic {
            let stop = resp_json
                .get("stop_reason")
                .and_then(|v| v.as_str())
                .unwrap_or("<absent>");
            let content_types: Vec<&str> = resp_json
                .get("content")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|c| c.get("type").and_then(|t| t.as_str()))
                        .collect()
                })
                .unwrap_or_default();
            tracing::debug!(
                "Cloud response fingerprint: stop_reason={} content_types={:?}",
                stop,
                content_types
            );
        }

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
            #[cfg(feature = "bedrock")]
            LlmProvider::Bedrock => "bedrock",
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
    use serial_test::serial;

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
    fn test_map_anthropic_stop_reason() {
        use super::map_anthropic_stop_reason;

        // A refusal is surfaced distinctly — not collapsed into Stop — so the
        // loop can fail over instead of reading it as an empty completion.
        assert_eq!(
            map_anthropic_stop_reason("refusal", false),
            FinishReason::Refusal
        );
        // Even a refused turn that somehow carried tool calls stays a Refusal.
        assert_eq!(
            map_anthropic_stop_reason("refusal", true),
            FinishReason::Refusal
        );
        // Known terminal/tool reasons map as before.
        assert_eq!(
            map_anthropic_stop_reason("tool_use", false),
            FinishReason::ToolCalls
        );
        assert_eq!(
            map_anthropic_stop_reason("max_tokens", false),
            FinishReason::MaxTokens
        );
        // end_turn with tool calls is a tool turn; without, a plain stop.
        assert_eq!(
            map_anthropic_stop_reason("end_turn", true),
            FinishReason::ToolCalls
        );
        assert_eq!(
            map_anthropic_stop_reason("end_turn", false),
            FinishReason::Stop
        );
        // A thinking-only turn (no text, no tool calls) reports stop_reason
        // end_turn and no tool calls -> Stop (the parser separately warns).
        assert_eq!(
            map_anthropic_stop_reason("pause_turn", false),
            FinishReason::Stop
        );
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

    /// FIX 1 + FIX 2: `build_anthropic_body` emits a single prompt-cache
    /// breakpoint on the byte-stable prefix, and forwards `options.extra`
    /// into the request body verbatim.
    ///
    /// Constructing a `CloudInferenceProvider` requires a real `LlmClient`,
    /// which is only buildable via `from_env()` (no bypass constructor
    /// exists) -- so this follows the same env-var-dance + `#[serial]`
    /// pattern as `test_cloud_provider_name_bedrock` below.
    #[serial]
    #[test]
    fn test_build_anthropic_body_cache_control_and_extra() {
        use crate::reasoning::inference::{InferenceOptions, ToolDefinition};

        std::env::set_var("ANTHROPIC_API_KEY", "test-key-not-real");
        for k in ["OPENROUTER_API_KEY", "OPENAI_API_KEY", "BEDROCK_MODEL_ID"] {
            std::env::remove_var(k);
        }

        let provider = CloudInferenceProvider::from_env()
            .expect("CloudInferenceProvider should resolve via ANTHROPIC_API_KEY");

        // Case 1: system prompt present -> the breakpoint goes on the
        // system block, emitted as a content-block array (not a bare
        // string), and extra params are forwarded into the body.
        let conv = Conversation::with_system("You are a helpful assistant.");
        let mut options = InferenceOptions {
            tool_definitions: vec![ToolDefinition {
                name: "search".into(),
                description: "Search the web".into(),
                parameters: serde_json::json!({"type": "object", "properties": {}}),
            }],
            ..Default::default()
        };
        options.extra.insert(
            "output_config".into(),
            serde_json::json!({"effort": "high"}),
        );

        let body = provider.build_anthropic_body(&conv, &options);

        let system = body["system"]
            .as_array()
            .expect("system should be a content-block array carrying cache_control");
        assert_eq!(system.len(), 1);
        assert_eq!(system[0]["type"], "text");
        assert_eq!(system[0]["text"], "You are a helpful assistant.");
        assert_eq!(system[0]["cache_control"]["type"], "ephemeral");

        // At most one breakpoint from this method: since system carried it,
        // the tool must not also carry one.
        let tools = body["tools"].as_array().expect("tools array");
        assert!(tools[0].get("cache_control").is_none());

        // FIX 2: `extra` lands in the body verbatim.
        assert_eq!(body["output_config"]["effort"], "high");

        // Case 2: no system prompt, tools present -> the breakpoint goes on
        // the LAST tool instead, so the tool-definitions prefix still caches.
        let conv_no_system = Conversation::new();
        let options_no_system = InferenceOptions {
            tool_definitions: vec![
                ToolDefinition {
                    name: "first_tool".into(),
                    description: "d1".into(),
                    parameters: serde_json::json!({"type": "object", "properties": {}}),
                },
                ToolDefinition {
                    name: "last_tool".into(),
                    description: "d2".into(),
                    parameters: serde_json::json!({"type": "object", "properties": {}}),
                },
            ],
            ..Default::default()
        };
        let body_no_system = provider.build_anthropic_body(&conv_no_system, &options_no_system);
        assert!(body_no_system.get("system").is_none());
        let tools_no_system = body_no_system["tools"].as_array().expect("tools array");
        assert_eq!(tools_no_system.len(), 2);
        assert!(tools_no_system[0].get("cache_control").is_none());
        assert_eq!(tools_no_system[1]["cache_control"]["type"], "ephemeral");

        std::env::remove_var("ANTHROPIC_API_KEY");
    }

    /// Verify that a Bedrock-configured `CloudInferenceProvider` reports
    /// `provider_name() == "bedrock"` without making any network calls.
    #[cfg(feature = "bedrock")]
    #[serial]
    #[test]
    fn test_cloud_provider_name_bedrock() {
        // Temporarily set the env vars used by `CloudInferenceProvider::from_env`.
        std::env::set_var(
            "BEDROCK_MODEL_ID",
            "anthropic.claude-3-5-sonnet-20241022-v2:0",
        );
        std::env::set_var("AWS_REGION", "us-east-1");
        for k in ["OPENROUTER_API_KEY", "OPENAI_API_KEY", "ANTHROPIC_API_KEY"] {
            std::env::remove_var(k);
        }
        let provider = CloudInferenceProvider::from_env()
            .expect("CloudInferenceProvider should resolve via BEDROCK_MODEL_ID");
        assert_eq!(provider.provider_name(), "bedrock");
        std::env::remove_var("BEDROCK_MODEL_ID");
        std::env::remove_var("AWS_REGION");
    }
}
