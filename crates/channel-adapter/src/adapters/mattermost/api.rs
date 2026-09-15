//! Mattermost REST API client.
//!
//! Handles bot authentication and message posting via the Mattermost v4 API.

use serde::{Deserialize, Serialize};

use crate::error::ChannelAdapterError;
use crate::transport::{self, PreparedPost};
use crate::types::{ChatDeliveryReceipt, ChatPlatform, OutboundMessage};

/// Mattermost API client.
#[derive(Clone)]
pub struct MattermostApiClient {
    client: reqwest::Client,
    server_url: String,
    bot_token: String,
}

/// Response from `GET /api/v4/users/me`.
#[derive(Debug, Deserialize)]
pub struct MeResponse {
    pub id: String,
    pub username: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
}

/// Request body for `POST /api/v4/posts`.
#[derive(Debug, Serialize)]
struct CreatePostRequest {
    channel_id: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    root_id: Option<String>,
}

/// Response from `POST /api/v4/posts`.
#[derive(Debug, Deserialize)]
struct CreatePostResponse {
    id: Option<String>,
    channel_id: Option<String>,
}

impl MattermostApiClient {
    pub fn new(server_url: &str, bot_token: &str) -> Result<Self, ChannelAdapterError> {
        if server_url.is_empty() {
            return Err(ChannelAdapterError::Config(
                "Mattermost server_url cannot be empty".to_string(),
            ));
        }
        if bot_token.is_empty() {
            return Err(ChannelAdapterError::Config(
                "Mattermost bot_token cannot be empty".to_string(),
            ));
        }

        let client = transport::client()?;

        // Strip trailing slash from server URL
        let server_url = transport::base_url(server_url, true)?
            .as_str()
            .trim_end_matches('/')
            .to_string();

        Ok(Self {
            client,
            server_url,
            bot_token: bot_token.to_string(),
        })
    }

    /// Verify bot token and get bot user info via `GET /api/v4/users/me`.
    pub async fn get_me(&self) -> Result<MeResponse, ChannelAdapterError> {
        let url = transport::append_segments(
            transport::base_url(&self.server_url, true)?,
            &["api", "v4", "users", "me"],
        )?;

        let resp = self
            .client
            .get(url)
            .bearer_auth(&self.bot_token)
            .send()
            .await
            .map_err(|e| {
                ChannelAdapterError::Connection(format!("Mattermost users/me failed: {}", e))
            })?;

        let me: MeResponse = transport::read_json(resp, transport::RECEIPT_LIMIT).await?;

        Ok(me)
    }

    pub(crate) fn prepare_post(
        &self,
        message: &OutboundMessage,
    ) -> Result<PreparedPost, ChannelAdapterError> {
        let url = transport::append_segments(
            transport::base_url(&self.server_url, true)?,
            &["api", "v4", "posts"],
        )?;
        let body = CreatePostRequest {
            channel_id: message.channel_id.clone(),
            message: message.content.clone(),
            root_id: message.thread_id.clone(),
        };
        Ok(PreparedPost::new(url, serde_json::json!(body)))
    }

    /// Create a post in a channel via `POST /api/v4/posts`.
    pub async fn create_post(
        &self,
        message: &OutboundMessage,
    ) -> Result<ChatDeliveryReceipt, ChannelAdapterError> {
        let prepared = self.prepare_post(message)?;

        let resp = self
            .client
            .post(&prepared.url)
            .bearer_auth(&self.bot_token)
            .json(&prepared.body)
            .send()
            .await
            .map_err(|e| {
                ChannelAdapterError::SendFailed(format!("Mattermost create_post failed: {}", e))
            })?;

        let post_resp: CreatePostResponse =
            transport::read_json(resp, transport::RECEIPT_LIMIT).await?;
        let confirmed = post_resp.channel_id.as_deref() == Some(&message.channel_id)
            && post_resp.id.as_ref().is_some_and(|id| !id.is_empty());

        Ok(ChatDeliveryReceipt {
            platform: ChatPlatform::Mattermost,
            channel_id: post_resp
                .channel_id
                .unwrap_or_else(|| message.channel_id.clone()),
            message_ts: post_resp.id,
            delivered_at: chrono::Utc::now(),
            success: confirmed,
            error: (!confirmed).then(|| "Mattermost receipt is not confirmed".into()),
        })
    }
}

/// Format agent output as Mattermost Markdown.
pub fn format_agent_response(content: &str, agent_name: &str) -> String {
    format!(
        "#### Agent: {}\n\n{}\n\n---\n_Powered by Symbiont_",
        agent_name, content
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_server_url_rejected() {
        let result = MattermostApiClient::new("", "token-123");
        assert!(result.is_err());
    }

    #[test]
    fn empty_bot_token_rejected() {
        let result = MattermostApiClient::new("https://mm.example.com", "");
        assert!(result.is_err());
    }

    #[test]
    fn valid_config_accepted() {
        let result = MattermostApiClient::new("https://mm.example.com", "token-123");
        assert!(result.is_ok());
    }

    #[test]
    fn trailing_slash_stripped() {
        let client = MattermostApiClient::new("https://mm.example.com/", "token-123").unwrap();
        assert_eq!(client.server_url, "https://mm.example.com");
    }

    #[test]
    fn format_markdown_response() {
        let md = format_agent_response("All checks passed.", "compliance-check");
        assert!(md.contains("#### Agent: compliance-check"));
        assert!(md.contains("All checks passed."));
        assert!(md.contains("Powered by Symbiont"));
    }

    #[test]
    fn me_response_deserialization() {
        let json =
            r#"{"id":"user-123","username":"symbi-bot","first_name":"Symbi","last_name":"Bot"}"#;
        let resp: MeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.id, "user-123");
        assert_eq!(resp.username.as_deref(), Some("symbi-bot"));
    }
}
