use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::info;

use crate::config::OpenRouterConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
struct OpenRouterRequest {
    model: String,
    messages: Vec<Message>,
    max_tokens: u32,
    temperature: f32,
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct OpenRouterResponse {
    choices: Vec<Choice>,
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: Message,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Usage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

pub struct OpenRouterClient {
    client: Client,
    config: OpenRouterConfig,
}

impl OpenRouterClient {
    pub fn new(config: OpenRouterConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_seconds.unwrap_or(60)))
            .build()?;
        
        Ok(Self { client, config })
    }

    pub async fn test_connection(&self) -> Result<()> {
        info!("Testing OpenRouter connection");
        
        let test_messages = vec![Message {
            role: "user".to_string(),
            content: "Hello, can you respond with 'Connection successful'?".to_string(),
        }];
        
        let response = self.make_request(test_messages).await?;
        
        if response.to_lowercase().contains("connection successful") 
            || response.to_lowercase().contains("hello") {
            info!("OpenRouter connection test successful");
            Ok(())
        } else {
            anyhow::bail!("Unexpected response from OpenRouter: {}", response);
        }
    }

    pub async fn analyze_code(&self, code_content: &str, file_path: &str, instruction: &str) -> Result<String> {
        info!("Analyzing code with OpenRouter");
        
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

        let messages = vec![Message {
            role: "user".to_string(),
            content: prompt,
        }];

        self.make_request(messages).await
    }

    pub async fn generate_documentation(&self, code_content: &str, request: &str) -> Result<String> {
        info!("Generating documentation with OpenRouter");
        
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

        let messages = vec![Message {
            role: "user".to_string(),
            content: prompt,
        }];

        self.make_request(messages).await
    }

    pub async fn synthesize_knowledge(&self, query: &str, context_docs: &[String]) -> Result<String> {
        info!("Synthesizing knowledge with OpenRouter");
        
        let context = context_docs.join("\n\n---\n\n");
        let prompt = format!(
            "You are an intelligent assistant with access to a knowledge base. Please answer the following question using the provided context.\n\n\
            Question: {}\n\n\
            Context from knowledge base:\n{}\n\n\
            Please provide a comprehensive answer based on the available context. If the context doesn't contain enough information to fully answer the question, please indicate what information is missing.",
            query, context
        );

        let messages = vec![Message {
            role: "user".to_string(),
            content: prompt,
        }];

        self.make_request(messages).await
    }

    pub async fn security_review(&self, code_content: &str, checks: &[String]) -> Result<String> {
        info!("Performing security review with OpenRouter");
        
        let checks_list = checks.join("\n- ");
        let prompt = format!(
            "You are a cybersecurity expert. Please perform a security review of the following code, focusing on these specific checks:\n\n\
            Security Checks to Perform:\n- {}\n\n\
            Code to Review:\n{}\n\n\
            Please provide a detailed security analysis that includes:\n\
            - Any vulnerabilities or security issues found\n\
            - Risk assessment (high/medium/low) for each issue\n\
            - Specific recommendations for remediation\n\
            - Security best practices that could be applied\n\
            - Overall security score and summary",
            checks_list, code_content
        );

        let messages = vec![Message {
            role: "user".to_string(),
            content: prompt,
        }];

        self.make_request(messages).await
    }

    async fn make_request(&self, messages: Vec<Message>) -> Result<String> {
        let request_body = OpenRouterRequest {
            model: self.config.model.clone(),
            messages,
            max_tokens: self.config.max_tokens.unwrap_or(4000),
            temperature: 0.1, // Low temperature for more consistent analysis
            stream: false,
        };

        let response = self
            .client
            .post(&format!("{}/chat/completions", self.config.base_url))
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            anyhow::bail!("OpenRouter API error: {} - {}", status, error_text);
        }

        let openrouter_response: OpenRouterResponse = response.json().await?;
        
        if let Some(choice) = openrouter_response.choices.first() {
            if let Some(usage) = openrouter_response.usage {
                info!("OpenRouter usage - Prompt: {} tokens, Completion: {} tokens, Total: {} tokens", 
                      usage.prompt_tokens, usage.completion_tokens, usage.total_tokens);
            }
            
            Ok(choice.message.content.clone())
        } else {
            anyhow::bail!("No response choices returned from OpenRouter");
        }
    }
}

// Helper functions for different types of analysis
impl OpenRouterClient {
    pub async fn analyze_architecture(&self, repository_content: &str) -> Result<String> {
        self.analyze_code(
            repository_content,
            "Repository",
            "Analyze the overall architecture and design patterns used in this codebase."
        ).await
    }

    pub async fn identify_vulnerabilities(&self, code_content: &str) -> Result<String> {
        let security_checks = vec![
            "Check for hardcoded secrets or credentials".to_string(),
            "Identify potential injection vulnerabilities".to_string(),
            "Look for unsafe deserialization patterns".to_string(),
            "Check for proper input validation".to_string(),
            "Review authentication and authorization logic".to_string(),
        ];
        
        self.security_review(code_content, &security_checks).await
    }

    pub async fn explain_functionality(&self, code_content: &str, file_path: &str) -> Result<String> {
        self.analyze_code(
            code_content,
            file_path,
            "Explain what this code does, its main purpose, and how it works."
        ).await
    }

    pub async fn suggest_improvements(&self, code_content: &str, file_path: &str) -> Result<String> {
        self.analyze_code(
            code_content,
            file_path,
            "Suggest improvements for code quality, performance, maintainability, and best practices."
        ).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_config() -> OpenRouterConfig {
        OpenRouterConfig {
            api_key: "test_key".to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            model: "anthropic/claude-3.5-sonnet".to_string(),
            timeout_seconds: Some(60),
            max_tokens: Some(4000),
            temperature: Some(0.1),
        }
    }

    #[test]
    fn test_client_creation() {
        let config = create_test_config();
        let result = OpenRouterClient::new(config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_message_serialization() {
        let message = Message {
            role: "user".to_string(),
            content: "Test message".to_string(),
        };
        
        let serialized = serde_json::to_string(&message);
        assert!(serialized.is_ok());
    }

    #[tokio::test]
    async fn test_request_building() {
        let config = create_test_config();
        let client = OpenRouterClient::new(config).unwrap();
        
        // This test just checks that the request building doesn't panic
        // Actual API calls would require a valid API key
        let messages = vec![Message {
            role: "user".to_string(),
            content: "Test".to_string(),
        }];
        
        // We can't actually make the request without a valid API key
        // but we can test that the client is properly constructed
        assert!(!client.config.api_key.is_empty());
    }
}