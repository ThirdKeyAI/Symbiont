//! HMAC-SHA256 request verification for Slack Events API.
//!
//! Slack signs every webhook request with an HMAC-SHA256 signature using
//! the app's signing secret. We verify this to ensure requests are authentic.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::error::ChannelAdapterError;

type HmacSha256 = Hmac<Sha256>;

/// Maximum allowed age of a Slack request timestamp (5 minutes).
const MAX_TIMESTAMP_AGE_SECS: i64 = 300;

/// Verify that an inbound Slack request is authentic.
///
/// Checks the `x-slack-signature` header against an HMAC-SHA256 computed
/// from the signing secret, timestamp, and request body.
pub fn verify_slack_signature(
    signing_secret: &str,
    timestamp: &str,
    body: &[u8],
    signature: &str,
) -> Result<(), ChannelAdapterError> {
    // Validate timestamp freshness to prevent replay attacks
    let ts: i64 = timestamp
        .parse()
        .map_err(|_| ChannelAdapterError::SignatureInvalid("invalid timestamp".to_string()))?;
    let now = chrono::Utc::now().timestamp();
    // Widen to i128 so adversarial timestamps near i64::MIN/MAX cannot
    // overflow the subtraction (debug builds panic; release builds wrap
    // and could let a stale request slip through the freshness check).
    let delta = (now as i128) - (ts as i128);
    if delta.unsigned_abs() > MAX_TIMESTAMP_AGE_SECS as u128 {
        return Err(ChannelAdapterError::SignatureInvalid(
            "request timestamp too old".to_string(),
        ));
    }

    // Compute expected signature: v0=HMAC-SHA256(secret, "v0:{timestamp}:{body}")
    let sig_basestring = format!("v0:{}:{}", timestamp, String::from_utf8_lossy(body));

    let mut mac = HmacSha256::new_from_slice(signing_secret.as_bytes())
        .map_err(|e| ChannelAdapterError::Internal(format!("HMAC init failed: {}", e)))?;
    mac.update(sig_basestring.as_bytes());
    let computed = mac.finalize().into_bytes();
    let computed_hex = format!("v0={}", hex::encode(computed));

    // Constant-time comparison to prevent timing attacks
    let expected_bytes = computed_hex.as_bytes();
    let actual_bytes = signature.as_bytes();

    if expected_bytes.len() != actual_bytes.len()
        || expected_bytes.ct_eq(actual_bytes).unwrap_u8() != 1
    {
        return Err(ChannelAdapterError::SignatureInvalid(
            "signature mismatch".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_signature_passes() {
        let secret = "8f742231b10e8888abcd99yyyzzz85a5";
        let timestamp = &chrono::Utc::now().timestamp().to_string();
        let body = b"token=xyzz0WbapA4vBCDEFasx0q6G&team_id=T1DC2JH3J";

        // Compute the correct signature
        let sig_basestring = format!("v0:{}:{}", timestamp, String::from_utf8_lossy(body));
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(sig_basestring.as_bytes());
        let computed = mac.finalize().into_bytes();
        let signature = format!("v0={}", hex::encode(computed));

        assert!(verify_slack_signature(secret, timestamp, body, &signature).is_ok());
    }

    #[test]
    fn invalid_signature_fails() {
        let secret = "8f742231b10e8888abcd99yyyzzz85a5";
        let timestamp = &chrono::Utc::now().timestamp().to_string();
        let body = b"some body";
        let bad_signature = "v0=deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

        let result = verify_slack_signature(secret, timestamp, body, bad_signature);
        assert!(result.is_err());
        match result.unwrap_err() {
            ChannelAdapterError::SignatureInvalid(msg) => {
                assert!(msg.contains("mismatch"));
            }
            other => panic!("expected SignatureInvalid, got: {:?}", other),
        }
    }

    #[test]
    fn extreme_timestamps_do_not_panic() {
        // Regression: the freshness check used `(now - ts).abs()` on i64,
        // which panics on subtract-overflow when ts is i64::MIN/MAX (or
        // wraps in release builds, potentially accepting stale requests).
        let secret = "test-secret";
        let body = b"body";
        let sig = "v0=00";

        for ts in [
            i64::MAX.to_string(),
            i64::MIN.to_string(),
            (i64::MAX - 1).to_string(),
            (i64::MIN + 1).to_string(),
        ] {
            let result = verify_slack_signature(secret, &ts, body, sig);
            // Must not panic; must reject as out-of-window.
            assert!(
                result.is_err(),
                "extreme timestamp {ts} must be rejected, got {result:?}",
            );
        }
    }

    #[test]
    fn old_timestamp_rejected() {
        let secret = "test-secret";
        let old_ts = (chrono::Utc::now().timestamp() - 600).to_string();
        let body = b"body";
        let sig = "v0=doesntmatter";

        let result = verify_slack_signature(secret, &old_ts, body, sig);
        assert!(result.is_err());
        match result.unwrap_err() {
            ChannelAdapterError::SignatureInvalid(msg) => {
                assert!(msg.contains("too old"));
            }
            other => panic!("expected SignatureInvalid, got: {:?}", other),
        }
    }
}
