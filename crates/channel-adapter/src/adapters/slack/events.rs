//! Slack Events API and slash command parsing.
//!
//! Handles inbound Slack webhook payloads: Events API envelopes (url_verification,
//! event_callback with message events) and slash command form payloads.

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::error::ChannelAdapterError;
use crate::types::{ChatPlatform, InboundMessage, SlashCommand};

/// Top-level Slack Events API envelope.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SlackEnvelope {
    /// Slack sends this to verify the webhook URL during app setup.
    UrlVerification { challenge: String },
    /// Normal event delivery.
    EventCallback {
        team_id: String,
        event: SlackEvent,
        event_id: String,
    },
}

/// A Slack event within an event_callback envelope.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SlackEvent {
    /// A message posted in a channel the bot is in.
    Message {
        channel: String,
        user: Option<String>,
        text: Option<String>,
        ts: String,
        thread_ts: Option<String>,
        #[serde(default)]
        bot_id: Option<String>,
        #[serde(default)]
        subtype: Option<String>,
    },
    /// The bot was mentioned with @.
    AppMention {
        channel: String,
        user: String,
        text: String,
        ts: String,
        thread_ts: Option<String>,
    },
}

/// Parsed slash command payload (URL-encoded form data from Slack).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SlackSlashCommand {
    pub command: String,
    pub text: Option<String>,
    pub user_id: String,
    pub user_name: String,
    pub channel_id: String,
    pub channel_name: Option<String>,
    pub team_id: String,
    pub team_domain: Option<String>,
    pub response_url: String,
    pub trigger_id: String,
}

/// Parse an Events API JSON payload into a normalized `InboundMessage`.
pub fn parse_event_to_message(
    envelope: &SlackEnvelope,
) -> Result<Option<InboundMessage>, ChannelAdapterError> {
    match envelope {
        SlackEnvelope::UrlVerification { .. } => Ok(None),
        SlackEnvelope::EventCallback {
            team_id,
            event,
            event_id,
            ..
        } => {
            match event {
                SlackEvent::Message {
                    channel,
                    user,
                    text,
                    ts,
                    thread_ts,
                    bot_id,
                    subtype,
                    ..
                } => {
                    // Skip bot messages and subtypes (edits, deletes, etc.)
                    if bot_id.is_some() || subtype.is_some() {
                        return Ok(None);
                    }
                    let user = user.as_deref().unwrap_or("unknown");
                    let text = text.as_deref().unwrap_or("");
                    let (agent_name, clean_text) = extract_agent_mention(text);

                    Ok(Some(InboundMessage {
                        id: event_id.clone(),
                        platform: ChatPlatform::Slack,
                        workspace_id: team_id.clone(),
                        channel_id: channel.clone(),
                        thread_id: thread_ts.clone().or_else(|| Some(ts.clone())),
                        sender_id: user.to_string(),
                        sender_name: user.to_string(),
                        content: clean_text,
                        command: agent_name.map(|name| SlashCommand {
                            name: "invoke".to_string(),
                            subcommand: None,
                            args: vec![],
                            agent_name: Some(name),
                        }),
                        timestamp: Utc::now(),
                        raw_payload: None,
                    }))
                }
                SlackEvent::AppMention {
                    channel,
                    user,
                    text,
                    ts,
                    thread_ts,
                    ..
                } => {
                    let (agent_name, clean_text) = extract_agent_mention(text);

                    Ok(Some(InboundMessage {
                        id: event_id.clone(),
                        platform: ChatPlatform::Slack,
                        workspace_id: team_id.clone(),
                        channel_id: channel.clone(),
                        thread_id: thread_ts.clone().or_else(|| Some(ts.clone())),
                        sender_id: user.clone(),
                        sender_name: user.clone(),
                        content: clean_text,
                        command: Some(SlashCommand {
                            name: "invoke".to_string(),
                            subcommand: None,
                            args: vec![],
                            agent_name,
                        }),
                        timestamp: Utc::now(),
                        raw_payload: None,
                    }))
                }
            }
        }
    }
}

/// Parse a slash command payload into a normalized `InboundMessage`.
pub fn parse_slash_command(cmd: &SlackSlashCommand) -> InboundMessage {
    let text = cmd.text.as_deref().unwrap_or("");
    let parsed = parse_command_text(text);

    InboundMessage {
        id: cmd.trigger_id.clone(),
        platform: ChatPlatform::Slack,
        workspace_id: cmd.team_id.clone(),
        channel_id: cmd.channel_id.clone(),
        thread_id: None,
        sender_id: cmd.user_id.clone(),
        sender_name: cmd.user_name.clone(),
        content: text.to_string(),
        command: Some(parsed),
        timestamp: Utc::now(),
        raw_payload: None,
    }
}

/// Extract agent name from `@agent-name rest of text` pattern.
///
/// Returns `(Some(agent_name), cleaned_text)` if found, else `(None, original)`.
fn extract_agent_mention(text: &str) -> (Option<String>, String) {
    // Slack formats mentions as <@U123ABC> — strip those first
    let cleaned = strip_slack_user_mentions(text).trim().to_string();

    // Check for "run <agent-name> <input>" pattern
    if let Some(rest) = cleaned.strip_prefix("run ") {
        let mut parts = rest.splitn(2, ' ');
        if let Some(agent) = parts.next() {
            let input = parts.next().unwrap_or("").to_string();
            return (Some(agent.to_string()), input);
        }
    }

    (None, cleaned)
}

