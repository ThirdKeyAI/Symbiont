//! Embedding service providers for generating vector embeddings
//!
//! Supports Ollama (local) and OpenAI (cloud) embedding providers,
//! with automatic provider detection from environment variables.

use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

use super::types::ContextError;
use super::vector_db::{EmbeddingService, MockEmbeddingService};

/// Embedding provider selection
#[derive(Debug, Clone, PartialEq)]
pub enum EmbeddingProvider {
    Ollama,
    OpenAi,
}

/// Configuration for an embedding service provider
#[derive(Debug, Clone)]
pub struct EmbeddingConfig {
    pub provider: EmbeddingProvider,
    pub model: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub dimension: usize,
    pub timeout_seconds: u64,
}

impl EmbeddingConfig {
    /// Resolve embedding configuration from environment variables.
    ///
    /// Returns `None` if no provider can be determined (no env vars set),
    /// which signals the caller to fall back to the mock service.
    ///
    /// Resolution order:
    /// 1. API key: `EMBEDDING_API_KEY` → `OPENAI_API_KEY` → None
    /// 2. Provider: `EMBEDDING_PROVIDER` explicit, or auto-detect from URL/key
    /// 3. Per-provider defaults for model, URL, and dimension
    /// 4. Overrides: `EMBEDDING_MODEL`, `EMBEDDING_API_BASE_URL`, `VECTOR_DIMENSION`
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("EMBEDDING_API_KEY")
            .ok()
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .filter(|k| !k.is_empty());

        let base_url = std::env::var("EMBEDDING_API_BASE_URL")
            .ok()
            .or_else(|| std::env::var("OPENAI_API_BASE_URL").ok())
            .filter(|u| !u.is_empty());

        let explicit_provider = std::env::var("EMBEDDING_PROVIDER")
            .ok()
            .filter(|p| !p.is_empty());

        let provider = if let Some(ref p) = explicit_provider {
            match p.to_lowercase().as_str() {
                "ollama" => EmbeddingProvider::Ollama,
                "openai" => EmbeddingProvider::OpenAi,
                _ => return None,
            }
        } else if let Some(ref url) = base_url {
            if url.contains("localhost") || url.contains("127.0.0.1") {
                EmbeddingProvider::Ollama
            } else if api_key.is_some() {
                EmbeddingProvider::OpenAi
            } else {
                return None;
            }
        } else if api_key.is_some() {
            EmbeddingProvider::OpenAi
        } else {
            return None;
        };

        let (default_model, default_url, default_dim) = match provider {
            EmbeddingProvider::Ollama => (
                "nomic-embed-text".to_string(),
                "http://localhost:11434".to_string(),
                768,
            ),
            EmbeddingProvider::OpenAi => (
                "text-embedding-3-small".to_string(),
                "https://api.openai.com/v1".to_string(),
                1536,
            ),
        };

        let model = std::env::var("EMBEDDING_MODEL")
            .ok()
            .filter(|m| !m.is_empty())
            .unwrap_or(default_model);

        let final_url = base_url.unwrap_or(default_url);

        let dimension = std::env::var("VECTOR_DIMENSION")
            .ok()
            .and_then(|d| d.parse::<usize>().ok())
            .unwrap_or(default_dim);

        Some(Self {
            provider,
            model,
            base_url: final_url,
            api_key,
            dimension,
            timeout_seconds: 30,
        })
    }
}

/// Ollama embedding service using the native `/api/embed` endpoint
pub struct OllamaEmbeddingService {
    client: reqwest::Client,
    model: String,
    base_url: String,
    dimension: usize,
}

impl OllamaEmbeddingService {
    pub fn new(config: &EmbeddingConfig) -> Result<Self, ContextError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_seconds))
            .build()
            .map_err(|e| ContextError::EmbeddingError {
                reason: format!("Failed to create HTTP client: {e}"),
            })?;

        Ok(Self {
            client,
            model: config.model.clone(),
            base_url: config.base_url.trim_end_matches('/').to_string(),
            dimension: config.dimension,
        })
    }
}

#[async_trait]
impl EmbeddingService for OllamaEmbeddingService {
    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>, ContextError> {
        let mut results = self.generate_batch_embeddings(vec![text]).await?;
        results.pop().ok_or_else(|| ContextError::EmbeddingError {
            reason: "Empty response from Ollama".to_string(),
        })
    }

    async fn generate_batch_embeddings(
        &self,
        texts: Vec<&str>,
    ) -> Result<Vec<Vec<f32>>, ContextError> {
        let url = format!("{}/api/embed", self.base_url);

        let body = serde_json::json!({
            "model": self.model,
            "input": texts,
        });

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| ContextError::EmbeddingError {
                reason: format!("Ollama request failed: {e}"),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(ContextError::EmbeddingError {
                reason: format!("Ollama returned {status}: {body_text}"),
            });
        }

        let json: serde_json::Value =
            resp.json()
                .await
                .map_err(|e| ContextError::EmbeddingError {
                    reason: format!("Failed to parse Ollama response: {e}"),
                })?;

        let embeddings = json
            .get("embeddings")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ContextError::EmbeddingError {
                reason: "Missing 'embeddings' field in Ollama response".to_string(),
            })?;

