//! WebSocket handler for the Coordinator Chat.
//!
//! Provides the Axum WebSocket upgrade endpoint at `GET /ws/chat`.
//! Authentication uses a `?token=` query parameter since the browser
//! WebSocket API cannot set custom headers.

#[cfg(feature = "http-api")]
use std::sync::Arc;

#[cfg(feature = "http-api")]
use axum::{
    extract::{
        ws::{Message, WebSocket},
        Query, State, WebSocketUpgrade,
    },
    http::StatusCode,
    response::IntoResponse,
};

#[cfg(feature = "http-api")]
use serde::Deserialize;

#[cfg(feature = "http-api")]
use subtle::ConstantTimeEq;

#[cfg(feature = "http-api")]
use tokio::sync::mpsc;

#[cfg(feature = "http-api")]
use super::coordinator::{CoordinatorSession, CoordinatorState};
#[cfg(feature = "http-api")]
use super::ws_types::{ClientMessage, ServerMessage};

/// Query parameters for the WebSocket endpoint.
#[cfg(feature = "http-api")]
#[derive(Debug, Deserialize)]
pub struct WsChatParams {
    token: Option<String>,
}

/// Validate a bearer token against the API key store or legacy env var.
///
/// Mirrors the logic in `auth_middleware` but works with a raw token string
/// instead of HTTP headers. Returns `true` if the token is valid.
#[cfg(feature = "http-api")]
fn validate_token(token: &str, key_store: Option<&Arc<super::api_keys::ApiKeyStore>>) -> bool {
    // Primary: API key store
    if let Some(store) = key_store {
        if store.has_records() {
            return store.validate_key(token).is_some();
        }
    }

    // Fallback: legacy env var
    match std::env::var("SYMBIONT_API_TOKEN") {
        Ok(expected) => bool::from(token.as_bytes().ct_eq(expected.as_bytes())),
        Err(_) => false,
    }
}

/// Axum handler for `GET /ws/chat`.
///
/// Validates the bearer token from query params, then upgrades to WebSocket.
#[cfg(feature = "http-api")]
pub async fn ws_chat_handler(
    ws: WebSocketUpgrade,
    State(coordinator_state): State<Arc<CoordinatorState>>,
    Query(params): Query<WsChatParams>,
    key_store: Option<axum::Extension<Arc<super::api_keys::ApiKeyStore>>>,
) -> Result<impl IntoResponse, StatusCode> {
    // Validate token from query params
    let token = params.token.as_deref().ok_or(StatusCode::UNAUTHORIZED)?;
    let store_ref = key_store.as_ref().map(|ext| &ext.0);

    if !validate_token(token, store_ref) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(ws.on_upgrade(move |socket| handle_socket(socket, coordinator_state)))
}

/// Drive a single WebSocket connection.
#[cfg(feature = "http-api")]
async fn handle_socket(socket: WebSocket, state: Arc<CoordinatorState>) {
    let (mut ws_writer, mut ws_reader) = socket.split();

    // Channel for outbound messages (session → WebSocket writer)
    let (out_tx, mut out_rx) = mpsc::channel::<ServerMessage>(64);

    // Create per-connection session
    let mut session = CoordinatorSession::new(state, out_tx.clone());

    // Writer task: forward ServerMessages to the WebSocket
    use axum::extract::ws::Message as WsMessage;
    use futures::SinkExt;

    let writer_handle = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            match serde_json::to_string(&msg) {
                Ok(json) => {
                    if ws_writer.send(WsMessage::Text(json)).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to serialize ServerMessage: {}", e);
                }
            }
        }
    });

    // Heartbeat: server ping every 30s
    let heartbeat_tx = out_tx.clone();
    let heartbeat_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            if heartbeat_tx.send(ServerMessage::Pong).await.is_err() {
                break;
            }
        }
    });

    // Reader loop: process incoming client messages
    use futures::StreamExt;

    while let Some(msg) = ws_reader.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!("WebSocket read error: {}", e);
                break;
            }
        };

        match msg {
            Message::Text(text) => match serde_json::from_str::<ClientMessage>(&text) {
                Ok(ClientMessage::ChatSend { content, .. }) => {
                    session.handle_chat(content).await;
                }
                Ok(ClientMessage::Ping) => {
                    let _ = out_tx.send(ServerMessage::Pong).await;
                }
                Err(e) => {
                    let _ = out_tx
                        .send(ServerMessage::Error {
                            request_id: None,
                            code: "PARSE_ERROR".into(),
                            message: format!("Invalid message: {}", e),
                        })
                        .await;
                }
            },
            Message::Close(_) => break,
            _ => {} // Ignore binary, ping, pong frames
        }
    }

    // Clean up
    heartbeat_handle.abort();
    drop(out_tx);
    let _ = writer_handle.await;

    tracing::info!("WebSocket connection closed");
}
