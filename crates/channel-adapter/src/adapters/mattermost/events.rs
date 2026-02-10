//! Mattermost outgoing webhook payload parsing.
//!
//! Parses inbound Mattermost outgoing webhook payloads into normalized `InboundMessage`.

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::error::ChannelAdapterError;
use crate::types::{ChatPlatform, InboundMessage, SlashCommand};

/// Mattermost outgoing webhook payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutgoingWebhookPayload {
    /// Webhook token for verification.
    pub token: Option<String>,
    /// Team ID.
    pub team_id: Option<String>,
    /// Team domain name.
    pub team_domain: Option<String>,
    /// Channel ID.
    pub channel_id: String,
    /// Channel name.
    pub channel_name: Option<String>,
    /// Timestamp of the message.
    pub timestamp: Option<i64>,
    /// User ID of the sender.
    pub user_id: String,
    /// Username of the sender.
    pub user_name: Option<String>,
    /// Post ID.
    pub post_id: Option<String>,
    /// Message text.
    pub text: Option<String>,
    /// Trigger word that caused this webhook to fire.
    pub trigger_word: Option<String>,
    /// File IDs attached to the message.
    pub file_ids: Option<String>,
}

/// Parse a Mattermost outgoing webhook payload into a normalized `InboundMessage`.
pub fn parse_webhook_to_message(
    payload: &OutgoingWebhookPayload,
) -> Result<InboundMessage, ChannelAdapterError> {
    let text = payload.text.as_deref().unwrap_or("");

    // Strip trigger word from the beginning if present
    let clean_text = if let Some(ref trigger) = payload.trigger_word {
        text.strip_prefix(trigger.as_str())
            .unwrap_or(text)
            .trim()
            .to_string()
    } else {
        text.trim().to_string()
    };

    let (agent_name, content) = extract_agent_mention(&clean_text);

    let id = payload
        .post_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    Ok(InboundMessage {
        id,
        platform: ChatPlatform::Mattermost,
        workspace_id: payload.team_id.clone().unwrap_or_default(),
        channel_id: payload.channel_id.clone(),
        thread_id: None, // Mattermost outgoing webhooks don't include thread info
        sender_id: payload.user_id.clone(),
        sender_name: payload
            .user_name
            .clone()
            .unwrap_or_else(|| payload.user_id.clone()),
        content,
        command: agent_name.map(|name| SlashCommand {
            name: "invoke".to_string(),
            subcommand: None,
            args: vec![],
            agent_name: Some(name),
        }),
        timestamp: Utc::now(),
        raw_payload: serde_json::to_value(payload).ok(),
    })
}

/// Extract agent name from "run <agent-name> <input>" pattern.
///
/// Returns `(Some(agent_name), cleaned_text)` if found, else `(None, original)`.
fn extract_agent_mention(text: &str) -> (Option<String>, String) {
    let trimmed = text.trim();

    if let Some(rest) = trimmed.strip_prefix("run ") {
        let mut parts = rest.splitn(2, ' ');
        if let Some(agent) = parts.next() {
            let input = parts.next().unwrap_or("").to_string();
            return (Some(agent.to_string()), input);
        }
    }

    (None, trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_webhook() {
        let payload = OutgoingWebhookPayload {
            token: Some("test-token".to_string()),
            team_id: Some("team-123".to_string()),
            team_domain: Some("acme".to_string()),
            channel_id: "ch-456".to_string(),
            channel_name: Some("general".to_string()),
            timestamp: Some(1700000000),
            user_id: "user-789".to_string(),
            user_name: Some("alice".to_string()),
            post_id: Some("post-001".to_string()),
            text: Some("run compliance-check on us-east-1".to_string()),
            trigger_word: None,
            file_ids: None,
        };

        let msg = parse_webhook_to_message(&payload).unwrap();
        assert_eq!(msg.platform, ChatPlatform::Mattermost);
        assert_eq!(msg.sender_name, "alice");
        assert_eq!(msg.content, "on us-east-1");
        let cmd = msg.command.unwrap();
        assert_eq!(cmd.agent_name.as_deref(), Some("compliance-check"));
    }

    #[test]
    fn parse_webhook_with_trigger_word() {
        let payload = OutgoingWebhookPayload {
            token: Some("test-token".to_string()),
            team_id: None,
            team_domain: None,
            channel_id: "ch-1".to_string(),
            channel_name: None,
            timestamp: None,
            user_id: "user-1".to_string(),
            user_name: Some("bob".to_string()),
            post_id: None,
            text: Some("@symbi run my-agent check all".to_string()),
            trigger_word: Some("@symbi".to_string()),
            file_ids: None,
        };

        let msg = parse_webhook_to_message(&payload).unwrap();
        assert_eq!(msg.content, "check all");
        let cmd = msg.command.unwrap();
        assert_eq!(cmd.agent_name.as_deref(), Some("my-agent"));
    }

    #[test]
    fn parse_webhook_no_run_prefix() {
        let payload = OutgoingWebhookPayload {
            token: None,
            team_id: None,
            team_domain: None,
            channel_id: "ch-1".to_string(),
            channel_name: None,
            timestamp: None,
            user_id: "user-1".to_string(),
            user_name: None,
            post_id: None,
            text: Some("hello world".to_string()),
            trigger_word: None,
            file_ids: None,
        };

        let msg = parse_webhook_to_message(&payload).unwrap();
        assert_eq!(msg.content, "hello world");
        assert!(msg.command.is_none());
    }

    #[test]
    fn payload_deserialization() {
        let json = r#"{
            "token": "tok-123",
            "team_id": "team-1",
            "channel_id": "ch-1",
            "user_id": "user-1",
            "user_name": "alice",
            "text": "hello"
        }"#;
        let payload: OutgoingWebhookPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.token.as_deref(), Some("tok-123"));
        assert_eq!(payload.user_id, "user-1");
    }

    #[test]
    fn extract_agent_from_run_prefix() {
        let (agent, rest) = extract_agent_mention("run my-agent check status");
        assert_eq!(agent.as_deref(), Some("my-agent"));
        assert_eq!(rest, "check status");
    }

    #[test]
    fn extract_agent_no_prefix() {
        let (agent, rest) = extract_agent_mention("just a message");
        assert!(agent.is_none());
        assert_eq!(rest, "just a message");
    }
}
