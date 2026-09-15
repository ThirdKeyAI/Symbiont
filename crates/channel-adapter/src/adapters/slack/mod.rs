//! Slack channel adapter — bidirectional Slack integration.
//!
//! Receives messages via Events API webhooks and slash commands,
//! sends responses via Slack Web API (`chat.postMessage`).

pub mod api;
pub mod events;
pub mod oauth;
pub mod signature;

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use tokio::sync::{watch, RwLock};

use crate::config::SlackConfig;
use crate::error::ChannelAdapterError;
use crate::traits::{ChannelAdapter, InboundHandler};
use crate::types::{AdapterHealth, ChatDeliveryReceipt, ChatPlatform, OutboundMessage};

use api::SlackApiClient;

/// Shared state for Axum webhook handlers.
struct SlackAdapterState {
    #[allow(dead_code)] // Used when response routing is wired up
    api_client: SlackApiClient,
    config: SlackConfig,
    handler: Arc<dyn InboundHandler>,
    last_message_at: RwLock<Option<chrono::DateTime<chrono::Utc>>>,
}

/// Bidirectional Slack adapter.
///
/// Starts an Axum HTTP server to receive Events API webhooks and slash commands,
/// and uses the Slack Web API to send responses.
pub struct SlackAdapter {
    config: SlackConfig,
    api_client: SlackApiClient,
    handler: Arc<dyn InboundHandler>,
    shutdown_tx: RwLock<Option<watch::Sender<()>>>,
    started_at: RwLock<Option<std::time::Instant>>,
    workspace_name: RwLock<Option<String>>,
    last_message_at: Arc<RwLock<Option<chrono::DateTime<chrono::Utc>>>>,
}

impl SlackAdapter {
    pub fn new(
        config: SlackConfig,
        handler: Arc<dyn InboundHandler>,
    ) -> Result<Self, ChannelAdapterError> {
        oauth::validate_token_format(&config.bot_token)?;

        // Every callback can carry an operator command. Deployment labels and
        // environment opt-outs cannot replace authenticated sender evidence.
        if config
            .signing_secret
            .as_ref()
            .is_none_or(|secret| secret.trim().is_empty())
        {
            return Err(ChannelAdapterError::Config(
                "Slack signing_secret is required for authenticated callbacks".into(),
            ));
        }

        let api_client = SlackApiClient::new(&config.bot_token)?;

        Ok(Self {
            config,
            api_client,
            handler,
            shutdown_tx: RwLock::new(None),
            started_at: RwLock::new(None),
            workspace_name: RwLock::new(None),
            last_message_at: Arc::new(RwLock::new(None)),
        })
    }
}

#[async_trait]
impl ChannelAdapter for SlackAdapter {
    async fn start(&self) -> Result<(), ChannelAdapterError> {
        if self.shutdown_tx.read().await.is_some() {
            return Err(ChannelAdapterError::AlreadyRunning);
        }

        // Verify bot token
        let auth = self.api_client.auth_test().await?;
        *self.workspace_name.write().await = auth.team.clone();

        tracing::info!(
            workspace = auth.team.as_deref().unwrap_or("unknown"),
            bot_user = auth.user.as_deref().unwrap_or("unknown"),
            "Slack adapter connected"
        );

        // Build Axum router for webhook receiver
        let state = Arc::new(SlackAdapterState {
            api_client: self.api_client.clone(),
            config: self.config.clone(),
            handler: self.handler.clone(),
            last_message_at: RwLock::new(None),
        });

        let app = callback_router(state.clone());

        let addr = format!("{}:{}", self.config.bind_address, self.config.webhook_port);
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .map_err(|e| ChannelAdapterError::Connection(format!("bind failed: {}", e)))?;

        let (shutdown_tx, mut shutdown_rx) = watch::channel(());
        *self.shutdown_tx.write().await = Some(shutdown_tx);
        *self.started_at.write().await = Some(std::time::Instant::now());

        let last_message = self.last_message_at.clone();

        tokio::spawn(async move {
            tracing::info!(addr = %addr, "Slack webhook server listening");
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.changed().await;
                })
                .await
                .unwrap_or_else(|e| tracing::error!("Slack webhook server error: {}", e));
            // Propagate last_message_at from state to adapter
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
                tracing::info!("Slack adapter stopped");
                Ok(())
            }
            None => Err(ChannelAdapterError::NotRunning),
        }
    }

    fn prepare_response(
        &self,
        response: &OutboundMessage,
    ) -> Result<serde_json::Value, ChannelAdapterError> {
        Ok(serde_json::json!(self.api_client.prepare_post(response)?))
    }

    async fn send_response(
        &self,
        response: OutboundMessage,
    ) -> Result<ChatDeliveryReceipt, ChannelAdapterError> {
        self.api_client.post_message(&response).await
    }

    fn platform(&self) -> ChatPlatform {
        ChatPlatform::Slack
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
            platform: ChatPlatform::Slack,
            workspace_name: self.workspace_name.read().await.clone(),
            channels_active: self.config.channels.len(),
            last_message_at: *self.last_message_at.read().await,
            uptime_secs: uptime,
        })
    }
}

