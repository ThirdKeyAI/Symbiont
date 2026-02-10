//! Mattermost channel adapter — bidirectional Mattermost integration.
//!
//! Receives messages via outgoing webhooks, sends responses via
//! the Mattermost v4 REST API with bot token authentication.

pub mod api;
pub mod events;
pub mod signature;

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use tokio::sync::{watch, RwLock};

use crate::config::MattermostConfig;
use crate::error::ChannelAdapterError;
use crate::traits::{ChannelAdapter, InboundHandler};
use crate::types::{AdapterHealth, ChatDeliveryReceipt, ChatPlatform, OutboundMessage};

use api::MattermostApiClient;

/// Shared state for Axum webhook handlers.
struct MattermostAdapterState {
    #[allow(dead_code)]
    api_client: MattermostApiClient,
    config: MattermostConfig,
    handler: Arc<dyn InboundHandler>,
    last_message_at: RwLock<Option<chrono::DateTime<chrono::Utc>>>,
}

/// Bidirectional Mattermost adapter.
///
/// Starts an Axum HTTP server to receive outgoing webhook payloads,
/// and uses the Mattermost v4 API to send responses.
pub struct MattermostAdapter {
    config: MattermostConfig,
    api_client: MattermostApiClient,
    handler: Arc<dyn InboundHandler>,
    shutdown_tx: RwLock<Option<watch::Sender<()>>>,
    started_at: RwLock<Option<std::time::Instant>>,
    bot_username: RwLock<Option<String>>,
    last_message_at: Arc<RwLock<Option<chrono::DateTime<chrono::Utc>>>>,
}

impl MattermostAdapter {
    pub fn new(
        config: MattermostConfig,
        handler: Arc<dyn InboundHandler>,
    ) -> Result<Self, ChannelAdapterError> {
        if config.server_url.is_empty() {
            return Err(ChannelAdapterError::Config(
                "Mattermost server_url is required".to_string(),
            ));
        }
        if config.bot_token.is_empty() {
            return Err(ChannelAdapterError::Config(
                "Mattermost bot_token is required".to_string(),
            ));
        }

        let api_client = MattermostApiClient::new(&config.server_url, &config.bot_token)?;

        Ok(Self {
            config,
            api_client,
            handler,
            shutdown_tx: RwLock::new(None),
            started_at: RwLock::new(None),
            bot_username: RwLock::new(None),
            last_message_at: Arc::new(RwLock::new(None)),
        })
    }
}

#[async_trait]
impl ChannelAdapter for MattermostAdapter {
    async fn start(&self) -> Result<(), ChannelAdapterError> {
        if self.shutdown_tx.read().await.is_some() {
            return Err(ChannelAdapterError::AlreadyRunning);
        }

        // Verify bot token via /api/v4/users/me
        let me = self.api_client.get_me().await?;
        *self.bot_username.write().await = me.username.clone();

        tracing::info!(
            server = %self.config.server_url,
            bot_user = me.username.as_deref().unwrap_or("unknown"),
            bot_id = %me.id,
            "Mattermost adapter connected"
        );

        // Build Axum router for webhook receiver
        let state = Arc::new(MattermostAdapterState {
            api_client: self.api_client.clone(),
            config: self.config.clone(),
            handler: self.handler.clone(),
            last_message_at: RwLock::new(None),
        });

        let app = Router::new()
            .route("/mattermost/webhook", post(handle_mattermost_webhook))
            .route("/health", axum::routing::get(health_check))
            .with_state(state.clone());

        let addr = format!("{}:{}", self.config.bind_address, self.config.webhook_port);
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .map_err(|e| ChannelAdapterError::Connection(format!("bind failed: {}", e)))?;

        let (shutdown_tx, mut shutdown_rx) = watch::channel(());
        *self.shutdown_tx.write().await = Some(shutdown_tx);
        *self.started_at.write().await = Some(std::time::Instant::now());

        let last_message = self.last_message_at.clone();

        tokio::spawn(async move {
            tracing::info!(addr = %addr, "Mattermost webhook server listening");
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.changed().await;
                })
                .await
                .unwrap_or_else(|e| tracing::error!("Mattermost webhook server error: {}", e));
            let ts = *state.last_message_at.read().await;
            *last_message.write().await = ts;
        });

        Ok(())
    }

    async fn stop(&self) -> Result<(), ChannelAdapterError> {
        let tx = self.shutdown_tx.write().await.take();
        match tx {
            Some(tx) => {
                let _ = tx.send(());
                *self.started_at.write().await = None;
                tracing::info!("Mattermost adapter stopped");
                Ok(())
            }
            None => Err(ChannelAdapterError::NotRunning),
        }
    }

    async fn send_response(
        &self,
        response: OutboundMessage,
    ) -> Result<ChatDeliveryReceipt, ChannelAdapterError> {
        self.api_client.create_post(&response).await
    }

    fn platform(&self) -> ChatPlatform {
        ChatPlatform::Mattermost
    }

    async fn check_health(&self) -> Result<AdapterHealth, ChannelAdapterError> {
        let connected = self.shutdown_tx.read().await.is_some();
        let uptime = self
            .started_at
            .read()
            .await
            .map(|s| s.elapsed().as_secs())
            .unwrap_or(0);

        Ok(AdapterHealth {
            connected,
            platform: ChatPlatform::Mattermost,
            workspace_name: self.bot_username.read().await.clone(),
            channels_active: self.config.channels.len(),
            last_message_at: *self.last_message_at.read().await,
            uptime_secs: uptime,
        })
    }
}

