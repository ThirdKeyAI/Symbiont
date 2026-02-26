//! Runtime context management for reasoning loops
//!
//! Manages the conversation context to keep it within token budgets.
//! Provides multiple strategies for context compression:
//! - SlidingWindow: keep the most recent messages
//! - ObservationMasking: replace old tool outputs but keep reasoning
//! - AnchoredSummary: keep system + first user + summarize middle + recent

use crate::reasoning::conversation::{Conversation, MessageRole};

/// Strategy for managing context within token budgets.
#[derive(Debug, Clone, Default)]
pub enum ContextStrategy {
    /// Keep the most recent messages that fit the budget.
    /// Simple and predictable.
    #[default]
    SlidingWindow,

    /// Replace old tool outputs with "[previous tool result omitted]"
    /// but keep the reasoning chain intact. Preserves decision history.
    ObservationMasking,

    /// Keep system prompt + first user message as anchors,
    /// summarize the middle, keep recent messages.
    AnchoredSummary {
        /// Number of recent messages to always keep.
        recent_count: usize,
    },
}

/// Manages conversation context to stay within token budgets.
pub trait ContextManager: Send + Sync {
    /// Apply context management to keep the conversation within budget.
    fn manage_context(&self, conversation: &mut Conversation, max_tokens: usize);

    /// Get the strategy name for logging.
    fn strategy_name(&self) -> &str;
}

/// Default context manager using configurable strategies.
pub struct DefaultContextManager {
    strategy: ContextStrategy,
}

impl DefaultContextManager {
    /// Create a new context manager with the given strategy.
    pub fn new(strategy: ContextStrategy) -> Self {
        Self { strategy }
    }

    /// Apply sliding window: keep system + most recent messages.
    fn apply_sliding_window(conversation: &mut Conversation, max_tokens: usize) {
        conversation.truncate_to_budget(max_tokens);
    }

    /// Apply observation masking: replace old tool results with placeholders.
    fn apply_observation_masking(conversation: &mut Conversation, max_tokens: usize) {
        if conversation.estimate_tokens() <= max_tokens {
            return;
        }

        let messages = conversation.messages().to_vec();
        let total = messages.len();
        if total <= 3 {
            return;
        }

        // Find tool result messages to mask, starting from oldest
        // Keep the most recent 6 messages (3 turns) intact
        let keep_recent = 6.min(total);
        let mut new_messages = Vec::new();

        for (i, msg) in messages.iter().enumerate() {
            if i >= total - keep_recent {
                // Keep recent messages as-is
                new_messages.push(msg.clone());
            } else if msg.role == MessageRole::Tool {
                // Replace old tool results with a placeholder
                let mut masked = msg.clone();
                masked.content = format!(
                    "[Previous {} result omitted for context management]",
                    msg.tool_name.as_deref().unwrap_or("tool")
                );
                new_messages.push(masked);
            } else {
                // Keep non-tool messages (reasoning, user input)
                new_messages.push(msg.clone());
            }
        }

        *conversation = Conversation::new();
        for msg in new_messages {
            conversation.push(msg);
        }

        // If still over budget, fall back to sliding window
        if conversation.estimate_tokens() > max_tokens {
            Self::apply_sliding_window(conversation, max_tokens);
        }
    }

    /// Apply anchored summary: keep anchors + summarize middle + recent.
    fn apply_anchored_summary(
        conversation: &mut Conversation,
        max_tokens: usize,
        recent_count: usize,
    ) {
        if conversation.estimate_tokens() <= max_tokens {
            return;
        }

        let messages = conversation.messages().to_vec();
        let total = messages.len();

        // Determine anchor messages (system + first user)
        let mut anchor_end = 0;
        for (i, msg) in messages.iter().enumerate() {
            if msg.role == MessageRole::System || (msg.role == MessageRole::User && i <= 1) {
                anchor_end = i + 1;
            } else {
                break;
            }
        }

        let keep_recent = recent_count.min(total.saturating_sub(anchor_end));
        let recent_start = total.saturating_sub(keep_recent);

        // If there's a middle section, create a summary placeholder
        let mut new_messages: Vec<_> = messages[..anchor_end].to_vec();

        if anchor_end < recent_start {
            let middle_count = recent_start - anchor_end;
            let tool_calls_in_middle = messages[anchor_end..recent_start]
                .iter()
                .filter(|m| !m.tool_calls.is_empty())
                .count();
            let tool_results_in_middle = messages[anchor_end..recent_start]
                .iter()
                .filter(|m| m.role == MessageRole::Tool)
                .count();

            let summary = format!(
                "[Context summary: {} messages omitted ({} tool calls, {} tool results). The conversation continued with the agent working on the task.]",
                middle_count, tool_calls_in_middle, tool_results_in_middle
            );
            new_messages.push(crate::reasoning::conversation::ConversationMessage::user(
                summary,
            ));
        }

        new_messages.extend(messages[recent_start..].to_vec());

        *conversation = Conversation::new();
        for msg in new_messages {
            conversation.push(msg);
        }

        // Final fallback
        if conversation.estimate_tokens() > max_tokens {
            Self::apply_sliding_window(conversation, max_tokens);
        }
    }
}

