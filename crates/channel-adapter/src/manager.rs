//! Channel adapter manager — lightweight orchestrator.
//!
//! Routes inbound messages to agents and sends responses back.
//! Community edition: no policy engine, no DLP, no identity mapping.
//! Enterprise hooks are an optional extension point.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::config::{ChannelConfig, PlatformSettings};
use crate::error::ChannelAdapterError;
use crate::logging::BasicInteractionLogger;
use crate::traits::{ChannelAdapter, InboundHandler};
use crate::types::{ChatPlatform, InboundMessage, OutboundMessage};

#[cfg(feature = "slack")]
use crate::adapters::slack::api::format_agent_response as format_slack_response;
#[cfg(feature = "slack")]
use crate::adapters::slack::SlackAdapter;

#[cfg(feature = "teams")]
use crate::adapters::teams::api::format_agent_response as format_teams_response;
#[cfg(feature = "teams")]
use crate::adapters::teams::TeamsAdapter;

#[cfg(feature = "mattermost")]
use crate::adapters::mattermost::api::format_agent_response as format_mattermost_response;
#[cfg(feature = "mattermost")]
use crate::adapters::mattermost::MattermostAdapter;

/// Callback to invoke an agent with a text input and get a response.
///
/// This is the bridge between the channel adapter and the agent runtime.
/// The runtime provides an implementation when integrating.
#[async_trait]
pub trait AgentInvoker: Send + Sync {
    /// Invoke an agent by name with the given input text.
    /// Returns the agent's response text.
    async fn invoke(&self, agent_name: &str, input: &str) -> Result<String, String>;
}

/// Lightweight orchestrator for channel adapters.
///
/// Manages adapter lifecycle, routes inbound messages to agents,
/// and sends responses back through the originating adapter.
pub struct ChannelAdapterManager {
    adapters: HashMap<String, Arc<dyn ChannelAdapter>>,
    invoker: Arc<dyn AgentInvoker>,
    logger: Arc<BasicInteractionLogger>,
    #[cfg(feature = "enterprise-hooks")]
    enterprise_hooks: Option<Arc<dyn crate::traits::EnterpriseChannelHooks>>,
}

impl ChannelAdapterManager {
    pub fn new(invoker: Arc<dyn AgentInvoker>, logger: Arc<BasicInteractionLogger>) -> Self {
        Self {
            adapters: HashMap::new(),
            invoker,
            logger,
            #[cfg(feature = "enterprise-hooks")]
            enterprise_hooks: None,
        }
    }

    #[cfg(feature = "enterprise-hooks")]
    pub fn set_enterprise_hooks(&mut self, hooks: Arc<dyn crate::traits::EnterpriseChannelHooks>) {
        self.enterprise_hooks = Some(hooks);
    }

