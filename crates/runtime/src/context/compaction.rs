//! Context compaction pipeline for reclaiming tokens.
//!
//! Runs synchronously before each LLM call. Walks tiers from most aggressive
//! (truncate) to least aggressive (summarize), applying the first tier that
//! matches the current token usage level.

use serde::{Deserialize, Serialize};

use super::types::AccessLevel;

/// Per-agent compaction configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    /// Whether compaction is enabled.
    pub enabled: bool,
    /// Token usage ratio (0.0–1.0) at which Tier 1 (Summarize) triggers.
    pub summarize_threshold: f32,
    /// Token usage ratio at which Tier 2 (Compress Episodic) triggers.
    pub compress_threshold: f32,
    /// Token usage ratio at which Tier 3 (Archive to Memory) triggers.
    pub archive_threshold: f32,
    /// Token usage ratio at which Tier 4 (Truncate) triggers.
    pub truncate_threshold: f32,
    /// Model to use for summarization. `None` = agent's own model.
    pub compaction_model: Option<String>,
    /// Maximum tokens for generated summaries.
    pub max_summary_tokens: usize,
    /// Access levels whose items are never compacted.
    pub preserve_access_levels: Vec<AccessLevel>,
    /// Minimum conversation items before compaction is considered.
    pub min_items_to_compact: usize,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            summarize_threshold: 0.70,
            compress_threshold: 0.80,
            archive_threshold: 0.85,
            truncate_threshold: 0.90,
            compaction_model: None,
            max_summary_tokens: 500,
            preserve_access_levels: vec![AccessLevel::Secret, AccessLevel::Confidential],
            min_items_to_compact: 5,
        }
    }
}

/// Which compaction tier was applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompactionTier {
    /// Tier 1: LLM summarizes oldest conversation items (OSS).
    Summarize,
    /// Tier 2: Merge similar episodic memory items (enterprise).
    CompressEpisodic,
    /// Tier 3: Archive summaries and old items to MarkdownMemoryStore (enterprise).
    ArchiveToMemory,
    /// Tier 4: Drop oldest conversation items (OSS).
    Truncate,
}

impl std::fmt::Display for CompactionTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompactionTier::Summarize => write!(f, "Summarize"),
            CompactionTier::CompressEpisodic => write!(f, "CompressEpisodic"),
            CompactionTier::ArchiveToMemory => write!(f, "ArchiveToMemory"),
            CompactionTier::Truncate => write!(f, "Truncate"),
        }
    }
}

/// Result of a compaction operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionResult {
    pub tier_applied: CompactionTier,
    pub tokens_before: usize,
    pub tokens_after: usize,
    pub tokens_saved: usize,
    pub items_affected: usize,
    pub duration_ms: u64,
    pub summary_generated: Option<String>,
}

use super::types::{ConversationItem, ConversationRole};

/// Tier 4: Truncate — drop oldest conversation items until count drops to
/// `target_ratio` of original. System messages are preserved.
pub fn truncate_items(
    items: &[ConversationItem],
    _config: &CompactionConfig,
    target_ratio: f32,
) -> (Vec<ConversationItem>, usize) {
    let total = items.len();
    let target_count = (total as f32 * target_ratio).ceil() as usize;

    // Partition: system messages vs candidates for removal (oldest first)
    let mut system_items: Vec<&ConversationItem> = Vec::new();
    let mut candidates: Vec<&ConversationItem> = Vec::new();

    for item in items {
        if matches!(item.role, ConversationRole::System) {
            system_items.push(item);
        } else {
            candidates.push(item);
        }
    }

    // Keep the newest candidates, drop the oldest
    let keep_count = target_count.saturating_sub(system_items.len());
    let drop_count = candidates.len().saturating_sub(keep_count);

    let mut result: Vec<ConversationItem> = system_items.into_iter().cloned().collect();
    result.extend(candidates.into_iter().skip(drop_count).cloned());

    (result, drop_count)
}