/// Axum handler for Mattermost outgoing webhooks.
async fn handle_mattermost_webhook(
    State(state): State<Arc<MattermostAdapterState>>,
    body: Bytes,
) -> impl IntoResponse {
    // Parse the webhook payload
    let payload: events::OutgoingWebhookPayload = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("Failed to parse Mattermost webhook: {}", e);
            return (StatusCode::BAD_REQUEST, format!("parse error: {}", e));
        }
    };

    // Verify webhook token if configured
    if let Some(ref secret) = state.config.webhook_secret {
        let received_token = payload.token.as_deref().unwrap_or("");
        if let Err(e) = signature::verify_webhook_token(secret, received_token) {
            tracing::warn!("Mattermost webhook token verification failed: {}", e);
            return (StatusCode::UNAUTHORIZED, "invalid token".to_string());
        }
    }

    // Parse into inbound message
    match events::parse_webhook_to_message(&payload) {
        Ok(msg) => {
            *state.last_message_at.write().await = Some(chrono::Utc::now());
            let handler = state.handler.clone();
            tokio::spawn(async move {
                if let Err(e) = handler.handle_message(msg).await {
                    tracing::error!("Failed to handle Mattermost message: {}", e);
                }
            });
            (StatusCode::OK, String::new())
        }
        Err(e) => {
            tracing::warn!("Mattermost webhook parse error: {}", e);
            (StatusCode::OK, String::new())
        }
    }
}

/// Health check endpoint.
async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoopHandler;

    #[async_trait]
    impl InboundHandler for NoopHandler {
        async fn handle_message(
            &self,
            _message: crate::types::InboundMessage,
        ) -> Result<(), ChannelAdapterError> {
            Ok(())
        }
    }

    #[test]
    fn mattermost_adapter_rejects_empty_server_url() {
        let config = MattermostConfig {
            server_url: "".to_string(),
            ..MattermostConfig::default()
        };
        let result = MattermostAdapter::new(config, Arc::new(NoopHandler));
        assert!(result.is_err());
    }

    #[test]
    fn mattermost_adapter_rejects_empty_bot_token() {
        let config = MattermostConfig {
            server_url: "https://mm.example.com".to_string(),
            bot_token: "".to_string(),
            ..MattermostConfig::default()
        };
        let result = MattermostAdapter::new(config, Arc::new(NoopHandler));
        assert!(result.is_err());
    }

    #[test]
    fn mattermost_adapter_accepts_valid_config() {
        let config = MattermostConfig {
            server_url: "https://mm.example.com".to_string(),
            bot_token: "token-123".to_string(),
            ..MattermostConfig::default()
        };
        let result = MattermostAdapter::new(config, Arc::new(NoopHandler));
        assert!(result.is_ok());
    }
}
