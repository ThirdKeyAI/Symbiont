//! Microsoft Teams channel adapter — bidirectional Teams integration.
//!
//! Receives messages via Bot Framework webhook, sends replies via
//! Bot Framework REST API with OAuth2 authentication.

pub mod api;
pub mod auth;
pub mod events;

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use tokio::sync::{watch, RwLock};

use crate::config::TeamsConfig;
use crate::error::ChannelAdapterError;
use crate::traits::{ChannelAdapter, InboundHandler};
use crate::types::{AdapterHealth, ChatDeliveryReceipt, ChatPlatform, OutboundMessage};

use api::TeamsApiClient;

/// Shared state for Axum webhook handlers.
struct TeamsAdapterState {
    #[allow(dead_code)] // Used when direct reply routing is wired up
    api_client: TeamsApiClient,
    config: TeamsConfig,
    handler: Arc<dyn InboundHandler>,
    last_message_at: RwLock<Option<chrono::DateTime<chrono::Utc>>>,
}

/// Bidirectional Microsoft Teams adapter.
///
/// Starts an Axum HTTP server to receive Bot Framework webhook activities,
/// and uses the Bot Framework REST API to send replies.
pub struct TeamsAdapter {
    config: TeamsConfig,
    api_client: TeamsApiClient,
    handler: Arc<dyn InboundHandler>,
    shutdown_tx: RwLock<Option<watch::Sender<()>>>,
    started_at: RwLock<Option<std::time::Instant>>,
    last_message_at: Arc<RwLock<Option<chrono::DateTime<chrono::Utc>>>>,
}

impl TeamsAdapter {
    pub fn new(
        config: TeamsConfig,
        handler: Arc<dyn InboundHandler>,
    ) -> Result<Self, ChannelAdapterError> {
        if config.tenant_id.is_empty() {
            return Err(ChannelAdapterError::Config(
                "Teams tenant_id is required".to_string(),
            ));
        }
        if config.client_id.is_empty() {
            return Err(ChannelAdapterError::Config(
                "Teams client_id is required".to_string(),
            ));
        }
        if config.client_secret.is_empty() {
            return Err(ChannelAdapterError::Config(
                "Teams client_secret is required".to_string(),
            ));
        }

        let api_client =
            TeamsApiClient::new(&config.tenant_id, &config.client_id, &config.client_secret)?;

        Ok(Self {
            config,
            api_client,
            handler,
            shutdown_tx: RwLock::new(None),
            started_at: RwLock::new(None),
            last_message_at: Arc::new(RwLock::new(None)),
        })
    }
}

#[async_trait]
impl ChannelAdapter for TeamsAdapter {
    async fn start(&self) -> Result<(), ChannelAdapterError> {
        if self.shutdown_tx.read().await.is_some() {
            return Err(ChannelAdapterError::AlreadyRunning);
        }

        // Verify credentials by acquiring an OAuth2 token
        self.api_client.get_oauth_token().await?;

        tracing::info!(
            tenant = %self.config.tenant_id,
            bot_id = %self.config.bot_id,
            "Teams adapter connected (OAuth2 token acquired)"
        );

        // Build Axum router for Bot Framework webhook
        let state = Arc::new(TeamsAdapterState {
            api_client: self.api_client.clone(),
            config: self.config.clone(),
            handler: self.handler.clone(),
            last_message_at: RwLock::new(None),
        });

        let app = Router::new()
            .route("/teams/messages", post(handle_teams_activity))
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
            tracing::info!(addr = %addr, "Teams webhook server listening");
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.changed().await;
                })
                .await
                .unwrap_or_else(|e| tracing::error!("Teams webhook server error: {}", e));
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
                tracing::info!("Teams adapter stopped");
                Ok(())
            }
            None => Err(ChannelAdapterError::NotRunning),
        }
    }

    async fn send_response(
        &self,
        response: OutboundMessage,
    ) -> Result<ChatDeliveryReceipt, ChannelAdapterError> {
        // Extract service_url and activity_id from metadata (set by
        // build_platform_response in the manager). Fall back to defaults
        // for direct send_response calls without metadata.
        let (service_url, activity_id) = if let Some(ref meta) = response.metadata {
            let surl = meta
                .get("service_url")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("https://smba.trafficmanager.net/teams/");
            let aid = meta
                .get("activity_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            (surl.to_string(), aid.to_string())
        } else {
            (
                "https://smba.trafficmanager.net/teams/".to_string(),
                response.thread_id.clone().unwrap_or_default(),
            )
        };

        self.api_client
            .reply_to_activity(&service_url, &response.channel_id, &activity_id, &response)
            .await
    }

    fn platform(&self) -> ChatPlatform {
        ChatPlatform::Teams
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
            platform: ChatPlatform::Teams,
            workspace_name: Some(format!("tenant:{}", self.config.tenant_id)),
            channels_active: 0,
            last_message_at: *self.last_message_at.read().await,
            uptime_secs: uptime,
        })
    }
}

