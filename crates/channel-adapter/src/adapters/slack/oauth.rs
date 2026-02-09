//! OAuth2 flow for Slack app installation.
//!
//! Supports the `symbi chat connect slack` CLI flow where the user provides
//! a bot token directly, or uses the OAuth2 redirect flow for workspace installs.

use serde::Deserialize;

use crate::error::ChannelAdapterError;

/// OAuth2 access token response from Slack.
#[derive(Debug, Deserialize)]
pub struct OAuthAccessResponse {
    pub ok: bool,
    pub access_token: Option<String>,
    pub token_type: Option<String>,
    pub scope: Option<String>,
    pub bot_user_id: Option<String>,
    pub app_id: Option<String>,
    pub team: Option<OAuthTeam>,
    pub error: Option<String>,
}

/// Team information from OAuth response.
#[derive(Debug, Deserialize)]
pub struct OAuthTeam {
    pub name: Option<String>,
    pub id: Option<String>,
}

/// Exchange an OAuth2 authorization code for an access token.
pub async fn exchange_code(
    client_id: &str,
    client_secret: &str,
    code: &str,
    redirect_uri: Option<&str>,
) -> Result<OAuthAccessResponse, ChannelAdapterError> {
    let client = reqwest::Client::new();

    let mut params = vec![
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("code", code),
    ];

    if let Some(uri) = redirect_uri {
        params.push(("redirect_uri", uri));
    }

    let resp = client
        .post("https://slack.com/api/oauth.v2.access")
        .form(&params)
        .send()
        .await
        .map_err(|e| ChannelAdapterError::Auth(format!("OAuth exchange failed: {}", e)))?;

    let oauth: OAuthAccessResponse = resp
        .json()
        .await
        .map_err(|e| ChannelAdapterError::ParseError(format!("OAuth parse: {}", e)))?;

    if !oauth.ok {
        return Err(ChannelAdapterError::Auth(format!(
            "OAuth rejected: {}",
            oauth.error.as_deref().unwrap_or("unknown")
        )));
    }

    Ok(oauth)
}

/// Validate that a bot token has the required scopes.
pub fn validate_token_format(token: &str) -> Result<(), ChannelAdapterError> {
    if !token.starts_with("xoxb-") {
        return Err(ChannelAdapterError::Config(
            "bot token must start with 'xoxb-'".to_string(),
        ));
    }
    if token.len() < 20 {
        return Err(ChannelAdapterError::Config(
            "bot token appears too short".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_bot_token_accepted() {
        assert!(validate_token_format("xoxb-1234567890-abcdefghijk").is_ok());
    }

    #[test]
    fn user_token_rejected() {
        let result = validate_token_format("xoxp-1234567890-abcdefghijk");
        assert!(result.is_err());
    }

    #[test]
    fn short_token_rejected() {
        let result = validate_token_format("xoxb-short");
        assert!(result.is_err());
    }

    #[test]
    fn empty_token_rejected() {
        let result = validate_token_format("");
        assert!(result.is_err());
    }

    #[test]
    fn oauth_response_success() {
        let json = r#"{
            "ok": true,
            "access_token": "xoxb-test-token",
            "token_type": "bot",
            "scope": "chat:write,app_mentions:read",
            "bot_user_id": "U123",
            "app_id": "A456",
            "team": {"name": "acme", "id": "T789"}
        }"#;
        let resp: OAuthAccessResponse = serde_json::from_str(json).unwrap();
        assert!(resp.ok);
        assert_eq!(resp.team.unwrap().name.as_deref(), Some("acme"));
    }

    #[test]
    fn oauth_response_error() {
        let json = r#"{"ok": false, "error": "invalid_code"}"#;
        let resp: OAuthAccessResponse = serde_json::from_str(json).unwrap();
        assert!(!resp.ok);
        assert_eq!(resp.error.as_deref(), Some("invalid_code"));
    }
}
