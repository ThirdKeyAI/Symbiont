//! Multi-turn conversation management
//!
//! Provides a `Conversation` type that manages a sequence of messages
//! across System, User, Assistant, ToolCall, and ToolResult roles.
//! Supports serialization to OpenAI and Anthropic API formats and
//! token estimation for context window management.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Role of a message in a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

/// A single tool call embedded in an assistant message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Unique identifier for this tool call (used to correlate with results).
    pub id: String,
    /// Name of the tool being called.
    pub name: String,
    /// JSON-encoded arguments for the tool.
    pub arguments: String,
}

/// A single message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    /// The role of the message sender.
    pub role: MessageRole,
    /// Text content of the message (may be empty for pure tool-call messages).
    pub content: String,
    /// Tool calls made by the assistant (only present when role is Assistant).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// The tool call ID this message is responding to (only present when role is Tool).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// The tool name this result corresponds to (only present when role is Tool).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

impl ConversationMessage {
    /// Create a system message.
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            tool_name: None,
        }
    }

    /// Create a user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            tool_name: None,
        }
    }

    /// Create an assistant message with text content.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            tool_name: None,
        }
    }

    /// Create an assistant message with tool calls.
    pub fn assistant_tool_calls(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: String::new(),
            tool_calls,
            tool_call_id: None,
            tool_name: None,
        }
    }

    /// Create a tool result message.
    pub fn tool_result(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            role: MessageRole::Tool,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
            tool_name: Some(tool_name.into()),
        }
    }

    /// Estimate token count for this message using the ~4 chars/token heuristic.
    /// For accurate counts, use a tokenizer; this is sufficient for budget enforcement.
    pub fn estimate_tokens(&self) -> usize {
        let mut chars = self.content.len();
        for tc in &self.tool_calls {
            chars += tc.name.len() + tc.arguments.len();
        }
        // ~4 characters per token, plus overhead for message framing
        (chars / 4).max(1) + 4
    }
}

/// An ordered sequence of conversation messages with serialization helpers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Conversation {
    messages: Vec<ConversationMessage>,
}

