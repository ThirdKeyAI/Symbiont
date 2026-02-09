use serde::{Deserialize, Serialize};

use crate::types::ChatPlatform;

/// Top-level configuration for a channel adapter instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConfig {
    /// Human-readable name for this adapter instance.
    pub name: String,
    /// Which platform this adapter connects to.
    pub platform: ChatPlatform,
    /// Platform-specific configuration.
    pub settings: PlatformSettings,
}

/// Platform-specific settings, determined by the `platform` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlatformSettings {
    Slack(SlackConfig),
    #[cfg(feature = "teams")]
    Teams(TeamsConfig),
    #[cfg(feature = "mattermost")]
    Mattermost(MattermostConfig),
}

/// Configuration for a Slack adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackConfig {
    /// Bot token (xoxb-...). Resolved from env or secret store at startup.
    pub bot_token: String,
    /// App-level token for Socket Mode (xapp-...), if using Socket Mode.
    pub app_token: Option<String>,
    /// Signing secret for verifying inbound webhook requests.
    pub signing_secret: Option<String>,
    /// Workspace ID (auto-detected on connect if not provided).
    pub workspace_id: Option<String>,
    /// Channels the bot should listen in. Empty = all channels the bot is in.
    pub channels: Vec<String>,
    /// Port for the webhook receiver server.
    #[serde(default = "default_webhook_port")]
    pub webhook_port: u16,
    /// Bind address for the webhook server.
    #[serde(default = "default_bind_address")]
    pub bind_address: String,
    /// Default agent to invoke when no agent is specified in the message.
    pub default_agent: Option<String>,
}

fn default_webhook_port() -> u16 {
    3100
}

fn default_bind_address() -> String {
    "0.0.0.0".to_string()
}

/// Configuration for a Microsoft Teams adapter.
#[cfg(feature = "teams")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamsConfig {
    /// Azure AD tenant ID.
    pub tenant_id: String,
    /// Azure AD application (client) ID.
    pub client_id: String,
    /// Azure AD client secret.
    pub client_secret: String,
    /// Bot ID registered in Bot Framework.
    pub bot_id: String,
    /// Webhook URL for inbound activities from Teams.
    pub webhook_url: Option<String>,
    /// Port for the webhook receiver server.
    #[serde(default = "default_teams_webhook_port")]
    pub webhook_port: u16,
    /// Bind address for the webhook server.
    #[serde(default = "default_bind_address")]
    pub bind_address: String,
    /// Default agent to invoke when no agent is specified.
    pub default_agent: Option<String>,
}

#[cfg(feature = "teams")]
fn default_teams_webhook_port() -> u16 {
    3200
}

#[cfg(feature = "teams")]
impl Default for TeamsConfig {
    fn default() -> Self {
        Self {
            tenant_id: String::new(),
            client_id: String::new(),
            client_secret: String::new(),
            bot_id: String::new(),
            webhook_url: None,
            webhook_port: default_teams_webhook_port(),
            bind_address: default_bind_address(),
            default_agent: None,
        }
    }
}

/// Configuration for a Mattermost adapter.
#[cfg(feature = "mattermost")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MattermostConfig {
    /// Mattermost server URL (e.g. "https://mattermost.example.com").
    pub server_url: String,
    /// Bot token for authentication.
    pub bot_token: String,
    /// Webhook secret for HMAC-SHA256 signature verification.
    pub webhook_secret: Option<String>,
    /// Team ID the bot operates in.
    pub team_id: Option<String>,
    /// Channels the bot should listen in.
    pub channels: Vec<String>,
    /// Port for the inbound webhook receiver.
    #[serde(default = "default_mattermost_webhook_port")]
    pub webhook_port: u16,
    /// Bind address for the webhook server.
    #[serde(default = "default_bind_address")]
    pub bind_address: String,
    /// Default agent to invoke when no agent is specified.
    pub default_agent: Option<String>,
}

#[cfg(feature = "mattermost")]
fn default_mattermost_webhook_port() -> u16 {
    3300
}

#[cfg(feature = "mattermost")]
impl Default for MattermostConfig {
    fn default() -> Self {
        Self {
            server_url: String::new(),
            bot_token: String::new(),
            webhook_secret: None,
            team_id: None,
            channels: Vec::new(),
            webhook_port: default_mattermost_webhook_port(),
            bind_address: default_bind_address(),
            default_agent: None,
        }
    }
}

impl Default for SlackConfig {
    fn default() -> Self {
        Self {
            bot_token: String::new(),
            app_token: None,
            signing_secret: None,
            workspace_id: None,
            channels: Vec::new(),
            webhook_port: default_webhook_port(),
            bind_address: default_bind_address(),
            default_agent: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slack_config_defaults() {
        let cfg = SlackConfig::default();
        assert_eq!(cfg.webhook_port, 3100);
        assert_eq!(cfg.bind_address, "0.0.0.0");
        assert!(cfg.channels.is_empty());
    }

    #[test]
    fn channel_config_serialization() {
        let cfg = ChannelConfig {
            name: "ops-slack".to_string(),
            platform: ChatPlatform::Slack,
            settings: PlatformSettings::Slack(SlackConfig {
                bot_token: "xoxb-test".to_string(),
                signing_secret: Some("secret123".to_string()),
                ..SlackConfig::default()
            }),
        };
        let json = serde_json::to_string_pretty(&cfg).unwrap();
        assert!(json.contains("ops-slack"));
        let parsed: ChannelConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "ops-slack");
    }
}
