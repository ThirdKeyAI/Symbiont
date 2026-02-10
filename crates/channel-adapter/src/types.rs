use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Supported chat platforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatPlatform {
    Slack,
    #[cfg(feature = "teams")]
    Teams,
    #[cfg(feature = "mattermost")]
    Mattermost,
}

impl std::fmt::Display for ChatPlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChatPlatform::Slack => write!(f, "slack"),
            #[cfg(feature = "teams")]
            ChatPlatform::Teams => write!(f, "teams"),
            #[cfg(feature = "mattermost")]
            ChatPlatform::Mattermost => write!(f, "mattermost"),
        }
    }
}

/// A message received from a chat platform, normalized across platforms.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMessage {
    pub id: String,
    pub platform: ChatPlatform,
    pub workspace_id: String,
    pub channel_id: String,
    pub thread_id: Option<String>,
    pub sender_id: String,
    pub sender_name: String,
    pub content: String,
    pub command: Option<SlashCommand>,
    pub timestamp: DateTime<Utc>,
    pub raw_payload: Option<serde_json::Value>,
}

/// A parsed slash command from a chat message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlashCommand {
    pub name: String,
    pub subcommand: Option<String>,
    pub args: Vec<String>,
    pub agent_name: Option<String>,
}

/// A response to send back to a chat platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundMessage {
    pub channel_id: String,
    pub thread_id: Option<String>,
    pub content: String,
    pub blocks: Option<serde_json::Value>,
    pub ephemeral: bool,
    pub user_id: Option<String>,
    /// Platform-specific routing metadata (e.g. Teams service_url).
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Receipt confirming a message was delivered to a chat platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatDeliveryReceipt {
    pub platform: ChatPlatform,
    pub channel_id: String,
    pub message_ts: Option<String>,
    pub delivered_at: DateTime<Utc>,
    pub success: bool,
    pub error: Option<String>,
}

/// Health status of a channel adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterHealth {
    pub connected: bool,
    pub platform: ChatPlatform,
    pub workspace_name: Option<String>,
    pub channels_active: usize,
    pub last_message_at: Option<DateTime<Utc>>,
    pub uptime_secs: u64,
}

/// A structured interaction log entry (community edition — non-cryptographic).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionLog {
    pub ts: DateTime<Utc>,
    pub platform: ChatPlatform,
    pub user: String,
    pub channel: String,
    pub agent: String,
    pub action: InteractionAction,
    pub success: bool,
    pub duration_ms: Option<u64>,
    pub error: Option<String>,
}

/// Types of interaction actions logged.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionAction {
    Invoke,
    Response,
    SlashCommand,
    Error,
}

/// Policy decision from enterprise hooks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PolicyDecision {
    Allow,
    Deny { reason: String },
    RequireApproval { approvers: Vec<String> },
}

/// Content after DLP filtering from enterprise hooks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilteredContent {
    pub content: String,
    pub redacted: bool,
    pub redaction_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_platform_display() {
        assert_eq!(ChatPlatform::Slack.to_string(), "slack");
    }

    #[test]
    fn chat_platform_serialization() {
        let json = serde_json::to_string(&ChatPlatform::Slack).unwrap();
        assert_eq!(json, "\"slack\"");
        let parsed: ChatPlatform = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ChatPlatform::Slack);
    }

    #[test]
    fn inbound_message_serialization() {
        let msg = InboundMessage {
            id: "msg-1".to_string(),
            platform: ChatPlatform::Slack,
            workspace_id: "T123".to_string(),
            channel_id: "C456".to_string(),
            thread_id: None,
            sender_id: "U789".to_string(),
            sender_name: "alice".to_string(),
            content: "hello agent".to_string(),
            command: None,
            timestamp: Utc::now(),
            raw_payload: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: InboundMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "msg-1");
        assert_eq!(parsed.sender_name, "alice");
    }

    #[test]
    fn outbound_message_serialization() {
        let msg = OutboundMessage {
            channel_id: "C456".to_string(),
            thread_id: Some("1234567890.123456".to_string()),
            content: "agent response".to_string(),
            blocks: None,
            ephemeral: false,
            user_id: None,
            metadata: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: OutboundMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.thread_id.as_deref(), Some("1234567890.123456"));
    }

    #[test]
    fn interaction_log_serialization() {
        let log = InteractionLog {
            ts: Utc::now(),
            platform: ChatPlatform::Slack,
            user: "U123".to_string(),
            channel: "#general".to_string(),
            agent: "my-helper".to_string(),
            action: InteractionAction::Invoke,
            success: true,
            duration_ms: Some(150),
            error: None,
        };
        let json = serde_json::to_string(&log).unwrap();
        assert!(json.contains("\"action\":\"invoke\""));
    }
}
