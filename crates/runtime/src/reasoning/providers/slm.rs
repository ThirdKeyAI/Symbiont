//! SLM inference provider
//!
//! Wraps the existing `SlmRunner` to implement `InferenceProvider`.
//! Since SLMs don't natively support tool calling or structured output,
//! this provider injects tool definitions and JSON schemas into the system
//! prompt and parses structured JSON from the text output.

use crate::models::runners::{ExecutionOptions, SlmRunner};
use crate::reasoning::conversation::Conversation;
use crate::reasoning::inference::*;
use async_trait::async_trait;
use std::sync::Arc;

/// SLM inference provider wrapping a `SlmRunner`.
pub struct SlmInferenceProvider {
    runner: Arc<dyn SlmRunner>,
    model_name: String,
}

impl SlmInferenceProvider {
    /// Create a new SLM provider from an existing runner.
    pub fn new(runner: Arc<dyn SlmRunner>, model_name: impl Into<String>) -> Self {
        Self {
            runner,
            model_name: model_name.into(),
        }
    }

    /// Build a single prompt string from a conversation, injecting tool
    /// definitions and response format instructions into the system prompt.
    fn build_prompt(conversation: &Conversation, options: &InferenceOptions) -> String {
        let mut parts = Vec::new();

        // Start with system message, augmented with tool/format instructions
        if let Some(sys) = conversation.system_message() {
            parts.push(format!("### System\n{}", sys.content));
        }

        // Inject tool definitions into the prompt
        if !options.tool_definitions.is_empty() {
            let mut tool_section = String::from("\n### Available Tools\nYou have access to the following tools. To call a tool, respond with a JSON object in this exact format:\n```json\n{\"tool_calls\": [{\"name\": \"<tool_name>\", \"arguments\": {<args>}}]}\n```\n\nTools:\n");
            for td in &options.tool_definitions {
                tool_section.push_str(&format!(
                    "- **{}**: {}\n  Parameters: {}\n",
                    td.name,
                    td.description,
                    serde_json::to_string_pretty(&td.parameters).unwrap_or_default()
                ));
            }
            tool_section
                .push_str("\nIf you don't need to call any tools, respond with plain text.\n");
            parts.push(tool_section);
        }

        // Inject response format instructions
        match &options.response_format {
            ResponseFormat::Text => {}
            ResponseFormat::JsonObject => {
                parts.push(
                    "\n### Response Format\nYou MUST respond with a valid JSON object. Do not include any text outside the JSON.".into(),
                );
            }
            ResponseFormat::JsonSchema { schema, .. } => {
                parts.push(format!(
                    "\n### Response Format\nYou MUST respond with a valid JSON object conforming to this schema:\n```json\n{}\n```\nDo not include any text outside the JSON.",
                    serde_json::to_string_pretty(schema).unwrap_or_default()
                ));
            }
        }

        // Add conversation history
        for msg in conversation.messages() {
            match msg.role {
                crate::reasoning::conversation::MessageRole::System => continue, // Already handled
                crate::reasoning::conversation::MessageRole::User => {
                    parts.push(format!("\n### User\n{}", msg.content));
                }
                crate::reasoning::conversation::MessageRole::Assistant => {
                    if !msg.tool_calls.is_empty() {
                        let tc_json: Vec<serde_json::Value> = msg
                            .tool_calls
                            .iter()
                            .map(|tc| {
                                serde_json::json!({
                                    "name": tc.name,
                                    "arguments": serde_json::from_str::<serde_json::Value>(&tc.arguments).unwrap_or(serde_json::json!({}))
                                })
                            })
                            .collect();
                        parts.push(format!(
                            "\n### Assistant\n```json\n{{\"tool_calls\": {}}}\n```",
                            serde_json::to_string(&tc_json).unwrap_or_default()
                        ));
                    } else {
                        parts.push(format!("\n### Assistant\n{}", msg.content));
                    }
                }
                crate::reasoning::conversation::MessageRole::Tool => {
                    let tool_name = msg.tool_name.as_deref().unwrap_or("unknown");
                    parts.push(format!(
                        "\n### Tool Result ({})\n{}",
                        tool_name, msg.content
                    ));
                }
            }
        }

        parts.push("\n### Assistant\n".into());
        parts.join("\n")
    }

    /// Attempt to extract tool calls from the SLM's text response.
    ///
    /// Looks for JSON blocks containing a `tool_calls` array, either bare
    /// or wrapped in markdown code fences.
    fn extract_tool_calls(text: &str) -> Vec<ToolCallRequest> {
        // Try to find JSON with tool_calls in the response
        let json_text = strip_markdown_fences(text);

        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&json_text) {
            if let Some(calls) = parsed.get("tool_calls").and_then(|c| c.as_array()) {
                return calls
                    .iter()
                    .enumerate()
                    .filter_map(|(i, call)| {
                        let name = call.get("name")?.as_str()?.to_string();
                        let arguments = call
                            .get("arguments")
                            .map(|a| serde_json::to_string(a).unwrap_or_default())
                            .unwrap_or_else(|| "{}".into());
                        Some(ToolCallRequest {
                            id: format!("slm_call_{}", i),
                            name,
                            arguments,
                        })
                    })
                    .collect();
            }
        }

        Vec::new()
    }
}

/// Strip markdown code fences from a string, returning the inner content.
pub fn strip_markdown_fences(text: &str) -> String {
    let trimmed = text.trim();

    // Handle ```json ... ``` or ``` ... ```
    if let Some(rest) = trimmed.strip_prefix("```") {
        // Skip the language tag on the first line
        let content = if let Some(idx) = rest.find('\n') {
            &rest[idx + 1..]
        } else {
            rest
        };
        if let Some(stripped) = content.strip_suffix("```") {
            return stripped.trim().to_string();
        }
        return content.trim().to_string();
    }

    trimmed.to_string()
}

