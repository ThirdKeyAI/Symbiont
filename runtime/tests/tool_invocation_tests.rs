//! Tool Invocation Enforcement Tests
//! 
//! Tests for verification enforcement during tool invocation

use std::collections::HashMap;
use std::sync::Arc;

use symbiont_runtime::integrations::{
    ToolInvocationEnforcer, DefaultToolInvocationEnforcer, ToolInvocationError,
    EnforcementPolicy, InvocationEnforcementConfig, InvocationContext,
    McpTool, ToolProvider, VerificationStatus, McpClient, MockMcpClient,
    SecureMcpClient, MockNativeSchemaPinClient, LocalKeyStore,
};
use symbiont_runtime::integrations::schemapin::VerificationResult;
use symbiont_runtime::types::AgentId;

fn create_test_tool_with_status(name: &str, status: VerificationStatus) -> McpTool {
    McpTool {
        name: name.to_string(),
        description: format!("Test tool: {}", name),
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "input": {"type": "string"}
            }
        }),
        provider: ToolProvider {
            identifier: "test.example.com".to_string(),
            name: "Test Provider".to_string(),
            public_key_url: "https://test.example.com/pubkey".to_string(),
            version: Some("1.0.0".to_string()),
        },
        verification_status: status,
        metadata: None,
    }
}

fn create_verified_tool(name: &str) -> McpTool {
    create_test_tool_with_status(name, VerificationStatus::Verified {
        result: Box::new(VerificationResult {
            success: true,
            message: "Test verification successful".to_string(),
            schema_hash: Some("test_hash".to_string()),
            public_key_url: Some("https://test.example.com/pubkey".to_string()),
            signature: None,
            metadata: None,
            timestamp: Some("2024-01-01T00:00:00Z".to_string()),
        }),
        verified_at: "2024-01-01T00:00:00Z".to_string(),
    })
}

fn create_failed_tool(name: &str) -> McpTool {
    create_test_tool_with_status(name, VerificationStatus::Failed {
        reason: "Test verification failed".to_string(),
        failed_at: "2024-01-01T00:00:00Z".to_string(),
    })
}

fn create_pending_tool(name: &str) -> McpTool {
    create_test_tool_with_status(name, VerificationStatus::Pending)
}

fn create_skipped_tool(name: &str) -> McpTool {
    create_test_tool_with_status(name, VerificationStatus::Skipped {
        reason: "Test skipped".to_string(),
    })
}

fn create_test_context(tool_name: &str) -> InvocationContext {
    InvocationContext {
        agent_id: AgentId::new(),
        tool_name: tool_name.to_string(),
        arguments: serde_json::json!({"input": "test"}),
        timestamp: chrono::Utc::now(),
        metadata: HashMap::new(),
    }
}

#[tokio::test]
async fn test_strict_mode_allows_verified_tools() {
    let config = InvocationEnforcementConfig {
        policy: EnforcementPolicy::Strict,
        ..Default::default()
    };
    let enforcer = DefaultToolInvocationEnforcer::with_config(config);
    
    let tool = create_verified_tool("verified_tool");
    let context = create_test_context("verified_tool");
    
    let decision = enforcer.check_invocation_allowed(&tool, &context).await.unwrap();
    assert!(matches!(decision, symbiont_runtime::integrations::EnforcementDecision::Allow));
}

#[tokio::test]
async fn test_strict_mode_blocks_unverified_tools() {
    let config = InvocationEnforcementConfig {
        policy: EnforcementPolicy::Strict,
        ..Default::default()
    };
    let enforcer = DefaultToolInvocationEnforcer::with_config(config);
    
    let tool = create_pending_tool("pending_tool");
    let context = create_test_context("pending_tool");
    
    let decision = enforcer.check_invocation_allowed(&tool, &context).await.unwrap();
    assert!(matches!(decision, symbiont_runtime::integrations::EnforcementDecision::Block { .. }));
}