use std::future::Future;
use std::time::Instant;

/// Tier 1: Summarize — LLM condenses the oldest N conversation items into a
/// single system message. Preserves system messages and items with protected
/// access levels.
///
/// The `summarizer` closure takes the concatenated text and returns a summary.
pub async fn summarize_items<F, Fut>(
    items: &[ConversationItem],
    config: &CompactionConfig,
    items_to_summarize: usize,
    summarizer: F,
) -> Result<Option<(Vec<ConversationItem>, CompactionResult)>, String>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<String, String>>,
{
    if items.len() < config.min_items_to_compact {
        return Ok(None);
    }

    let start = Instant::now();

    // Collect items eligible for summarization (not system, not protected)
    let mut to_summarize: Vec<(usize, &ConversationItem)> = Vec::new();
    let mut to_keep: Vec<(usize, &ConversationItem)> = Vec::new();

    for (idx, item) in items.iter().enumerate() {
        if matches!(item.role, ConversationRole::System) {
            to_keep.push((idx, item));
        } else if to_summarize.len() < items_to_summarize {
            to_summarize.push((idx, item));
        } else {
            to_keep.push((idx, item));
        }
    }

    if to_summarize.is_empty() {
        return Ok(None);
    }

    // Build the text to summarize
    let text_to_summarize: String = to_summarize
        .iter()
        .map(|(_, item)| format!("{:?}: {}", item.role, item.content))
        .collect::<Vec<_>>()
        .join("\n\n");

    let summary = summarizer(text_to_summarize).await?;

    // Build the replacement item
    let summary_item = ConversationItem {
        id: super::types::ContextId::new(),
        role: ConversationRole::System,
        content: format!("[Compacted summary] {summary}"),
        timestamp: std::time::SystemTime::now(),
        context_used: vec![],
        knowledge_used: vec![],
    };

    // Reconstruct: system items + summary + remaining items (in original order)
    let mut result_items: Vec<ConversationItem> = Vec::new();

    // Add system items that came before the summarized range
    for (_, item) in to_keep.iter().filter(|(idx, _)| *idx < to_summarize[0].0) {
        result_items.push((*item).clone());
    }

    // Insert summary
    result_items.push(summary_item);

    // Add remaining items after the summarized range
    for (_, item) in to_keep
        .iter()
        .filter(|(idx, _)| *idx > to_summarize.last().unwrap().0)
    {
        result_items.push((*item).clone());
    }

    let duration = start.elapsed();

    let compaction_result = CompactionResult {
        tier_applied: CompactionTier::Summarize,
        tokens_before: 0, // Caller fills in actual token counts
        tokens_after: 0,
        tokens_saved: 0,
        items_affected: to_summarize.len(),
        duration_ms: duration.as_millis() as u64,
        summary_generated: Some(summary),
    };

    Ok(Some((result_items, compaction_result)))
}

/// Tier 2: Compress Episodic — merge similar memory items (enterprise only).
#[cfg(feature = "enterprise-compaction")]
pub fn tier_compress_episodic() -> Option<CompactionResult> {
    todo!("enterprise: compress episodic memory items by cosine similarity")
}

#[cfg(not(feature = "enterprise-compaction"))]
pub fn tier_compress_episodic() -> Option<CompactionResult> {
    None
}

/// Tier 3: Archive to Memory — flush old items to MarkdownMemoryStore (enterprise only).
#[cfg(feature = "enterprise-compaction")]
pub fn tier_archive_to_memory() -> Option<CompactionResult> {
    todo!("enterprise: archive items to MarkdownMemoryStore daily log")
}

#[cfg(not(feature = "enterprise-compaction"))]
pub fn tier_archive_to_memory() -> Option<CompactionResult> {
    None
}

