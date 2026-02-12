//! Tool Invocation Enforcement
//!
//! Provides verification enforcement for tool invocations to ensure only
//! verified tools can be executed based on configurable policies.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use thiserror::Error;

use crate::integrations::mcp::{McpClient, McpTool, VerificationStatus};
use crate::logging::{ModelInteractionType, ModelLogger, RequestData, ResponseData};
use crate::routing::{error::TaskType, ModelRequest, RoutingContext, RoutingEngine};
use crate::types::{AgentId, RuntimeError};
use dashmap::DashMap;
use std::sync::Arc;

/// Tool invocation enforcement errors
#[derive(Error, Debug, Clone)]
pub enum ToolInvocationError {
    #[error("Tool invocation blocked: {tool_name} - {reason}")]
    InvocationBlocked { tool_name: String, reason: String },

    #[error("Tool not found: {tool_name}")]
    ToolNotFound { tool_name: String },

    #[error("Verification required but tool is not verified: {tool_name} (status: {status})")]
    VerificationRequired { tool_name: String, status: String },

    #[error("Tool verification failed: {tool_name} - {reason}")]
    VerificationFailed { tool_name: String, reason: String },

    #[error("Enforcement policy violation: {policy} - {reason}")]
    PolicyViolation { policy: String, reason: String },

    #[error("Tool invocation timeout: {tool_name}")]
    Timeout { tool_name: String },

    #[error("No MCP client configured for tool execution: {reason}")]
    NoMcpClient { reason: String },

    #[error("Runtime error during tool invocation: {source}")]
    Runtime {
        #[from]
        source: RuntimeError,
    },
}

/// Tool invocation enforcement policies
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum EnforcementPolicy {
    /// Strict mode - only verified tools can be executed
    #[default]
    Strict,
    /// Permissive mode - unverified tools are allowed with warnings
    Permissive,
    /// Development mode - allows unverified tools and logs warnings
    Development,
    /// Disabled - no verification enforcement
    Disabled,
}

/// Configuration for tool invocation enforcement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvocationEnforcementConfig {
    /// Primary enforcement policy
    pub policy: EnforcementPolicy,
    /// Whether to block tools with failed verification
    pub block_failed_verification: bool,
    /// Whether to block tools with pending verification
    pub block_pending_verification: bool,
    /// Whether to allow skipped verification in development
    pub allow_skipped_in_dev: bool,
    /// Timeout for tool invocation verification checks
    pub verification_timeout: Duration,
    /// Maximum number of warning logs before escalation
    pub max_warnings_before_escalation: usize,
}

impl Default for InvocationEnforcementConfig {
    fn default() -> Self {
        Self {
            policy: EnforcementPolicy::Strict,
            block_failed_verification: true,
            block_pending_verification: true,
            allow_skipped_in_dev: false,
            verification_timeout: Duration::from_secs(5),
            max_warnings_before_escalation: 10,
        }
    }
}

/// Tool invocation context
#[derive(Debug, Clone)]
pub struct InvocationContext {
    /// Agent requesting the invocation
    pub agent_id: AgentId,
    /// Tool name being invoked
    pub tool_name: String,
    /// Arguments for the tool invocation
    pub arguments: serde_json::Value,
    /// Timestamp of invocation request
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Additional metadata
    pub metadata: HashMap<String, String>,
    /// Optional AgentPin credential verification result
    pub agent_credential: Option<crate::integrations::agentpin::AgentVerificationResult>,
}

/// Tool invocation result
#[derive(Debug, Clone)]
pub struct InvocationResult {
    /// Whether the invocation was successful
    pub success: bool,
    /// Result data from tool execution
    pub result: serde_json::Value,
    /// Execution time
    pub execution_time: Duration,
    /// Any warnings generated during invocation
    pub warnings: Vec<String>,
    /// Metadata about the invocation
    pub metadata: HashMap<String, String>,
}

/// Tool invocation enforcement decision
#[derive(Debug, Clone)]
pub enum EnforcementDecision {
    /// Allow the invocation to proceed
    Allow,
    /// Block the invocation with reason
    Block { reason: String },
    /// Allow with warnings
    AllowWithWarnings { warnings: Vec<String> },
}