#[tokio::test]
async fn test_strict_mode_blocks_failed_tools() {
    let config = InvocationEnforcementConfig {
        policy: EnforcementPolicy::Strict,
        ..Default::default()
    };
    let enforcer = DefaultToolInvocationEnforcer::with_config(config);
    
    let tool = create_failed_tool("failed_tool");
    let context = create_test_context("failed_tool");
    
    let decision = enforcer.check_invocation_allowed(&tool, &context).await.unwrap();
    assert!(matches!(decision, symbiont_runtime::integrations::EnforcementDecision::Block { .. }));
}

#[tokio::test]
async fn test_permissive_mode_allows_with_warnings() {
    let config = InvocationEnforcementConfig {
        policy: EnforcementPolicy::Permissive,
        block_pending_verification: false,
        ..Default::default()
    };
    let enforcer = DefaultToolInvocationEnforcer::with_config(config);
    
    let tool = create_pending_tool("pending_tool");
    let context = create_test_context("pending_tool");
    
    let decision = enforcer.check_invocation_allowed(&tool, &context).await.unwrap();
    assert!(matches!(decision, symbiont_runtime::integrations::EnforcementDecision::AllowWithWarnings { .. }));
}

#[tokio::test]
async fn test_development_mode_allows_skipped_tools() {
    let config = InvocationEnforcementConfig {
        policy: EnforcementPolicy::Development,
        allow_skipped_in_dev: true,
        ..Default::default()
    };
    let enforcer = DefaultToolInvocationEnforcer::with_config(config);
    
    let tool = create_skipped_tool("skipped_tool");
    let context = create_test_context("skipped_tool");
    
    let decision = enforcer.check_invocation_allowed(&tool, &context).await.unwrap();
    assert!(matches!(decision, symbiont_runtime::integrations::EnforcementDecision::AllowWithWarnings { .. }));
}

#[tokio::test]
async fn test_disabled_mode_allows_all_tools() {
    let config = InvocationEnforcementConfig {
        policy: EnforcementPolicy::Disabled,
        ..Default::default()
    };
    let enforcer = DefaultToolInvocationEnforcer::with_config(config);
    
    let tool = create_failed_tool("failed_tool");
    let context = create_test_context("failed_tool");
    
    let decision = enforcer.check_invocation_allowed(&tool, &context).await.unwrap();
    assert!(matches!(decision, symbiont_runtime::integrations::EnforcementDecision::Allow));
}

#[tokio::test]
async fn test_execute_tool_blocks_unverified_in_strict_mode() {
    let config = InvocationEnforcementConfig {
        policy: EnforcementPolicy::Strict,
        ..Default::default()
    };
    let enforcer = DefaultToolInvocationEnforcer::with_config(config);
    
    let tool = create_pending_tool("pending_tool");
    let context = create_test_context("pending_tool");
    
    let result = enforcer.execute_tool_with_enforcement(&tool, context).await;
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), ToolInvocationError::InvocationBlocked { .. }));
}

#[tokio::test]
async fn test_execute_tool_succeeds_with_verified_tool() {
    let config = InvocationEnforcementConfig {
        policy: EnforcementPolicy::Strict,
        ..Default::default()
    };
    let enforcer = DefaultToolInvocationEnforcer::with_config(config);
    
    let tool = create_verified_tool("verified_tool");
    let context = create_test_context("verified_tool");
    
    let result = enforcer.execute_tool_with_enforcement(&tool, context).await.unwrap();
    assert!(result.success);
    assert!(result.warnings.is_empty());
}

#[tokio::test]
async fn test_execute_tool_succeeds_with_warnings_in_permissive_mode() {
    let config = InvocationEnforcementConfig {
        policy: EnforcementPolicy::Permissive,
        block_pending_verification: false,
        ..Default::default()
    };
    let enforcer = DefaultToolInvocationEnforcer::with_config(config);
    
    let tool = create_pending_tool("pending_tool");
    let context = create_test_context("pending_tool");
    
    let result = enforcer.execute_tool_with_enforcement(&tool, context).await.unwrap();
    assert!(result.success);
    assert!(!result.warnings.is_empty());
}

