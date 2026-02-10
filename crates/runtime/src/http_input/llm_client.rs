//! LLM client for OpenAI-compatible chat completions
//!
//! Auto-detects provider from environment variables and provides a unified
//! interface for chat completion requests.

#[cfg(feature = "http-input")]
use crate::types::RuntimeError;

/// Supported LLM providers
#[cfg(feature = "http-input")]
#[derive(Debug, Clone)]
pub enum LlmProvider {
    OpenRouter,
    OpenAI,
    Anthropic,
}

#[cfg(feature = "http-input")]
impl std::fmt::Display for LlmProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmProvider::OpenRouter => write!(f, "OpenRouter"),
            LlmProvider::OpenAI => write!(f, "OpenAI"),
            LlmProvider::Anthropic => write!(f, "Anthropic"),
        }
    }
}

/// OpenAI-compatible chat completions client
#[cfg(feature = "http-input")]
pub struct LlmClient {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    model: String,
    provider: LlmProvider,
}

#[cfg(feature = "http-input")]
impl LlmClient {
    /// Auto-detect LLM provider from environment variables.
    ///
    /// Checks in order:
    /// 1. `OPENROUTER_API_KEY` → OpenRouter (model from `OPENROUTER_MODEL`)
    /// 2. `OPENAI_API_KEY` → OpenAI (model from `CHAT_MODEL`)
    /// 3. `ANTHROPIC_API_KEY` → Anthropic (model from `ANTHROPIC_MODEL`)
    ///
    /// Returns `None` if no API key is found.
    pub fn from_env() -> Option<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .ok()?;

        if let Ok(api_key) = std::env::var("OPENROUTER_API_KEY") {
            let model = std::env::var("OPENROUTER_MODEL")
                .unwrap_or_else(|_| "anthropic/claude-sonnet-4".to_string());
            let base_url = std::env::var("OPENROUTER_BASE_URL")
                .unwrap_or_else(|_| "https://openrouter.ai/api/v1".to_string());
            tracing::info!(
                "LLM client initialized: provider=OpenRouter model={}",
                model
            );
            return Some(Self {
                client,
                api_key,
                base_url,
                model,
                provider: LlmProvider::OpenRouter,
            });
        }

        if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
            let model = std::env::var("CHAT_MODEL").unwrap_or_else(|_| "gpt-4o".to_string());
            let base_url = std::env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
            tracing::info!("LLM client initialized: provider=OpenAI model={}", model);
            return Some(Self {
                client,
                api_key,
                base_url,
                model,
                provider: LlmProvider::OpenAI,
            });
        }

        if let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") {
            let model = std::env::var("ANTHROPIC_MODEL")
                .unwrap_or_else(|_| "claude-sonnet-4-5-20250514".to_string());
            let base_url = std::env::var("ANTHROPIC_BASE_URL")
                .unwrap_or_else(|_| "https://api.anthropic.com/v1".to_string());
            tracing::info!("LLM client initialized: provider=Anthropic model={}", model);
            return Some(Self {
                client,
                api_key,
                base_url,
                model,
                provider: LlmProvider::Anthropic,
            });
        }

        tracing::info!("No LLM API key found in environment, LLM invocation disabled");
        None
    }

    /// Get the model name
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Get the provider
    pub fn provider(&self) -> &LlmProvider {
        &self.provider
    }

    /// Send a chat completion request with system and user messages.
    pub async fn chat_completion(&self, system: &str, user: &str) -> Result<String, RuntimeError> {
        match self.provider {
            LlmProvider::Anthropic => self.anthropic_completion(system, user).await,
            _ => self.openai_completion(system, user).await,
        }
    }

    /// OpenAI-compatible chat completion (works for OpenRouter and OpenAI)
    async fn openai_completion(&self, system: &str, user: &str) -> Result<String, RuntimeError> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user }
            ],
            "max_tokens": 4096,
            "temperature": 0.3
        });

        let start = std::time::Instant::now();

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| RuntimeError::Internal(format!("LLM request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(RuntimeError::Internal(format!(
                "LLM API error ({}): {}",
                status, error_text
            )));
        }

        let resp_json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| RuntimeError::Internal(format!("Failed to parse LLM response: {}", e)))?;

        let latency = start.elapsed();

        // Log usage if available
        if let Some(usage) = resp_json.get("usage") {
            tracing::info!(
                "LLM usage: provider={} model={} prompt_tokens={} completion_tokens={} total_tokens={} latency={:?}",
                self.provider,
                self.model,
                usage.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                usage.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                usage.get("total_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                latency,
            );
        }

        resp_json
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| RuntimeError::Internal("No content in LLM response choices".to_string()))
    }

    /// Anthropic Messages API completion
    async fn anthropic_completion(&self, system: &str, user: &str) -> Result<String, RuntimeError> {
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 4096,
            "system": system,
            "messages": [
                { "role": "user", "content": user }
            ]
        });

        let start = std::time::Instant::now();

        let response = self
            .client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| RuntimeError::Internal(format!("Anthropic request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(RuntimeError::Internal(format!(
                "Anthropic API error ({}): {}",
                status, error_text
            )));
        }

        let resp_json: serde_json::Value = response.json().await.map_err(|e| {
            RuntimeError::Internal(format!("Failed to parse Anthropic response: {}", e))
        })?;

        let latency = start.elapsed();

        // Log usage
        if let Some(usage) = resp_json.get("usage") {
            tracing::info!(
                "LLM usage: provider=Anthropic model={} input_tokens={} output_tokens={} latency={:?}",
                self.model,
                usage.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                usage.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                latency,
            );
        }

        // Anthropic returns content as array of content blocks
        resp_json
            .get("content")
            .and_then(|c| c.as_array())
            .and_then(|blocks| {
                blocks
                    .iter()
                    .find(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
            })
            .and_then(|b| b.get("text"))
            .and_then(|t| t.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| {
                RuntimeError::Internal("No text content in Anthropic response".to_string())
            })
    }
}

#[cfg(all(test, feature = "http-input"))]
mod tests {
    use super::*;

    #[test]
    fn test_provider_display() {
        assert_eq!(format!("{}", LlmProvider::OpenRouter), "OpenRouter");
        assert_eq!(format!("{}", LlmProvider::OpenAI), "OpenAI");
        assert_eq!(format!("{}", LlmProvider::Anthropic), "Anthropic");
    }

    #[test]
    fn test_from_env_no_keys() {
        // Remove any existing keys for the test
        std::env::remove_var("OPENROUTER_API_KEY");
        std::env::remove_var("OPENAI_API_KEY");
        std::env::remove_var("ANTHROPIC_API_KEY");

        let client = LlmClient::from_env();
        assert!(client.is_none());
    }
}
