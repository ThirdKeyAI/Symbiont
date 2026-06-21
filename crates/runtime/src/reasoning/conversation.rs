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

    /// Estimate token count for this message.
    ///
    /// Uses ~3.3 chars/token (Anthropic's tokenizer averages 3-3.5 for mixed content
    /// including JSON, code, and prose). Adds per-message framing overhead for the
    /// role field, JSON structure, and tool metadata that the API sees but we don't
    /// count in raw content length.
    pub fn estimate_tokens(&self) -> usize {
        let mut chars = self.content.len();
        for tc in &self.tool_calls {
            // Tool call JSON structure adds overhead beyond just name+args
            chars += tc.name.len() + tc.arguments.len() + tc.id.len() + 30; // JSON framing
        }
        if let Some(ref id) = self.tool_call_id {
            chars += id.len() + 20; // tool_result framing
        }
        // ~3.3 chars per token (10 tokens per 33 chars), plus per-message overhead
        // The overhead covers role, JSON structure, content block wrapping
        (chars * 10 / 33).max(1) + 7
    }
}

/// Group consecutive non-system messages into atomic truncation units.
///
/// An assistant message that issues `tool_use` blocks plus the
/// following `tool_result` messages that satisfy those tool_uses form
/// one atomic group. A bare user, bare assistant, or any other message
/// is its own group. Groups must be dropped whole — splitting between
/// an `assistant{tool_use}` and its matching `tool_result` violates
/// Anthropic's Messages API pairing invariant.
///
/// Returns indices into the input slice. The caller is responsible for
/// not including the system message in `messages`.
fn group_for_truncation(messages: &[ConversationMessage]) -> Vec<Vec<usize>> {
    use std::collections::HashSet;
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    let mut pending: HashSet<String> = HashSet::new();

    for (i, msg) in messages.iter().enumerate() {
        current.push(i);
        if !msg.tool_calls.is_empty() {
            // Assistant turn with tool_use: open expectations for each
            // tool_call's matching tool_result.
            for tc in &msg.tool_calls {
                pending.insert(tc.id.clone());
            }
        } else if msg.role == MessageRole::Tool {
            // Tool result message: closes one pending expectation.
            if let Some(id) = &msg.tool_call_id {
                pending.remove(id);
            }
        }
        // Close the group when all expectations are satisfied.
        if pending.is_empty() {
            groups.push(std::mem::take(&mut current));
        }
    }
    // Stragglers: an assistant turn whose tool_results haven't all
    // arrived yet (the loop is mid-iteration). Keep them as one group
    // so truncation can't strand them.
    if !current.is_empty() {
        groups.push(current);
    }
    groups
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

    /// Remove and return the last message, if any. Used to roll back a
    /// speculative user turn when a delegated request fails, so a retry
    /// starts from a clean thread.
    pub fn pop(&mut self) -> Option<ConversationMessage> {
        self.messages.pop()
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

        // Build raw messages first, then merge consecutive same-role messages.
        // Anthropic requires that all tool_result blocks for a given assistant
        // message's tool_use blocks appear in the immediately following user message.
        let mut raw_messages: Vec<serde_json::Value> = Vec::new();

        for msg in self
            .messages
            .iter()
            .filter(|m| m.role != MessageRole::System)
        {
            let role_str = match msg.role {
                MessageRole::User | MessageRole::Tool => "user",
                MessageRole::Assistant => "assistant",
                MessageRole::System => unreachable!(),
            };

            let serialized = if msg.role == MessageRole::Tool {
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
            };

            // Merge consecutive messages with the same role by combining content blocks.
            // This is critical for tool_result blocks: Anthropic requires all tool_results
            // for a set of tool_use blocks to be in a single user message.
            if let Some(last) = raw_messages.last_mut() {
                let last_role = last.get("role").and_then(|r| r.as_str()).unwrap_or("");
                if last_role == role_str {
                    // Merge content into the previous message
                    let prev_content = last.get_mut("content").unwrap();
                    let new_content = serialized.get("content").unwrap();

                    // Ensure both are arrays for merging
                    let prev_arr = if prev_content.is_array() {
                        prev_content.as_array_mut().unwrap()
                    } else {
                        // Convert string content to a text block array
                        let text = prev_content.as_str().unwrap_or("").to_string();
                        *prev_content = serde_json::json!([{"type": "text", "text": text}]);
                        prev_content.as_array_mut().unwrap()
                    };

                    if new_content.is_array() {
                        prev_arr.extend(new_content.as_array().unwrap().iter().cloned());
                    } else {
                        let text = new_content.as_str().unwrap_or("").to_string();
                        prev_arr.push(serde_json::json!({"type": "text", "text": text}));
                    }

                    continue;
                }
            }

            raw_messages.push(serialized);
        }

        (system, raw_messages)
    }

    /// Truncate the conversation to fit within a token budget.
    ///
    /// Preserves the system message and the most recent messages.
    /// Removes older messages from the middle until the budget is met.
    ///
    /// Tool-use/tool-result pairing invariant. Anthropic's Messages API
    /// rejects any conversation where a `tool_result` block lacks a
    /// matching `tool_use` block in the immediately preceding assistant
    /// turn. To preserve that invariant under truncation, this method
    /// groups consecutive `assistant{tool_calls}` → `tool_result*`
    /// sequences into atomic units and drops them whole-group rather
    /// than splitting them. A standalone message (plain user, plain
    /// assistant text, or system) is its own group.
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

        let start_idx = if system_msg.is_some() { 1 } else { 0 };
        let non_system: Vec<ConversationMessage> = self.messages.drain(start_idx..).collect();

        // Group consecutive messages so a tool_use assistant turn and
        // its tool_result followups stay together. The invariant: every
        // tool_result must be in the same group as the assistant
        // message that issued the matching tool_use.
        let groups = group_for_truncation(&non_system);

        // Walk groups from newest to oldest, accepting whole groups
        // until the next one would exceed budget. The oldest accepted
        // group anchors the kept window; older groups are dropped.
        let mut kept_group_indices: Vec<usize> = Vec::new();
        let mut used_tokens = 0usize;
        for (gi, group) in groups.iter().enumerate().rev() {
            let group_tokens: usize = group.iter().map(|i| non_system[*i].estimate_tokens()).sum();
            if used_tokens + group_tokens > remaining_budget {
                break;
            }
            used_tokens += group_tokens;
            kept_group_indices.push(gi);
        }
        kept_group_indices.reverse();

        // Floor: never produce a conversation with zero non-system
        // messages — Anthropic rejects `messages: []` outright. If the
        // most recent group alone exceeds the budget (typical when a
        // single tool_result is massive — paginated query results,
        // file dumps, etc.) it's still better to ship that one group
        // and let the API surface a max-tokens-style error than to
        // ship an empty conversation that 400s with a misleading
        // "messages required" complaint. The loss-of-context warning
        // logged via context_manager makes the situation visible.
        if kept_group_indices.is_empty() && !groups.is_empty() {
            kept_group_indices.push(groups.len() - 1);
            tracing::warn!(
                most_recent_group_tokens = groups[groups.len() - 1]
                    .iter()
                    .map(|i| non_system[*i].estimate_tokens())
                    .sum::<usize>(),
                budget = remaining_budget,
                "truncate_to_budget: most recent group exceeds budget; \
                 keeping it anyway to preserve conversation structure. \
                 Caller likely needs to shrink individual tool results."
            );
        }

        let mut kept: Vec<ConversationMessage> = Vec::new();
        for gi in kept_group_indices {
            for i in &groups[gi] {
                kept.push(non_system[*i].clone());
            }
        }

        // Safety net: if the FIRST kept message is a tool_result (which
        // can happen if the first group is malformed in caller-supplied
        // data), strip leading tool_results until the conversation
        // starts with a non-tool message. Anthropic rejects a leading
        // user-tool_result with no prior tool_use.
        while kept.first().is_some_and(|m| m.role == MessageRole::Tool) {
            kept.remove(0);
        }

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
        // 13 * 10/33 = 3, max(3,1) + 7 = 10
        assert_eq!(tokens, 10);
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

    /// Truncation must NEVER strand a `tool_result` whose matching
    /// `tool_use` (carried in the prior assistant message) got dropped.
    /// Building a conversation with many assistant→tool_result pairs
    /// and truncating to a small budget should result in either both
    /// kept or both dropped for every pair.
    #[test]
    fn test_truncate_preserves_tool_use_tool_result_pairing() {
        let mut conv = Conversation::with_system("sys");
        // 8 pairs of (user, assistant_tool_call, tool_result, assistant_text)
        for i in 0..8 {
            conv.push(ConversationMessage::user(format!(
                "Ask {} with enough characters to consume tokens for the budget test",
                i
            )));
            conv.push(ConversationMessage::assistant_tool_calls(vec![ToolCall {
                id: format!("call_{}", i),
                name: "search".into(),
                arguments: format!(r#"{{"q":"query {}"}}"#, i),
            }]));
            conv.push(ConversationMessage::tool_result(
                format!("call_{}", i),
                "search",
                format!(
                    "Long result text for tool call {}. Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt.",
                    i
                ),
            ));
            conv.push(ConversationMessage::assistant(format!(
                "Reasoning for round {} concluding with a summary statement",
                i
            )));
        }
        // Truncate to a budget that forces dropping multiple groups.
        conv.truncate_to_budget(400);

        // For every tool_result kept, the matching assistant_tool_call
        // must also be kept AND appear immediately before it (after
        // accounting for any text-only assistant interleaving).
        let messages = conv.messages();
        let mut available_tool_ids: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for msg in messages {
            if !msg.tool_calls.is_empty() {
                for tc in &msg.tool_calls {
                    available_tool_ids.insert(tc.id.clone());
                }
            }
            if msg.role == MessageRole::Tool {
                let id = msg
                    .tool_call_id
                    .as_deref()
                    .expect("tool_result must carry tool_call_id");
                assert!(
                    available_tool_ids.contains(id),
                    "orphaned tool_result for {id}: matching tool_use was not preserved by truncation"
                );
            }
        }
        // System message preserved (or absent if truncation nuked it,
        // which would itself be a regression).
        assert_eq!(messages[0].role, MessageRole::System);
    }

    /// Floor: a single oversize group is still preserved rather than
    /// dropping everything. Returning an empty `messages: []` body to
    /// Anthropic results in a misleading 400; better to ship the one
    /// oversize group and let the API report a max-tokens error.
    #[test]
    fn test_truncate_keeps_at_least_one_group_even_if_oversize() {
        let mut conv = Conversation::with_system("sys");
        // Single user message that alone exceeds the budget by a wide
        // margin — simulates a query_findings tool_result dumping
        // hundreds of KB of finding metadata.
        let big = "X".repeat(50_000);
        conv.push(ConversationMessage::user(big));

        // Budget far below the message's token count.
        conv.truncate_to_budget(100);

        // The non-system message must still be present — anything else
        // would produce `messages: []` and a misleading 400.
        let non_system_count = conv
            .messages()
            .iter()
            .filter(|m| m.role != MessageRole::System)
            .count();
        assert!(
            non_system_count >= 1,
            "truncation must keep at least one non-system message even \
             when the only candidate group exceeds the budget"
        );
    }

    /// Edge case: when truncation would drop the assistant_tool_use
    /// message but keep its tool_result, the safety net should reject
    /// the leading tool_result rather than ship a malformed conversation.
    #[test]
    fn test_truncate_strips_orphaned_leading_tool_results() {
        let mut conv = Conversation::with_system("sys");
        // Build a sequence where a budget-driven cut would otherwise
        // leave a tool_result as the first non-system message.
        conv.push(ConversationMessage::user("u1 with some longer text"));
        conv.push(ConversationMessage::assistant_tool_calls(vec![ToolCall {
            id: "c_old".into(),
            name: "t".into(),
            arguments: "{}".into(),
        }]));
        conv.push(ConversationMessage::tool_result(
            "c_old",
            "t",
            "old result with substantial text content",
        ));
        // Fresh group
        conv.push(ConversationMessage::user("u2"));
        conv.push(ConversationMessage::assistant("a2"));

        conv.truncate_to_budget(60);

        let first_non_system = conv
            .messages()
            .iter()
            .find(|m| m.role != MessageRole::System)
            .expect("expected at least one non-system message");
        assert_ne!(
            first_non_system.role,
            MessageRole::Tool,
            "post-truncation conversation must not start with an orphaned tool_result"
        );
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