fn callback_router(state: Arc<SlackAdapterState>) -> Router {
    Router::new()
        .route("/slack/events", post(handle_slack_event))
        .route("/slack/commands", post(handle_slash_command))
        .route("/health", axum::routing::get(health_check))
        .with_state(state)
}

/// Axum handler for Slack Events API.
async fn handle_slack_event(
    State(state): State<Arc<SlackAdapterState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let Some(secret) = state
        .config
        .signing_secret
        .as_deref()
        .filter(|secret| !secret.trim().is_empty())
    else {
        return (
            StatusCode::UNAUTHORIZED,
            "signing secret required".to_string(),
        );
    };
    {
        let timestamp = headers
            .get("x-slack-request-timestamp")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let sig = headers
            .get("x-slack-signature")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if let Err(e) = signature::verify_slack_signature(secret, timestamp, &body, sig) {
            tracing::warn!("Slack signature verification failed: {}", e);
            return (StatusCode::UNAUTHORIZED, "invalid signature".to_string());
        }
    }

    // Parse the envelope
    let envelope: events::SlackEnvelope = match serde_json::from_slice(&body) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("Failed to parse Slack event: {}", e);
            return (StatusCode::BAD_REQUEST, format!("parse error: {}", e));
        }
    };

    // Handle URL verification challenge
    if let events::SlackEnvelope::UrlVerification { ref challenge } = envelope {
        return (StatusCode::OK, challenge.clone());
    }

    // Parse into inbound message and dispatch
    match events::parse_event_to_message(&envelope) {
        Ok(Some(msg)) => {
            *state.last_message_at.write().await = Some(chrono::Utc::now());
            let handler = state.handler.clone();
            // Handle asynchronously to avoid blocking the webhook response
            tokio::spawn(async move {
                if let Err(e) = handler.handle_message(msg).await {
                    tracing::error!("Failed to handle inbound message: {}", e);
                }
            });
            (StatusCode::OK, String::new())
        }
        Ok(None) => (StatusCode::OK, String::new()),
        Err(e) => {
            tracing::warn!("Event parse error: {}", e);
            (StatusCode::OK, String::new()) // Slack expects 200 even on errors
        }
    }
}