#[tokio::test]
async fn test_mcp_client_integration_strict_mode() {
    let client = MockMcpClient::new_success();
    
    // Add a verified tool
    let tool = create_verified_tool("test_tool");
    let _ = client.discover_tool(tool.clone()).await.unwrap();
    
    // Test invocation with verified tool
    let context = create_test_context("test_tool");
    let result = client.invoke_tool("test_tool", serde_json::json!({"input": "test"}), context).await.unwrap();
    assert!(result.success);
}

#[tokio::test]
async fn test_mcp_client_integration_blocks_unverified() {
    let client = MockMcpClient::new_failure();
    
    // Try to add an unverified tool (should fail in MockMcpClient::new_failure)
    let tool = create_pending_tool("test_tool");
    let result = client.discover_tool(tool).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_secure_mcp_client_with_enforcer() {
    let config = symbiont_runtime::integrations::McpClientConfig::default();
    let schema_pin = Arc::new(MockNativeSchemaPinClient::new_success());
    let key_store = Arc::new(LocalKeyStore::new().unwrap());
    
    let client = SecureMcpClient::new(config, schema_pin, key_store);
    
    // Add a tool that will be verified
    let tool = create_test_tool_with_status("test_tool", VerificationStatus::Pending);
    let event = client.discover_tool(tool).await.unwrap();
    assert!(event.tool.verification_status.is_verified());
    
    // Test invocation
    let context = create_test_context("test_tool");
    let result = client.invoke_tool("test_tool", serde_json::json!({"input": "test"}), context).await.unwrap();
    assert!(result.success);
}

#[tokio::test]
async fn test_enforcement_policy_configuration() {
    let mut config = InvocationEnforcementConfig::default();
    assert_eq!(config.policy, EnforcementPolicy::Strict);
    
    config.policy = EnforcementPolicy::Permissive;
    let enforcer = DefaultToolInvocationEnforcer::with_config(config.clone());
    assert_eq!(enforcer.get_enforcement_config().policy, EnforcementPolicy::Permissive);
    
    let mut mutable_enforcer = DefaultToolInvocationEnforcer::new();
    mutable_enforcer.update_enforcement_config(config);
    assert_eq!(mutable_enforcer.get_enforcement_config().policy, EnforcementPolicy::Permissive);
}

#[tokio::test]
async fn test_error_message_clarity() {
    let config = InvocationEnforcementConfig {
        policy: EnforcementPolicy::Strict,
        ..Default::default()
    };
    let enforcer = DefaultToolInvocationEnforcer::with_config(config);
    
    let tool = create_failed_tool("failed_tool");
    let context = create_test_context("failed_tool");
    
    let result = enforcer.execute_tool_with_enforcement(&tool, context).await;
    assert!(result.is_err());
    
    if let Err(ToolInvocationError::InvocationBlocked { tool_name, reason }) = result {
        assert_eq!(tool_name, "failed_tool");
        assert!(reason.contains("verification failed"));
    } else {
        panic!("Expected InvocationBlocked error");
    }
}

#[tokio::test]
async fn test_warning_escalation() {
    let config = InvocationEnforcementConfig {
        policy: EnforcementPolicy::Permissive,
        block_pending_verification: false,
        max_warnings_before_escalation: 2,
        ..Default::default()
    };
    let enforcer = DefaultToolInvocationEnforcer::with_config(config);
    
    let tool = create_pending_tool("pending_tool");
    
    // First invocation - should succeed with warning
    let context1 = create_test_context("pending_tool");
    let result1 = enforcer.execute_tool_with_enforcement(&tool, context1).await.unwrap();
    assert!(result1.success);
    assert!(!result1.warnings.is_empty());
    assert!(!result1.metadata.contains_key("escalated"));
    
    // Second invocation - should succeed with warning and escalation
    let context2 = create_test_context("pending_tool");
    let result2 = enforcer.execute_tool_with_enforcement(&tool, context2).await.unwrap();
    assert!(result2.success);
    assert!(!result2.warnings.is_empty());
    assert!(result2.metadata.contains_key("escalated"));
}