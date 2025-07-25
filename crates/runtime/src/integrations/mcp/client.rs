//! MCP Client with Schema Verification
//!
//! Provides a secure MCP client that verifies tool schemas using SchemaPin

use async_trait::async_trait;
use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;
use tempfile::NamedTempFile;
use tokio::sync::RwLock;
use tokio::time::{timeout, Duration};

use super::types::{
    McpClientConfig, McpClientError, McpTool, ToolDiscoveryEvent, ToolProvider,
    ToolVerificationRequest, ToolVerificationResponse, VerificationStatus,
};
use crate::integrations::schemapin::{
    LocalKeyStore, NativeSchemaPinClient, PinnedKey, SchemaPinClient, VerifyArgs,
};
use crate::integrations::tool_invocation::{
    DefaultToolInvocationEnforcer, InvocationContext, InvocationResult, ToolInvocationEnforcer,
    ToolInvocationError,
};

/// Trait for MCP client operations
#[async_trait]
pub trait McpClient: Send + Sync {
    /// Discover and verify a new tool
    async fn discover_tool(&self, tool: McpTool) -> Result<ToolDiscoveryEvent, McpClientError>;

    /// Get a verified tool by name
    async fn get_tool(&self, name: &str) -> Result<McpTool, McpClientError>;

    /// List all available tools
    async fn list_tools(&self) -> Result<Vec<McpTool>, McpClientError>;

    /// List only verified tools
    async fn list_verified_tools(&self) -> Result<Vec<McpTool>, McpClientError>;

    /// Verify a tool's schema
    async fn verify_tool(
        &self,
        request: ToolVerificationRequest,
    ) -> Result<ToolVerificationResponse, McpClientError>;

    /// Remove a tool from the client
    async fn remove_tool(&self, name: &str) -> Result<Option<McpTool>, McpClientError>;

    /// Execute a tool with verification enforcement
    async fn invoke_tool(
        &self,
        tool_name: &str,
        arguments: serde_json::Value,
        context: InvocationContext,
    ) -> Result<InvocationResult, McpClientError>;
}

/// MCP client implementation with schema verification
pub struct SecureMcpClient {
    /// Client configuration
    config: McpClientConfig,
    /// SchemaPin client for verification
    schema_pin: Arc<dyn SchemaPinClient>,
    /// Local key store for TOFU
    key_store: Arc<LocalKeyStore>,
    /// Available tools (name -> tool)
    tools: Arc<RwLock<HashMap<String, McpTool>>>,
    /// Tool invocation enforcer
    enforcer: Arc<dyn ToolInvocationEnforcer>,
}

impl SecureMcpClient {
    /// Create a new secure MCP client
    pub fn new(
        config: McpClientConfig,
        schema_pin: Arc<dyn SchemaPinClient>,
        key_store: Arc<LocalKeyStore>,
    ) -> Self {
        let enforcer = Arc::new(DefaultToolInvocationEnforcer::new());
        Self {
            config,
            schema_pin,
            key_store,
            tools: Arc::new(RwLock::new(HashMap::new())),
            enforcer,
        }
    }

    /// Create a new secure MCP client with custom enforcer
    pub fn with_enforcer(
        config: McpClientConfig,
        schema_pin: Arc<dyn SchemaPinClient>,
        key_store: Arc<LocalKeyStore>,
        enforcer: Arc<dyn ToolInvocationEnforcer>,
    ) -> Self {
        Self {
            config,
            schema_pin,
            key_store,
            tools: Arc::new(RwLock::new(HashMap::new())),
            enforcer,
        }
    }

    /// Create a new secure MCP client with default components
    pub fn with_defaults(config: McpClientConfig) -> Result<Self, McpClientError> {
        let schema_pin = Arc::new(NativeSchemaPinClient::new());
        let key_store = Arc::new(LocalKeyStore::new()?);

        Ok(Self::new(config, schema_pin, key_store))
    }