#[async_trait]
impl InferenceProvider for SlmInferenceProvider {
    async fn complete(
        &self,
        conversation: &Conversation,
        options: &InferenceOptions,
    ) -> Result<InferenceResponse, InferenceError> {
        let prompt = Self::build_prompt(conversation, options);

        let exec_options = ExecutionOptions {
            timeout: Some(std::time::Duration::from_secs(60)),
            temperature: Some(options.temperature),
            max_tokens: Some(options.max_tokens),
            custom_parameters: Default::default(),
        };

        let result = self
            .runner
            .execute(&prompt, Some(exec_options))
            .await
            .map_err(|e| InferenceError::Provider(format!("SLM execution failed: {}", e)))?;

        let response_text = result.response.clone();
        let tool_calls = Self::extract_tool_calls(&response_text);

        let finish_reason = if !tool_calls.is_empty() {
            FinishReason::ToolCalls
        } else {
            FinishReason::Stop
        };

        let content = if !tool_calls.is_empty() {
            // If we extracted tool calls, the text content is whatever remains
            // outside the JSON block (may be empty)
            String::new()
        } else {
            response_text
        };

        let usage = Usage {
            prompt_tokens: result.metadata.input_tokens.unwrap_or(0),
            completion_tokens: result.metadata.output_tokens.unwrap_or(0),
            total_tokens: result
                .metadata
                .input_tokens
                .unwrap_or(0)
                .saturating_add(result.metadata.output_tokens.unwrap_or(0)),
        };

        Ok(InferenceResponse {
            content,
            tool_calls,
            finish_reason,
            usage,
            model: self.model_name.clone(),
        })
    }

    fn provider_name(&self) -> &str {
        "slm"
    }

    fn default_model(&self) -> &str {
        &self.model_name
    }

    fn supports_native_tools(&self) -> bool {
        false
    }

    fn supports_structured_output(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reasoning::conversation::ConversationMessage;

    #[test]
    fn test_strip_markdown_fences_json() {
        let input = "```json\n{\"tool_calls\": [{\"name\": \"search\", \"arguments\": {\"q\": \"test\"}}]}\n```";
        let result = strip_markdown_fences(input);
        assert!(result.starts_with('{'));
        assert!(result.ends_with('}'));
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(parsed.get("tool_calls").is_some());
    }

    #[test]
    fn test_strip_markdown_fences_plain() {
        let input = "```\n{\"key\": \"value\"}\n```";
        let result = strip_markdown_fences(input);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["key"], "value");
    }

    #[test]
    fn test_strip_markdown_fences_no_fences() {
        let input = "{\"key\": \"value\"}";
        let result = strip_markdown_fences(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_extract_tool_calls_valid() {
        let text = r#"```json
{"tool_calls": [{"name": "web_search", "arguments": {"query": "rust"}}]}
```"#;
        let calls = SlmInferenceProvider::extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web_search");
        assert_eq!(calls[0].id, "slm_call_0");
    }

    #[test]
    fn test_extract_tool_calls_no_tools() {
        let text = "I don't need any tools for this. The answer is 42.";
        let calls = SlmInferenceProvider::extract_tool_calls(text);
        assert!(calls.is_empty());
    }

    #[test]
    fn test_extract_tool_calls_multiple() {
        let text = r#"{"tool_calls": [
            {"name": "search", "arguments": {"q": "a"}},
            {"name": "read", "arguments": {"path": "/tmp/x"}}
        ]}"#;
        let calls = SlmInferenceProvider::extract_tool_calls(text);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "search");
        assert_eq!(calls[1].name, "read");
    }

    #[test]
    fn test_build_prompt_basic() {
        let mut conv = Conversation::with_system("You are helpful.");
        conv.push(ConversationMessage::user("What is 2+2?"));

        let opts = InferenceOptions::default();
        let prompt = SlmInferenceProvider::build_prompt(&conv, &opts);

        assert!(prompt.contains("### System"));
        assert!(prompt.contains("You are helpful."));
        assert!(prompt.contains("### User"));
        assert!(prompt.contains("What is 2+2?"));
        assert!(prompt.contains("### Assistant"));
    }

    #[test]
    fn test_build_prompt_with_tools() {
        let conv = Conversation::with_system("Agent");
        let opts = InferenceOptions {
            tool_definitions: vec![ToolDefinition {
                name: "search".into(),
                description: "Search the web".into(),
                parameters: serde_json::json!({"type": "object", "properties": {"q": {"type": "string"}}}),
            }],
            ..Default::default()
        };

        let prompt = SlmInferenceProvider::build_prompt(&conv, &opts);
        assert!(prompt.contains("### Available Tools"));
        assert!(prompt.contains("search"));
        assert!(prompt.contains("Search the web"));
        assert!(prompt.contains("tool_calls"));
    }

    #[test]
    fn test_build_prompt_with_json_schema() {
        let conv = Conversation::with_system("Agent");
        let opts = InferenceOptions {
            response_format: ResponseFormat::JsonSchema {
                schema: serde_json::json!({"type": "object", "properties": {"answer": {"type": "string"}}}),
                name: Some("Answer".into()),
            },
            ..Default::default()
        };

        let prompt = SlmInferenceProvider::build_prompt(&conv, &opts);
        assert!(prompt.contains("### Response Format"));
        assert!(prompt.contains("JSON object conforming to this schema"));
    }
}