        embeddings
            .iter()
            .map(|emb| {
                emb.as_array()
                    .ok_or_else(|| ContextError::EmbeddingError {
                        reason: "Invalid embedding array in Ollama response".to_string(),
                    })?
                    .iter()
                    .map(|v| {
                        v.as_f64()
                            .map(|f| f as f32)
                            .ok_or_else(|| ContextError::EmbeddingError {
                                reason: "Invalid float in embedding".to_string(),
                            })
                    })
                    .collect::<Result<Vec<f32>, _>>()
            })
            .collect()
    }

    fn embedding_dimension(&self) -> usize {
        self.dimension
    }

    fn max_text_length(&self) -> usize {
        8192
    }
}

/// OpenAI-compatible embedding service
pub struct OpenAiEmbeddingService {
    client: reqwest::Client,
    model: String,
    base_url: String,
    api_key: String,
    dimension: usize,
}

impl OpenAiEmbeddingService {
    pub fn new(config: &EmbeddingConfig) -> Result<Self, ContextError> {
        let api_key = config
            .api_key
            .clone()
            .filter(|k| !k.is_empty())
            .ok_or_else(|| ContextError::EmbeddingError {
                reason: "OpenAI embedding service requires an API key".to_string(),
            })?;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_seconds))
            .build()
            .map_err(|e| ContextError::EmbeddingError {
                reason: format!("Failed to create HTTP client: {e}"),
            })?;

        Ok(Self {
            client,
            model: config.model.clone(),
            base_url: config.base_url.trim_end_matches('/').to_string(),
            api_key,
            dimension: config.dimension,
        })
    }
}

#[async_trait]
impl EmbeddingService for OpenAiEmbeddingService {
    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>, ContextError> {
        let mut results = self.generate_batch_embeddings(vec![text]).await?;
        results.pop().ok_or_else(|| ContextError::EmbeddingError {
            reason: "Empty response from OpenAI".to_string(),
        })
    }

    async fn generate_batch_embeddings(
        &self,
        texts: Vec<&str>,
    ) -> Result<Vec<Vec<f32>>, ContextError> {
        let url = format!("{}/embeddings", self.base_url);

        let body = serde_json::json!({
            "model": self.model,
            "input": texts,
        });

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| ContextError::EmbeddingError {
                reason: format!("OpenAI request failed: {e}"),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(ContextError::EmbeddingError {
                reason: format!("OpenAI returned {status}: {body_text}"),
            });
        }

        let json: serde_json::Value =
            resp.json()
                .await
                .map_err(|e| ContextError::EmbeddingError {
                    reason: format!("Failed to parse OpenAI response: {e}"),
                })?;

        // Log token usage
        if let Some(usage) = json.get("usage") {
            tracing::debug!(
                prompt_tokens = usage.get("prompt_tokens").and_then(|v| v.as_u64()),
                total_tokens = usage.get("total_tokens").and_then(|v| v.as_u64()),
                "OpenAI embedding token usage"
            );
        }

        let data = json.get("data").and_then(|v| v.as_array()).ok_or_else(|| {
            ContextError::EmbeddingError {
                reason: "Missing 'data' field in OpenAI response".to_string(),
            }
        })?;

        // Sort by index to ensure correct ordering
        let mut indexed: Vec<(usize, Vec<f32>)> = data
            .iter()
            .map(|item| {
                let index = item.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

                let embedding = item
                    .get("embedding")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| ContextError::EmbeddingError {
                        reason: "Missing 'embedding' in OpenAI response item".to_string(),
                    })?
                    .iter()
                    .map(|v| {
                        v.as_f64()
                            .map(|f| f as f32)
                            .ok_or_else(|| ContextError::EmbeddingError {
                                reason: "Invalid float in embedding".to_string(),
                            })
                    })
                    .collect::<Result<Vec<f32>, _>>()?;

                Ok((index, embedding))
            })
            .collect::<Result<Vec<_>, ContextError>>()?;

        indexed.sort_by_key(|(i, _)| *i);

        Ok(indexed.into_iter().map(|(_, emb)| emb).collect())
    }

    fn embedding_dimension(&self) -> usize {
        self.dimension
    }

    fn max_text_length(&self) -> usize {
        8191 // OpenAI token limit
    }
}

