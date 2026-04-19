use serde::{Deserialize, Serialize};

/// Configuration for the Slack approval channel.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SlackApprovalConfig {
    /// Whether Slack approval is enabled.
    pub enabled: bool,
    /// Slack Bot OAuth token (xoxb-...).
    pub bot_token: Option<String>,
    /// Slack signing secret for verifying webhook callbacks.
    pub signing_secret: Option<String>,
    /// Default Slack channel ID to post approval requests to.
    pub channel_id: Option<String>,
    /// Port for the Slack callback HTTP server.
    pub callback_port: Option<u16>,
}

/// Resolved Slack configuration with all required fields present.
#[derive(Debug, Clone)]
pub struct ResolvedSlackConfig {
    pub bot_token: String,
    pub signing_secret: String,
    pub channel_id: String,
    pub callback_port: u16,
}

/// Errors during configuration resolution.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Slack approval enabled but `{field}` is not set")]
    MissingField { field: &'static str },
}

impl SlackApprovalConfig {
    /// Resolve the configuration, ensuring all required fields are present.
    ///
    /// Returns `None` if Slack is not enabled. Returns an error if enabled
    /// but missing required fields.
    pub fn resolve(&self) -> Result<Option<ResolvedSlackConfig>, ConfigError> {
        if !self.enabled {
            return Ok(None);
        }

        let bot_token = self
            .bot_token
            .clone()
            .ok_or(ConfigError::MissingField { field: "bot_token" })?;
        let signing_secret = self.signing_secret.clone().ok_or(ConfigError::MissingField {
            field: "signing_secret",
        })?;
        let channel_id = self
            .channel_id
            .clone()
            .ok_or(ConfigError::MissingField { field: "channel_id" })?;
        let callback_port = self.callback_port.unwrap_or(3456);

        Ok(Some(ResolvedSlackConfig {
            bot_token,
            signing_secret,
            channel_id,
            callback_port,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_returns_none() {
        let cfg = SlackApprovalConfig::default();
        assert!(cfg.resolve().unwrap().is_none());
    }

    #[test]
    fn enabled_missing_token_errors() {
        let cfg = SlackApprovalConfig {
            enabled: true,
            ..Default::default()
        };
        let err = cfg.resolve().unwrap_err();
        assert!(err.to_string().contains("bot_token"));
    }

    #[test]
    fn enabled_fully_specified_resolves() {
        let cfg = SlackApprovalConfig {
            enabled: true,
            bot_token: Some("xoxb-test".into()),
            signing_secret: Some("secret".into()),
            channel_id: Some("C123".into()),
            callback_port: Some(4000),
        };
        let resolved = cfg.resolve().unwrap().unwrap();
        assert_eq!(resolved.bot_token, "xoxb-test");
        assert_eq!(resolved.callback_port, 4000);
    }

    #[test]
    fn default_callback_port() {
        let cfg = SlackApprovalConfig {
            enabled: true,
            bot_token: Some("xoxb-test".into()),
            signing_secret: Some("secret".into()),
            channel_id: Some("C123".into()),
            callback_port: None,
        };
        let resolved = cfg.resolve().unwrap().unwrap();
        assert_eq!(resolved.callback_port, 3456);
    }
}
