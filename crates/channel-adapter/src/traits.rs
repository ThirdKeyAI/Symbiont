use async_trait::async_trait;

use crate::error::ChannelAdapterError;
use crate::types::{
    AdapterHealth, ChatDeliveryReceipt, ChatPlatform, InboundMessage, OutboundMessage,
};
#[cfg(feature = "enterprise-hooks")]
use crate::types::{FilteredContent, InteractionLog, PolicyDecision};

/// Core trait for chat platform adapters.
///
/// Implementors handle bidirectional communication with a specific chat platform
/// (Slack, Teams, Mattermost). The community edition ships with Slack; enterprise
/// adds Teams and Mattermost.
#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    /// Start receiving messages from the platform.
    async fn start(&self) -> Result<(), ChannelAdapterError>;

    /// Stop the adapter gracefully.
    async fn stop(&self) -> Result<(), ChannelAdapterError>;

    /// Send a response back to the platform.
    async fn send_response(
        &self,
        response: OutboundMessage,
    ) -> Result<ChatDeliveryReceipt, ChannelAdapterError>;

    /// Which platform this adapter handles.
    fn platform(&self) -> ChatPlatform;

    /// Check adapter connectivity and health.
    async fn check_health(&self) -> Result<AdapterHealth, ChannelAdapterError>;
}

/// Callback for inbound messages from any adapter.
///
/// The `ChannelAdapterManager` implements this to route messages to agents.
#[async_trait]
pub trait InboundHandler: Send + Sync {
    async fn handle_message(&self, message: InboundMessage) -> Result<(), ChannelAdapterError>;
}

/// Extension point for enterprise governance layer.
///
/// Community edition runs without these hooks. When present, the manager calls
/// them at the appropriate points in the message lifecycle.
#[cfg(feature = "enterprise-hooks")]
#[async_trait]
pub trait EnterpriseChannelHooks: Send + Sync {
    /// Called before agent invocation — policy check, identity mapping.
    async fn pre_invoke(&self, msg: &InboundMessage)
        -> Result<PolicyDecision, ChannelAdapterError>;

    /// Called before sending response — DLP filtering.
    async fn filter_response(
        &self,
        content: &str,
        channel: &str,
    ) -> Result<FilteredContent, ChannelAdapterError>;

    /// Called after response is sent — cryptographic audit logging.
    async fn post_invoke(&self, log: &InteractionLog) -> Result<(), ChannelAdapterError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ChatPlatform;

    struct MockAdapter;

    #[async_trait]
    impl ChannelAdapter for MockAdapter {
        async fn start(&self) -> Result<(), ChannelAdapterError> {
            Ok(())
        }
        async fn stop(&self) -> Result<(), ChannelAdapterError> {
            Ok(())
        }
        async fn send_response(
            &self,
            _response: OutboundMessage,
        ) -> Result<ChatDeliveryReceipt, ChannelAdapterError> {
            Ok(ChatDeliveryReceipt {
                platform: ChatPlatform::Slack,
                channel_id: "C123".to_string(),
                message_ts: Some("1234567890.123456".to_string()),
                delivered_at: chrono::Utc::now(),
                success: true,
                error: None,
            })
        }
        fn platform(&self) -> ChatPlatform {
            ChatPlatform::Slack
        }
        async fn check_health(&self) -> Result<AdapterHealth, ChannelAdapterError> {
            Ok(AdapterHealth {
                connected: true,
                platform: ChatPlatform::Slack,
                workspace_name: Some("test-workspace".to_string()),
                channels_active: 1,
                last_message_at: None,
                uptime_secs: 0,
            })
        }
    }

    #[tokio::test]
    async fn mock_adapter_lifecycle() {
        let adapter = MockAdapter;
        assert!(adapter.start().await.is_ok());
        assert_eq!(adapter.platform(), ChatPlatform::Slack);
        let health = adapter.check_health().await.unwrap();
        assert!(health.connected);
        assert!(adapter.stop().await.is_ok());
    }

    #[tokio::test]
    async fn mock_adapter_send() {
        let adapter = MockAdapter;
        let msg = OutboundMessage {
            channel_id: "C123".to_string(),
            thread_id: None,
            content: "hello".to_string(),
            blocks: None,
            ephemeral: false,
            user_id: None,
            metadata: None,
        };
        let receipt = adapter.send_response(msg).await.unwrap();
        assert!(receipt.success);
        assert_eq!(receipt.platform, ChatPlatform::Slack);
    }
}