impl Conversation {
    /// Create a new empty conversation.
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
        }
    }

    /// Create a conversation with a system message.
    pub fn with_system(system_prompt: impl Into<String>) -> Self {
        Self {
            messages: vec![ConversationMessage::system(system_prompt)],
        }
    }

    /// Append a message to the conversation.
    pub fn push(&mut self, message: ConversationMessage) {
        self.messages.push(message);
    }

    /// Get the messages in the conversation.
    pub fn messages(&self) -> &[ConversationMessage] {
        &self.messages
    }

    /// Get the number of messages.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Check if the conversation is empty.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Estimate total token count across all messages.
    pub fn estimate_tokens(&self) -> usize {
        self.messages.iter().map(|m| m.estimate_tokens()).sum()
    }

    /// Get the system message if present (first message with System role).
    pub fn system_message(&self) -> Option<&ConversationMessage> {
        self.messages.iter().find(|m| m.role == MessageRole::System)
    }

    /// Get the last assistant message.
    pub fn last_assistant_message(&self) -> Option<&ConversationMessage> {
        self.messages
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::Assistant)
    }

    /// Serialize to OpenAI chat completions format.
    ///
    /// Produces a JSON array of message objects with `role`, `content`,
    /// and optionally `tool_calls` or `tool_call_id` fields.
    pub fn to_openai_messages(&self) -> Vec<serde_json::Value> {
        self.messages
            .iter()
            .map(|msg| {
                let mut obj = serde_json::Map::new();
                let role_str = match msg.role {
                    MessageRole::System => "system",
                    MessageRole::User => "user",
                    MessageRole::Assistant => "assistant",
                    MessageRole::Tool => "tool",
                };
                obj.insert("role".into(), serde_json::Value::String(role_str.into()));

                if !msg.content.is_empty() {
                    obj.insert(
                        "content".into(),
                        serde_json::Value::String(msg.content.clone()),
                    );
                } else if msg.role != MessageRole::Assistant {
                    // OpenAI requires content for non-assistant messages
                    obj.insert("content".into(), serde_json::Value::String(String::new()));
                }

                if !msg.tool_calls.is_empty() {
                    let tool_calls: Vec<serde_json::Value> = msg
                        .tool_calls
                        .iter()
                        .map(|tc| {
                            serde_json::json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.name,
                                    "arguments": tc.arguments,
                                }
                            })
                        })
                        .collect();
                    obj.insert("tool_calls".into(), serde_json::Value::Array(tool_calls));
                }

                if let Some(ref id) = msg.tool_call_id {
                    obj.insert("tool_call_id".into(), serde_json::Value::String(id.clone()));
                }

                serde_json::Value::Object(obj)
            })
            .collect()
    }

    /// Serialize to Anthropic Messages API format.
    ///
    /// Returns `(system_prompt, messages)` because Anthropic takes the system
    /// message as a separate top-level field.
    pub fn to_anthropic_messages(&self) -> (Option<String>, Vec<serde_json::Value>) {
        let system = self
            .messages
            .iter()
            .find(|m| m.role == MessageRole::System)
            .map(|m| m.content.clone());

        let messages = self
            .messages
            .iter()
            .filter(|m| m.role != MessageRole::System)
            .map(|msg| {
                let role_str = match msg.role {
                    MessageRole::User | MessageRole::Tool => "user",
                    MessageRole::Assistant => "assistant",
                    MessageRole::System => unreachable!(),
                };

                if msg.role == MessageRole::Tool {
                    // Anthropic tool results go as user messages with tool_result content blocks
                    serde_json::json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": msg.tool_call_id.as_deref().unwrap_or(""),
                            "content": msg.content,
                        }]
                    })
                } else if !msg.tool_calls.is_empty() {
                    // Assistant message with tool use
                    let mut content_blocks: Vec<serde_json::Value> = Vec::new();
                    if !msg.content.is_empty() {
                        content_blocks.push(serde_json::json!({
                            "type": "text",
                            "text": msg.content,
                        }));
                    }
                    for tc in &msg.tool_calls {
                        let args: serde_json::Value =
                            serde_json::from_str(&tc.arguments).unwrap_or(serde_json::json!({}));
                        content_blocks.push(serde_json::json!({
                            "type": "tool_use",
                            "id": tc.id,
                            "name": tc.name,
                            "input": args,
                        }));
                    }
                    serde_json::json!({
                        "role": role_str,
                        "content": content_blocks,
                    })
                } else {
                    serde_json::json!({
                        "role": role_str,
                        "content": msg.content,
                    })
                }
            })
            .collect();

        (system, messages)
    }

    /// Truncate the conversation to fit within a token budget.
    ///
    /// Preserves the system message and the most recent messages.
    /// Removes older messages from the middle until the budget is met.
    pub fn truncate_to_budget(&mut self, max_tokens: usize) {
        if self.estimate_tokens() <= max_tokens {
            return;
        }

        let system_msg = if self
            .messages
            .first()
            .is_some_and(|m| m.role == MessageRole::System)
        {
            Some(self.messages[0].clone())
        } else {
            None
        };

        let system_tokens = system_msg.as_ref().map_or(0, |m| m.estimate_tokens());
        let remaining_budget = max_tokens.saturating_sub(system_tokens);

        // Keep messages from the end until we exceed the budget
        let start_idx = if system_msg.is_some() { 1 } else { 0 };
        let non_system: Vec<ConversationMessage> = self.messages.drain(start_idx..).rev().collect();

        let mut kept = Vec::new();
        let mut used_tokens = 0;
        for msg in non_system {
            let msg_tokens = msg.estimate_tokens();
            if used_tokens + msg_tokens > remaining_budget {
                break;
            }
            used_tokens += msg_tokens;
            kept.push(msg);
        }
        kept.reverse();

        self.messages.clear();
        if let Some(sys) = system_msg {
            self.messages.push(sys);
        }
        self.messages.extend(kept);
    }

    /// Insert a system-level knowledge context message after the initial system message.
    /// If a previous knowledge context exists (identified by a marker prefix), replace it.
    pub fn inject_knowledge_context(&mut self, context: impl Into<String>) {
        let marker = "[KNOWLEDGE_CONTEXT]";
        let content = format!("{}\n{}", marker, context.into());
        let msg = ConversationMessage::system(content);

        // Find and replace existing knowledge context, or insert after system message
        if let Some(pos) = self
            .messages
            .iter()
            .position(|m| m.role == MessageRole::System && m.content.starts_with(marker))
        {
            self.messages[pos] = msg;
        } else {
            // Insert after first system message (position 1), or at 0 if no system msg
            let insert_pos = if self
                .messages
                .first()
                .is_some_and(|m| m.role == MessageRole::System)
            {
                1
            } else {
                0
            };
            self.messages.insert(insert_pos, msg);
        }
    }

    /// Get metadata about the conversation for logging.
    pub fn metadata(&self) -> HashMap<String, String> {
        let mut meta = HashMap::new();
        meta.insert("message_count".into(), self.messages.len().to_string());
        meta.insert(
            "estimated_tokens".into(),
            self.estimate_tokens().to_string(),
        );
        meta.insert(
            "has_system".into(),
            self.system_message().is_some().to_string(),
        );
        let tool_call_count: usize = self.messages.iter().map(|m| m.tool_calls.len()).sum();
        meta.insert("tool_call_count".into(), tool_call_count.to_string());
        let tool_result_count = self
            .messages
            .iter()
            .filter(|m| m.role == MessageRole::Tool)
            .count();
        meta.insert("tool_result_count".into(), tool_result_count.to_string());
        meta
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conversation_creation() {
        let conv = Conversation::with_system("You are a helpful assistant.");
        assert_eq!(conv.len(), 1);
        assert!(!conv.is_empty());
        assert!(conv.system_message().is_some());
    }

    #[test]
    fn test_message_constructors() {
        let sys = ConversationMessage::system("system");
        assert_eq!(sys.role, MessageRole::System);
        assert_eq!(sys.content, "system");

        let user = ConversationMessage::user("hello");
        assert_eq!(user.role, MessageRole::User);

        let asst = ConversationMessage::assistant("hi there");
        assert_eq!(asst.role, MessageRole::Assistant);

        let tool = ConversationMessage::tool_result("call_1", "search", "results here");
        assert_eq!(tool.role, MessageRole::Tool);
        assert_eq!(tool.tool_call_id.as_deref(), Some("call_1"));
        assert_eq!(tool.tool_name.as_deref(), Some("search"));
    }

    #[test]
    fn test_openai_serialization_roundtrip() {
        let mut conv = Conversation::with_system("You are a test agent.");
        conv.push(ConversationMessage::user("Search for rust crates"));
        conv.push(ConversationMessage::assistant_tool_calls(vec![ToolCall {
            id: "call_1".into(),
            name: "web_search".into(),
            arguments: r#"{"query":"rust crates"}"#.into(),
        }]));
        conv.push(ConversationMessage::tool_result(
            "call_1",
            "web_search",
            "Found: serde, tokio, reqwest",
        ));
        conv.push(ConversationMessage::assistant(
            "I found serde, tokio, and reqwest.",
        ));

        let openai_msgs = conv.to_openai_messages();
        assert_eq!(openai_msgs.len(), 5);

        // Verify system message
        assert_eq!(openai_msgs[0]["role"], "system");
        assert_eq!(openai_msgs[0]["content"], "You are a test agent.");

        // Verify user message
        assert_eq!(openai_msgs[1]["role"], "user");

        // Verify assistant with tool calls
        assert_eq!(openai_msgs[2]["role"], "assistant");
        assert!(openai_msgs[2]["tool_calls"].is_array());
        let tool_calls = openai_msgs[2]["tool_calls"].as_array().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0]["function"]["name"], "web_search");

        // Verify tool result
        assert_eq!(openai_msgs[3]["role"], "tool");
        assert_eq!(openai_msgs[3]["tool_call_id"], "call_1");

        // Verify final assistant
        assert_eq!(openai_msgs[4]["role"], "assistant");
    }

    #[test]
    fn test_anthropic_serialization() {
        let mut conv = Conversation::with_system("System prompt here.");
        conv.push(ConversationMessage::user("Hello"));
        conv.push(ConversationMessage::assistant_tool_calls(vec![ToolCall {
            id: "tu_1".into(),
            name: "calculator".into(),
            arguments: r#"{"expr":"2+2"}"#.into(),
        }]));
        conv.push(ConversationMessage::tool_result("tu_1", "calculator", "4"));
        conv.push(ConversationMessage::assistant("The result is 4."));

        let (system, messages) = conv.to_anthropic_messages();
        assert_eq!(system.as_deref(), Some("System prompt here."));
        // System is excluded from messages
        assert_eq!(messages.len(), 4);

        // User message
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "Hello");

        // Assistant with tool_use
        assert_eq!(messages[1]["role"], "assistant");
        let content = messages[1]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "tool_use");
        assert_eq!(content[0]["name"], "calculator");

        // Tool result as user message
        assert_eq!(messages[2]["role"], "user");
        let result_content = messages[2]["content"].as_array().unwrap();
        assert_eq!(result_content[0]["type"], "tool_result");
        assert_eq!(result_content[0]["tool_use_id"], "tu_1");

        // Final assistant
        assert_eq!(messages[3]["role"], "assistant");
    }

    #[test]
    fn test_token_estimation() {
        let msg = ConversationMessage::user("Hello, world!"); // 13 chars
        let tokens = msg.estimate_tokens();
        // 13/4 = 3, max(3,1) + 4 = 7
        assert_eq!(tokens, 7);
    }

    #[test]
    fn test_conversation_token_estimation() {
        let mut conv = Conversation::with_system("Be helpful.");
        conv.push(ConversationMessage::user("Hi"));
        conv.push(ConversationMessage::assistant("Hello!"));
        let total = conv.estimate_tokens();
        assert!(total > 0);
    }

    #[test]
    fn test_truncate_to_budget() {
        let mut conv = Conversation::with_system("sys");
        for i in 0..20 {
            conv.push(ConversationMessage::user(format!(
                "Message number {} with some extra text to take up tokens",
                i
            )));
            conv.push(ConversationMessage::assistant(format!("Reply {}", i)));
        }

        let original_len = conv.len();
        assert!(original_len > 10);

        conv.truncate_to_budget(100);
        assert!(conv.len() < original_len);
        // System message preserved
        assert_eq!(conv.messages()[0].role, MessageRole::System);
        assert!(conv.estimate_tokens() <= 100);
    }

    #[test]
    fn test_metadata() {
        let mut conv = Conversation::with_system("sys");
        conv.push(ConversationMessage::user("hi"));
        conv.push(ConversationMessage::assistant_tool_calls(vec![
            ToolCall {
                id: "c1".into(),
                name: "t1".into(),
                arguments: "{}".into(),
            },
            ToolCall {
                id: "c2".into(),
                name: "t2".into(),
                arguments: "{}".into(),
            },
        ]));
        conv.push(ConversationMessage::tool_result("c1", "t1", "ok"));
        conv.push(ConversationMessage::tool_result("c2", "t2", "ok"));

        let meta = conv.metadata();
        assert_eq!(meta["message_count"], "5");
        assert_eq!(meta["has_system"], "true");
        assert_eq!(meta["tool_call_count"], "2");
        assert_eq!(meta["tool_result_count"], "2");
    }

    #[test]
    fn test_last_assistant_message() {
        let mut conv = Conversation::new();
        assert!(conv.last_assistant_message().is_none());

        conv.push(ConversationMessage::user("hi"));
        conv.push(ConversationMessage::assistant("first"));
        conv.push(ConversationMessage::user("more"));
        conv.push(ConversationMessage::assistant("second"));

        assert_eq!(conv.last_assistant_message().unwrap().content, "second");
    }

    #[test]
    fn test_inject_knowledge_context_after_system() {
        let mut conv = Conversation::with_system("You are helpful.");
        conv.push(ConversationMessage::user("hello"));
        conv.inject_knowledge_context("Some knowledge here");

        assert_eq!(conv.len(), 3);
        // Knowledge context should be at position 1 (after system)
        assert_eq!(conv.messages()[0].role, MessageRole::System);
        assert_eq!(conv.messages()[0].content, "You are helpful.");
        assert!(conv.messages()[1].content.contains("[KNOWLEDGE_CONTEXT]"));
        assert!(conv.messages()[1].content.contains("Some knowledge here"));
        assert_eq!(conv.messages()[2].role, MessageRole::User);
    }

    #[test]
    fn test_inject_knowledge_context_replaces_existing() {
        let mut conv = Conversation::with_system("System prompt");
        conv.inject_knowledge_context("First knowledge");
        conv.inject_knowledge_context("Updated knowledge");

        // Should still have just system + one knowledge context
        let knowledge_msgs: Vec<_> = conv
            .messages()
            .iter()
            .filter(|m| m.content.contains("[KNOWLEDGE_CONTEXT]"))
            .collect();
        assert_eq!(knowledge_msgs.len(), 1);
        assert!(knowledge_msgs[0].content.contains("Updated knowledge"));
    }

    #[test]
    fn test_inject_knowledge_context_no_system_message() {
        let mut conv = Conversation::new();
        conv.push(ConversationMessage::user("hello"));
        conv.inject_knowledge_context("Knowledge without system");

        assert_eq!(conv.len(), 2);
        // Knowledge context should be at position 0
        assert!(conv.messages()[0].content.contains("[KNOWLEDGE_CONTEXT]"));
        assert_eq!(conv.messages()[1].role, MessageRole::User);
    }

    #[test]
    fn test_serde_roundtrip() {
        let mut conv = Conversation::with_system("test");
        conv.push(ConversationMessage::user("hello"));
        conv.push(ConversationMessage::assistant_tool_calls(vec![ToolCall {
            id: "tc1".into(),
            name: "search".into(),
            arguments: r#"{"q":"test"}"#.into(),
        }]));

        let json = serde_json::to_string(&conv).unwrap();
        let restored: Conversation = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.len(), conv.len());
        assert_eq!(restored.messages()[2].tool_calls[0].name, "search");
    }
}
