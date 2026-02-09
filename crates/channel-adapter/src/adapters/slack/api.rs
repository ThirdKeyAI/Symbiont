//! Slack Web API client for sending messages.
//!
//! Wraps `chat.postMessage` and `auth.test` endpoints with bot token auth.

use serde::{Deserialize, Serialize};

use crate::error::ChannelAdapterError;
use crate::types::{ChatDeliveryReceipt, ChatPlatform, OutboundMessage};

/// Slack Web API client.
#[derive(Clone)]
pub struct SlackApiClient {
    client: reqwest::Client,
    bot_token: String,
}

/// Response from Slack's `auth.test` API.
#[derive(Debug, Deserialize)]
pub struct AuthTestResponse {
    pub ok: bool,
    pub team: Option<String>,
    pub team_id: Option<String>,
    pub user: Option<String>,
    pub user_id: Option<String>,
    pub bot_id: Option<String>,
    pub error: Option<String>,
}

/// Response from Slack's `chat.postMessage` API.
#[derive(Debug, Deserialize)]
pub struct PostMessageResponse {
    pub ok: bool,
    pub ts: Option<String>,
    pub channel: Option<String>,
    pub error: Option<String>,
}

/// Slack Block Kit section block for formatted responses.
#[derive(Debug, Clone, Serialize)]
pub struct SlackBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<SlackTextObject>,
}

/// Text object within a Slack block.
#[derive(Debug, Clone, Serialize)]
pub struct SlackTextObject {
    #[serde(rename = "type")]
    pub text_type: String,
    pub text: String,
}

impl SlackApiClient {
    pub fn new(bot_token: &str) -> Result<Self, ChannelAdapterError> {
        if bot_token.is_empty() {
            return Err(ChannelAdapterError::Config(
                "bot token cannot be empty".to_string(),
            ));
        }

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| ChannelAdapterError::Internal(format!("HTTP client init: {}", e)))?;

        Ok(Self {
            client,
            bot_token: bot_token.to_string(),
        })
    }

    /// Verify the bot token and get workspace info via `auth.test`.
    pub async fn auth_test(&self) -> Result<AuthTestResponse, ChannelAdapterError> {
        let resp = self
            .client
            .post("https://slack.com/api/auth.test")
            .bearer_auth(&self.bot_token)
            .send()
            .await
            .map_err(|e| ChannelAdapterError::Connection(format!("auth.test failed: {}", e)))?;

        let auth: AuthTestResponse = resp
            .json()
            .await
            .map_err(|e| ChannelAdapterError::ParseError(format!("auth.test parse: {}", e)))?;

        if !auth.ok {
            return Err(ChannelAdapterError::Auth(format!(
                "auth.test rejected: {}",
                auth.error.as_deref().unwrap_or("unknown")
            )));
        }

        Ok(auth)
    }

    /// Send a message to a Slack channel via `chat.postMessage`.
    pub async fn post_message(
        &self,
        message: &OutboundMessage,
    ) -> Result<ChatDeliveryReceipt, ChannelAdapterError> {
        let mut payload = serde_json::json!({
            "channel": message.channel_id,
            "text": message.content,
        });

        if let Some(ref thread_ts) = message.thread_id {
            payload["thread_ts"] = serde_json::Value::String(thread_ts.clone());
        }

        if let Some(ref blocks) = message.blocks {
            payload["blocks"] = blocks.clone();
        }

        let resp = self
            .client
            .post("https://slack.com/api/chat.postMessage")
            .bearer_auth(&self.bot_token)
            .json(&payload)
            .send()
            .await
            .map_err(|e| {
                ChannelAdapterError::SendFailed(format!("chat.postMessage failed: {}", e))
            })?;

        let post_resp: PostMessageResponse = resp.json().await.map_err(|e| {
            ChannelAdapterError::ParseError(format!("chat.postMessage parse: {}", e))
        })?;

        Ok(ChatDeliveryReceipt {
            platform: ChatPlatform::Slack,
            channel_id: post_resp
                .channel
                .unwrap_or_else(|| message.channel_id.clone()),
            message_ts: post_resp.ts,
            delivered_at: chrono::Utc::now(),
            success: post_resp.ok,
            error: post_resp.error,
        })
    }
}

/// Format agent output as Slack Block Kit blocks for readability.
pub fn format_agent_response(content: &str, agent_name: &str) -> serde_json::Value {
    serde_json::json!([
        {
            "type": "header",
            "text": {
                "type": "plain_text",
                "text": format!("Agent: {}", agent_name)
            }
        },
        {
            "type": "section",
            "text": {
                "type": "mrkdwn",
                "text": content
            }
        },
        {
            "type": "context",
            "elements": [
                {
                    "type": "mrkdwn",
                    "text": format!("_Powered by Symbiont_")
                }
            ]
        }
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_token_rejected() {
        let result = SlackApiClient::new("");
        assert!(result.is_err());
    }

    #[test]
    fn format_agent_response_blocks() {
        let blocks = format_agent_response("All checks passed.", "compliance-check");
        let arr = blocks.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0]["type"], "header");
        assert_eq!(arr[1]["type"], "section");
        assert!(arr[1]["text"]["text"]
            .as_str()
            .unwrap()
            .contains("All checks passed."));
    }

    #[test]
    fn auth_test_response_deserialization() {
        let json = r#"{"ok":true,"team":"acme","team_id":"T123","user":"bot","user_id":"U456","bot_id":"B789"}"#;
        let resp: AuthTestResponse = serde_json::from_str(json).unwrap();
        assert!(resp.ok);
        assert_eq!(resp.team.as_deref(), Some("acme"));
        assert_eq!(resp.bot_id.as_deref(), Some("B789"));
    }

    #[test]
    fn post_message_response_success() {
        let json = r#"{"ok":true,"ts":"1234567890.123456","channel":"C456"}"#;
        let resp: PostMessageResponse = serde_json::from_str(json).unwrap();
        assert!(resp.ok);
        assert_eq!(resp.ts.as_deref(), Some("1234567890.123456"));
    }

    #[test]
    fn post_message_response_error() {
        let json = r#"{"ok":false,"error":"channel_not_found"}"#;
        let resp: PostMessageResponse = serde_json::from_str(json).unwrap();
        assert!(!resp.ok);
        assert_eq!(resp.error.as_deref(), Some("channel_not_found"));
    }
}