/// Axum handler for Slack slash commands (URL-encoded form data).
async fn handle_slash_command(
    State(state): State<Arc<SlackAdapterState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let Some(secret) = state
        .config
        .signing_secret
        .as_deref()
        .filter(|secret| !secret.trim().is_empty())
    else {
        return (
            StatusCode::UNAUTHORIZED,
            "signing secret required".to_string(),
        );
    };
    {
        let timestamp = headers
            .get("x-slack-request-timestamp")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let sig = headers
            .get("x-slack-signature")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if let Err(e) = signature::verify_slack_signature(secret, timestamp, &body, sig) {
            tracing::warn!("Slash command signature verification failed: {}", e);
            return (StatusCode::UNAUTHORIZED, "invalid signature".to_string());
        }
    }

    // Parse URL-encoded form data
    let cmd: events::SlackSlashCommand = match serde_urlencoded::from_bytes(&body) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to parse slash command: {}", e);
            return (StatusCode::BAD_REQUEST, format!("parse error: {}", e));
        }
    };

    let msg = events::parse_slash_command(&cmd);
    *state.last_message_at.write().await = Some(chrono::Utc::now());

    let handler = state.handler.clone();
    tokio::spawn(async move {
        if let Err(e) = handler.handle_message(msg).await {
            tracing::error!("Failed to handle slash command: {}", e);
        }
    });

    (StatusCode::OK, "Processing...".to_string())
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
    fn slack_adapter_rejects_empty_token() {
        let config = SlackConfig {
            bot_token: "".to_string(),
            ..SlackConfig::default()
        };
        let result = SlackAdapter::new(config, Arc::new(NoopHandler));
        assert!(result.is_err());
    }

    #[test]
    fn slack_adapter_rejects_invalid_token() {
        let config = SlackConfig {
            bot_token: "xoxp-user-token-not-bot".to_string(),
            ..SlackConfig::default()
        };
        let result = SlackAdapter::new(config, Arc::new(NoopHandler));
        assert!(result.is_err());
    }

    #[test]
    fn slack_adapter_accepts_valid_token() {
        let config = SlackConfig {
            bot_token: format!(
                "xoxb-{}-{}-{}",
                "0000000000000", "0000000000000", "fakefakefakefakefakefake"
            ),
            signing_secret: Some("synthetic-signing-secret".into()),
            ..SlackConfig::default()
        };
        let result = SlackAdapter::new(config, Arc::new(NoopHandler));
        assert!(result.is_ok());
    }
    fn callback_config() -> SlackConfig {
        SlackConfig {
            // Assembled rather than written as a literal so it cannot be
            // mistaken for a real credential by secret scanners. The adapter
            // only requires the xoxb- prefix and a minimum length.
            bot_token: format!(
                "xoxb-{}-{}-{}",
                "000000000000", "000000000000", "fakefakefakefakefakefake"
            ),
            signing_secret: Some("synthetic-callback-secret".into()),
            ..Default::default()
        }
    }

    #[test]
    fn slack_callbacks_require_a_nonempty_signing_secret() {
        for secret in [None, Some(String::new()), Some("   ".into())] {
            let mut config = callback_config();
            config.signing_secret = secret;
            assert!(SlackAdapter::new(config, Arc::new(NoopHandler)).is_err());
        }
    }

    #[tokio::test]
    async fn callback_http_authenticates_before_forwarding_control_commands() {
        use hmac::{Hmac, Mac};
        struct Receiver(tokio::sync::mpsc::UnboundedSender<crate::types::InboundMessage>);
        #[async_trait]
        impl InboundHandler for Receiver {
            async fn handle_message(
                &self,
                message: crate::types::InboundMessage,
            ) -> Result<(), ChannelAdapterError> {
                self.0.send(message).unwrap();
                Ok(())
            }
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap();
        for configured in [false, true] {
            let mut config = callback_config();
            let secret = config.signing_secret.clone().unwrap();
            if !configured {
                config.signing_secret = None;
            }
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let state = Arc::new(SlackAdapterState {
                api_client: SlackApiClient::new(&config.bot_token).unwrap(),
                config,
                handler: Arc::new(Receiver(tx)),
                last_message_at: RwLock::new(None),
            });
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let router = callback_router(state);
            let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                axum::serve(listener, router)
                    .with_graceful_shutdown(async {
                        let _ = stop_rx.await;
                    })
                    .await
                    .unwrap();
            });
            let command = events::SlackSlashCommand {
                command: "/symbi".into(),
                text: Some("gate show 0123456789abcdef".into()),
                user_id: "operator".into(),
                user_name: "Operator".into(),
                channel_id: "approvers".into(),
                channel_name: None,
                team_id: "fixture-workspace".into(),
                team_domain: None,
                response_url: "https://example.invalid/unused".into(),
                trigger_id: "fixture".into(),
            };
            let event = serde_json::json!({"type":"event_callback","team_id":"fixture-workspace","event_id":"fixture",
                "event":{"type":"message","channel":"approvers","user":"operator","text":"/symbi gate show 0123456789abcdef","ts":"1"}}).to_string();
            for (route, body) in [
                (
                    "/slack/commands",
                    serde_urlencoded::to_string(command).unwrap(),
                ),
                ("/slack/events", event),
            ] {
                for valid in [false, true] {
                    let timestamp = chrono::Utc::now().timestamp().to_string();
                    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).unwrap();
                    mac.update(format!("v0:{timestamp}:").as_bytes());
                    mac.update(body.as_bytes());
                    let signature = format!("v0={}", hex::encode(mac.finalize().into_bytes()));
                    let mut request = client
                        .post(format!("http://{address}{route}"))
                        .body(body.clone());
                    if valid {
                        request = request
                            .header("x-slack-request-timestamp", timestamp)
                            .header("x-slack-signature", signature);
                    }
                    let response = request.send().await.unwrap();
                    assert_eq!(
                        response.status().as_u16(),
                        if configured && valid { 200 } else { 401 }
                    );
                    if configured && valid {
                        let message =
                            tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
                                .await
                                .unwrap()
                                .unwrap();
                        assert_eq!(message.sender_id, "operator");
                        if route == "/slack/commands" {
                            let command = message.command.unwrap();
                            assert_eq!(command.name, "symbi");
                            assert_eq!(command.subcommand.as_deref(), Some("gate"));
                            assert_eq!(command.args, vec!["show", "0123456789abcdef"]);
                        }
                    } else {
                        assert!(rx.try_recv().is_err());
                    }
                }
            }
            stop_tx.send(()).unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(3), server)
                .await
                .unwrap()
                .unwrap();
            assert!(tokio::net::TcpStream::connect(address).await.is_err());
        }
    }
}
