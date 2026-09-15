//! Microsoft Teams Bot Framework API client.
//!
//! Handles OAuth2 token acquisition (client credentials flow) and sending
//! replies via the Bot Framework REST API.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::error::ChannelAdapterError;
use crate::transport::{self, PreparedPost};
use crate::types::{ChatDeliveryReceipt, ChatPlatform, OutboundMessage};

/// Teams API client with OAuth2 token caching.
#[derive(Clone)]
pub struct TeamsApiClient {
    client: reqwest::Client,
    tenant_id: String,
    client_id: String,
    client_secret: String,
    token_cache: Arc<RwLock<Option<CachedToken>>>,
}

/// Cached OAuth2 access token with expiry tracking.
struct CachedToken {
    access_token: String,
    expires_at: std::time::Instant,
}

/// OAuth2 token response from Azure AD.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    token_type: String,
}

/// Reply activity sent to Bot Framework.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReplyActivity {
    #[serde(rename = "type")]
    activity_type: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    attachments: Option<Vec<Attachment>>,
}

/// An attachment in a Bot Framework reply (e.g. Adaptive Card).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Attachment {
    content_type: String,
    content: serde_json::Value,
}

/// Response from Bot Framework when posting a reply.
#[derive(Debug, Deserialize)]
struct ReplyResponse {
    id: Option<String>,
}

impl TeamsApiClient {
    pub fn new(
        tenant_id: &str,
        client_id: &str,
        client_secret: &str,
    ) -> Result<Self, ChannelAdapterError> {
        if tenant_id.is_empty() {
            return Err(ChannelAdapterError::Config(
                "Teams tenant_id cannot be empty".to_string(),
            ));
        }
        if client_id.is_empty() {
            return Err(ChannelAdapterError::Config(
                "Teams client_id cannot be empty".to_string(),
            ));
        }
        if client_secret.is_empty() {
            return Err(ChannelAdapterError::Config(
                "Teams client_secret cannot be empty".to_string(),
            ));
        }

        if !tenant_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.'))
            || matches!(tenant_id, "." | "..")
        {
            return Err(ChannelAdapterError::Config(
                "invalid Teams tenant identifier".into(),
            ));
        }
        let client = transport::client()?;

        Ok(Self {
            client,
            tenant_id: tenant_id.to_string(),
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
            token_cache: Arc::new(RwLock::new(None)),
        })
    }

    /// Acquire an OAuth2 access token via client credentials grant.
    ///
    /// Caches the token and refreshes it when expired (with a 60-second buffer).
    pub async fn get_oauth_token(&self) -> Result<String, ChannelAdapterError> {
        // Check cache first
        {
            let cache = self.token_cache.read().await;
            if let Some(ref cached) = *cache {
                if cached.expires_at
                    > std::time::Instant::now() + std::time::Duration::from_secs(60)
                {
                    return Ok(cached.access_token.clone());
                }
            }
        }

        // Token expired or not cached — acquire new token
        let token_url = format!(
            "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
            self.tenant_id
        );

        let params = [
            ("grant_type", "client_credentials"),
            ("client_id", &self.client_id),
            ("client_secret", &self.client_secret),
            ("scope", "https://api.botframework.com/.default"),
        ];

        let resp = self
            .client
            .post(&token_url)
            .form(&params)
            .send()
            .await
            .map_err(|e| {
                ChannelAdapterError::Auth(format!("OAuth2 token request failed: {}", e))
            })?;

        let token_resp: TokenResponse =
            transport::read_json(resp, transport::RECEIPT_LIMIT).await?;
        if !token_resp.token_type.eq_ignore_ascii_case("bearer")
            || token_resp.access_token.is_empty()
            || token_resp.expires_in == 0
        {
            return Err(ChannelAdapterError::Auth(
                "invalid OAuth2 token response".into(),
            ));
        }
        let expires_at = std::time::Instant::now()
            .checked_add(std::time::Duration::from_secs(token_resp.expires_in))
            .ok_or_else(|| ChannelAdapterError::Auth("invalid OAuth2 token expiry".into()))?;

        let access_token = token_resp.access_token.clone();

        // Cache the token
        let mut cache = self.token_cache.write().await;
        *cache = Some(CachedToken {
            access_token: token_resp.access_token,
            expires_at,
        });

        Ok(access_token)
    }

    pub(crate) fn prepare_reply(
        &self,
        message: &OutboundMessage,
    ) -> Result<PreparedPost, ChannelAdapterError> {
        let (service_url, activity_id) = reply_destination(message)?;
        prepare_reply(service_url, &message.channel_id, activity_id, message)
    }

    pub async fn reply(
        &self,
        message: &OutboundMessage,
    ) -> Result<ChatDeliveryReceipt, ChannelAdapterError> {
        let (service_url, activity_id) = reply_destination(message)?;
        self.reply_to_activity(service_url, &message.channel_id, activity_id, message)
            .await
    }

