//! MCP Client Integration Tests
//!
//! Tests for the MCP client with schema verification functionality

use std::sync::Arc;
use tempfile::TempDir;

use symbi_runtime::integrations::{
    KeyStoreConfig, LocalKeyStore, McpClient, McpClientConfig, McpClientError, McpTool,
    MockMcpClient, MockNativeSchemaPinClient, PinnedKey, SecureMcpClient, ToolProvider,
    ToolVerificationRequest, VerificationStatus,
};

/// Create a test tool for testing
fn create_test_tool(name: &str, provider_id: &str) -> McpTool {
    McpTool {
        name: name.to_string(),
        description: format!("Test tool: {}", name),
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "input": {
                    "type": "string",
                    "description": "Input parameter"
                }
            },
            "required": ["input"]
        }),
        provider: ToolProvider {
            identifier: provider_id.to_string(),
            name: format!("Provider for {}", provider_id),
            public_key_url: format!("https://{}/pubkey", provider_id),
            version: Some("1.0.0".to_string()),
        },
        verification_status: VerificationStatus::Pending,
        metadata: None,
        sensitive_params: vec![],
    }
}

/// Create a test key store with temporary directory
fn create_test_key_store() -> (LocalKeyStore, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let store_path = temp_dir.path().join("test_keys.json");

    let config = KeyStoreConfig {
        store_path,
        create_if_missing: true,
        file_permissions: Some(0o600),
    };

    let store = LocalKeyStore::with_config(config).unwrap();
    (store, temp_dir)
}

/// Pre-pin a synthetic test key so that `fetch_and_pin_key` skips the real
/// HTTPS fetch (TOFU early-return path).  This mirrors production behaviour
/// after the first successful key fetch.
fn prepin_test_key(key_store: &LocalKeyStore, provider_id: &str) {
    let key = PinnedKey::new(
        provider_id.to_string(),
        format!("test_public_key_for_{}", provider_id),
        "ES256".to_string(),
        format!("sha256:test_fingerprint_for_{}", provider_id),
    );
    key_store.pin_key(key).unwrap();
}

#[tokio::test]
async fn test_mock_client_successful_discovery() {
    let client = MockMcpClient::new_success();
    let tool = create_test_tool("test_tool", "example.com");

    // Discover the tool
    let event = client.discover_tool(tool.clone()).await.unwrap();

    // Verify the discovery event
    assert_eq!(event.tool.name, tool.name);
    assert!(event.tool.verification_status.is_verified());
    assert_eq!(event.source, "mock");

    // Verify we can retrieve the tool
    let retrieved_tool = client.get_tool(&tool.name).await.unwrap();
    assert_eq!(retrieved_tool.name, tool.name);
    assert!(retrieved_tool.verification_status.is_verified());

    // Verify it appears in the tools list
    let tools = client.list_tools().await.unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, tool.name);

    // Verify it appears in verified tools list
    let verified_tools = client.list_verified_tools().await.unwrap();
    assert_eq!(verified_tools.len(), 1);
    assert_eq!(verified_tools[0].name, tool.name);
}

#[tokio::test]
async fn test_mock_client_failed_discovery() {
    let client = MockMcpClient::new_failure();
    let tool = create_test_tool("test_tool", "example.com");

    // Attempt to discover the tool - should fail
    let result = client.discover_tool(tool).await;
    assert!(result.is_err());

    match result {
        Err(McpClientError::VerificationFailed { reason }) => {
            assert_eq!(reason, "Mock verification failed");
        }
        _ => panic!("Expected VerificationFailed error"),
    }

    // Verify no tools are available
    let tools = client.list_tools().await.unwrap();
    assert_eq!(tools.len(), 0);

    let verified_tools = client.list_verified_tools().await.unwrap();
    assert_eq!(verified_tools.len(), 0);
}

#[tokio::test]
async fn test_secure_client_with_successful_verification() {
    let config = McpClientConfig::default();
    let schema_pin = Arc::new(MockNativeSchemaPinClient::new_success());
    let (key_store, _temp_dir) = create_test_key_store();

    // Pre-pin the provider key so the HTTPS fetch is skipped (TOFU early return)
    prepin_test_key(&key_store, "secure.example.com");

    let key_store = Arc::new(key_store);

    let client = SecureMcpClient::new(config, schema_pin, key_store);
    let tool = create_test_tool("secure_tool", "secure.example.com");

    // Discover the tool
    let event = client.discover_tool(tool.clone()).await.unwrap();

    // Verify successful verification
    assert!(event.tool.verification_status.is_verified());

    // Verify the tool is available
    let retrieved_tool = client.get_tool(&tool.name).await.unwrap();
    assert_eq!(retrieved_tool.name, tool.name);

    // Verify it appears in verified tools
    let verified_tools = client.list_verified_tools().await.unwrap();
    assert_eq!(verified_tools.len(), 1);
}