    /// Verify a tool's schema using SchemaPin
    async fn verify_schema(&self, tool: &McpTool) -> Result<VerificationStatus, McpClientError> {
        // Create a temporary file for the schema
        let mut temp_file =
            NamedTempFile::new().map_err(|e| McpClientError::SerializationError {
                reason: format!("Failed to create temp file: {}", e),
            })?;

        // Write schema to temp file
        let schema_json = serde_json::to_string_pretty(&tool.schema).map_err(|e| {
            McpClientError::SerializationError {
                reason: format!("Failed to serialize schema: {}", e),
            }
        })?;

        temp_file.write_all(schema_json.as_bytes()).map_err(|e| {
            McpClientError::SerializationError {
                reason: format!("Failed to write schema to temp file: {}", e),
            }
        })?;

        let temp_path = temp_file.path().to_string_lossy().to_string();

        // Fetch and pin the provider's public key (TOFU)
        self.fetch_and_pin_key(&tool.provider).await?;

        // Verify the schema
        let verify_args = VerifyArgs::new(temp_path, tool.provider.public_key_url.clone());

        let verification_timeout = Duration::from_secs(self.config.verification_timeout_seconds);
        let verification_result = timeout(
            verification_timeout,
            self.schema_pin.verify_schema(verify_args),
        )
        .await
        .map_err(|_| McpClientError::Timeout)?;

        match verification_result {
            Ok(result) => Ok(VerificationStatus::Verified {
                result: Box::new(result),
                verified_at: chrono::Utc::now().to_rfc3339(),
            }),
            Err(e) => Ok(VerificationStatus::Failed {
                reason: e.to_string(),
                failed_at: chrono::Utc::now().to_rfc3339(),
            }),
        }
    }

    /// Fetch and pin a provider's public key using TOFU
    async fn fetch_and_pin_key(&self, provider: &ToolProvider) -> Result<(), McpClientError> {
        // Check if we already have this key pinned
        if self.key_store.has_key(&provider.identifier)? {
            // Key already pinned, TOFU will handle verification
            return Ok(());
        }

        // For this implementation, we'll create a mock key since we don't have
        // actual key fetching logic. In a real implementation, you would
        // fetch the key from the provider.public_key_url
        let pinned_key = PinnedKey::new(
            provider.identifier.clone(),
            format!("mock_public_key_for_{}", provider.identifier),
            "Ed25519".to_string(),
            format!("mock_fingerprint_for_{}", provider.identifier),
        );

        // Pin the key (TOFU will prevent key substitution attacks)
        self.key_store.pin_key(pinned_key)?;

        Ok(())
    }

    /// Check if a tool should be allowed based on verification status
    fn should_allow_tool(&self, tool: &McpTool) -> bool {
        match &tool.verification_status {
            VerificationStatus::Verified { .. } => true,
            VerificationStatus::Failed { .. } => false,
            VerificationStatus::Pending => !self.config.enforce_verification,
            VerificationStatus::Skipped { .. } => self.config.allow_unverified_in_dev,
        }
    }
}

