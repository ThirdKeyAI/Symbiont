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
use crate::traits::ChannelAdapter;
#[cfg(any(feature = "slack", feature = "teams", feature = "mattermost"))]
use crate::traits::InboundHandler;
use crate::types::ChatDeliveryReceipt;
use crate::types::InboundMessage;
use crate::types::{ChatPlatform, OutboundMessage};

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

    /// Own invocation through actual delivery. Runtime implementations override
    /// this to require policy and audit for the final formatted message. The SDK
    /// default leaves governance to its embedding application.
    async fn invoke_and_deliver(
        &self,
        agent_name: &str,
        message: &InboundMessage,
        adapter: Arc<dyn ChannelAdapter>,
    ) -> Result<(), String> {
        let content = self.invoke(agent_name, &message.content).await?;
        let receipt = adapter
            .send_response(build_platform_response(message, &content, agent_name))
            .await
            .map_err(|error| error.to_string())?;
        if !receipt.success
            || receipt.platform != message.platform
            || receipt.channel_id != message.channel_id
        {
            return Err("response delivery was not confirmed for the requested destination".into());
        }
        Ok(())
    }
}

/// Lightweight orchestrator for channel adapters.
///
/// Manages adapter lifecycle, routes inbound messages to agents,
/// and sends responses back through the originating adapter.
pub struct ChannelAdapterManager {
    adapters: HashMap<String, Arc<dyn ChannelAdapter>>,
    // Consumed only when constructing platform adapter handlers (`ManagerInboundHandler`),
    // which are gated behind the platform features. Stored unconditionally so `new()`
    // keeps a stable signature across all build configurations.
    #[cfg_attr(
        not(any(feature = "slack", feature = "teams", feature = "mattermost")),
        allow(dead_code)
    )]
    invoker: Arc<dyn AgentInvoker>,
    #[cfg_attr(
        not(any(feature = "slack", feature = "teams", feature = "mattermost")),
        allow(dead_code)
    )]
    logger: Arc<BasicInteractionLogger>,
    interceptor: Option<Arc<dyn crate::traits::InboundCommandInterceptor>>,
    #[cfg(feature = "enterprise-hooks")]
    enterprise_hooks: Option<Arc<dyn crate::traits::EnterpriseChannelHooks>>,
}

impl ChannelAdapterManager {
    pub fn new(invoker: Arc<dyn AgentInvoker>, logger: Arc<BasicInteractionLogger>) -> Self {
        Self {
            adapters: HashMap::new(),
            invoker,
            logger,
            interceptor: None,
            #[cfg(feature = "enterprise-hooks")]
            enterprise_hooks: None,
        }
    }

    /// Set the inbound command interceptor. Must be called before `register_adapter`,
    /// since each handler is built at registration time.
    pub fn set_interceptor(&mut self, icpt: Arc<dyn crate::traits::InboundCommandInterceptor>) {
        self.interceptor = Some(icpt);
    }

    #[cfg(feature = "enterprise-hooks")]
    pub fn set_enterprise_hooks(&mut self, hooks: Arc<dyn crate::traits::EnterpriseChannelHooks>) {
        self.enterprise_hooks = Some(hooks);
    }

    /// Register and start an adapter from configuration.
    ///
    /// When no platform feature is compiled in, this is a lib-only surface with no
    /// adapter implementations available, so registration always fails with a
    /// configuration error mirroring the per-platform "not enabled" errors.
    #[cfg(not(any(feature = "slack", feature = "teams", feature = "mattermost")))]
    pub async fn register_adapter(
        &mut self,
        config: ChannelConfig,
    ) -> Result<(), ChannelAdapterError> {
        match config.settings {
            PlatformSettings::Slack(_) => Err(ChannelAdapterError::Config(
                "Slack adapter not enabled (compile with 'slack' feature)".to_string(),
            )),
        }
    }

    /// Register and start an adapter from configuration.
    #[cfg(any(feature = "slack", feature = "teams", feature = "mattermost"))]
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
                    interceptor: self.interceptor.clone(),
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
                    interceptor: self.interceptor.clone(),
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
                    interceptor: self.interceptor.clone(),
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

    /// Send a message to the first adapter matching `platform`.
    pub async fn send_to(
        &self,
        platform: ChatPlatform,
        msg: OutboundMessage,
    ) -> Result<ChatDeliveryReceipt, ChannelAdapterError> {
        let adapter = self
            .adapters
            .values()
            .find(|a| a.platform() == platform)
            .ok_or_else(|| ChannelAdapterError::SendFailed("no adapter for platform".into()))?;
        adapter.send_response(msg).await
    }
}

/// Internal handler that routes inbound messages to agents and sends responses.
///
/// Only constructed by `register_adapter` for a concrete platform adapter, so it is
/// compiled only when at least one platform feature is enabled. The `#[cfg(test)]`
/// blocks below also rely on a platform feature being active (default = `slack`).
#[cfg(any(feature = "slack", feature = "teams", feature = "mattermost"))]
struct ManagerInboundHandler {
    invoker: Arc<dyn AgentInvoker>,
    logger: Arc<BasicInteractionLogger>,
    /// Reference to the adapter for sending responses back.
    /// Set after adapter creation via `set_adapter()`.
    adapter: tokio::sync::RwLock<Option<Arc<dyn ChannelAdapter>>>,
    default_agent: Option<String>,
    interceptor: Option<Arc<dyn crate::traits::InboundCommandInterceptor>>,
    #[cfg(feature = "enterprise-hooks")]
    #[allow(dead_code)]
    enterprise_hooks: Option<Arc<dyn crate::traits::EnterpriseChannelHooks>>,
}

