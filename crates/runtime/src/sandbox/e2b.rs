//! E2B.dev sandbox implementation
//!
//! Provides integration with E2B.dev cloud sandboxing service for secure code execution.

use super::{ExecutionResult, SandboxRunner};
use async_trait::async_trait;
use std::collections::HashMap;

/// E2B sandbox implementation for cloud-based code execution
#[derive(Debug, Clone)]
pub struct E2BSandbox {
    /// API key for E2B.dev service
    pub api_key: String,
    /// E2B service endpoint URL
    pub endpoint: String,
}

impl E2BSandbox {
    /// Create a new E2B sandbox instance
    ///
    /// # Arguments
    /// * `api_key` - API key for E2B.dev service authentication
    /// * `endpoint` - E2B service endpoint URL
    ///
    /// # Returns
    /// New E2BSandbox instance
    pub fn new(api_key: String, endpoint: String) -> Self {
        Self { api_key, endpoint }
    }

    /// Create E2B sandbox with default endpoint
    ///
    /// # Arguments
    /// * `api_key` - API key for E2B.dev service authentication
    ///
    /// # Returns
    /// New E2BSandbox instance with default endpoint
    pub fn new_with_default_endpoint(api_key: String) -> Self {
        Self::new(api_key, "https://api.e2b.dev".to_string())
    }
}

#[async_trait]
impl SandboxRunner for E2BSandbox {
    async fn execute(
        &self,
        code: &str,
        env: HashMap<String, String>,
    ) -> Result<ExecutionResult, anyhow::Error> {
        tracing::debug!(
            "E2B sandbox execution requested for {} chars of code with {} env vars",
            code.len(),
            env.len()
        );

        // Create HTTP client with timeout
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to create HTTP client: {}", e))?;

        // Prepare execution request payload
        let execution_request = serde_json::json!({
            "code": code,
            "environment": env,
            "timeout": 30000, // 30 seconds in milliseconds
            "language": "python" // Default to Python, could be configurable
        });

        let execution_url = format!("{}/v1/sandboxes/execute", self.endpoint);
        
        // Make HTTP request to E2B API
        let start_time = std::time::Instant::now();
        let response = client
            .post(&execution_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&execution_request)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("E2B API request failed: {}", e))?;

        let execution_duration = start_time.elapsed().as_millis() as u64;
        let status_code = response.status();

        if !status_code.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "E2B execution failed with status {}: {}",
                status_code,
                error_text
            ));
        }

        // Parse response
        let response_json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to parse E2B response: {}", e))?;

        // Extract execution results
        let success = response_json
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let stdout = response_json
            .get("stdout")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let stderr = response_json
            .get("stderr")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let exit_code = response_json
            .get("exit_code")
            .and_then(|v| v.as_i64())
            .unwrap_or(if success { 0 } else { 1 }) as i32;

        tracing::info!(
            "E2B execution completed in {}ms, exit_code: {}, success: {}",
            execution_duration,
            exit_code,
            success
        );

        if success {
            Ok(ExecutionResult::success(stdout, execution_duration))
        } else {
            Ok(ExecutionResult::failure(
                exit_code,
                stderr,
                execution_duration,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_e2b_sandbox_creation() {
        let sandbox = E2BSandbox::new(
            "test_api_key".to_string(),
            "https://test.e2b.dev".to_string(),
        );
        
        assert_eq!(sandbox.api_key, "test_api_key");
        assert_eq!(sandbox.endpoint, "https://test.e2b.dev");
    }

    #[test]
    fn test_e2b_sandbox_default_endpoint() {
        let sandbox = E2BSandbox::new_with_default_endpoint("test_api_key".to_string());
        
        assert_eq!(sandbox.api_key, "test_api_key");
        assert_eq!(sandbox.endpoint, "https://api.e2b.dev");
    }

    #[tokio::test]
    async fn test_e2b_sandbox_execute() {
        let sandbox = E2BSandbox::new(
            "test_api_key".to_string(),
            "https://test.e2b.dev".to_string(),
        );
        
        let mut env = HashMap::new();
        env.insert("TEST_VAR".to_string(), "test_value".to_string());
        
        let result = sandbox.execute("print('hello')", env).await.unwrap();
        
        assert!(result.success);
        assert_eq!(result.exit_code, 0);
        assert!(!result.stdout.is_empty());
    }
}