/// Strip Slack user mention tags like `<@U123ABC>` from text.
fn strip_slack_user_mentions(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut in_mention = false;
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '<' && chars.peek() == Some(&'@') {
            in_mention = true;
            continue;
        }
        if in_mention {
            if c == '>' {
                in_mention = false;
            }
            continue;
        }
        result.push(c);
    }

    result
}

/// Parse slash command text like "agent-name subcommand arg1 arg2".
fn parse_command_text(text: &str) -> SlashCommand {
    let parts: Vec<&str> = text.split_whitespace().collect();
    match parts.len() {
        0 => SlashCommand {
            name: "help".to_string(),
            subcommand: None,
            args: vec![],
            agent_name: None,
        },
        1 => SlashCommand {
            name: "invoke".to_string(),
            subcommand: None,
            args: vec![],
            agent_name: Some(parts[0].to_string()),
        },
        _ => SlashCommand {
            name: "invoke".to_string(),
            subcommand: Some(parts[1].to_string()),
            args: parts[2..].iter().map(|s| s.to_string()).collect(),
            agent_name: Some(parts[0].to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_url_verification() {
        let json = r#"{"type":"url_verification","challenge":"abc123"}"#;
        let envelope: SlackEnvelope = serde_json::from_str(json).unwrap();
        let result = parse_event_to_message(&envelope).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn parse_app_mention_event() {
        let json = r#"{
            "type": "event_callback",
            "team_id": "T123",
            "event_id": "Ev123",
            "event": {
                "type": "app_mention",
                "channel": "C456",
                "user": "U789",
                "text": "<@U00BOT> run compliance-check on us-east-1",
                "ts": "1234567890.123456"
            }
        }"#;
        let envelope: SlackEnvelope = serde_json::from_str(json).unwrap();
        let msg = parse_event_to_message(&envelope).unwrap().unwrap();
        assert_eq!(msg.platform, ChatPlatform::Slack);
        assert_eq!(msg.channel_id, "C456");
        assert_eq!(msg.sender_id, "U789");
        let cmd = msg.command.unwrap();
        assert_eq!(cmd.agent_name.as_deref(), Some("compliance-check"));
        assert_eq!(msg.content, "on us-east-1");
    }

    #[test]
    fn parse_message_event_with_bot_id_skipped() {
        let json = r#"{
            "type": "event_callback",
            "team_id": "T123",
            "event_id": "Ev124",
            "event": {
                "type": "message",
                "channel": "C456",
                "user": "U789",
                "text": "bot response",
                "ts": "1234567890.123456",
                "bot_id": "B001"
            }
        }"#;
        let envelope: SlackEnvelope = serde_json::from_str(json).unwrap();
        let result = parse_event_to_message(&envelope).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn parse_slash_command_with_agent() {
        let cmd = SlackSlashCommand {
            command: "/symbi".to_string(),
            text: Some("my-agent check status".to_string()),
            user_id: "U123".to_string(),
            user_name: "alice".to_string(),
            channel_id: "C456".to_string(),
            channel_name: Some("general".to_string()),
            team_id: "T789".to_string(),
            team_domain: Some("acme".to_string()),
            response_url: "https://hooks.slack.com/response".to_string(),
            trigger_id: "trigger123".to_string(),
        };
        let msg = parse_slash_command(&cmd);
        assert_eq!(msg.sender_name, "alice");
        let parsed_cmd = msg.command.unwrap();
        assert_eq!(parsed_cmd.agent_name.as_deref(), Some("my-agent"));
        assert_eq!(parsed_cmd.subcommand.as_deref(), Some("check"));
    }

    #[test]
    fn parse_slash_command_empty() {
        let cmd = SlackSlashCommand {
            command: "/symbi".to_string(),
            text: None,
            user_id: "U123".to_string(),
            user_name: "alice".to_string(),
            channel_id: "C456".to_string(),
            channel_name: None,
            team_id: "T789".to_string(),
            team_domain: None,
            response_url: "https://hooks.slack.com/response".to_string(),
            trigger_id: "trigger456".to_string(),
        };
        let msg = parse_slash_command(&cmd);
        let parsed_cmd = msg.command.unwrap();
        assert_eq!(parsed_cmd.name, "help");
        assert!(parsed_cmd.agent_name.is_none());
    }

    #[test]
    fn strip_slack_mentions() {
        let text = "<@U123ABC> run my-agent check all";
        let stripped = strip_slack_user_mentions(text);
        assert_eq!(stripped, " run my-agent check all");
    }

    #[test]
    fn extract_agent_from_mention() {
        let text = "<@U00BOT> run compliance-check on us-east-1";
        let (agent, rest) = extract_agent_mention(text);
        assert_eq!(agent.as_deref(), Some("compliance-check"));
        assert_eq!(rest, "on us-east-1");
    }

    #[test]
    fn extract_agent_no_run_prefix() {
        let text = "hello world";
        let (agent, rest) = extract_agent_mention(text);
        assert!(agent.is_none());
        assert_eq!(rest, "hello world");
    }
}