    /// Register and start an adapter from configuration.
    pub async fn register_adapter(
        &mut self,
        config: ChannelConfig,
    ) -> Result<(), ChannelAdapterError> {
        let name = config.name.clone();

        let adapter: Arc<dyn ChannelAdapter> = match config.settings {
            #[cfg(feature = "slack")]
            PlatformSettings::Slack(ref slack_config) => {
                let handler = Arc::new(ManagerInboundHandler {
                    invoker: self.invoker.clone(),
                    logger: self.logger.clone(),
                    adapter: tokio::sync::RwLock::new(None),
                    default_agent: slack_config.default_agent.clone(),
                    #[cfg(feature = "enterprise-hooks")]
                    enterprise_hooks: self.enterprise_hooks.clone(),
                });
                let adapter: Arc<dyn ChannelAdapter> =
                    Arc::new(SlackAdapter::new(slack_config.clone(), handler.clone())?);
                // Wire the adapter back into the handler for response delivery
                handler.set_adapter(adapter.clone()).await;
                adapter
            }
            #[cfg(not(feature = "slack"))]
            PlatformSettings::Slack(_) => {
                return Err(ChannelAdapterError::Config(
                    "Slack adapter not enabled (compile with 'slack' feature)".to_string(),
                ));
            }
            #[cfg(feature = "teams")]
            PlatformSettings::Teams(ref teams_config) => {
                let handler = Arc::new(ManagerInboundHandler {
                    invoker: self.invoker.clone(),
                    logger: self.logger.clone(),
                    adapter: tokio::sync::RwLock::new(None),
                    default_agent: teams_config.default_agent.clone(),
                    #[cfg(feature = "enterprise-hooks")]
                    enterprise_hooks: self.enterprise_hooks.clone(),
                });
                let adapter: Arc<dyn ChannelAdapter> =
                    Arc::new(TeamsAdapter::new(teams_config.clone(), handler.clone())?);
                handler.set_adapter(adapter.clone()).await;
                adapter
            }
            #[cfg(feature = "mattermost")]
            PlatformSettings::Mattermost(ref mm_config) => {
                let handler = Arc::new(ManagerInboundHandler {
                    invoker: self.invoker.clone(),
                    logger: self.logger.clone(),
                    adapter: tokio::sync::RwLock::new(None),
                    default_agent: mm_config.default_agent.clone(),
                    #[cfg(feature = "enterprise-hooks")]
                    enterprise_hooks: self.enterprise_hooks.clone(),
                });
                let adapter: Arc<dyn ChannelAdapter> =
                    Arc::new(MattermostAdapter::new(mm_config.clone(), handler.clone())?);
                handler.set_adapter(adapter.clone()).await;
                adapter
            }
        };

        adapter.start().await?;
        self.adapters.insert(name, adapter);
        Ok(())
    }

    /// Stop and remove an adapter.
    pub async fn remove_adapter(&mut self, name: &str) -> Result<(), ChannelAdapterError> {
        match self.adapters.remove(name) {
            Some(adapter) => adapter.stop().await,
            None => Err(ChannelAdapterError::Config(format!(
                "no adapter named '{}'",
                name
            ))),
        }
    }

    /// Get health status of all adapters.
    pub async fn health(&self) -> HashMap<String, Result<crate::types::AdapterHealth, String>> {
        let mut results = HashMap::new();
        for (name, adapter) in &self.adapters {
            let health = adapter.check_health().await.map_err(|e| e.to_string());
            results.insert(name.clone(), health);
        }
        results
    }

    /// Stop all adapters.
    pub async fn shutdown(&mut self) -> Vec<(String, Result<(), ChannelAdapterError>)> {
        let mut results = Vec::new();
        let names: Vec<String> = self.adapters.keys().cloned().collect();
        for name in names {
            if let Some(adapter) = self.adapters.remove(&name) {
                let result = adapter.stop().await;
                results.push((name, result));
            }
        }
        results
    }

    /// List registered adapter names.
    pub fn list_adapters(&self) -> Vec<(String, ChatPlatform)> {
        self.adapters
            .iter()
            .map(|(name, adapter)| (name.clone(), adapter.platform()))
            .collect()
    }
}

/// Internal handler that routes inbound messages to agents and sends responses.
struct ManagerInboundHandler {
    invoker: Arc<dyn AgentInvoker>,
    logger: Arc<BasicInteractionLogger>,
    /// Reference to the adapter for sending responses back.
    /// Set after adapter creation via `set_adapter()`.
    adapter: tokio::sync::RwLock<Option<Arc<dyn ChannelAdapter>>>,
    default_agent: Option<String>,
    #[cfg(feature = "enterprise-hooks")]
    #[allow(dead_code)]
    enterprise_hooks: Option<Arc<dyn crate::traits::EnterpriseChannelHooks>>,
}

impl ManagerInboundHandler {
    /// Set the adapter reference after construction (needed because the adapter
    /// and handler have a circular dependency at creation time).
    async fn set_adapter(&self, adapter: Arc<dyn ChannelAdapter>) {
        *self.adapter.write().await = Some(adapter);
    }
}