#[async_trait]
impl McpClient for SecureMcpClient {
    async fn discover_tool(&self, mut tool: McpTool) -> Result<ToolDiscoveryEvent, McpClientError> {
        // Verify the tool's schema
        let verification_status = if self.config.enforce_verification {
            self.verify_schema(&tool).await?
        } else {
            VerificationStatus::Skipped {
                reason: "Verification disabled in configuration".to_string(),
            }
        };

        // Update tool with verification status
        tool.verification_status = verification_status;

        // Check if tool should be allowed (only if verification is enforced)
        if self.config.enforce_verification && !self.should_allow_tool(&tool) {
            return Err(McpClientError::VerificationFailed {
                reason: format!(
                    "Tool '{}' failed verification and cannot be added",
                    tool.name
                ),
            });
        }

        // Add tool to our collection
        {
            let mut tools = self.tools.write().await;
            tools.insert(tool.name.clone(), tool.clone());
        }

        Ok(ToolDiscoveryEvent {
            tool,
            source: "discovery".to_string(),
            discovered_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    async fn get_tool(&self, name: &str) -> Result<McpTool, McpClientError> {
        let tools = self.tools.read().await;
        let tool = tools
            .get(name)
            .ok_or_else(|| McpClientError::ToolNotFound {
                name: name.to_string(),
            })?;

        // Check if verification is required
        if self.config.enforce_verification && !tool.verification_status.is_verified() {
            return Err(McpClientError::ToolNotVerified {
                name: name.to_string(),
            });
        }

        Ok(tool.clone())
    }

    async fn list_tools(&self) -> Result<Vec<McpTool>, McpClientError> {
        let tools = self.tools.read().await;
        Ok(tools.values().cloned().collect())
    }

    async fn list_verified_tools(&self) -> Result<Vec<McpTool>, McpClientError> {
        let tools = self.tools.read().await;
        Ok(tools
            .values()
            .filter(|tool| tool.verification_status.is_verified())
            .cloned()
            .collect())
    }

    async fn verify_tool(
        &self,
        request: ToolVerificationRequest,
    ) -> Result<ToolVerificationResponse, McpClientError> {
        let mut warnings = Vec::new();

        // Check if tool exists in our collection
        let tool_exists = {
            let tools = self.tools.read().await;
            tools.contains_key(&request.tool.name)
        };

        // If tool is already verified and not forcing re-verification, return current status
        if !request.force_reverify && tool_exists {
            let tools = self.tools.read().await;
            if let Some(existing_tool) = tools.get(&request.tool.name) {
                if existing_tool.verification_status.is_verified() {
                    warnings
                        .push("Tool already verified, use force_reverify to re-verify".to_string());
                    return Ok(ToolVerificationResponse {
                        tool_name: request.tool.name,
                        status: existing_tool.verification_status.clone(),
                        warnings,
                    });
                }
            }
        }

        // Perform verification
        let verification_status = self.verify_schema(&request.tool).await?;

        // Update tool in collection if it exists
        if tool_exists {
            let mut tools = self.tools.write().await;
            if let Some(existing_tool) = tools.get_mut(&request.tool.name) {
                existing_tool.verification_status = verification_status.clone();
            }
        }

        Ok(ToolVerificationResponse {
            tool_name: request.tool.name,
            status: verification_status,
            warnings,
        })
    }

    async fn remove_tool(&self, name: &str) -> Result<Option<McpTool>, McpClientError> {
        let mut tools = self.tools.write().await;
        Ok(tools.remove(name))
    }

    async fn invoke_tool(
        &self,
        tool_name: &str,
        _arguments: serde_json::Value,
        context: InvocationContext,
    ) -> Result<InvocationResult, McpClientError> {
        // Get the tool first
        let tool = self.get_tool(tool_name).await?;

        // Execute with enforcement
        self.enforcer
            .execute_tool_with_enforcement(&tool, context)
            .await
            .map_err(|e| match e {
                ToolInvocationError::InvocationBlocked {
                    tool_name,
                    reason: _,
                } => McpClientError::ToolNotVerified { name: tool_name },
                ToolInvocationError::ToolNotFound { tool_name } => {
                    McpClientError::ToolNotFound { name: tool_name }
                }
                ToolInvocationError::VerificationRequired { tool_name, .. } => {
                    McpClientError::ToolNotVerified { name: tool_name }
                }
                ToolInvocationError::VerificationFailed {
                    tool_name: _,
                    reason,
                } => McpClientError::VerificationFailed { reason },
                _ => McpClientError::CommunicationError {
                    reason: e.to_string(),
                },
            })
    }
}

/// Mock MCP client for testing
pub struct MockMcpClient {
    tools: Arc<RwLock<HashMap<String, McpTool>>>,
    should_verify_successfully: bool,
}

impl MockMcpClient {
    /// Create a new mock client that succeeds verification
    pub fn new_success() -> Self {
        Self {
            tools: Arc::new(RwLock::new(HashMap::new())),
            should_verify_successfully: true,
        }
    }

    /// Create a new mock client that fails verification
    pub fn new_failure() -> Self {
        Self {
            tools: Arc::new(RwLock::new(HashMap::new())),
            should_verify_successfully: false,
        }
    }
}

#[async_trait]
impl McpClient for MockMcpClient {
    async fn discover_tool(&self, mut tool: McpTool) -> Result<ToolDiscoveryEvent, McpClientError> {
        // Mock verification
        tool.verification_status = if self.should_verify_successfully {
            VerificationStatus::Verified {
                result: Box::new(crate::integrations::schemapin::VerificationResult {
                    success: true,
                    message: "Mock verification successful".to_string(),
                    schema_hash: Some("mock_hash".to_string()),
                    public_key_url: Some(tool.provider.public_key_url.clone()),
                    signature: None,
                    metadata: None,
                    timestamp: Some(chrono::Utc::now().to_rfc3339()),
                }),
                verified_at: chrono::Utc::now().to_rfc3339(),
            }
        } else {
            VerificationStatus::Failed {
                reason: "Mock verification failed".to_string(),
                failed_at: chrono::Utc::now().to_rfc3339(),
            }
        };

        if !self.should_verify_successfully {
            return Err(McpClientError::VerificationFailed {
                reason: "Mock verification failed".to_string(),
            });
        }

        let mut tools = self.tools.write().await;
        tools.insert(tool.name.clone(), tool.clone());

        Ok(ToolDiscoveryEvent {
            tool,
            source: "mock".to_string(),
            discovered_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    async fn get_tool(&self, name: &str) -> Result<McpTool, McpClientError> {
        let tools = self.tools.read().await;
        tools
            .get(name)
            .cloned()
            .ok_or_else(|| McpClientError::ToolNotFound {
                name: name.to_string(),
            })
    }

    async fn list_tools(&self) -> Result<Vec<McpTool>, McpClientError> {
        let tools = self.tools.read().await;
        Ok(tools.values().cloned().collect())
    }

    async fn list_verified_tools(&self) -> Result<Vec<McpTool>, McpClientError> {
        let tools = self.tools.read().await;
        Ok(tools
            .values()
            .filter(|tool| tool.verification_status.is_verified())
            .cloned()
            .collect())
    }

    async fn verify_tool(
        &self,
        request: ToolVerificationRequest,
    ) -> Result<ToolVerificationResponse, McpClientError> {
        let status = if self.should_verify_successfully {
            VerificationStatus::Verified {
                result: Box::new(crate::integrations::schemapin::VerificationResult {
                    success: true,
                    message: "Mock verification successful".to_string(),
                    schema_hash: Some("mock_hash".to_string()),
                    public_key_url: Some(request.tool.provider.public_key_url.clone()),
                    signature: None,
                    metadata: None,
                    timestamp: Some(chrono::Utc::now().to_rfc3339()),
                }),
                verified_at: chrono::Utc::now().to_rfc3339(),
            }
        } else {
            VerificationStatus::Failed {
                reason: "Mock verification failed".to_string(),
                failed_at: chrono::Utc::now().to_rfc3339(),
            }
        };

        Ok(ToolVerificationResponse {
            tool_name: request.tool.name,
            status,
            warnings: vec![],
        })
    }

    async fn remove_tool(&self, name: &str) -> Result<Option<McpTool>, McpClientError> {
        let mut tools = self.tools.write().await;
        Ok(tools.remove(name))
    }

    async fn invoke_tool(
        &self,
        tool_name: &str,
        arguments: serde_json::Value,
        _context: InvocationContext,
    ) -> Result<InvocationResult, McpClientError> {
        // Get the tool to check its verification status
        let tool = self.get_tool(tool_name).await?;

        // Mock enforcement - only allow verified tools if should_verify_successfully is true
        if !self.should_verify_successfully && !tool.verification_status.is_verified() {
            return Err(McpClientError::ToolNotVerified {
                name: tool_name.to_string(),
            });
        }

        // Return mock successful result
        Ok(InvocationResult {
            success: true,
            result: serde_json::json!({
                "status": "success",
                "tool": tool_name,
                "arguments": arguments
            }),
            execution_time: Duration::from_millis(50),
            warnings: vec![],
            metadata: std::collections::HashMap::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::schemapin::MockNativeSchemaPinClient;

    fn create_test_tool() -> McpTool {
        McpTool {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "input": {"type": "string"}
                }
            }),
            provider: ToolProvider {
                identifier: "example.com".to_string(),
                name: "Example Provider".to_string(),
                public_key_url: "https://example.com/pubkey".to_string(),
                version: Some("1.0.0".to_string()),
            },
            verification_status: VerificationStatus::Pending,
            metadata: None,
        }
    }

    #[tokio::test]
    async fn test_mock_client_success() {
        let client = MockMcpClient::new_success();
        let tool = create_test_tool();

        let event = client.discover_tool(tool.clone()).await.unwrap();
        assert!(event.tool.verification_status.is_verified());

        let retrieved_tool = client.get_tool(&tool.name).await.unwrap();
        assert_eq!(retrieved_tool.name, tool.name);
    }

    #[tokio::test]
    async fn test_mock_client_failure() {
        let client = MockMcpClient::new_failure();
        let tool = create_test_tool();

        let result = client.discover_tool(tool).await;
        assert!(result.is_err());
        assert!(matches!(
            result,
            Err(McpClientError::VerificationFailed { .. })
        ));
    }

    #[tokio::test]
    async fn test_secure_client_with_mock_components() {
        let config = McpClientConfig::default();
        let schema_pin = Arc::new(MockNativeSchemaPinClient::new_success());
        let key_store = Arc::new(LocalKeyStore::new().unwrap());

        let client = SecureMcpClient::new(config, schema_pin, key_store);
        let tool = create_test_tool();

        let event = client.discover_tool(tool.clone()).await.unwrap();
        assert!(event.tool.verification_status.is_verified());

        let tools = client.list_verified_tools().await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, tool.name);
    }

    #[tokio::test]
    async fn test_verification_enforcement() {
        let config = McpClientConfig {
            enforce_verification: true,
            ..Default::default()
        };

        let schema_pin = Arc::new(MockNativeSchemaPinClient::new_failure());
        let key_store = Arc::new(LocalKeyStore::new().unwrap());

        let client = SecureMcpClient::new(config, schema_pin, key_store);
        let tool = create_test_tool();

        let result = client.discover_tool(tool).await;
        assert!(result.is_err());
        assert!(matches!(
            result,
            Err(McpClientError::VerificationFailed { .. })
        ));
    }
}