/// Trait for tool invocation enforcement
#[async_trait]
pub trait ToolInvocationEnforcer: Send + Sync {
    /// Check if a tool invocation should be allowed based on verification status
    async fn check_invocation_allowed(
        &self,
        tool: &McpTool,
        context: &InvocationContext,
    ) -> Result<EnforcementDecision, ToolInvocationError>;

    /// Execute a tool invocation with enforcement checks
    async fn execute_tool_with_enforcement(
        &self,
        tool: &McpTool,
        context: InvocationContext,
    ) -> Result<InvocationResult, ToolInvocationError>;

    /// Get the current enforcement configuration
    fn get_enforcement_config(&self) -> &InvocationEnforcementConfig;

    /// Update the enforcement configuration
    fn update_enforcement_config(&mut self, config: InvocationEnforcementConfig);
}

/// Recursively mask sensitive argument values in a JSON object.
///
/// Keys matching any entry in `sensitive_params` (case-sensitive) are replaced
/// with `[REDACTED:sensitive_param]`. Nested objects are traversed recursively.
pub fn mask_sensitive_arguments(
    arguments: &serde_json::Value,
    sensitive_params: &[String],
) -> serde_json::Value {
    if sensitive_params.is_empty() {
        return arguments.clone();
    }

    match arguments {
        serde_json::Value::Object(map) => {
            let mut masked = serde_json::Map::new();
            for (key, value) in map {
                if sensitive_params.iter().any(|p| p == key) {
                    masked.insert(
                        key.clone(),
                        serde_json::Value::String(format!("[REDACTED:{}]", key)),
                    );
                } else {
                    masked.insert(
                        key.clone(),
                        mask_sensitive_arguments(value, sensitive_params),
                    );
                }
            }
            serde_json::Value::Object(masked)
        }
        serde_json::Value::Array(arr) => serde_json::Value::Array(
            arr.iter()
                .map(|v| mask_sensitive_arguments(v, sensitive_params))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Default implementation of tool invocation enforcement
pub struct DefaultToolInvocationEnforcer {
    config: InvocationEnforcementConfig,
    warning_counts: DashMap<String, usize>,
    model_logger: Option<Arc<ModelLogger>>,
    routing_engine: Option<Arc<dyn RoutingEngine>>,
    mcp_client: Option<Arc<dyn McpClient>>,
}

impl DefaultToolInvocationEnforcer {
    /// Create a new tool invocation enforcer with default configuration
    pub fn new() -> Self {
        Self {
            config: InvocationEnforcementConfig::default(),
            warning_counts: DashMap::new(),
            model_logger: None,
            routing_engine: None,
            mcp_client: None,
        }
    }

    /// Create a new tool invocation enforcer with custom configuration
    pub fn with_config(config: InvocationEnforcementConfig) -> Self {
        Self {
            config,
            warning_counts: DashMap::new(),
            model_logger: None,
            routing_engine: None,
            mcp_client: None,
        }
    }

    /// Create a new tool invocation enforcer with model logging
    pub fn with_logger(config: InvocationEnforcementConfig, logger: Arc<ModelLogger>) -> Self {
        Self {
            config,
            warning_counts: DashMap::new(),
            model_logger: Some(logger),
            routing_engine: None,
            mcp_client: None,
        }
    }

    /// Create a new tool invocation enforcer with routing engine
    pub fn with_routing(
        config: InvocationEnforcementConfig,
        logger: Option<Arc<ModelLogger>>,
        routing_engine: Arc<dyn RoutingEngine>,
    ) -> Self {
        Self {
            config,
            warning_counts: DashMap::new(),
            model_logger: logger,
            routing_engine: Some(routing_engine),
            mcp_client: None,
        }
    }

    /// Create a new tool invocation enforcer with an MCP client for real tool execution
    pub fn with_mcp_client(
        config: InvocationEnforcementConfig,
        mcp_client: Arc<dyn McpClient>,
    ) -> Self {
        Self {
            config,
            warning_counts: DashMap::new(),
            model_logger: None,
            routing_engine: None,
            mcp_client: Some(mcp_client),
        }
    }

    /// Check verification status and determine if tool should be allowed
    fn check_verification_status(&self, tool: &McpTool) -> EnforcementDecision {
        match &self.config.policy {
            EnforcementPolicy::Disabled => EnforcementDecision::Allow,
            EnforcementPolicy::Development => {
                match &tool.verification_status {
                    VerificationStatus::Verified { .. } => EnforcementDecision::Allow,
                    VerificationStatus::Failed { reason, .. } => {
                        if self.config.block_failed_verification {
                            EnforcementDecision::Block {
                                reason: format!("Tool verification failed: {}", reason),
                            }
                        } else {
                            EnforcementDecision::AllowWithWarnings {
                                warnings: vec![format!("Tool '{}' has failed verification: {}", tool.name, reason)],
                            }
                        }
                    }
                    VerificationStatus::Pending => {
                        EnforcementDecision::AllowWithWarnings {
                            warnings: vec![format!("Tool '{}' verification is pending", tool.name)],
                        }
                    }
                    VerificationStatus::Skipped { reason } => {
                        if self.config.allow_skipped_in_dev {
                            EnforcementDecision::AllowWithWarnings {
                                warnings: vec![format!("Tool '{}' verification was skipped: {}", tool.name, reason)],
                            }
                        } else {
                            EnforcementDecision::Block {
                                reason: format!("Tool verification was skipped: {}", reason),
                            }
                        }
                    }
                }
            }
            EnforcementPolicy::Permissive => {
                match &tool.verification_status {
                    VerificationStatus::Verified { .. } => EnforcementDecision::Allow,
                    VerificationStatus::Failed { reason, .. } => {
                        if self.config.block_failed_verification {
                            EnforcementDecision::Block {
                                reason: format!("Tool verification failed: {}", reason),
                            }
                        } else {
                            EnforcementDecision::AllowWithWarnings {
                                warnings: vec![format!("Tool '{}' has failed verification: {}", tool.name, reason)],
                            }
                        }
                    }
                    VerificationStatus::Pending => {
                        if self.config.block_pending_verification {
                            EnforcementDecision::AllowWithWarnings {
                                warnings: vec![format!("Tool '{}' verification is pending", tool.name)],
                            }
                        } else {
                            EnforcementDecision::Allow
                        }
                    }
                    VerificationStatus::Skipped { reason } => {
                        EnforcementDecision::AllowWithWarnings {
                            warnings: vec![format!("Tool '{}' verification was skipped: {}", tool.name, reason)],
                        }
                    }
                }
            }
            EnforcementPolicy::Strict => {
                match &tool.verification_status {
                    VerificationStatus::Verified { .. } => EnforcementDecision::Allow,
                    VerificationStatus::Failed { reason, .. } => {
                        EnforcementDecision::Block {
                            reason: format!("Tool verification failed: {}", reason),
                        }
                    }
                    VerificationStatus::Pending => {
                        EnforcementDecision::Block {
                            reason: "Tool verification is pending - only verified tools are allowed in strict mode".to_string(),
                        }
                    }
                    VerificationStatus::Skipped { reason } => {
                        EnforcementDecision::Block {
                            reason: format!("Tool verification was skipped: {} - only verified tools are allowed in strict mode", reason),
                        }
                    }
                }
            }
        }
    }

    /// Increment warning count for a tool and check if escalation is needed
    fn handle_warning(&self, tool_name: &str, warning: &str) -> bool {
        let mut count = self
            .warning_counts
            .entry(tool_name.to_string())
            .or_insert(0);
        *count += 1;

        if *count >= self.config.max_warnings_before_escalation {
            tracing::warn!(
                "Tool '{}' has exceeded warning threshold ({} warnings): {}",
                tool_name,
                *count,
                warning
            );
            // Reset count after escalation
            *count = 0;
            true
        } else {
            tracing::warn!(
                "Tool '{}' warning (count: {}): {}",
                tool_name,
                *count,
                warning
            );
            false
        }
    }

    /// Use routing engine to determine best model for tool execution
    #[allow(dead_code)]
    async fn route_tool_execution(
        &self,
        tool: &McpTool,
        context: &InvocationContext,
    ) -> Result<Option<String>, ToolInvocationError> {
        if let Some(ref routing_engine) = self.routing_engine {
            // Classify the tool task type based on tool description and arguments
            let task_type = self.classify_tool_task(tool, context);

            // Create routing context
            let routing_context = RoutingContext::new(
                context.agent_id,
                task_type,
                format!("Tool: {} - {}", tool.name, tool.description),
            );

            // Create model request
            let _model_request = ModelRequest::from_task(format!(
                "Execute tool '{}' with arguments: {}",
                tool.name, context.arguments
            ));

            // Get routing decision
            match routing_engine.route_request(&routing_context).await {
                Ok(decision) => {
                    tracing::debug!("Routing decision for tool '{}': {:?}", tool.name, decision);
                    // Return the routing decision info for logging/metadata
                    Ok(Some(format!("{:?}", decision)))
                }
                Err(e) => {
                    tracing::warn!("Routing failed for tool '{}': {}", tool.name, e);
                    Ok(None)
                }
            }
        } else {
            Ok(None)
        }
    }

    /// Classify tool execution into routing task types
    #[allow(dead_code)]
    fn classify_tool_task(&self, tool: &McpTool, context: &InvocationContext) -> TaskType {
        let tool_name_lower = tool.name.to_lowercase();
        let description_lower = tool.description.to_lowercase();

        // Analyze tool name and description to determine task type
        if tool_name_lower.contains("code")
            || description_lower.contains("code")
            || tool_name_lower.contains("program")
            || description_lower.contains("program")
        {
            TaskType::CodeGeneration
        } else if tool_name_lower.contains("analyze")
            || description_lower.contains("analy")
            || tool_name_lower.contains("inspect")
            || description_lower.contains("inspect")
        {
            TaskType::Analysis
        } else if tool_name_lower.contains("extract")
            || description_lower.contains("extract")
            || tool_name_lower.contains("parse")
            || description_lower.contains("parse")
        {
            TaskType::Extract
        } else if tool_name_lower.contains("summarize") || description_lower.contains("summar") {
            TaskType::Summarization
        } else if tool_name_lower.contains("translate") || description_lower.contains("translat") {
            TaskType::Translation
        } else if tool_name_lower.contains("reason")
            || description_lower.contains("reason")
            || tool_name_lower.contains("logic")
            || description_lower.contains("logic")
        {
            TaskType::Reasoning
        } else if tool_name_lower.contains("template") || description_lower.contains("template") {
            TaskType::Template
        } else if context.arguments.to_string().len() < 100 {
            // Simple tools with minimal arguments
            TaskType::Intent
        } else {
            // Default to QA for general tools
            TaskType::QA
        }
    }
    /// Execute the actual tool via the configured MCP client.
    /// Returns an error if no MCP client is configured.
    async fn execute_actual_tool(
        &self,
        tool: &McpTool,
        context: &InvocationContext,
        _execution_time: Duration,
    ) -> Result<InvocationResult, ToolInvocationError> {
        let mcp_client =
            self.mcp_client
                .as_ref()
                .ok_or_else(|| ToolInvocationError::NoMcpClient {
                    reason: format!(
                        "No MCP client configured for tool execution of '{}'",
                        tool.name
                    ),
                })?;

        tracing::info!(
            "Executing tool '{}' for agent {} via MCP (provider: '{}')",
            tool.name,
            context.agent_id,
            tool.provider.identifier
        );

        let result = mcp_client
            .invoke_tool(&tool.name, context.arguments.clone(), context.clone())
            .await
            .map_err(|e| ToolInvocationError::VerificationFailed {
                tool_name: tool.name.clone(),
                reason: format!("MCP invocation failed: {}", e),
            })?;

        Ok(result)
    }
}

impl Default for DefaultToolInvocationEnforcer {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ToolInvocationEnforcer for DefaultToolInvocationEnforcer {
    async fn check_invocation_allowed(
        &self,
        tool: &McpTool,
        _context: &InvocationContext,
    ) -> Result<EnforcementDecision, ToolInvocationError> {
        Ok(self.check_verification_status(tool))
    }

    async fn execute_tool_with_enforcement(
        &self,
        tool: &McpTool,
        context: InvocationContext,
    ) -> Result<InvocationResult, ToolInvocationError> {
        let start_time = Instant::now();

        // Check if invocation is allowed
        let decision = self.check_invocation_allowed(tool, &context).await?;

        // Mask sensitive arguments before logging
        let redacted_arguments =
            mask_sensitive_arguments(&context.arguments, &tool.sensitive_params);

        // Prepare request data for logging
        let request_data = RequestData {
            prompt: format!("Tool invocation: {}", tool.name),
            tool_name: Some(tool.name.clone()),
            tool_arguments: Some(redacted_arguments),
            parameters: {
                let mut params = HashMap::new();
                params.insert(
                    "verification_status".to_string(),
                    serde_json::Value::String(format!("{:?}", tool.verification_status)),
                );
                params.insert(
                    "enforcement_policy".to_string(),
                    serde_json::Value::String(format!("{:?}", self.config.policy)),
                );
                params
            },
        };

        match decision {
            EnforcementDecision::Allow => {
                let execution_time = start_time.elapsed();

                // Prepare successful response data
                let response_data = ResponseData {
                    content: "Tool invocation allowed and executed".to_string(),
                    tool_result: Some(
                        serde_json::json!({"status": "success", "message": "Tool invocation allowed"}),
                    ),
                    confidence: Some(1.0),
                    metadata: HashMap::new(),
                };

                // Log the tool invocation if logger is available
                if let Some(ref logger) = self.model_logger {
                    let metadata = {
                        let mut meta = HashMap::new();
                        meta.insert(
                            "tool_provider".to_string(),
                            tool.provider.identifier.clone(),
                        );
                        meta.insert("enforcement_decision".to_string(), "allow".to_string());
                        meta.insert("agent_id".to_string(), context.agent_id.to_string());
                        meta
                    };

                    if let Err(e) = logger
                        .log_interaction(
                            context.agent_id,
                            ModelInteractionType::ToolCall,
                            &tool.name,
                            request_data,
                            response_data,
                            execution_time,
                            metadata,
                            None, // No token usage for tool calls
                            None,
                        )
                        .await
                    {
                        tracing::warn!("Failed to log tool invocation: {}", e);
                    }
                }

                // Execute the actual tool
                self.execute_actual_tool(tool, &context, execution_time)
                    .await
            }
            EnforcementDecision::Block { reason } => {
                let execution_time = start_time.elapsed();

                // Log the blocked invocation if logger is available
                if let Some(ref logger) = self.model_logger {
                    let response_data = ResponseData {
                        content: "Tool invocation blocked".to_string(),
                        tool_result: Some(
                            serde_json::json!({"status": "blocked", "reason": &reason}),
                        ),
                        confidence: Some(1.0),
                        metadata: HashMap::new(),
                    };

                    let metadata = {
                        let mut meta = HashMap::new();
                        meta.insert(
                            "tool_provider".to_string(),
                            tool.provider.identifier.clone(),
                        );
                        meta.insert("enforcement_decision".to_string(), "block".to_string());
                        meta.insert("agent_id".to_string(), context.agent_id.to_string());
                        meta
                    };

                    if let Err(e) = logger
                        .log_interaction(
                            context.agent_id,
                            ModelInteractionType::ToolCall,
                            &tool.name,
                            request_data,
                            response_data,
                            execution_time,
                            metadata,
                            None,
                            Some(reason.clone()),
                        )
                        .await
                    {
                        tracing::warn!("Failed to log blocked tool invocation: {}", e);
                    }
                }

                Err(ToolInvocationError::InvocationBlocked {
                    tool_name: tool.name.clone(),
                    reason,
                })
            }
            EnforcementDecision::AllowWithWarnings { warnings } => {
                let execution_time = start_time.elapsed();

                // Handle warnings
                let mut escalated = false;
                for warning in &warnings {
                    if self.handle_warning(&tool.name, warning) {
                        escalated = true;
                    }
                }

                // Prepare response data with warnings
                let response_data = ResponseData {
                    content: "Tool invocation allowed with warnings".to_string(),
                    tool_result: Some(serde_json::json!({
                        "status": "success",
                        "message": "Tool invocation allowed with warnings",
                        "warnings": &warnings
                    })),
                    confidence: Some(0.8), // Lower confidence due to warnings
                    metadata: HashMap::new(),
                };

                // Log the tool invocation with warnings if logger is available
                if let Some(ref logger) = self.model_logger {
                    let metadata = {
                        let mut meta = HashMap::new();
                        meta.insert(
                            "tool_provider".to_string(),
                            tool.provider.identifier.clone(),
                        );
                        meta.insert(
                            "enforcement_decision".to_string(),
                            "allow_with_warnings".to_string(),
                        );
                        meta.insert("agent_id".to_string(), context.agent_id.to_string());
                        meta.insert("warnings_count".to_string(), warnings.len().to_string());
                        if escalated {
                            meta.insert("escalated".to_string(), "true".to_string());
                        }
                        meta
                    };

                    if let Err(e) = logger
                        .log_interaction(
                            context.agent_id,
                            ModelInteractionType::ToolCall,
                            &tool.name,
                            request_data,
                            response_data,
                            execution_time,
                            metadata,
                            None,
                            None,
                        )
                        .await
                    {
                        tracing::warn!("Failed to log tool invocation with warnings: {}", e);
                    }
                }

                // Execute the actual tool with warnings
                let mut result = self
                    .execute_actual_tool(tool, &context, execution_time)
                    .await?;
                result.warnings.extend(warnings);
                if escalated {
                    result
                        .metadata
                        .insert("escalated".to_string(), "true".to_string());
                }
                Ok(result)
            }
        }
    }

    fn get_enforcement_config(&self) -> &InvocationEnforcementConfig {
        &self.config
    }

    fn update_enforcement_config(&mut self, config: InvocationEnforcementConfig) {
        self.config = config;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::mcp::{McpTool, MockMcpClient, ToolProvider, VerificationStatus};
    use crate::integrations::schemapin::VerificationResult;

    fn create_test_tool(verification_status: VerificationStatus) -> McpTool {
        McpTool {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
            schema: serde_json::json!({"type": "object"}),
            provider: ToolProvider {
                identifier: "test.example.com".to_string(),
                name: "Test Provider".to_string(),
                public_key_url: "https://test.example.com/pubkey".to_string(),
                version: Some("1.0.0".to_string()),
            },
            verification_status,
            metadata: None,
            sensitive_params: vec![],
        }
    }

    fn create_test_context() -> InvocationContext {
        InvocationContext {
            agent_id: AgentId::new(),
            tool_name: "test_tool".to_string(),
            arguments: serde_json::json!({"test": "value"}),
            timestamp: chrono::Utc::now(),
            metadata: HashMap::new(),
            agent_credential: None,
        }
    }

    async fn create_enforcer_with_mock_mcp(
        config: InvocationEnforcementConfig,
        tool: &McpTool,
    ) -> DefaultToolInvocationEnforcer {
        let mock_client = Arc::new(MockMcpClient::new_success());
        // Register the tool so invoke_tool can find it
        let _ = mock_client.discover_tool(tool.clone()).await;
        DefaultToolInvocationEnforcer {
            config,
            warning_counts: DashMap::new(),
            model_logger: None,
            routing_engine: None,
            mcp_client: Some(mock_client),
        }
    }

    #[tokio::test]
    async fn test_strict_mode_allows_verified_tools() {
        let enforcer = DefaultToolInvocationEnforcer::with_config(InvocationEnforcementConfig {
            policy: EnforcementPolicy::Strict,
            ..Default::default()
        });

        let tool = create_test_tool(VerificationStatus::Verified {
            result: Box::new(VerificationResult {
                success: true,
                message: "Test verification".to_string(),
                schema_hash: Some("hash123".to_string()),
                public_key_url: Some("https://test.example.com/pubkey".to_string()),
                signature: None,
                metadata: None,
                timestamp: Some("2024-01-01T00:00:00Z".to_string()),
            }),
            verified_at: "2024-01-01T00:00:00Z".to_string(),
        });

        let context = create_test_context();
        let decision = enforcer
            .check_invocation_allowed(&tool, &context)
            .await
            .unwrap();

        assert!(matches!(decision, EnforcementDecision::Allow));
    }

    #[tokio::test]
    async fn test_strict_mode_blocks_unverified_tools() {
        let enforcer = DefaultToolInvocationEnforcer::with_config(InvocationEnforcementConfig {
            policy: EnforcementPolicy::Strict,
            ..Default::default()
        });

        let tool = create_test_tool(VerificationStatus::Pending);
        let context = create_test_context();
        let decision = enforcer
            .check_invocation_allowed(&tool, &context)
            .await
            .unwrap();

        assert!(matches!(decision, EnforcementDecision::Block { .. }));
    }

    #[tokio::test]
    async fn test_permissive_mode_allows_with_warnings() {
        let enforcer = DefaultToolInvocationEnforcer::with_config(InvocationEnforcementConfig {
            policy: EnforcementPolicy::Permissive,
            block_pending_verification: true,
            ..Default::default()
        });

        let tool = create_test_tool(VerificationStatus::Pending);
        let context = create_test_context();
        let decision = enforcer
            .check_invocation_allowed(&tool, &context)
            .await
            .unwrap();

        assert!(matches!(
            decision,
            EnforcementDecision::AllowWithWarnings { .. }
        ));
    }

    #[tokio::test]
    async fn test_disabled_mode_allows_all_tools() {
        let enforcer = DefaultToolInvocationEnforcer::with_config(InvocationEnforcementConfig {
            policy: EnforcementPolicy::Disabled,
            ..Default::default()
        });

        let tool = create_test_tool(VerificationStatus::Failed {
            reason: "Test failure".to_string(),
            failed_at: "2024-01-01T00:00:00Z".to_string(),
        });
        let context = create_test_context();
        let decision = enforcer
            .check_invocation_allowed(&tool, &context)
            .await
            .unwrap();

        assert!(matches!(decision, EnforcementDecision::Allow));
    }

    #[tokio::test]
    async fn test_execute_tool_blocks_unverified_in_strict_mode() {
        let enforcer = DefaultToolInvocationEnforcer::with_config(InvocationEnforcementConfig {
            policy: EnforcementPolicy::Strict,
            ..Default::default()
        });

        let tool = create_test_tool(VerificationStatus::Pending);
        let context = create_test_context();
        let result = enforcer.execute_tool_with_enforcement(&tool, context).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ToolInvocationError::InvocationBlocked { .. }
        ));
    }

    #[tokio::test]
    async fn test_execute_tool_succeeds_with_warnings() {
        let tool = create_test_tool(VerificationStatus::Pending);
        let enforcer = create_enforcer_with_mock_mcp(
            InvocationEnforcementConfig {
                policy: EnforcementPolicy::Permissive,
                block_pending_verification: true,
                ..Default::default()
            },
            &tool,
        )
        .await;

        let context = create_test_context();
        let result = enforcer
            .execute_tool_with_enforcement(&tool, context)
            .await
            .unwrap();

        assert!(result.success);
        assert!(!result.warnings.is_empty());
    }

    #[tokio::test]
    async fn test_execute_tool_fails_without_mcp_client() {
        let enforcer = DefaultToolInvocationEnforcer::with_config(InvocationEnforcementConfig {
            policy: EnforcementPolicy::Disabled,
            ..Default::default()
        });

        let tool = create_test_tool(VerificationStatus::Pending);
        let context = create_test_context();
        let result = enforcer.execute_tool_with_enforcement(&tool, context).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ToolInvocationError::NoMcpClient { .. }
        ));
    }

    #[test]
    fn test_mask_sensitive_arguments_empty_list() {
        let args = serde_json::json!({"user": "alice", "password": "s3cret"});
        let masked = mask_sensitive_arguments(&args, &[]);
        assert_eq!(masked, args);
    }

    #[test]
    fn test_mask_sensitive_arguments_flat() {
        let args = serde_json::json!({"user": "alice", "password": "s3cret", "token": "abc"});
        let masked =
            mask_sensitive_arguments(&args, &["password".to_string(), "token".to_string()]);
        assert_eq!(masked["user"], "alice");
        assert_eq!(masked["password"], "[REDACTED:password]");
        assert_eq!(masked["token"], "[REDACTED:token]");
    }

    #[test]
    fn test_mask_sensitive_arguments_nested() {
        let args = serde_json::json!({
            "config": {
                "api_key": "sk-123",
                "endpoint": "https://api.example.com"
            },
            "name": "test"
        });
        let masked = mask_sensitive_arguments(&args, &["api_key".to_string()]);
        assert_eq!(masked["config"]["api_key"], "[REDACTED:api_key]");
        assert_eq!(masked["config"]["endpoint"], "https://api.example.com");
        assert_eq!(masked["name"], "test");
    }
}
