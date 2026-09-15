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
/// instead of HTTP headers. Retains the validated caller for durable admission.
#[cfg(feature = "http-api")]
fn validate_token(
    token: &str,
    key_store: Option<&Arc<super::api_keys::ApiKeyStore>>,
) -> Option<super::invocations::AuthenticatedCaller> {
    if token.trim().is_empty() || token.len() > 8192 {
        return None;
    }
    // Primary: API key store
    if let Some(store) = key_store {
        if store.has_records() {
            // Coordinator tools inspect the whole fleet. A key limited to
            // particular agents cannot grant this operator-level capability.
            return store
                .validate_key(token)
                .filter(|key| key.agent_scope.is_none())
                .map(|key| super::invocations::AuthenticatedCaller::verified(token, Some(&key)));
        }
    }

    // Fallback: legacy env var
    match std::env::var("SYMBIONT_API_TOKEN") {
        Ok(expected) => bool::from(token.as_bytes().ct_eq(expected.as_bytes()))
            .then(|| super::invocations::AuthenticatedCaller::verified(token, None)),
        Err(_) => None,
    }
}

/// Axum handler for `GET /ws/chat`.
///
/// Validates the bearer token from query params, then upgrades to WebSocket.
//
// SECURITY: the WebSocket upgrade carries the auth token as a query
// parameter rather than an `Authorization` header. This is the
// browser-WebSocket-API limitation — `new WebSocket(url)` does not
// allow custom headers, so query-param auth is the only practical
// path. The tradeoff:
//
//   - Pro: works from every browser, no two-step handshake required.
//   - Con: the URL (including the token) appears in access logs,
//     proxy logs, and browser history. Mitigations in place:
//       * the token is matched against the Argon2-hashed
//         `ApiKeyStore` (`validate_token` → `store.validate_key`)
//         using constant-time comparison (`subtle::ConstantTimeEq`
//         on the legacy fallback path);
//       * the WebSocket upgrade itself replaces the HTTP request,
//         so the token-bearing URL is not re-sent on every frame.
//
// TODO: implement two-step handshake — `POST /api/v1/auth/ws-session`
// returns a short-lived (single-use, <60s) session ID; the client
// then opens `/ws/chat?session=<id>`. That keeps the long-lived API
// token out of every log line while preserving browser
// compatibility. See SECURITY_AUDIT.md L2.
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

    let caller = validate_token(token, store_ref).ok_or(StatusCode::UNAUTHORIZED)?;

    Ok(ws
        .max_message_size(128 * 1024)
        .max_frame_size(128 * 1024)
        .on_upgrade(move |socket| handle_socket(socket, coordinator_state, caller)))
}

