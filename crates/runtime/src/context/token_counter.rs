//! Multi-model token counting for context compaction.
//!
//! Provides a [`TokenCounter`] trait with implementations for various LLM
//! providers. Uses tiktoken-rs for OpenAI/Claude models and falls back to
//! a character-based heuristic for unknown models.

use super::types::ConversationItem;

/// Trait for counting tokens in text and messages.
pub trait TokenCounter: Send + Sync {
    /// Count tokens in a single string.
    fn count_tokens(&self, text: &str) -> usize;

    /// Count tokens across a slice of conversation items.
    fn count_messages(&self, messages: &[ConversationItem]) -> usize {
        messages
            .iter()
            .map(|m| self.count_tokens(&m.content) + 4) // 4 tokens per-message overhead
            .sum()
    }

    /// Return the model's maximum context window size in tokens.
    fn model_context_limit(&self) -> usize;
}

/// Look up the context window limit for a model by name.
pub fn context_limit_for_model(model: &str) -> usize {
    let m = model.to_lowercase();

    if m.contains("claude") {
        return 200_000;
    }
    if m.contains("gpt-4o") || m.contains("gpt-4-turbo") || m.contains("o1") || m.contains("o3") {
        return 128_000;
    }
    if m.contains("gpt-4") {
        return 128_000;
    }
    if m.contains("gemini") {
        return 1_000_000;
    }
    if m.contains("qwen") {
        return 131_072;
    }
    if m.contains("llama") {
        return 128_000;
    }
    if m.contains("mistral") || m.contains("mixtral") {
        return 32_000;
    }
    if m.contains("deepseek") {
        return 128_000;
    }
    if m.contains("kimi") || m.contains("moonshot") {
        return 128_000;
    }
    if m.contains("command-r") {
        return 128_000;
    }

    // Conservative default
    32_000
}

/// Token counter using tiktoken-rs (cl100k_base or o200k_base).
///
/// Works natively for OpenAI models. For Claude, uses cl100k_base as an
/// approximation (both are BPE with similar vocab sizes).
pub struct TiktokenCounter {
    bpe: tiktoken_rs::CoreBPE,
    context_limit: usize,
}

impl TiktokenCounter {
    /// Create a counter for the given model name.
    ///
    /// Resolution order:
    /// 1. o200k_base for GPT-4o family
    /// 2. cl100k_base for GPT-4, Claude, and everything else
    pub fn for_model(model: &str) -> Self {
        let model_lower = model.to_lowercase();

        // Try o200k_base for GPT-4o family
        if model_lower.contains("gpt-4o")
            || model_lower.contains("o1")
            || model_lower.contains("o3")
        {
            if let Ok(bpe) = tiktoken_rs::o200k_base() {
                return Self {
                    bpe,
                    context_limit: context_limit_for_model(model),
                };
            }
        }

        // cl100k_base for GPT-4, Claude, and everything else tiktoken supports
        let bpe = tiktoken_rs::cl100k_base().expect("tiktoken-rs failed to load cl100k_base");
        Self {
            bpe,
            context_limit: context_limit_for_model(model),
        }
    }
}

impl TokenCounter for TiktokenCounter {
    fn count_tokens(&self, text: &str) -> usize {
        self.bpe.encode_with_special_tokens(text).len()
    }

    fn model_context_limit(&self) -> usize {
        self.context_limit
    }
}

/// Create the best available token counter for the given model.
///
/// Resolution:
/// 1. tiktoken-rs for OpenAI, Claude, and well-known models
/// 2. Heuristic fallback for unknown models
pub fn create_token_counter(model: &str) -> Box<dyn TokenCounter> {
    let m = model.to_lowercase();

    // tiktoken works well for OpenAI, Claude (cl100k approx), and most major models
    let use_tiktoken = m.contains("gpt")
        || m.contains("claude")
        || m.contains("o1")
        || m.contains("o3")
        || m.contains("text-embedding");

    if use_tiktoken {
        Box::new(TiktokenCounter::for_model(model))
    } else {
        // For Qwen, Llama, Mistral, Gemini, etc. — use heuristic
        // (HuggingFace tokenizer loading requires network/cache and is deferred to a future PR)
        Box::new(HeuristicTokenCounter::new(context_limit_for_model(model)))
    }
}

/// Heuristic token counter: chars / 3.5, rounded up, +15% safety margin.
pub struct HeuristicTokenCounter {
    context_limit: usize,
}

impl HeuristicTokenCounter {
    pub fn new(context_limit: usize) -> Self {
        Self { context_limit }
    }
}

impl TokenCounter for HeuristicTokenCounter {
    fn count_tokens(&self, text: &str) -> usize {
        let raw = (text.len() as f64 / 3.5).ceil() as usize;
        raw + raw / 7 // +~15% safety margin
    }

    fn model_context_limit(&self) -> usize {
        self.context_limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heuristic_counter_counts_tokens() {
        let counter = HeuristicTokenCounter::new(32_000);
        let count = counter.count_tokens("hello world");
        assert!(count > 0, "should count some tokens");
        assert!(count < 20, "heuristic should be reasonable for short text");
    }

    #[test]
    fn heuristic_counter_empty_string() {
        let counter = HeuristicTokenCounter::new(32_000);
        assert_eq!(counter.count_tokens(""), 0);
    }

    #[test]
    fn heuristic_counter_context_limit() {
        let counter = HeuristicTokenCounter::new(128_000);
        assert_eq!(counter.model_context_limit(), 128_000);
    }

    #[test]
    fn tiktoken_counter_counts_gpt4o() {
        let counter = TiktokenCounter::for_model("gpt-4o");
        let count = counter.count_tokens("Hello, world!");
        assert!(count > 0);
        assert!(
            count < 10,
            "short greeting should be under 10 tokens, got {count}"
        );
        assert_eq!(counter.model_context_limit(), 128_000);
    }

    #[test]
    fn tiktoken_counter_counts_claude() {
        let counter = TiktokenCounter::for_model("claude-sonnet-4-5-20250929");
        let count = counter.count_tokens("Hello, world!");
        assert!(count > 0);
        assert_eq!(counter.model_context_limit(), 200_000);
    }

    #[test]
    fn factory_returns_tiktoken_for_openai() {
        let counter = create_token_counter("gpt-4o");
        let count = counter.count_tokens("Hello");
        assert!(count > 0);
        assert_eq!(counter.model_context_limit(), 128_000);
    }

    #[test]
    fn factory_returns_tiktoken_for_claude() {
        let counter = create_token_counter("claude-haiku-4-5-20251001");
        let count = counter.count_tokens("Hello");
        assert!(count > 0);
        assert_eq!(counter.model_context_limit(), 200_000);
    }

    #[test]
    fn factory_returns_heuristic_for_unknown() {
        let counter = create_token_counter("my-custom-local-model");
        let count = counter.count_tokens("Hello");
        assert!(count > 0);
        assert_eq!(counter.model_context_limit(), 32_000);
    }
}