/// Create an embedding service from a resolved config.
pub fn create_embedding_service(
    config: &EmbeddingConfig,
) -> Result<Arc<dyn EmbeddingService>, ContextError> {
    match config.provider {
        EmbeddingProvider::Ollama => {
            tracing::info!(
                model = %config.model,
                url = %config.base_url,
                dimension = config.dimension,
                "Using Ollama embedding service"
            );
            Ok(Arc::new(OllamaEmbeddingService::new(config)?))
        }
        EmbeddingProvider::OpenAi => {
            tracing::info!(
                model = %config.model,
                url = %config.base_url,
                dimension = config.dimension,
                "Using OpenAI embedding service"
            );
            Ok(Arc::new(OpenAiEmbeddingService::new(config)?))
        }
    }
}

/// Create an embedding service from environment variables, falling back to
/// `MockEmbeddingService` when no provider is configured.
pub fn create_embedding_service_from_env(
    fallback_dimension: usize,
) -> Result<Arc<dyn EmbeddingService>, ContextError> {
    match EmbeddingConfig::from_env() {
        Some(config) => create_embedding_service(&config),
        None => {
            tracing::debug!(
                dimension = fallback_dimension,
                "No embedding provider configured, using mock embedding service"
            );
            Ok(Arc::new(MockEmbeddingService::new(fallback_dimension)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    /// Helper: clear all embedding-related env vars before each test
    fn clear_env() {
        for var in &[
            "EMBEDDING_PROVIDER",
            "EMBEDDING_API_KEY",
            "OPENAI_API_KEY",
            "EMBEDDING_API_BASE_URL",
            "OPENAI_API_BASE_URL",
            "EMBEDDING_MODEL",
            "VECTOR_DIMENSION",
        ] {
            std::env::remove_var(var);
        }
    }

    #[test]
    #[serial]
    fn test_embedding_config_defaults_ollama() {
        clear_env();
        std::env::set_var("EMBEDDING_PROVIDER", "ollama");

        let config = EmbeddingConfig::from_env().expect("should resolve");
        assert_eq!(config.provider, EmbeddingProvider::Ollama);
        assert_eq!(config.model, "nomic-embed-text");
        assert_eq!(config.base_url, "http://localhost:11434");
        assert_eq!(config.dimension, 768);
        assert!(config.api_key.is_none());
    }

    #[test]
    #[serial]
    fn test_embedding_config_defaults_openai() {
        clear_env();
        std::env::set_var("EMBEDDING_PROVIDER", "openai");
        std::env::set_var("OPENAI_API_KEY", "sk-test");

        let config = EmbeddingConfig::from_env().expect("should resolve");
        assert_eq!(config.provider, EmbeddingProvider::OpenAi);
        assert_eq!(config.model, "text-embedding-3-small");
        assert_eq!(config.base_url, "https://api.openai.com/v1");
        assert_eq!(config.dimension, 1536);
        assert_eq!(config.api_key.as_deref(), Some("sk-test"));
    }

    #[test]
    #[serial]
    fn test_embedding_config_auto_detect_openai_from_key() {
        clear_env();
        std::env::set_var("OPENAI_API_KEY", "sk-auto");

        let config = EmbeddingConfig::from_env().expect("should resolve");
        assert_eq!(config.provider, EmbeddingProvider::OpenAi);
        assert_eq!(config.api_key.as_deref(), Some("sk-auto"));
    }

    #[test]
    #[serial]
    fn test_embedding_config_auto_detect_ollama_from_localhost_url() {
        clear_env();
        std::env::set_var("EMBEDDING_API_BASE_URL", "http://localhost:11434");

        let config = EmbeddingConfig::from_env().expect("should resolve");
        assert_eq!(config.provider, EmbeddingProvider::Ollama);
    }

    #[test]
    #[serial]
    fn test_embedding_config_none_when_no_provider() {
        clear_env();
        assert!(EmbeddingConfig::from_env().is_none());
    }

    #[test]
    #[serial]
    fn test_embedding_config_dimension_override() {
        clear_env();
        std::env::set_var("EMBEDDING_PROVIDER", "ollama");
        std::env::set_var("VECTOR_DIMENSION", "1024");

        let config = EmbeddingConfig::from_env().expect("should resolve");
        assert_eq!(config.dimension, 1024);
    }

    #[test]
    #[serial]
    fn test_create_embedding_service_from_env_fallback() {
        clear_env();

        let svc = create_embedding_service_from_env(256).expect("should return mock");
        assert_eq!(svc.embedding_dimension(), 256);
    }

    #[tokio::test]
    #[serial]
    async fn test_mock_fallback_generates_embeddings() {
        clear_env();

        let svc = create_embedding_service_from_env(128).expect("should return mock");
        let emb = svc.generate_embedding("hello world").await.unwrap();
        assert_eq!(emb.len(), 128);

        // Verify it's normalized (magnitude ≈ 1.0)
        let mag: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((mag - 1.0).abs() < 0.01);
    }
}
