use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
    Router,
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::sync::Arc;
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::slack_relay::SlackApprovalRelay;

type HmacSha256 = Hmac<Sha256>;

/// Application state for the Axum HTTP server.
#[derive(Clone)]
struct AppState {
    relay: Arc<SlackApprovalRelay>,
}

/// Create the Axum router for handling Slack interaction callbacks.
pub fn slack_callback_router(relay: Arc<SlackApprovalRelay>) -> Router {
    let state = AppState { relay };
    Router::new()
        .route("/slack/interactions", post(handle_interaction))
        .with_state(state)
}

/// Handle a Slack interaction payload (button press callback).
async fn handle_interaction(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // Verify HMAC-SHA256 signature
    let timestamp = headers
        .get("X-Slack-Request-Timestamp")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let signature = headers
        .get("X-Slack-Signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let body_str = String::from_utf8_lossy(&body);

    if !verify_signature(state.relay.signing_secret(), timestamp, &body_str, signature) {
        tracing::warn!("Invalid Slack signature");
        return StatusCode::UNAUTHORIZED;
    }

    // Parse the payload
    let payload_str = if let Some(encoded) = body_str.strip_prefix("payload=") {
        urlencoding::decode(encoded)
            .unwrap_or_default()
            .into_owned()
    } else {
        body_str.into_owned()
    };

    let payload: serde_json::Value = match serde_json::from_str(&payload_str) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("Failed to parse Slack payload: {e}");
            return StatusCode::BAD_REQUEST;
        }
    };

    // Extract action details
    let actions = match payload["actions"].as_array() {
        Some(a) if !a.is_empty() => a,
        _ => return StatusCode::OK, // No actions, acknowledge
    };

    let action = &actions[0];
    let action_id = action["action_id"].as_str().unwrap_or("");
    let value = action["value"].as_str().unwrap_or("");
    let user_id = payload["user"]["id"].as_str().unwrap_or("unknown");
    let message_ts = payload["message"]["ts"].as_str().unwrap_or("");

    let request_id = match Uuid::parse_str(value) {
        Ok(id) => id,
        Err(_) => {
            tracing::warn!("Invalid request_id in action value: {value}");
            return StatusCode::BAD_REQUEST;
        }
    };

    state
        .relay
        .handle_callback(request_id, action_id, user_id, message_ts)
        .await;

    StatusCode::OK
}

/// Verify a Slack request signature using HMAC-SHA256.
fn verify_signature(signing_secret: &str, timestamp: &str, body: &str, expected: &str) -> bool {
    let sig_basestring = format!("v0:{timestamp}:{body}");

    let mut mac = match HmacSha256::new_from_slice(signing_secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(sig_basestring.as_bytes());
    let result = mac.finalize();
    let computed = format!("v0={}", hex::encode(result.into_bytes()));

    computed.as_bytes().ct_eq(expected.as_bytes()).into()
}

/// Start the Slack callback HTTP server.
pub async fn serve_slack_callbacks(
    relay: Arc<SlackApprovalRelay>,
    port: u16,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let app = slack_callback_router(relay);
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Slack callback server listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_valid_signature() {
        let secret = "8f742231b10e8888abcd99yez56789d0";
        let timestamp = "1531420618";
        let body = "token=xyzz0WbapA4vBCDEFasx0YGkhA&team_id=T1DC2JH3J";

        // Compute the expected signature
        let sig_basestring = format!("v0:{timestamp}:{body}");
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(sig_basestring.as_bytes());
        let result = mac.finalize();
        let expected = format!("v0={}", hex::encode(result.into_bytes()));

        assert!(verify_signature(secret, timestamp, body, &expected));
    }

    #[test]
    fn verify_invalid_signature() {
        assert!(!verify_signature("secret", "123", "body", "v0=wrong"));
    }

    #[test]
    fn verify_empty_secret() {
        // Empty secret should still work (not panic)
        assert!(!verify_signature("", "123", "body", "v0=bad"));
    }
}
