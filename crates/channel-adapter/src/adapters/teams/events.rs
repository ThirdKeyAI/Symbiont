//! Bot Framework Activity parsing for Microsoft Teams.
//!
//! Parses inbound Bot Framework Activity JSON into normalized `InboundMessage`.

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::error::ChannelAdapterError;
use crate::types::{ChatPlatform, InboundMessage, SlashCommand};

/// Bot Framework Activity — the core message type from Teams.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    /// Activity type (e.g. "message", "conversationUpdate").
    #[serde(rename = "type")]
    pub activity_type: String,
    /// Unique activity ID.
    pub id: Option<String>,
    /// Timestamp of the activity.
    pub timestamp: Option<String>,
    /// Service URL for sending replies.
    pub service_url: Option<String>,
    /// Channel ID (e.g. "msteams").
    pub channel_id: Option<String>,
    /// Sender information.
    pub from: Option<ActivityParticipant>,
    /// Conversation information.
    pub conversation: Option<ActivityConversation>,
    /// Recipient (the bot).
    pub recipient: Option<ActivityParticipant>,
    /// Message text content.
    pub text: Option<String>,
    /// ID of the activity being replied to.
    pub reply_to_id: Option<String>,
}

/// A participant in a Bot Framework activity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityParticipant {
    /// Participant ID.
    pub id: String,
    /// Display name.
    pub name: Option<String>,
    /// AAD object ID (for Azure AD users).
    pub aad_object_id: Option<String>,
}

/// Conversation metadata in a Bot Framework activity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityConversation {
    /// Conversation ID.
    pub id: String,
    /// Conversation type (e.g. "personal", "groupChat", "channel").
    pub conversation_type: Option<String>,
    /// Tenant ID.
    pub tenant_id: Option<String>,
}