impl Default for DefaultContextManager {
    fn default() -> Self {
        Self::new(ContextStrategy::SlidingWindow)
    }
}

impl ContextManager for DefaultContextManager {
    fn manage_context(&self, conversation: &mut Conversation, max_tokens: usize) {
        match &self.strategy {
            ContextStrategy::SlidingWindow => {
                Self::apply_sliding_window(conversation, max_tokens);
            }
            ContextStrategy::ObservationMasking => {
                Self::apply_observation_masking(conversation, max_tokens);
            }
            ContextStrategy::AnchoredSummary { recent_count } => {
                Self::apply_anchored_summary(conversation, max_tokens, *recent_count);
            }
        }
    }

    fn strategy_name(&self) -> &str {
        match self.strategy {
            ContextStrategy::SlidingWindow => "sliding_window",
            ContextStrategy::ObservationMasking => "observation_masking",
            ContextStrategy::AnchoredSummary { .. } => "anchored_summary",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reasoning::conversation::{ConversationMessage, ToolCall};

    fn build_long_conversation() -> Conversation {
        let mut conv = Conversation::with_system("You are a research agent.");
        for i in 0..20 {
            conv.push(ConversationMessage::user(format!(
                "Research question {} about a topic that requires multiple paragraphs of text to describe properly",
                i
            )));
            conv.push(ConversationMessage::assistant_tool_calls(vec![ToolCall {
                id: format!("call_{}", i),
                name: "web_search".into(),
                arguments: format!(r#"{{"query": "topic {} detailed information"}}"#, i),
            }]));
            conv.push(ConversationMessage::tool_result(
                format!("call_{}", i),
                "web_search",
                format!("Here are the detailed results for query {}. Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.", i),
            ));
            conv.push(ConversationMessage::assistant(format!(
                "Based on the search results for question {}, I found that the topic involves multiple interesting aspects that we should discuss in detail.",
                i
            )));
        }
        conv
    }

    #[test]
    fn test_sliding_window_no_truncation_needed() {
        let mgr = DefaultContextManager::new(ContextStrategy::SlidingWindow);
        let mut conv = Conversation::with_system("sys");
        conv.push(ConversationMessage::user("hi"));
        conv.push(ConversationMessage::assistant("hello"));

        let original_tokens = conv.estimate_tokens();
        mgr.manage_context(&mut conv, 10000);
        assert_eq!(conv.estimate_tokens(), original_tokens);
    }

    #[test]
    fn test_sliding_window_truncation() {
        let mgr = DefaultContextManager::new(ContextStrategy::SlidingWindow);
        let mut conv = build_long_conversation();
        let original_len = conv.len();

        mgr.manage_context(&mut conv, 200);
        assert!(conv.len() < original_len);
        assert!(conv.estimate_tokens() <= 200);
        // System message preserved
        assert_eq!(conv.messages()[0].role, MessageRole::System);
    }

    #[test]
    fn test_observation_masking() {
        let mgr = DefaultContextManager::new(ContextStrategy::ObservationMasking);
        let mut conv = build_long_conversation();

        mgr.manage_context(&mut conv, 500);

        // Check that old tool results are masked
        let mut found_masked = false;
        for msg in conv.messages() {
            if msg.role == MessageRole::Tool && msg.content.contains("omitted") {
                found_masked = true;
                break;
            }
        }
        // The masking should have replaced some old tool results
        // (or fallen back to sliding window if still over budget)
        assert!(found_masked || conv.estimate_tokens() <= 500);
    }

    #[test]
    fn test_anchored_summary() {
        let mgr = DefaultContextManager::new(ContextStrategy::AnchoredSummary { recent_count: 6 });
        let mut conv = build_long_conversation();
        let original_len = conv.len();

        mgr.manage_context(&mut conv, 500);
        assert!(conv.len() < original_len);

        // System message preserved
        assert_eq!(conv.messages()[0].role, MessageRole::System);

        // Check for summary message
        let has_summary = conv
            .messages()
            .iter()
            .any(|m| m.content.contains("Context summary"));
        // Either has summary or was small enough not to need it
        assert!(has_summary || conv.estimate_tokens() <= 500);
    }

    #[test]
    fn test_strategy_name() {
        assert_eq!(
            DefaultContextManager::new(ContextStrategy::SlidingWindow).strategy_name(),
            "sliding_window"
        );
        assert_eq!(
            DefaultContextManager::new(ContextStrategy::ObservationMasking).strategy_name(),
            "observation_masking"
        );
        assert_eq!(
            DefaultContextManager::new(ContextStrategy::AnchoredSummary { recent_count: 4 })
                .strategy_name(),
            "anchored_summary"
        );
    }

    #[test]
    fn test_context_within_budget_untouched() {
        let mgr = DefaultContextManager::new(ContextStrategy::ObservationMasking);
        let mut conv = Conversation::with_system("sys");
        conv.push(ConversationMessage::user("short"));

        let before = conv.len();
        mgr.manage_context(&mut conv, 100_000);
        assert_eq!(conv.len(), before);
    }
}
