//! WebSocket message types for the Coordinator Chat protocol.
//!
//! Defines the client→server and server→client message envelopes used over
//! the `/ws/chat` WebSocket connection. Each `ChatSend` from the client
//! triggers a stream of `ServerMessage` variants correlated by `request_id`.

#[cfg(feature = "http-api")]
use serde::{Deserialize, Serialize};

/// Messages sent from the browser client to the server.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    /// Send a chat message to the coordinator.
    ChatSend {
        /// Client-generated message id (for dedup / optimistic UI).
        id: String,
        /// Natural-language content.
        content: String,
    },
    /// Client ping (keepalive).
    Ping,
}

/// Messages sent from the server to the browser client.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    /// A chunk of the assistant's streaming response.
    ChatChunk {
        /// Server-generated UUID correlating all events for one user message.
        request_id: String,
        /// Chunk content (may be empty on the final `done` message).
        content: String,
        /// `true` on the last chunk — signals the response is complete.
        done: bool,
    },
    /// A tool call has started executing.
    ToolCallStarted {
        request_id: String,
        call_id: String,
        tool_name: String,
        arguments: String,
    },
    /// A tool call has completed.
    ToolCallResult {
        request_id: String,
        call_id: String,
        result: String,
        is_error: bool,
    },
    /// A policy decision was made for an action.
    PolicyDecision {
        request_id: String,
        action: String,
        decision: String,
        reason: String,
    },
    /// An error occurred processing the request.
    Error {
        request_id: Option<String>,
        code: String,
        message: String,
    },
    /// Server pong (keepalive response).
    Pong,
}