#[async_trait]
impl InboundHandler for ManagerInboundHandler {
    async fn handle_message(&self, message: InboundMessage) -> Result<(), ChannelAdapterError> {
        let start = std::time::Instant::now();

        // Extract agent name from command or use default
        let agent_name = message
            .command
            .as_ref()
            .and_then(|cmd| cmd.agent_name.as_deref())
            .or(self.default_agent.as_deref())
            .unwrap_or("default");

        #[cfg(feature = "enterprise-hooks")]
        {
            // Enterprise pre-invoke: policy check, identity mapping
            if let Some(ref hooks) = self.enterprise_hooks {
                match hooks.pre_invoke(&message).await? {
                    crate::types::PolicyDecision::Deny { reason } => {
                        tracing::warn!(
                            user = %message.sender_id,
                            agent = %agent_name,
                            "Policy denied: {}",
                            reason
                        );
                        return Err(ChannelAdapterError::PolicyDenied(reason));
                    }
                    crate::types::PolicyDecision::RequireApproval { .. } => {
                        tracing::info!(
                            user = %message.sender_id,
                            agent = %agent_name,
                            "Approval required — not yet implemented"
                        );
                        return Err(ChannelAdapterError::PolicyDenied(
                            "approval required".to_string(),
                        ));
                    }
                    crate::types::PolicyDecision::Allow => {}
                }
            }
        }

        // Invoke the agent
        let result = self.invoker.invoke(agent_name, &message.content).await;

        let (success, duration_ms) = match &result {
            Ok(_) => (true, Some(start.elapsed().as_millis() as u64)),
            Err(_) => (false, Some(start.elapsed().as_millis() as u64)),
        };

        // Log the interaction
        let log_entry = BasicInteractionLogger::invoke_entry(
            message.platform,
            &message.sender_id,
            &message.channel_id,
            agent_name,
            success,
            duration_ms,
            result.as_ref().err().cloned(),
        );
        self.logger.log(&log_entry).await;

        #[cfg(feature = "enterprise-hooks")]
        {
            // Enterprise post-invoke: crypto audit
            if let Some(ref hooks) = self.enterprise_hooks {
                if let Err(e) = hooks.post_invoke(&log_entry).await {
                    tracing::warn!("Enterprise post-invoke hook failed: {}", e);
                }
            }
        }

        match result {
            Ok(response_text) => {
                // Format response based on platform
                let response = build_platform_response(&message, &response_text, agent_name);

                // Send response through the adapter
                let adapter_guard = self.adapter.read().await;
                if let Some(ref adapter) = *adapter_guard {
                    match adapter.send_response(response).await {
                        Ok(receipt) => {
                            tracing::info!(
                                agent = %agent_name,
                                channel = %message.channel_id,
                                delivered = %receipt.success,
                                "Agent response delivered"
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                agent = %agent_name,
                                channel = %message.channel_id,
                                error = %e,
                                "Failed to deliver agent response"
                            );
                        }
                    }
                } else {
                    tracing::warn!(
                        agent = %agent_name,
                        channel = %message.channel_id,
                        "No adapter available for response delivery"
                    );
                }

                Ok(())
            }
            Err(e) => {
                tracing::error!(
                    agent = %agent_name,
                    channel = %message.channel_id,
                    error = %e,
                    "Agent invocation failed"
                );
                Err(ChannelAdapterError::AgentError(e))
            }
        }
    }
}