/// Parse a Bot Framework Activity into a normalized `InboundMessage`.
///
/// Returns `None` for non-message activities or self-messages (where from.id == bot_id).
pub fn parse_activity_to_message(
    activity: &Activity,
    bot_id: &str,
) -> Result<Option<InboundMessage>, ChannelAdapterError> {
    // Only process "message" activities
    if activity.activity_type != "message" {
        return Ok(None);
    }

    // Skip self-messages
    if let Some(ref from) = activity.from {
        if from.id == bot_id {
            return Ok(None);
        }
    }

    let text = activity.text.as_deref().unwrap_or("");

    // Strip bot mention from Teams messages (Teams adds "<at>BotName</at>" prefix)
    let clean_text = strip_teams_mention(text);
    let (agent_name, content) = extract_agent_mention(&clean_text);

    let from = activity.from.as_ref();
    let sender_id = from
        .map(|f| f.id.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let sender_name = from
        .and_then(|f| f.name.clone())
        .unwrap_or_else(|| sender_id.clone());

    let conversation_id = activity
        .conversation
        .as_ref()
        .map(|c| c.id.clone())
        .unwrap_or_default();

    let activity_id = activity
        .id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    Ok(Some(InboundMessage {
        id: activity_id,
        platform: ChatPlatform::Teams,
        workspace_id: activity
            .conversation
            .as_ref()
            .and_then(|c| c.tenant_id.clone())
            .unwrap_or_default(),
        channel_id: conversation_id,
        thread_id: activity.reply_to_id.clone(),
        sender_id,
        sender_name,
        content,
        command: agent_name.map(|name| SlashCommand {
            name: "invoke".to_string(),
            subcommand: None,
            args: vec![],
            agent_name: Some(name),
        }),
        timestamp: Utc::now(),
        raw_payload: serde_json::to_value(activity).ok(),
    }))
}

/// Strip Teams bot mention tags like `<at>BotName</at>` from text.
fn strip_teams_mention(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut in_mention = false;
    let mut chars = text.chars().peekable();
    let mut tag_buf = String::new();

    while let Some(c) = chars.next() {
        if c == '<' {
            tag_buf.clear();
            tag_buf.push(c);
            // Collect the full tag
            for tc in chars.by_ref() {
                tag_buf.push(tc);
                if tc == '>' {
                    break;
                }
            }
            if tag_buf.starts_with("<at>") {
                in_mention = true;
            } else if tag_buf == "</at>" {
                in_mention = false;
            } else if !in_mention {
                result.push_str(&tag_buf);
            }
        } else if !in_mention {
            result.push(c);
        }
    }

    result.trim().to_string()
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
    fn parse_message_activity() {
        let activity = Activity {
            activity_type: "message".to_string(),
            id: Some("act-123".to_string()),
            timestamp: Some("2024-01-01T00:00:00Z".to_string()),
            service_url: Some("https://smba.trafficmanager.net/teams/".to_string()),
            channel_id: Some("msteams".to_string()),
            from: Some(ActivityParticipant {
                id: "user-456".to_string(),
                name: Some("Alice".to_string()),
                aad_object_id: None,
            }),
            conversation: Some(ActivityConversation {
                id: "conv-789".to_string(),
                conversation_type: Some("personal".to_string()),
                tenant_id: Some("tenant-001".to_string()),
            }),
            recipient: Some(ActivityParticipant {
                id: "bot-001".to_string(),
                name: Some("SymbiBot".to_string()),
                aad_object_id: None,
            }),
            text: Some("run compliance-check on us-east-1".to_string()),
            reply_to_id: None,
        };

        let msg = parse_activity_to_message(&activity, "bot-001")
            .unwrap()
            .unwrap();
        assert_eq!(msg.platform, ChatPlatform::Teams);
        assert_eq!(msg.sender_name, "Alice");
        assert_eq!(msg.content, "on us-east-1");
        let cmd = msg.command.unwrap();
        assert_eq!(cmd.agent_name.as_deref(), Some("compliance-check"));
    }

    #[test]
    fn skip_non_message_activity() {
        let activity = Activity {
            activity_type: "conversationUpdate".to_string(),
            id: None,
            timestamp: None,
            service_url: None,
            channel_id: None,
            from: None,
            conversation: None,
            recipient: None,
            text: None,
            reply_to_id: None,
        };

        let result = parse_activity_to_message(&activity, "bot-001").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn skip_self_message() {
        let activity = Activity {
            activity_type: "message".to_string(),
            id: Some("act-self".to_string()),
            timestamp: None,
            service_url: None,
            channel_id: None,
            from: Some(ActivityParticipant {
                id: "bot-001".to_string(),
                name: Some("SymbiBot".to_string()),
                aad_object_id: None,
            }),
            conversation: Some(ActivityConversation {
                id: "conv-1".to_string(),
                conversation_type: None,
                tenant_id: None,
            }),
            recipient: None,
            text: Some("bot response".to_string()),
            reply_to_id: None,
        };

        let result = parse_activity_to_message(&activity, "bot-001").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn strip_teams_mention_tags() {
        let text = "<at>SymbiBot</at> run compliance-check on all";
        let stripped = strip_teams_mention(text);
        assert_eq!(stripped, "run compliance-check on all");
    }

    #[test]
    fn strip_teams_mention_no_mention() {
        let text = "hello world";
        let stripped = strip_teams_mention(text);
        assert_eq!(stripped, "hello world");
    }

    #[test]
    fn extract_agent_from_run_prefix() {
        let (agent, rest) = extract_agent_mention("run my-agent check status");
        assert_eq!(agent.as_deref(), Some("my-agent"));
        assert_eq!(rest, "check status");
    }

    #[test]
    fn extract_agent_no_prefix() {
        let (agent, rest) = extract_agent_mention("hello world");
        assert!(agent.is_none());
        assert_eq!(rest, "hello world");
    }

    #[test]
    fn activity_deserialization() {
        let json = r#"{
            "type": "message",
            "id": "act-1",
            "timestamp": "2024-01-01T00:00:00Z",
            "serviceUrl": "https://smba.trafficmanager.net/teams/",
            "channelId": "msteams",
            "from": {"id": "user-1", "name": "Alice"},
            "conversation": {"id": "conv-1", "conversationType": "personal"},
            "recipient": {"id": "bot-1", "name": "Bot"},
            "text": "hello"
        }"#;
        let activity: Activity = serde_json::from_str(json).unwrap();
        assert_eq!(activity.activity_type, "message");
        assert_eq!(activity.from.unwrap().name.as_deref(), Some("Alice"));
    }
}
