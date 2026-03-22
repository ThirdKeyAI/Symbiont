//! Session state types for interactive CLI tool sessions.

use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Unique session identifier.
pub type SessionId = String;

/// Session lifecycle status.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionStatus {
    Spawning,
    Ready,
    Busy,
    TimedOut,
    Terminated,
}

/// Current state of a session.
#[derive(Debug, Clone)]
pub struct SessionState {
    pub status: SessionStatus,
    pub prompt: String,
    pub inferred_state: String,
    pub interaction_count: u32,
    pub started_at: Instant,
    pub last_interaction_at: Instant,
    pub session_id: SessionId,
}

/// Direction of a transcript entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TranscriptDirection {
    Command,
    Response,
    PolicyDenied,
    System,
}

/// A single entry in the session transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptEntry {
    pub timestamp_ms: u64,
    pub direction: TranscriptDirection,
    pub content: String,
    pub command_name: Option<String>,
    pub duration_ms: Option<u64>,
}

/// Full session transcript for evidence.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionTranscript {
    pub entries: Vec<TranscriptEntry>,
}

impl SessionTranscript {
    pub fn append(
        &mut self,
        direction: TranscriptDirection,
        content: &str,
        command_name: Option<&str>,
    ) {
        self.entries.push(TranscriptEntry {
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            direction,
            content: content.to_string(),
            command_name: command_name.map(String::from),
            duration_ms: None,
        });
    }
}