#[tokio::test]
async fn test_secure_client_with_failed_verification() {
    let config = McpClientConfig::default();
    let schema_pin = Arc::new(MockNativeSchemaPinClient::new_failure());
    let (key_store, _temp_dir) = create_test_key_store();

    // Pre-pin the provider key so the HTTPS fetch is skipped (TOFU early return)
    prepin_test_key(&key_store, "failing.example.com");

    let key_store = Arc::new(key_store);

    let client = SecureMcpClient::new(config, schema_pin, key_store);
    let tool = create_test_tool("failing_tool", "failing.example.com");

    // Attempt to discover the tool - should fail
    let result = client.discover_tool(tool).await;
    assert!(result.is_err());

    match result {
        Err(McpClientError::VerificationFailed { .. }) => {
            // Expected
        }
        _ => panic!("Expected VerificationFailed error"),
    }

    // Verify no tools are available
    let tools = client.list_tools().await.unwrap();
    assert_eq!(tools.len(), 0);
}

#[tokio::test]
async fn test_secure_client_verification_disabled() {
    let config = McpClientConfig {
        enforce_verification: false,
        ..Default::default()
    };

    let schema_pin = Arc::new(MockNativeSchemaPinClient::new_failure());
    let (key_store, _temp_dir) = create_test_key_store();
    let key_store = Arc::new(key_store);

    let client = SecureMcpClient::new(config, schema_pin, key_store);
    let tool = create_test_tool("unverified_tool", "unverified.example.com");

    // Discover the tool - should succeed even with failed verification
    let event = client.discover_tool(tool.clone()).await.unwrap();

    // Verify the tool has skipped verification status
    match event.tool.verification_status {
        VerificationStatus::Skipped { reason } => {
            assert_eq!(reason, "Verification disabled in configuration");
        }
        _ => panic!("Expected Skipped verification status"),
    }

    // Tool should be available
    let retrieved_tool = client.get_tool(&tool.name).await.unwrap();
    assert_eq!(retrieved_tool.name, tool.name);
}

#[tokio::test]
async fn test_tool_verification_request() {
    let client = MockMcpClient::new_success();
    let tool = create_test_tool("verify_tool", "verify.example.com");

    let request = ToolVerificationRequest {
        tool: tool.clone(),
        force_reverify: false,
    };

    let response = client.verify_tool(request).await.unwrap();

    assert_eq!(response.tool_name, tool.name);
    assert!(response.status.is_verified());
    assert!(response.warnings.is_empty());
}

#[tokio::test]
async fn test_tool_verification_force_reverify() {
    let client = MockMcpClient::new_success();
    let tool = create_test_tool("reverify_tool", "reverify.example.com");

    // First, discover the tool
    let _event = client.discover_tool(tool.clone()).await.unwrap();

    // Now force re-verification
    let request = ToolVerificationRequest {
        tool: tool.clone(),
        force_reverify: true,
    };

    let response = client.verify_tool(request).await.unwrap();

    assert_eq!(response.tool_name, tool.name);
    assert!(response.status.is_verified());
}

#[tokio::test]
async fn test_tool_not_found() {
    let client = MockMcpClient::new_success();

    let result = client.get_tool("nonexistent_tool").await;
    assert!(result.is_err());

    match result {
        Err(McpClientError::ToolNotFound { name }) => {
            assert_eq!(name, "nonexistent_tool");
        }
        _ => panic!("Expected ToolNotFound error"),
    }
}

#[tokio::test]
async fn test_tool_removal() {
    let client = MockMcpClient::new_success();
    let tool = create_test_tool("removable_tool", "removable.example.com");

    // Discover the tool
    let _event = client.discover_tool(tool.clone()).await.unwrap();

    // Verify it exists
    let retrieved_tool = client.get_tool(&tool.name).await.unwrap();
    assert_eq!(retrieved_tool.name, tool.name);

    // Remove the tool
    let removed_tool = client.remove_tool(&tool.name).await.unwrap();
    assert!(removed_tool.is_some());
    assert_eq!(removed_tool.unwrap().name, tool.name);

    // Verify it's gone
    let result = client.get_tool(&tool.name).await;
    assert!(result.is_err());
    assert!(matches!(result, Err(McpClientError::ToolNotFound { .. })));

    // Verify tools list is empty
    let tools = client.list_tools().await.unwrap();
    assert_eq!(tools.len(), 0);
}