/// Build a platform-appropriate outbound message with formatted content.
fn build_platform_response(
    message: &InboundMessage,
    content: &str,
    agent_name: &str,
) -> OutboundMessage {
    match message.platform {
        #[cfg(feature = "slack")]
        ChatPlatform::Slack => {
            let blocks = format_slack_response(content, agent_name);
            OutboundMessage {
                channel_id: message.channel_id.clone(),
                thread_id: message.thread_id.clone(),
                content: content.to_string(),
                blocks: Some(blocks),
                ephemeral: false,
                user_id: None,
                metadata: None,
            }
        }
        #[cfg(feature = "teams")]
        ChatPlatform::Teams => {
            let card = format_teams_response(content, agent_name);
            // Extract service_url and activity id from the raw_payload
            // so the adapter can route the reply correctly.
            let teams_meta = message.raw_payload.as_ref().map(|payload| {
                serde_json::json!({
                    "service_url": payload.get("serviceUrl")
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                    "activity_id": payload.get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                })
            });
            OutboundMessage {
                channel_id: message.channel_id.clone(),
                thread_id: message.thread_id.clone(),
                content: content.to_string(),
                blocks: Some(card),
                ephemeral: false,
                user_id: None,
                metadata: teams_meta,
            }
        }
        #[cfg(feature = "mattermost")]
        ChatPlatform::Mattermost => {
            let formatted = format_mattermost_response(content, agent_name);
            OutboundMessage {
                channel_id: message.channel_id.clone(),
                thread_id: message.thread_id.clone(),
                content: formatted,
                blocks: None,
                ephemeral: false,
                user_id: None,
                metadata: None,
            }
        }
        // Fallback for platforms without specific formatting
        #[allow(unreachable_patterns)]
        _ => OutboundMessage {
            channel_id: message.channel_id.clone(),
            thread_id: message.thread_id.clone(),
            content: content.to_string(),
            blocks: None,
            ephemeral: false,
            user_id: None,
            metadata: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EchoInvoker;

    #[async_trait]
    impl AgentInvoker for EchoInvoker {
        async fn invoke(&self, agent_name: &str, input: &str) -> Result<String, String> {
            Ok(format!("[{}] echo: {}", agent_name, input))
        }
    }

    struct FailInvoker;

    #[async_trait]
    impl AgentInvoker for FailInvoker {
        async fn invoke(&self, _agent_name: &str, _input: &str) -> Result<String, String> {
            Err("agent crashed".to_string())
        }
    }

    #[tokio::test]
    async fn manager_inbound_handler_success() {
        let logger = Arc::new(BasicInteractionLogger::new(None));
        let handler = ManagerInboundHandler {
            invoker: Arc::new(EchoInvoker),
            logger: logger.clone(),
            adapter: tokio::sync::RwLock::new(None),
            default_agent: Some("echo".to_string()),
            #[cfg(feature = "enterprise-hooks")]
            enterprise_hooks: None,
        };

        let msg = InboundMessage {
            id: "test-1".to_string(),
            platform: ChatPlatform::Slack,
            workspace_id: "T123".to_string(),
            channel_id: "C456".to_string(),
            thread_id: None,
            sender_id: "U789".to_string(),
            sender_name: "alice".to_string(),
            content: "hello world".to_string(),
            command: None,
            timestamp: chrono::Utc::now(),
            raw_payload: None,
        };

        let result = handler.handle_message(msg).await;
        assert!(result.is_ok());
        assert_eq!(logger.interaction_count().await, 1);
    }

    #[tokio::test]
    async fn manager_inbound_handler_agent_failure() {
        let logger = Arc::new(BasicInteractionLogger::new(None));
        let handler = ManagerInboundHandler {
            invoker: Arc::new(FailInvoker),
            logger: logger.clone(),
            adapter: tokio::sync::RwLock::new(None),
            default_agent: Some("broken".to_string()),
            #[cfg(feature = "enterprise-hooks")]
            enterprise_hooks: None,
        };

        let msg = InboundMessage {
            id: "test-2".to_string(),
            platform: ChatPlatform::Slack,
            workspace_id: "T123".to_string(),
            channel_id: "C456".to_string(),
            thread_id: None,
            sender_id: "U789".to_string(),
            sender_name: "bob".to_string(),
            content: "do something".to_string(),
            command: None,
            timestamp: chrono::Utc::now(),
            raw_payload: None,
        };

        let result = handler.handle_message(msg).await;
        assert!(result.is_err());
        // Interaction should still be logged
        assert_eq!(logger.interaction_count().await, 1);
    }

    #[test]
    fn manager_list_empty() {
        let invoker = Arc::new(EchoInvoker);
        let logger = Arc::new(BasicInteractionLogger::new(None));
        let manager = ChannelAdapterManager::new(invoker, logger);
        assert!(manager.list_adapters().is_empty());
    }
}
