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

#[derive(Clone)]
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

    /// Generate a response for a general prompt
    pub async fn generate_response(&self, prompt: &str) -> Result<String> {
        info!("Generating response with OpenRouter");
        
        let messages = vec![Message {
            role: "user".to_string(),
            content: prompt.to_string(),
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
            .post(format!("{}/chat/completions", self.config.base_url))
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

/// Represents an interpreted instruction from natural language
pub struct Interpretation {
    pub intent: Intent,
    pub scope: Scope,
    pub complexity: Complexity,
    pub files_affected: Vec<String>,
    pub dependencies: Vec<String>,
    pub risks: Vec<String>,
}

/// The primary intent category of the instruction
#[derive(Debug, Clone)]
pub enum Intent {
    Create,
    Modify,
    Delete,
    Refactor,
    Debug,
    Test,
    Document,
    Optimize,
}

/// Scope of changes required
#[derive(Debug, Clone)]
pub enum Scope {
    File,
    Module,
    Package,
    Repository,
}

/// Complexity assessment of the task
#[derive(Debug, Clone)]
pub enum Complexity {
    Simple,
    Medium,
    Complex,
    Unsafe,
}

// Helper functions for different types of analysis
impl OpenRouterClient {
    pub async fn generate_execution_plan(&self, user_prompt: &str, repo_context: &str) -> Result<crate::planner::ExecutionPlan> {
        info!("Generating execution plan using OpenRouter");
        
        let system_prompt = r#"You are an expert software engineer tasked with converting natural language requests into structured execution plans.

Your job is to analyze the user's request in the context of the provided repository information and generate a JSON object that represents a detailed execution plan.

The JSON should match this exact structure:
{
  "steps": [
    {
      "description": "Human readable description of what this step does",
      "action_type": "CreateFile" | "ModifyFile" | "DeleteFile" | "RunCommand" | "Validate",
      "files": ["array", "of", "file", "paths"],
      "requires_confirmation": true | false
    }
  ],
  "estimated_duration": "PT5M", // ISO 8601 duration format
  "risk_level": "Low" | "Medium" | "High" | "Critical"
}

Guidelines:
- Break down the request into logical, sequential steps
- Be specific about which files need to be modified
- Set requires_confirmation to true for potentially dangerous operations
- Estimate realistic durations (use ISO 8601 format like PT5M for 5 minutes)
- Assess risk appropriately based on the scope of changes
- Include validation steps where appropriate
- Be conservative with risk assessment - err on the side of caution

Return ONLY the JSON object, no additional text or formatting."#;

        let user_message = format!(
            "User Request: {}\n\nRepository Context:\n{}\n\nPlease generate an execution plan for this request.",
            user_prompt, repo_context
        );

        let messages = vec![
            Message {
                role: "system".to_string(),
                content: system_prompt.to_string(),
            },
            Message {
                role: "user".to_string(),
                content: user_message,
            }
        ];

        let response = self.make_request(messages).await?;
        
        // Parse the JSON response into an ExecutionPlan
        self.parse_execution_plan(&response).await
    }

    async fn parse_execution_plan(&self, json_response: &str) -> Result<crate::planner::ExecutionPlan> {
        use crate::planner::{ExecutionPlan, ExecutionStep, ActionType, RiskLevel};
        use std::time::Duration;
        
        // Clean the response - remove any markdown formatting
        let cleaned_response = json_response
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();
        
        #[derive(serde::Deserialize)]
        struct JsonExecutionPlan {
            steps: Vec<JsonExecutionStep>,
            estimated_duration: String,
            risk_level: String,
        }
        
        #[derive(serde::Deserialize)]
        struct JsonExecutionStep {
            description: String,
            action_type: String,
            files: Vec<String>,
            requires_confirmation: bool,
        }
        
        let json_plan: JsonExecutionPlan = serde_json::from_str(cleaned_response)
            .map_err(|e| anyhow::anyhow!("Failed to parse execution plan JSON: {}. Response was: {}", e, cleaned_response))?;
        
        // Convert JSON to our internal types
        let steps = json_plan.steps.into_iter().map(|step| {
            let action_type = match step.action_type.as_str() {
                "CreateFile" => ActionType::CreateFile,
                "ModifyFile" => ActionType::ModifyFile,
                "DeleteFile" => ActionType::DeleteFile,
                "RunCommand" => ActionType::RunCommand,
                "Validate" => ActionType::Validate,
                _ => ActionType::ModifyFile, // Default fallback
            };
            
            ExecutionStep {
                description: step.description,
                action_type,
                files: step.files,
                requires_confirmation: step.requires_confirmation,
            }
        }).collect();
        
        let risk_level = match json_plan.risk_level.as_str() {
            "Low" => RiskLevel::Low,
            "Medium" => RiskLevel::Medium,
            "High" => RiskLevel::High,
            "Critical" => RiskLevel::Critical,
            _ => RiskLevel::Medium, // Default fallback
        };
        
        // Parse ISO 8601 duration or use a default
        let estimated_duration = parse_iso8601_duration(&json_plan.estimated_duration)
            .unwrap_or_else(|_| Duration::from_secs(300)); // 5 minutes default
        
        Ok(ExecutionPlan {
            steps,
            estimated_duration,
            risk_level,
        })
    }

    pub async fn interpret_prompt(&self, _prompt: &str, _repo_context: &str) -> Result<Interpretation> {
        // This could be implemented to provide more detailed analysis
        Ok(Interpretation {
            intent: Intent::Modify,
            scope: Scope::File,
            complexity: Complexity::Medium,
            files_affected: vec![],
            dependencies: vec![],
            risks: vec![],
        })
    }

    pub async fn generate_code_changes(&self, _interpretation: &Interpretation, _context: &str) -> Result<String> {
        // Placeholder for future implementation
        Ok("Code changes would be generated here".to_string())
    }

    pub async fn validate_proposed_changes(&self, _changes: &str, _context: &str) -> Result<String> {
        // Placeholder for future implementation
        Ok("Validation results would be here".to_string())
    }

    pub async fn explain_changes(&self, _changes: &str, _rationale: &str) -> Result<String> {
        // Placeholder for future implementation
        Ok("Change explanation would be here".to_string())
    }

    pub async fn assess_risk(&self, _changes: &str, _context: &str) -> Result<String> {
        // Placeholder for future implementation
        Ok("Risk assessment would be here".to_string())
    }
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

/// Parse ISO 8601 duration string into std::time::Duration
fn parse_iso8601_duration(duration_str: &str) -> Result<std::time::Duration> {
    // Simple parser for basic ISO 8601 durations like PT5M, PT1H30M, PT45S
    if !duration_str.starts_with("PT") {
        return Err(anyhow::anyhow!("Invalid ISO 8601 duration format: {}", duration_str));
    }
    
    let duration_part = &duration_str[2..]; // Remove "PT" prefix
    let mut total_seconds = 0u64;
    let mut current_number = String::new();
    
    for ch in duration_part.chars() {
        if ch.is_ascii_digit() {
            current_number.push(ch);
        } else if !current_number.is_empty() {
            let number: u64 = current_number.parse()
                .map_err(|_| anyhow::anyhow!("Invalid number in duration: {}", current_number))?;
            
            match ch {
                'H' => total_seconds += number * 3600, // hours
                'M' => total_seconds += number * 60,   // minutes
                'S' => total_seconds += number,        // seconds
                _ => return Err(anyhow::anyhow!("Unsupported duration unit: {}", ch)),
            }
            
            current_number.clear();
        }
    }
    
    Ok(std::time::Duration::from_secs(total_seconds))
}