/// Drive a single WebSocket connection.
#[cfg(all(feature = "http-api", unix))]
async fn handle_socket(
    socket: WebSocket,
    state: Arc<CoordinatorState>,
    caller: super::invocations::AuthenticatedCaller,
) {
    use super::chat_invocations::{self, AdmittedChat};
    use crate::reasoning::invocation::OpenInvocation;
    use futures::{SinkExt, StreamExt};
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    let cancellation = CancellationToken::new();
    let _cancel_on_drop = cancellation.clone().drop_guard();
    let (mut ws_writer, mut ws_reader) = socket.split();
    let (out_tx, mut out_rx) = mpsc::channel::<ServerMessage>(64);
    // One active turn and one queued message per connection. The socket reader
    // remains live during inference so disconnect immediately cancels the run.
    let (in_tx, mut in_rx) = mpsc::channel::<AdmittedChat>(1);
    let actor_cancel = cancellation.clone();
    let actor_tx = out_tx.clone();
    let actor_state = state.clone();
    let actor = tokio::spawn(async move {
        let mut session = CoordinatorSession::new(actor_state, actor_tx);
        loop {
            let content = tokio::select! {
                biased;
                _ = actor_cancel.cancelled() => break,
                content = in_rx.recv() => match content { Some(content) => content, None => break },
            };
            session
                .handle_admitted_cancellable(content, actor_cancel.child_token())
                .await;
        }
    });

    let writer_cancel = cancellation.clone();
    let writer = tokio::spawn(async move {
        loop {
            let msg = tokio::select! {
                biased;
                _ = writer_cancel.cancelled() => break,
                msg = out_rx.recv() => match msg { Some(msg) => msg, None => break },
            };
            let Ok(json) = serde_json::to_string(&msg) else {
                break;
            };
            let sent = tokio::select! {
                biased;
                _ = writer_cancel.cancelled() => break,
                sent = tokio::time::timeout(Duration::from_secs(5), ws_writer.send(Message::Text(json))) => sent,
            };
            if !matches!(sent, Ok(Ok(()))) {
                break;
            }
        }
        writer_cancel.cancel();
    });

    let heartbeat_cancel = cancellation.clone();
    let heartbeat_tx = out_tx.clone();
    let heartbeat = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            tokio::select! {
                biased;
                _ = heartbeat_cancel.cancelled() => break,
                _ = interval.tick() => { let _ = heartbeat_tx.try_send(ServerMessage::Pong); }
            }
        }
    });

    loop {
        let next = tokio::select! {
            biased;
            _ = cancellation.cancelled() => break,
            next = ws_reader.next() => next,
        };
        let Some(Ok(msg)) = next else { break };
        match msg {
            Message::Text(text) => {
                match serde_json::from_str::<ClientMessage>(&text) {
                    Ok(
                        message @ (ClientMessage::ChatSend { .. }
                        | ClientMessage::ChatInspect { .. }),
                    ) => {
                        let inspect_only = matches!(&message, ClientMessage::ChatInspect { .. });
                        let (id, content) = match message {
                            ClientMessage::ChatSend { id, content }
                            | ClientMessage::ChatInspect { id, content } => (id, content),
                            _ => unreachable!(),
                        };
                        if content.len() > 64 * 1024 {
                            let _ = out_tx.try_send(ServerMessage::Error {
                                request_id: None,
                                code: "MESSAGE_TOO_LARGE".into(),
                                message: "Chat message exceeds 64 KiB".into(),
                            });
                            continue;
                        }
                        let Some(id) = (id.len() == 36)
                            .then(|| Uuid::parse_str(&id).ok())
                            .flatten()
                            .filter(|id| !id.is_nil())
                        else {
                            let _ = out_tx.try_send(ServerMessage::Error {
                                request_id: None,
                                code: "INVALID_INVOCATION_ID".into(),
                                message:
                                    "Supply a non-nil UUID in ChatSend.id and retain it for retries"
                                        .into(),
                            });
                            continue;
                        };
                        // Lookup remains available even while the queue is full.
                        // No caller or content change can mint another claim at this ID.
                        match state.lookup_chat(&caller, id, &content).await {
                            Ok(Some(existing)) => {
                                if !chat_invocations::existing(&out_tx, id, existing, true).await {
                                    break;
                                }
                                continue;
                            }
                            Err(error) => {
                                if !chat_invocations::admission_error(&out_tx, id, &error).await {
                                    break;
                                }
                                continue;
                            }
                            Ok(None) if inspect_only => {
                                if !chat_invocations::error(&out_tx, id, "INVOCATION_NOT_FOUND", "No claim exists for this message; inspection did not submit work").await { break; }
                                continue;
                            }
                            Ok(None) => {}
                        }
                        let permit = match in_tx.try_reserve() {
                            Ok(permit) => permit,
                            Err(_) => {
                                if !chat_invocations::error(
                                    &out_tx,
                                    id,
                                    "SESSION_BUSY",
                                    "A chat message is already queued; this ID was not admitted",
                                )
                                .await
                                {
                                    break;
                                }
                                continue;
                            }
                        };
                        match state.admit_chat(&caller, id, &content).await {
                            Ok(OpenInvocation::Fresh(invocation)) => {
                                if !chat_invocations::send(
                                    &out_tx,
                                    ServerMessage::AuditOpened {
                                        request_id: id.to_string(),
                                        audit: invocation.audit().clone(),
                                    },
                                )
                                .await
                                {
                                    break;
                                }
                                // The queued request owns its durable claim. Disconnect
                                // drops it unresolved rather than making it executable again.
                                permit.send(AdmittedChat {
                                    id,
                                    content,
                                    invocation,
                                });
                            }
                            Ok(OpenInvocation::Existing(existing)) => {
                                if !chat_invocations::existing(&out_tx, id, existing, true).await {
                                    break;
                                }
                            }
                            Err(error) => {
                                if !chat_invocations::admission_error(&out_tx, id, &error).await {
                                    break;
                                }
                            }
                        }
                    }
                    Ok(ClientMessage::Ping) => {
                        let _ = out_tx.try_send(ServerMessage::Pong);
                    }
                    Err(_) => {
                        let _ = out_tx.try_send(ServerMessage::Error {
                            request_id: None,
                            code: "PARSE_ERROR".into(),
                            message: "Invalid chat message".into(),
                        });
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    cancellation.cancel();
    drop(in_tx);
    drop(out_tx);
    // The session owns its sender until the run's terminal audit is complete.
    // Join it before waiting for the writer; waiting on a live sender deadlocks.
    if let Err(error) = actor.await {
        tracing::warn!(%error, "Coordinator session owner failed");
    }
    let _ = heartbeat.await;
    let _ = writer.await;
    tracing::info!("WebSocket connection closed");
}

#[cfg(all(feature = "http-api", not(unix)))]
async fn handle_socket(
    mut socket: WebSocket,
    _state: Arc<CoordinatorState>,
    _caller: super::invocations::AuthenticatedCaller,
) {
    let message = ServerMessage::Error {
        request_id: None,
        code: "AUDIT_UNAVAILABLE".into(),
        message: "Durable chat admission is unavailable on this platform".into(),
    };
    if let Ok(json) = serde_json::to_string(&message) {
        let _ = socket.send(Message::Text(json)).await;
    }
    let _ = socket.close().await;
}
