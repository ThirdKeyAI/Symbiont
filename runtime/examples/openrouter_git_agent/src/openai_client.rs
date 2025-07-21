use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::info;

use crate::config::OpenAIConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
    usage: Option<ChatUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

#[derive(Debug, Serialize)]
struct EmbeddingRequest {
    model: String,
    input: EmbeddingInput,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum EmbeddingInput {
    String(String),
    Array(Vec<String>),
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
    usage: Option<EmbeddingUsage>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
    index: usize,
}

#[derive(Debug, Deserialize)]
struct EmbeddingUsage {
    prompt_tokens: u32,
    total_tokens: u32,
}

pub struct OpenAICompatibleClient {
    client: Client,
    config: OpenAIConfig,
}

impl OpenAICompatibleClient {
    pub fn new(config: OpenAIConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_seconds.unwrap_or(60)))
            .build()?;
        
        Ok(Self { client, config })
    }

    pub async fn test_connection(&self) -> Result<()> {
        info!("Testing OpenAI-compatible API connection");
        
        let test_messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "Hello, can you respond with 'Connection successful'?".to_string(),
        }];
        
        let response = self.chat_completion(test_messages).await?;
        
        if response.to_lowercase().contains("connection successful") 
            || response.to_lowercase().contains("hello") {
            info!("OpenAI-compatible API connection test successful");
            Ok(())
        } else {
            anyhow::bail!("Unexpected response from OpenAI-compatible API: {}", response);
        }
    }

    pub async fn chat_completion(&self, messages: Vec<ChatMessage>) -> Result<String> {
        let request_body = ChatCompletionRequest {
            model: self.config.chat_model.clone(),
            messages,
            max_tokens: self.config.max_tokens,
            temperature: self.config.temperature,
            stream: false,
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", self.config.base_url))
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            anyhow::bail!("OpenAI-compatible API error: {} - {}", status, error_text);
        }

        let chat_response: ChatCompletionResponse = response.json().await?;
        
        if let Some(choice) = chat_response.choices.first() {
            if let Some(usage) = chat_response.usage {
                info!(
                    "OpenAI-compatible API usage - Prompt: {} tokens, Completion: {} tokens, Total: {} tokens", 
                    usage.prompt_tokens, usage.completion_tokens, usage.total_tokens
                );
            }
            
            Ok(choice.message.content.clone())
        } else {
            anyhow::bail!("No response choices returned from OpenAI-compatible API");
        }
    }

    pub async fn create_embeddings(&self, input: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let embedding_base_url = self.config.embedding_api_base_url
            .as_ref()
            .unwrap_or(&self.config.base_url);
        
        let embedding_api_key = self.config.embedding_api_key
            .as_ref()
            .unwrap_or(&self.config.api_key);

        let request_body = EmbeddingRequest {
            model: self.config.embedding_model.clone(),
            input: if input.len() == 1 {
                EmbeddingInput::String(input[0].clone())
            } else {
                EmbeddingInput::Array(input)
            },
        };

        let response = self
            .client
            .post(format!("{}/embeddings", embedding_base_url))
            .header("Authorization", format!("Bearer {}", embedding_api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            anyhow::bail!("Embedding API error: {} - {}", status, error_text);
        }

        let embedding_response: EmbeddingResponse = response.json().await?;
        
        if let Some(usage) = embedding_response.usage {
            info!(
                "Embedding API usage - Prompt: {} tokens, Total: {} tokens", 
                usage.prompt_tokens, usage.total_tokens
            );
        }

        let mut embeddings = vec![vec![]; embedding_response.data.len()];
        for data in embedding_response.data {
            embeddings[data.index] = data.embedding;
        }

        Ok(embeddings)
    }

    pub async fn analyze_code(&self, code_content: &str, file_path: &str, instruction: &str) -> Result<String> {
        info!("Analyzing code with OpenAI-compatible API");
        
        let prompt = format!(
            "You are an expert software engineer. Please analyze the following code/repository content and answer the question.\n\n\
            Context: {}\n\
            File/Repository: {}\n\
            Question: {}\n\n\
            Please provide a comprehensive analysis focusing on:\n\
            - Code structure and architecture\n\
            - Key functionality and purpose\n\
            - Notable patterns or technologies used\n\
            - Any potential improvements or observations\n\n\
            Code/Content:\n{}",
            instruction, file_path, instruction, code_content
        );

        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: prompt,
        }];

        self.chat_completion(messages).await
    }

    pub async fn generate_documentation(&self, code_content: &str, request: &str) -> Result<String> {
        info!("Generating documentation with OpenAI-compatible API");
        
        let prompt = format!(
            "You are a technical documentation expert. Please generate comprehensive documentation based on the following request and code content.\n\n\
            Documentation Request: {}\n\n\
            Code Content:\n{}\n\n\
            Please provide well-structured documentation in Markdown format that includes:\n\
            - Clear explanations of functionality\n\
            - Usage examples where appropriate\n\
            - Installation or setup instructions if relevant\n\
            - API documentation for public interfaces\n\
            - Any important notes or considerations",
            request, code_content
        );

        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: prompt,
        }];

        self.chat_completion(messages).await
    }

    pub async fn synthesize_knowledge(&self, query: &str, context_docs: &[String]) -> Result<String> {
        info!("Synthesizing knowledge with OpenAI-compatible API");
        
        let context = context_docs.join("\n\n---\n\n");
        let prompt = format!(
            "You are an intelligent assistant with access to a knowledge base. Please answer the following question using the provided context.\n\n\
            Question: {}\n\n\
            Context from knowledge base:\n{}\n\n\
            Please provide a comprehensive answer based on the available context. If the context doesn't contain enough information to fully answer the question, please indicate what information is missing.",
            query, context
        );

        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: prompt,
        }];

        self.chat_completion(messages).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_config() -> OpenAIConfig {
        OpenAIConfig {
            api_key: "test_key".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            chat_model: "gpt-3.5-turbo".to_string(),
            embedding_model: "text-embedding-ada-002".to_string(),
            embedding_api_base_url: None,
            embedding_api_key: None,
            timeout_seconds: Some(60),
            max_tokens: Some(4000),
            temperature: Some(0.1),
        }
    }

    #[test]
    fn test_client_creation() {
        let config = create_test_config();
        let result = OpenAICompatibleClient::new(config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_message_serialization() {
        let message = ChatMessage {
            role: "user".to_string(),
            content: "Test message".to_string(),
        };
        
        let serialized = serde_json::to_string(&message);
        assert!(serialized.is_ok());
    }

    #[tokio::test]
    async fn test_request_building() {
        let config = create_test_config();
        let client = OpenAICompatibleClient::new(config).unwrap();
        
        // This test just checks that the request building doesn't panic
        // Actual API calls would require a valid API key
        let _messages = [ChatMessage {
            role: "user".to_string(),
            content: "Test".to_string(),
        }];
        
        // We can't actually make the request without a valid API key
        // but we can test that the client is properly constructed
        assert!(!client.config.api_key.is_empty());
    }
}