    /// Send only to the explicitly supplied service URL. Shipping callback routes
    /// authenticate it against the signed serviceurl claim before invocation.
    pub async fn reply_to_activity(
        &self,
        service_url: &str,
        conversation_id: &str,
        activity_id: &str,
        message: &OutboundMessage,
    ) -> Result<ChatDeliveryReceipt, ChannelAdapterError> {
        let prepared = prepare_reply(service_url, conversation_id, activity_id, message)?;
        let token = self.get_oauth_token().await?;
        let response = self
            .client
            .post(&prepared.url)
            .bearer_auth(&token)
            .json(&prepared.body)
            .send()
            .await
            .map_err(|e| {
                ChannelAdapterError::SendFailed(format!("Bot Framework reply failed: {e}"))
            })?;
        let reply: ReplyResponse = transport::read_json(response, transport::RECEIPT_LIMIT).await?;
        let confirmed = reply.id.as_ref().is_some_and(|id| !id.is_empty());
        Ok(ChatDeliveryReceipt {
            platform: ChatPlatform::Teams,
            channel_id: conversation_id.to_string(),
            message_ts: reply.id,
            delivered_at: chrono::Utc::now(),
            success: confirmed,
            error: (!confirmed).then(|| "Bot Framework receipt has no activity ID".into()),
        })
    }
}

fn reply_destination(message: &OutboundMessage) -> Result<(&str, &str), ChannelAdapterError> {
    let metadata = message.metadata.as_ref().ok_or_else(|| {
        ChannelAdapterError::Config("Teams reply requires destination metadata".into())
    })?;
    let service_url = metadata
        .get("service_url")
        .and_then(|value| value.as_str())
        .ok_or_else(|| ChannelAdapterError::Config("Teams reply requires a service URL".into()))?;
    let activity_id = metadata
        .get("activity_id")
        .and_then(|value| value.as_str())
        .ok_or_else(|| ChannelAdapterError::Config("Teams reply requires an activity ID".into()))?;
    Ok((service_url, activity_id))
}

fn prepare_reply(
    service_url: &str,
    conversation_id: &str,
    activity_id: &str,
    message: &OutboundMessage,
) -> Result<PreparedPost, ChannelAdapterError> {
    let url = transport::append_segments(
        transport::base_url(service_url, false)?,
        &[
            "v3",
            "conversations",
            conversation_id,
            "activities",
            activity_id,
        ],
    )?;
    let reply = ReplyActivity {
        activity_type: "message".into(),
        text: message.content.clone(),
        attachments: message.blocks.as_ref().map(|card| {
            vec![Attachment {
                content_type: "application/vnd.microsoft.card.adaptive".into(),
                content: card.clone(),
            }]
        }),
    };
    Ok(PreparedPost::new(url, serde_json::json!(reply)))
}

/// Format agent output as a Teams Adaptive Card.
pub fn format_agent_response(content: &str, agent_name: &str) -> serde_json::Value {
    serde_json::json!({
        "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
        "type": "AdaptiveCard",
        "version": "1.4",
        "body": [
            {
                "type": "TextBlock",
                "text": format!("Agent: {}", agent_name),
                "weight": "Bolder",
                "size": "Medium"
            },
            {
                "type": "TextBlock",
                "text": content,
                "wrap": true
            },
            {
                "type": "TextBlock",
                "text": "Powered by Symbiont",
                "isSubtle": true,
                "size": "Small",
                "spacing": "Medium"
            }
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tenant_id_rejected() {
        let result = TeamsApiClient::new("", "client-id", "secret");
        assert!(result.is_err());
    }

    #[test]
    fn empty_client_id_rejected() {
        let result = TeamsApiClient::new("tenant", "", "secret");
        assert!(result.is_err());
    }

    #[test]
    fn empty_client_secret_rejected() {
        let result = TeamsApiClient::new("tenant", "client-id", "");
        assert!(result.is_err());
    }

    #[test]
    fn valid_config_accepted() {
        let result = TeamsApiClient::new("tenant-123", "client-456", "secret-789");
        assert!(result.is_ok());
    }

    #[test]
    fn format_adaptive_card() {
        let card = format_agent_response("Analysis complete.", "compliance-check");
        assert_eq!(card["type"], "AdaptiveCard");
        let body = card["body"].as_array().unwrap();
        assert_eq!(body.len(), 3);
        assert!(body[0]["text"]
            .as_str()
            .unwrap()
            .contains("compliance-check"));
        assert!(body[1]["text"]
            .as_str()
            .unwrap()
            .contains("Analysis complete."));
    }

    #[test]
    fn token_response_deserialization() {
        let json = r#"{"access_token":"eyJ...","expires_in":3600,"token_type":"Bearer"}"#;
        let resp: TokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.access_token, "eyJ...");
        assert_eq!(resp.expires_in, 3600);
    }
}