/// Axum handler for Bot Framework activities.
async fn handle_teams_activity(
    State(state): State<Arc<TeamsAdapterState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // Validate JWT token from Authorization header
    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let token = match auth::extract_bearer_token(auth_header) {
        Some(t) => t,
        None => {
            tracing::warn!("Teams activity missing Authorization header");
            return (
                StatusCode::UNAUTHORIZED,
                "missing authorization".to_string(),
            );
        }
    };

    // TODO: In production, set skip_jwks_verification to false
    if let Err(e) = auth::validate_bot_framework_token(token, &state.config.client_id, true).await {
        tracing::warn!("Teams JWT validation failed: {}", e);
        return (StatusCode::UNAUTHORIZED, "invalid token".to_string());
    }

    // Parse the activity
    let activity: events::Activity = match serde_json::from_slice(&body) {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!("Failed to parse Teams activity: {}", e);
            return (StatusCode::BAD_REQUEST, format!("parse error: {}", e));
        }
    };

    // Parse into inbound message
    match events::parse_activity_to_message(&activity, &state.config.bot_id) {
        Ok(Some(msg)) => {
            *state.last_message_at.write().await = Some(chrono::Utc::now());

            // Store service_url and activity_id for reply routing
            let service_url = activity.service_url.clone().unwrap_or_default();
            let activity_id = activity.id.clone().unwrap_or_default();
            let conversation_id = msg.channel_id.clone();

            let handler = state.handler.clone();
            tokio::spawn(async move {
                if let Err(e) = handler.handle_message(msg).await {
                    tracing::error!(
                        conversation = %conversation_id,
                        activity = %activity_id,
                        service_url = %service_url,
                        error = %e,
                        "Failed to handle Teams message"
                    );
                }
            });

            (StatusCode::OK, String::new())
        }
        Ok(None) => (StatusCode::OK, String::new()),
        Err(e) => {
            tracing::warn!("Teams activity parse error: {}", e);
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
    fn teams_adapter_rejects_empty_tenant_id() {
        let config = TeamsConfig {
            tenant_id: "".to_string(),
            ..TeamsConfig::default()
        };
        let result = TeamsAdapter::new(config, Arc::new(NoopHandler));
        assert!(result.is_err());
    }

    #[test]
    fn teams_adapter_rejects_empty_client_id() {
        let config = TeamsConfig {
            tenant_id: "tenant-123".to_string(),
            client_id: "".to_string(),
            ..TeamsConfig::default()
        };
        let result = TeamsAdapter::new(config, Arc::new(NoopHandler));
        assert!(result.is_err());
    }

    #[test]
    fn teams_adapter_rejects_empty_client_secret() {
        let config = TeamsConfig {
            tenant_id: "tenant-123".to_string(),
            client_id: "client-456".to_string(),
            client_secret: "".to_string(),
            ..TeamsConfig::default()
        };
        let result = TeamsAdapter::new(config, Arc::new(NoopHandler));
        assert!(result.is_err());
    }

    #[test]
    fn teams_adapter_accepts_valid_config() {
        let config = TeamsConfig {
            tenant_id: "tenant-123".to_string(),
            client_id: "client-456".to_string(),
            client_secret: "secret-789".to_string(),
            bot_id: "bot-001".to_string(),
            ..TeamsConfig::default()
        };
        let result = TeamsAdapter::new(config, Arc::new(NoopHandler));
        assert!(result.is_ok());
    }
}