#[cfg(any(feature = "slack", feature = "teams", feature = "mattermost"))]
impl ManagerInboundHandler {
    /// Set the adapter reference after construction (needed because the adapter
    /// and handler have a circular dependency at creation time).
    async fn set_adapter(&self, adapter: Arc<dyn ChannelAdapter>) {
        *self.adapter.write().await = Some(adapter);
    }
}

#[cfg(any(feature = "slack", feature = "teams", feature = "mattermost"))]
#[async_trait]
impl InboundHandler for ManagerInboundHandler {
    async fn handle_message(&self, message: InboundMessage) -> Result<(), ChannelAdapterError> {
        // Check interceptor first; if it handles the message, skip agent invocation.
        if let Some(icpt) = &self.interceptor {
            if let Some(reply) = icpt.try_handle(&message).await {
                if let Some(adapter) = self.adapter.read().await.as_ref() {
                    let _ = adapter
                        .send_response(OutboundMessage {
                            channel_id: message.channel_id.clone(),
                            thread_id: message.thread_id.clone(),
                            content: reply,
                            blocks: None,
                            ephemeral: true,
                            user_id: Some(message.sender_id.clone()),
                            metadata: None,
                        })
                        .await;
                }
                return Ok(());
            }
        }

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

        // The invoker owns the full response lifecycle, including the final
        // formatted send. Never send again after an audited invoker returns.
        let adapter = self.adapter.read().await.clone();
        let result = match adapter {
            Some(adapter) => {
                self.invoker
                    .invoke_and_deliver(agent_name, &message, adapter)
                    .await
            }
            None => Err("no adapter available for response delivery".into()),
        };

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
            Ok(()) => Ok(()),
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
///
/// Shared with runtime invokers so authorization covers the actual formatting.
pub fn build_platform_response(
    message: &InboundMessage,
    content: &str,
    _agent_name: &str,
) -> OutboundMessage {
    match message.platform {
        #[cfg(feature = "slack")]
        ChatPlatform::Slack => {
            let blocks = format_slack_response(content, _agent_name);
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
            let card = format_teams_response(content, _agent_name);
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
            let formatted = format_mattermost_response(content, _agent_name);
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

    #[cfg(any(feature = "slack", feature = "teams", feature = "mattermost"))]
    struct FailInvoker;

    #[cfg(any(feature = "slack", feature = "teams", feature = "mattermost"))]
    #[async_trait]
    impl AgentInvoker for FailInvoker {
        async fn invoke(&self, _agent_name: &str, _input: &str) -> Result<String, String> {
            Err("agent crashed".to_string())
        }
    }

    #[cfg(any(feature = "slack", feature = "teams", feature = "mattermost"))]
    #[tokio::test]
    async fn manager_inbound_handler_success() {
        let logger = Arc::new(BasicInteractionLogger::new(None));
        let handler = ManagerInboundHandler {
            invoker: Arc::new(EchoInvoker),
            logger: logger.clone(),
            adapter: tokio::sync::RwLock::new(Some(super::delivery_tests::adapter(true))),
            default_agent: Some("echo".to_string()),
            interceptor: None,
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

    #[cfg(any(feature = "slack", feature = "teams", feature = "mattermost"))]
    #[tokio::test]
    async fn manager_inbound_handler_agent_failure() {
        let logger = Arc::new(BasicInteractionLogger::new(None));
        let handler = ManagerInboundHandler {
            invoker: Arc::new(FailInvoker),
            logger: logger.clone(),
            adapter: tokio::sync::RwLock::new(Some(super::delivery_tests::adapter(true))),
            default_agent: Some("broken".to_string()),
            interceptor: None,
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

#[cfg(test)]
#[cfg(any(feature = "slack", feature = "teams", feature = "mattermost"))]
impl ManagerInboundHandler {
    pub(crate) fn for_test(
        invoker: Arc<dyn AgentInvoker>,
        interceptor: Option<Arc<dyn crate::traits::InboundCommandInterceptor>>,
    ) -> Self {
        Self {
            invoker,
            logger: Arc::new(BasicInteractionLogger::new(None)),
            adapter: tokio::sync::RwLock::new(None),
            default_agent: None,
            interceptor,
            #[cfg(feature = "enterprise-hooks")]
            enterprise_hooks: None,
        }
    }
}

#[cfg(test)]
#[cfg(any(feature = "slack", feature = "teams", feature = "mattermost"))]
mod interceptor_tests {
    use super::*;
    use crate::types::{ChatPlatform, InboundMessage};
    use std::sync::{Arc, Mutex};

    struct CountingInvoker {
        calls: Mutex<usize>,
    }

    #[async_trait]
    impl AgentInvoker for CountingInvoker {
        async fn invoke(&self, _agent: &str, _input: &str) -> Result<String, String> {
            *self.calls.lock().unwrap() += 1;
            Ok("agent".into())
        }
    }

    struct ShortCircuit;

    #[async_trait]
    impl crate::traits::InboundCommandInterceptor for ShortCircuit {
        async fn try_handle(&self, _m: &InboundMessage) -> Option<String> {
            Some("handled".into())
        }
    }

    fn msg() -> InboundMessage {
        InboundMessage {
            id: "m".into(),
            platform: ChatPlatform::Slack,
            workspace_id: "w".into(),
            channel_id: "C".into(),
            thread_id: None,
            sender_id: "U".into(),
            sender_name: "U".into(),
            content: "hello".into(),
            command: None,
            timestamp: chrono::Utc::now(),
            raw_payload: None,
        }
    }

    #[tokio::test]
    async fn interceptor_short_circuits_agent() {
        let invoker = Arc::new(CountingInvoker {
            calls: Mutex::new(0),
        });
        let handler =
            ManagerInboundHandler::for_test(invoker.clone(), Some(Arc::new(ShortCircuit)));
        handler.handle_message(msg()).await.unwrap();
        assert_eq!(
            *invoker.calls.lock().unwrap(),
            0,
            "agent must not be invoked when interceptor handles"
        );
    }

    #[tokio::test]
    async fn no_interceptor_invokes_agent() {
        let invoker = Arc::new(CountingInvoker {
            calls: Mutex::new(0),
        });
        let handler = ManagerInboundHandler::for_test(invoker.clone(), None);
        handler
            .set_adapter(super::delivery_tests::adapter(true))
            .await;
        let _ = handler.handle_message(msg()).await;
        assert_eq!(*invoker.calls.lock().unwrap(), 1);
    }
}

#[cfg(all(
    test,
    any(feature = "slack", feature = "teams", feature = "mattermost")
))]
mod delivery_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct ReceiptAdapter {
        success: bool,
    }
    #[async_trait]
    impl ChannelAdapter for ReceiptAdapter {
        async fn start(&self) -> Result<(), ChannelAdapterError> {
            Ok(())
        }
        async fn stop(&self) -> Result<(), ChannelAdapterError> {
            Ok(())
        }
        fn platform(&self) -> ChatPlatform {
            ChatPlatform::Slack
        }
        async fn check_health(&self) -> Result<crate::types::AdapterHealth, ChannelAdapterError> {
            Err(ChannelAdapterError::Internal(
                "fixture health unavailable".into(),
            ))
        }
        async fn send_response(
            &self,
            message: OutboundMessage,
        ) -> Result<ChatDeliveryReceipt, ChannelAdapterError> {
            Ok(ChatDeliveryReceipt {
                platform: ChatPlatform::Slack,
                channel_id: message.channel_id,
                message_ts: Some("fixture-receipt".into()),
                delivered_at: chrono::Utc::now(),
                success: self.success,
                error: None,
            })
        }
    }
    pub(super) fn adapter(success: bool) -> Arc<dyn ChannelAdapter> {
        Arc::new(ReceiptAdapter { success })
    }
    fn message() -> InboundMessage {
        InboundMessage {
            id: "m".into(),
            platform: ChatPlatform::Slack,
            workspace_id: "w".into(),
            channel_id: "C1".into(),
            thread_id: None,
            sender_id: "U1".into(),
            sender_name: "fixture".into(),
            content: "input".into(),
            command: None,
            timestamp: chrono::Utc::now(),
            raw_payload: None,
        }
    }
    struct Plain;
    #[async_trait]
    impl AgentInvoker for Plain {
        async fn invoke(&self, _: &str, _: &str) -> Result<String, String> {
            Ok("response".into())
        }
    }
    #[tokio::test]
    async fn missing_adapter_and_negative_receipt_are_errors() {
        let handler = ManagerInboundHandler::for_test(Arc::new(Plain), None);
        assert!(handler.handle_message(message()).await.is_err());
        handler.set_adapter(adapter(false)).await;
        assert!(handler.handle_message(message()).await.is_err());
        handler.set_adapter(adapter(true)).await;
        assert!(handler.handle_message(message()).await.is_ok());
    }
    struct Owned(AtomicUsize);
    #[async_trait]
    impl AgentInvoker for Owned {
        async fn invoke(&self, _: &str, _: &str) -> Result<String, String> {
            panic!("manager bypassed owned delivery")
        }
        async fn invoke_and_deliver(
            &self,
            _: &str,
            _: &InboundMessage,
            _: Arc<dyn ChannelAdapter>,
        ) -> Result<(), String> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }
    #[tokio::test]
    async fn manager_uses_the_owned_delivery_contract() {
        let invoker = Arc::new(Owned(AtomicUsize::new(0)));
        let handler = ManagerInboundHandler::for_test(invoker.clone(), None);
        handler.set_adapter(adapter(false)).await;
        handler.handle_message(message()).await.unwrap();
        assert_eq!(invoker.0.load(Ordering::SeqCst), 1);
    }
}
