//! Bridge between the knowledge/context system and the reasoning loop.
//!
//! `KnowledgeBridge` lets the reasoning loop access and update the agent's
//! knowledge store. It is opt-in: when provided to `ReasoningLoopRunner`,
//! it injects relevant context before each reasoning step and exposes
//! `recall_knowledge` / `store_knowledge` as LLM-callable tools.

use std::sync::Arc;
use std::time::SystemTime;

use serde::Deserialize;

use crate::context::manager::ContextManager as KnowledgeContextManager;
use crate::context::types::*;
use crate::reasoning::conversation::{Conversation, MessageRole};
use crate::reasoning::inference::ToolDefinition;
use crate::types::AgentId;

/// Configuration for knowledge integration with the reasoning loop.
#[derive(Debug, Clone)]
pub struct KnowledgeConfig {
    /// Max knowledge items to inject per iteration.
    pub max_context_items: usize,
    /// Relevance threshold for knowledge retrieval (0.0–1.0).
    pub relevance_threshold: f32,
    /// Whether to auto-store learnings after loop completion.
    pub auto_persist: bool,
}

impl Default for KnowledgeConfig {
    fn default() -> Self {
        Self {
            max_context_items: 5,
            relevance_threshold: 0.3,
            auto_persist: true,
        }
    }
}

/// Bridges the knowledge/context system into the reasoning loop.
pub struct KnowledgeBridge {
    context_manager: Arc<dyn KnowledgeContextManager>,
    config: KnowledgeConfig,
}

impl KnowledgeBridge {
    pub fn new(
        context_manager: Arc<dyn KnowledgeContextManager>,
        config: KnowledgeConfig,
    ) -> Self {
        Self {
            context_manager,
            config,
        }
    }

    /// Retrieve relevant knowledge and inject as a system-level context message.
    /// Called BEFORE each reasoning step.
    ///
    /// Returns the number of knowledge items injected.
    pub async fn inject_context(
        &self,
        agent_id: &AgentId,
        conversation: &mut Conversation,
    ) -> Result<usize, ContextError> {
        // Extract search terms from recent user/tool messages
        let search_terms = extract_search_terms(conversation);
        if search_terms.is_empty() {
            return Ok(0);
        }

        // Query the context manager for relevant items
        let query = ContextQuery {
            query_type: QueryType::Hybrid,
            search_terms: search_terms.clone(),
            time_range: None,
            memory_types: vec![],
            relevance_threshold: self.config.relevance_threshold,
            max_results: self.config.max_context_items,
            include_embeddings: false,
        };

        let context_items = self.context_manager.query_context(*agent_id, query).await?;

        // Also search the knowledge base
        let search_query = search_terms.join(" ");
        let knowledge_items = self
            .context_manager
            .search_knowledge(*agent_id, &search_query, self.config.max_context_items)
            .await?;

        // Combine and format results
        let mut lines = Vec::new();

        for item in &context_items {
            lines.push(format!(
                "- [memory, relevance={:.2}] {}",
                item.relevance_score, item.content
            ));
        }

        for item in &knowledge_items {
            lines.push(format!(
                "- [knowledge/{:?}, confidence={:.2}] {}",
                item.knowledge_type, item.confidence, item.content
            ));
        }

        let total_items = context_items.len() + knowledge_items.len();

        if !lines.is_empty() {
            let context_text = format!(
                "The following relevant knowledge and context was retrieved for this conversation:\n{}",
                lines.join("\n")
            );
            conversation.inject_knowledge_context(context_text);
        }

        Ok(total_items)
    }