#[tokio::test]
async fn test_multiple_tools_discovery() {
    let client = MockMcpClient::new_success();

    let tool1 = create_test_tool("tool1", "provider1.com");
    let tool2 = create_test_tool("tool2", "provider2.com");
    let tool3 = create_test_tool("tool3", "provider1.com"); // Same provider as tool1

    // Discover all tools
    let _event1 = client.discover_tool(tool1.clone()).await.unwrap();
    let _event2 = client.discover_tool(tool2.clone()).await.unwrap();
    let _event3 = client.discover_tool(tool3.clone()).await.unwrap();

    // Verify all tools are available
    let tools = client.list_tools().await.unwrap();
    assert_eq!(tools.len(), 3);

    let verified_tools = client.list_verified_tools().await.unwrap();
    assert_eq!(verified_tools.len(), 3);

    // Verify individual tool retrieval
    let retrieved_tool1 = client.get_tool(&tool1.name).await.unwrap();
    let retrieved_tool2 = client.get_tool(&tool2.name).await.unwrap();
    let retrieved_tool3 = client.get_tool(&tool3.name).await.unwrap();

    assert_eq!(retrieved_tool1.name, tool1.name);
    assert_eq!(retrieved_tool2.name, tool2.name);
    assert_eq!(retrieved_tool3.name, tool3.name);
}

#[tokio::test]
async fn test_key_store_tofu_mechanism() {
    let config = McpClientConfig::default();
    let schema_pin = Arc::new(MockNativeSchemaPinClient::new_success());
    let (key_store, _temp_dir) = create_test_key_store();

    // Pre-pin the provider key to simulate a prior successful HTTPS fetch.
    // In production the first `discover_tool` call fetches the key over HTTPS;
    // here we pre-pin so the test doesn't require network access.
    prepin_test_key(&key_store, "tofu.example.com");

    let key_store = Arc::new(key_store);

    let client = SecureMcpClient::new(config, schema_pin, key_store.clone());

    // Create two tools from the same provider
    let tool1 = create_test_tool("tool1", "tofu.example.com");
    let tool2 = create_test_tool("tool2", "tofu.example.com");

    // Discover first tool - key is already pinned from pre-pin above
    let _event1 = client.discover_tool(tool1).await.unwrap();

    // Verify key is pinned
    assert!(key_store.has_key("tofu.example.com").unwrap());

    // Discover second tool from same provider - should use existing key
    let _event2 = client.discover_tool(tool2).await.unwrap();

    // Verify both tools are available
    let tools = client.list_tools().await.unwrap();
    assert_eq!(tools.len(), 2);
}

#[tokio::test]
async fn test_verification_status_methods() {
    // Test verified status
    let verified_status = VerificationStatus::Verified {
        result: Box::new(symbi_runtime::integrations::VerificationResult {
            success: true,
            message: "Test verification".to_string(),
            schema_hash: Some("hash123".to_string()),
            public_key_url: Some("https://example.com/key".to_string()),
            signature: None,
            metadata: None,
            timestamp: Some("2024-01-01T00:00:00Z".to_string()),
        }),
        verified_at: "2024-01-01T00:00:00Z".to_string(),
    };

    assert!(verified_status.is_verified());
    assert!(!verified_status.is_failed());
    assert!(!verified_status.is_pending());
    assert!(verified_status.verification_result().is_some());

    // Test failed status
    let failed_status = VerificationStatus::Failed {
        reason: "Invalid signature".to_string(),
        failed_at: "2024-01-01T00:00:00Z".to_string(),
    };

    assert!(!failed_status.is_verified());
    assert!(failed_status.is_failed());
    assert!(!failed_status.is_pending());
    assert!(failed_status.verification_result().is_none());

    // Test pending status
    let pending_status = VerificationStatus::Pending;

    assert!(!pending_status.is_verified());
    assert!(!pending_status.is_failed());
    assert!(pending_status.is_pending());
    assert!(pending_status.verification_result().is_none());

    // Test skipped status
    let skipped_status = VerificationStatus::Skipped {
        reason: "Development mode".to_string(),
    };

    assert!(!skipped_status.is_verified());
    assert!(!skipped_status.is_failed());
    assert!(!skipped_status.is_pending());
    assert!(skipped_status.verification_result().is_none());
}

#[tokio::test]
async fn test_client_config_defaults() {
    let config = McpClientConfig::default();

    assert!(config.enforce_verification);
    assert!(!config.allow_unverified_in_dev);
    assert_eq!(config.verification_timeout_seconds, 30);
    assert_eq!(config.max_concurrent_verifications, 5);
}