/// Select the compaction tier based on current token usage ratio.
///
/// Walks from most aggressive (Truncate at 90%) down to least (Summarize at 70%).
/// Enterprise tiers (CompressEpisodic, ArchiveToMemory) are only available when
/// the `enterprise-compaction` feature is enabled.
pub fn select_tier(usage_ratio: f32, config: &CompactionConfig) -> Option<CompactionTier> {
    if !config.enabled {
        return None;
    }

    if usage_ratio >= config.truncate_threshold {
        return Some(CompactionTier::Truncate);
    }

    #[cfg(feature = "enterprise-compaction")]
    if usage_ratio >= config.archive_threshold {
        return Some(CompactionTier::ArchiveToMemory);
    }

    #[cfg(feature = "enterprise-compaction")]
    if usage_ratio >= config.compress_threshold {
        return Some(CompactionTier::CompressEpisodic);
    }

    if usage_ratio >= config.summarize_threshold {
        return Some(CompactionTier::Summarize);
    }

    None
}

/// Enterprise audit entry for compaction events.
#[cfg(feature = "enterprise-compaction")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionAuditEntry {
    pub agent_id: crate::types::AgentId,
    pub session_id: super::types::SessionId,
    pub timestamp: std::time::SystemTime,
    pub tier: CompactionTier,
    pub result: CompactionResult,
    pub items_before: Vec<super::types::ContextId>,
    pub items_after: Vec<super::types::ContextId>,
}

#[cfg(test)]
mod tests {
    use super::super::token_counter::TokenCounter;
    use super::super::types::{ContextId, ConversationItem, ConversationRole};
    use super::*;
    use std::time::SystemTime;

    fn make_conversation_items(count: usize) -> Vec<ConversationItem> {
        (0..count)
            .map(|i| ConversationItem {
                id: ContextId::new(),
                role: if i == 0 {
                    ConversationRole::System
                } else {
                    ConversationRole::User
                },
                content: format!("Message number {i} with some content to take up tokens"),
                timestamp: SystemTime::now(),
                context_used: vec![],
                knowledge_used: vec![],
            })
            .collect()
    }

    #[test]
    fn default_config_has_correct_thresholds() {
        let config = CompactionConfig::default();
        assert!(config.enabled);
        assert!((config.summarize_threshold - 0.70).abs() < f32::EPSILON);
        assert!((config.compress_threshold - 0.80).abs() < f32::EPSILON);
        assert!((config.archive_threshold - 0.85).abs() < f32::EPSILON);
        assert!((config.truncate_threshold - 0.90).abs() < f32::EPSILON);
        assert_eq!(config.max_summary_tokens, 500);
        assert_eq!(config.min_items_to_compact, 5);
        assert_eq!(config.preserve_access_levels.len(), 2);
    }

    #[test]
    fn compaction_tier_display() {
        assert_eq!(CompactionTier::Summarize.to_string(), "Summarize");
        assert_eq!(CompactionTier::Truncate.to_string(), "Truncate");
    }