    /// Persist learnings from the completed conversation.
    /// Called AFTER loop completion if auto_persist is true.
    pub async fn persist_learnings(
        &self,
        agent_id: &AgentId,
        conversation: &Conversation,
    ) -> Result<(), ContextError> {
        // Extract assistant responses as episodic memory
        let assistant_messages: Vec<&str> = conversation
            .messages()
            .iter()
            .filter(|m| m.role == MessageRole::Assistant && !m.content.is_empty())
            .map(|m| m.content.as_str())
            .collect();

        if assistant_messages.is_empty() {
            return Ok(());
        }

        let summary = if assistant_messages.len() == 1 {
            assistant_messages[0].to_string()
        } else {
            // Combine into a summary, truncating if very long
            let combined = assistant_messages.join("\n---\n");
            if combined.len() > 2000 {
                format!("{}...", &combined[..2000])
            } else {
                combined
            }
        };

        let memory_update = MemoryUpdate {
            operation: UpdateOperation::Add,
            target: MemoryTarget::Working("last_conversation_summary".to_string()),
            data: serde_json::Value::String(summary),
        };

        self.context_manager
            .update_memory(*agent_id, vec![memory_update])
            .await
    }

    /// Return tool definitions for knowledge tools.
    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        vec![recall_tool_def(), store_tool_def()]
    }

    /// Handle a knowledge tool call. Returns the tool result content.
    pub async fn handle_tool_call(
        &self,
        agent_id: &AgentId,
        tool_name: &str,
        arguments: &str,
    ) -> Result<String, String> {
        match tool_name {
            "recall_knowledge" => self.handle_recall(agent_id, arguments).await,
            "store_knowledge" => self.handle_store(agent_id, arguments).await,
            _ => Err(format!("Unknown knowledge tool: {}", tool_name)),
        }
    }

    /// Returns true if the given tool name is a knowledge tool handled by this bridge.
    pub fn is_knowledge_tool(tool_name: &str) -> bool {
        matches!(tool_name, "recall_knowledge" | "store_knowledge")
    }

    async fn handle_recall(
        &self,
        agent_id: &AgentId,
        arguments: &str,
    ) -> Result<String, String> {
        #[derive(Deserialize)]
        struct RecallArgs {
            query: String,
            #[serde(default = "default_limit")]
            limit: usize,
        }
        fn default_limit() -> usize {
            5
        }

        let args: RecallArgs =
            serde_json::from_str(arguments).map_err(|e| format!("Invalid arguments: {}", e))?;

        let items = self
            .context_manager
            .search_knowledge(*agent_id, &args.query, args.limit)
            .await
            .map_err(|e| format!("Knowledge search failed: {}", e))?;

        if items.is_empty() {
            return Ok("No relevant knowledge found.".to_string());
        }

        let mut lines = Vec::new();
        for item in &items {
            lines.push(format!(
                "- [{:?}, confidence={:.2}] {}",
                item.knowledge_type, item.confidence, item.content
            ));
        }
        Ok(lines.join("\n"))
    }

    async fn handle_store(
        &self,
        agent_id: &AgentId,
        arguments: &str,
    ) -> Result<String, String> {
        #[derive(Deserialize)]
        struct StoreArgs {
            subject: String,
            predicate: String,
            object: String,
            #[serde(default = "default_confidence")]
            confidence: f32,
        }
        fn default_confidence() -> f32 {
            0.8
        }

        let args: StoreArgs =
            serde_json::from_str(arguments).map_err(|e| format!("Invalid arguments: {}", e))?;

        let fact = KnowledgeFact {
            id: KnowledgeId::new(),
            subject: args.subject.clone(),
            predicate: args.predicate.clone(),
            object: args.object.clone(),
            confidence: args.confidence,
            source: KnowledgeSource::Experience,
            created_at: SystemTime::now(),
            verified: false,
        };

        let knowledge_id = self
            .context_manager
            .add_knowledge(*agent_id, Knowledge::Fact(fact))
            .await
            .map_err(|e| format!("Failed to store knowledge: {}", e))?;

        Ok(format!(
            "Stored fact: {} {} {} (id: {})",
            args.subject, args.predicate, args.object, knowledge_id.0
        ))
    }
}

fn recall_tool_def() -> ToolDefinition {
    ToolDefinition {
        name: "recall_knowledge".to_string(),
        description: "Search the agent's knowledge base for relevant information. Use this to recall facts, procedures, or patterns that may help with the current task.".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query to find relevant knowledge"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default: 5)",
                    "default": 5
                }
            },
            "required": ["query"]
        }),
    }
}

