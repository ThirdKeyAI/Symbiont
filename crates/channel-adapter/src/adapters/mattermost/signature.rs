//! Token-based webhook verification for Mattermost outgoing webhooks.
//!
//! Mattermost outgoing webhooks include a `token` field (shared secret)
//! rather than HMAC signature headers. We verify this token matches
//! the configured webhook secret using constant-time comparison.

use subtle::ConstantTimeEq;

use crate::error::ChannelAdapterError;

/// Verify that the webhook token matches the configured secret.
///
/// Uses constant-time comparison to prevent timing attacks.
pub fn verify_webhook_token(
    configured_secret: &str,
    received_token: &str,
) -> Result<(), ChannelAdapterError> {
    if configured_secret.is_empty() {
        return Err(ChannelAdapterError::Config(
            "webhook secret not configured".to_string(),
        ));
    }

    let expected = configured_secret.as_bytes();
    let actual = received_token.as_bytes();

    if expected.len() != actual.len() || expected.ct_eq(actual).unwrap_u8() != 1 {
        return Err(ChannelAdapterError::SignatureInvalid(
            "webhook token mismatch".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_token_passes() {
        let secret = "abc123secret456";
        assert!(verify_webhook_token(secret, secret).is_ok());
    }

    #[test]
    fn invalid_token_fails() {
        let result = verify_webhook_token("correct-secret", "wrong-secret");
        assert!(result.is_err());
        match result.unwrap_err() {
            ChannelAdapterError::SignatureInvalid(msg) => {
                assert!(msg.contains("mismatch"));
            }
            other => panic!("expected SignatureInvalid, got: {:?}", other),
        }
    }

    #[test]
    fn empty_configured_secret_fails() {
        let result = verify_webhook_token("", "any-token");
        assert!(result.is_err());
        match result.unwrap_err() {
            ChannelAdapterError::Config(msg) => {
                assert!(msg.contains("not configured"));
            }
            other => panic!("expected Config, got: {:?}", other),
        }
    }

    #[test]
    fn different_length_tokens_fail() {
        let result = verify_webhook_token("short", "much-longer-token");
        assert!(result.is_err());
    }
}