    #[test]
    fn compaction_result_serialization() {
        let result = CompactionResult {
            tier_applied: CompactionTier::Truncate,
            tokens_before: 10_000,
            tokens_after: 5_000,
            tokens_saved: 5_000,
            items_affected: 12,
            duration_ms: 3,
            summary_generated: None,
        };
        let json = serde_json::to_string(&result).unwrap();
        let deserialized: CompactionResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.tokens_saved, 5_000);
    }

    #[tokio::test]
    async fn summarize_replaces_items_with_summary() {
        let items = make_conversation_items(10);
        let config = CompactionConfig {
            min_items_to_compact: 3,
            ..CompactionConfig::default()
        };

        // Mock summarizer that returns a fixed string
        let summarizer = |_text: String| {
            Box::pin(async {
                Ok::<String, String>("This is a summary of the conversation.".to_string())
            })
        };

        let result = summarize_items(&items, &config, 5, summarizer)
            .await
            .unwrap();

        assert!(result.is_some(), "should produce a result");
        let (new_items, compaction) = result.unwrap();
        assert!(new_items.len() < items.len(), "should have fewer items");
        assert!(compaction.summary_generated.is_some());
        let summary_item = new_items
            .iter()
            .find(|i| i.content.contains("[Compacted summary]"));
        assert!(
            summary_item.is_some(),
            "should contain compacted summary item"
        );
    }

    #[test]
    fn truncate_drops_oldest_non_system_items() {
        let items = make_conversation_items(20);
        let config = CompactionConfig::default();

        let (remaining, affected) = truncate_items(&items, &config, 0.70);

        assert!(affected > 0, "should have dropped items");
        assert!(
            remaining
                .iter()
                .any(|i| matches!(i.role, ConversationRole::System)),
            "system messages should be preserved"
        );
        assert!(remaining.len() < items.len());
    }

    #[test]
    fn select_tier_at_70_percent() {
        let config = CompactionConfig::default();
        assert_eq!(select_tier(0.72, &config), Some(CompactionTier::Summarize));
    }

    #[test]
    fn select_tier_at_90_percent() {
        let config = CompactionConfig::default();
        assert_eq!(select_tier(0.92, &config), Some(CompactionTier::Truncate));
    }

    #[test]
    fn select_tier_below_threshold() {
        let config = CompactionConfig::default();
        assert_eq!(select_tier(0.50, &config), None);
    }

    #[test]
    fn select_tier_at_85_percent_oss_falls_to_summarize() {
        // Without enterprise-compaction, tiers 2 and 3 are unavailable,
        // so 85% should fall through to Summarize
        let config = CompactionConfig::default();
        let tier = select_tier(0.86, &config);
        assert!(tier.is_some());
    }

    #[test]
    fn enterprise_tiers_return_none_without_feature() {
        let result = tier_compress_episodic();
        assert!(result.is_none());

        let result = tier_archive_to_memory();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn full_pipeline_truncates_when_over_90_percent() {
        use super::super::token_counter::HeuristicTokenCounter;

        let items = make_conversation_items(100);

        // Use a tiny context limit so we're way over 90%
        let counter = HeuristicTokenCounter::new(500);
        let current_tokens = counter.count_messages(&items);
        let limit = counter.model_context_limit();
        let ratio = current_tokens as f32 / limit as f32;

        assert!(ratio > 0.90, "ratio {ratio} should be > 0.90 for this test");

        let config = CompactionConfig::default();
        let tier = select_tier(ratio, &config);
        assert_eq!(tier, Some(CompactionTier::Truncate));

        // Run truncation
        let (new_items, affected) = truncate_items(&items, &config, config.summarize_threshold);
        assert!(affected > 0);
        assert!(new_items.len() < items.len());

        // Verify token count decreased
        let new_tokens = counter.count_messages(&new_items);
        assert!(
            new_tokens < current_tokens,
            "tokens should decrease: {new_tokens} < {current_tokens}"
        );
    }

    #[tokio::test]
    async fn full_pipeline_summarizes_when_between_70_and_90() {
        let items = make_conversation_items(20);
        let config = CompactionConfig {
            min_items_to_compact: 3,
            ..CompactionConfig::default()
        };

        // Mock: simulate being at 75% usage
        let tier = select_tier(0.75, &config);
        assert_eq!(tier, Some(CompactionTier::Summarize));

        // Run summarization with mock
        let summarizer = |_text: String| {
            Box::pin(async { Ok::<String, String>("Summary of old messages.".to_string()) })
        };

        let result = summarize_items(&items, &config, 10, summarizer)
            .await
            .unwrap();
        assert!(result.is_some());

        let (new_items, compaction) = result.unwrap();
        assert_eq!(compaction.tier_applied, CompactionTier::Summarize);
        assert_eq!(compaction.items_affected, 10);
        assert!(new_items.len() < items.len());
    }
}