fn store_tool_def() -> ToolDefinition {
    ToolDefinition {
        name: "store_knowledge".to_string(),
        description: "Store a new fact in the agent's knowledge base for future reference. Use this to remember important information learned during the conversation.".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "subject": {
                    "type": "string",
                    "description": "The subject of the fact (e.g., 'Rust')"
                },
                "predicate": {
                    "type": "string",
                    "description": "The relationship (e.g., 'is_a')"
                },
                "object": {
                    "type": "string",
                    "description": "The object of the fact (e.g., 'systems programming language')"
                },
                "confidence": {
                    "type": "number",
                    "description": "Confidence level 0.0-1.0 (default: 0.8)",
                    "default": 0.8
                }
            },
            "required": ["subject", "predicate", "object"]
        }),
    }
}

/// Extract search terms from the most recent user and tool messages in the conversation.
fn extract_search_terms(conversation: &Conversation) -> Vec<String> {
    let messages = conversation.messages();
    let mut terms = Vec::new();

    // Look at the last few messages for search context
    for msg in messages.iter().rev().take(5) {
        match msg.role {
            MessageRole::User | MessageRole::Tool => {
                // Extract meaningful words (skip very short words and common stop words)
                let words: Vec<&str> = msg
                    .content
                    .split_whitespace()
                    .filter(|w| w.len() > 3)
                    .take(10)
                    .collect();
                for word in words {
                    let cleaned = word.trim_matches(|c: char| !c.is_alphanumeric());
                    if !cleaned.is_empty() && !terms.contains(&cleaned.to_string()) {
                        terms.push(cleaned.to_string());
                    }
                }
            }
            _ => {}
        }
        // Limit total terms
        if terms.len() >= 15 {
            break;
        }
    }

    terms
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reasoning::conversation::ConversationMessage;

    #[test]
    fn test_knowledge_config_default() {
        let config = KnowledgeConfig::default();
        assert_eq!(config.max_context_items, 5);
        assert!((config.relevance_threshold - 0.3).abs() < f32::EPSILON);
        assert!(config.auto_persist);
    }

    #[test]
    fn test_extract_search_terms_from_user_message() {
        let mut conv = Conversation::new();
        conv.push(ConversationMessage::user(
            "What is the weather forecast for tomorrow?",
        ));

        let terms = extract_search_terms(&conv);
        assert!(!terms.is_empty());
        assert!(terms.contains(&"weather".to_string()));
        assert!(terms.contains(&"forecast".to_string()));
        assert!(terms.contains(&"tomorrow".to_string()));
    }

    #[test]
    fn test_extract_search_terms_skips_short_words() {
        let mut conv = Conversation::new();
        conv.push(ConversationMessage::user("I am at the big house"));

        let terms = extract_search_terms(&conv);
        // "I", "am", "at", "the" are all <= 3 chars, should be skipped
        assert!(terms.contains(&"house".to_string()));
        assert!(!terms.iter().any(|t| t.len() <= 3));
    }

    #[test]
    fn test_extract_search_terms_empty_conversation() {
        let conv = Conversation::new();
        let terms = extract_search_terms(&conv);
        assert!(terms.is_empty());
    }

    #[test]
    fn test_extract_search_terms_ignores_assistant() {
        let mut conv = Conversation::new();
        conv.push(ConversationMessage::assistant(
            "Here is some information about databases",
        ));

        let terms = extract_search_terms(&conv);
        assert!(terms.is_empty());
    }

    #[test]
    fn test_tool_definitions() {
        let recall = recall_tool_def();
        assert_eq!(recall.name, "recall_knowledge");
        assert!(recall.parameters["required"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("query")));

        let store = store_tool_def();
        assert_eq!(store.name, "store_knowledge");
        assert!(store.parameters["required"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("subject")));
    }

    #[test]
    fn test_is_knowledge_tool() {
        assert!(KnowledgeBridge::is_knowledge_tool("recall_knowledge"));
        assert!(KnowledgeBridge::is_knowledge_tool("store_knowledge"));
        assert!(!KnowledgeBridge::is_knowledge_tool("web_search"));
        assert!(!KnowledgeBridge::is_knowledge_tool(""));
    }
}
