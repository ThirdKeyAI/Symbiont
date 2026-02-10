//! JWT validation for inbound Bot Framework requests.
//!
//! Validates the `Authorization: Bearer <jwt>` header on inbound activities
//! from Microsoft Bot Framework. In production, this verifies the JWT against
//! Microsoft's OpenID metadata and JWKS keys. A dev mode option allows
//! skipping full JWKS verification for local testing.

use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};

use crate::error::ChannelAdapterError;

/// OpenID metadata endpoint for Bot Framework tokens.
const OPENID_METADATA_URL: &str =
    "https://login.botframework.com/v1/.well-known/openidconfiguration";

/// Expected issuer for Bot Framework tokens.
const BOT_FRAMEWORK_ISSUER: &str = "https://api.botframework.com";

/// JWT claims from a Bot Framework token.
#[derive(Debug, Serialize, Deserialize)]
pub struct BotFrameworkClaims {
    /// Issuer — must be `https://api.botframework.com`.
    pub iss: String,
    /// Audience — must match the bot's app ID (client_id).
    pub aud: String,
    /// Expiry timestamp.
    pub exp: usize,
    /// Not-before timestamp.
    #[serde(default)]
    pub nbf: usize,
    /// Service URL (optional).
    #[serde(rename = "serviceurl", default)]
    pub service_url: Option<String>,
}

/// OpenID configuration response.
#[derive(Debug, Deserialize)]
struct OpenIdConfig {
    jwks_uri: String,
    #[allow(dead_code)]
    issuer: String,
}

/// JWKS key set response.
#[derive(Debug, Deserialize)]
struct JwksResponse {
    keys: Vec<JwkKey>,
}

/// A single JWK key.
#[derive(Debug, Deserialize)]
struct JwkKey {
    #[allow(dead_code)]
    kty: String,
    kid: Option<String>,
    n: Option<String>,
    e: Option<String>,
}

/// Validate a Bot Framework JWT token.
///
/// In dev mode (`skip_jwks_verification = true`), only validates token structure
/// and claims without verifying the cryptographic signature against JWKS.
/// In production, fetches JWKS keys and fully verifies the signature.
pub async fn validate_bot_framework_token(
    token: &str,
    client_id: &str,
    skip_jwks_verification: bool,
) -> Result<BotFrameworkClaims, ChannelAdapterError> {
    if skip_jwks_verification {
        // Dev mode: decode without signature verification, just validate claims
        let mut validation = Validation::default();
        validation.insecure_disable_signature_validation();
        validation.set_audience(&[client_id]);
        validation.set_issuer(&[BOT_FRAMEWORK_ISSUER]);

        let token_data =
            decode::<BotFrameworkClaims>(token, &DecodingKey::from_secret(b""), &validation)
                .map_err(|e| ChannelAdapterError::Auth(format!("JWT validation failed: {}", e)))?;

        return Ok(token_data.claims);
    }

    // Production: fetch JWKS and verify signature
    let client = reqwest::Client::new();

    // Fetch OpenID metadata to get JWKS URI
    let openid_config: OpenIdConfig = client
        .get(OPENID_METADATA_URL)
        .send()
        .await
        .map_err(|e| ChannelAdapterError::Auth(format!("Failed to fetch OpenID metadata: {}", e)))?
        .json()
        .await
        .map_err(|e| {
            ChannelAdapterError::Auth(format!("Failed to parse OpenID metadata: {}", e))
        })?;

    // Fetch JWKS keys
    let jwks: JwksResponse = client
        .get(&openid_config.jwks_uri)
        .send()
        .await
        .map_err(|e| ChannelAdapterError::Auth(format!("Failed to fetch JWKS: {}", e)))?
        .json()
        .await
        .map_err(|e| ChannelAdapterError::Auth(format!("Failed to parse JWKS: {}", e)))?;

    // Decode token header to find the key ID
    let header = jsonwebtoken::decode_header(token)
        .map_err(|e| ChannelAdapterError::Auth(format!("Invalid JWT header: {}", e)))?;

    let kid = header
        .kid
        .ok_or_else(|| ChannelAdapterError::Auth("JWT missing kid claim".to_string()))?;

    // Find the matching key
    let jwk = jwks
        .keys
        .iter()
        .find(|k| k.kid.as_deref() == Some(&kid))
        .ok_or_else(|| {
            ChannelAdapterError::Auth(format!("No matching JWKS key for kid: {}", kid))
        })?;

    let n = jwk
        .n
        .as_ref()
        .ok_or_else(|| ChannelAdapterError::Auth("JWKS key missing 'n' component".to_string()))?;
    let e = jwk
        .e
        .as_ref()
        .ok_or_else(|| ChannelAdapterError::Auth("JWKS key missing 'e' component".to_string()))?;

    let decoding_key = DecodingKey::from_rsa_components(n, e)
        .map_err(|e| ChannelAdapterError::Auth(format!("Invalid RSA key components: {}", e)))?;

    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[client_id]);
    validation.set_issuer(&[BOT_FRAMEWORK_ISSUER]);

    let token_data =
        decode::<BotFrameworkClaims>(token, &decoding_key, &validation).map_err(|e| {
            ChannelAdapterError::Auth(format!("JWT signature verification failed: {}", e))
        })?;

    Ok(token_data.claims)
}

/// Extract the Bearer token from an Authorization header value.
pub fn extract_bearer_token(auth_header: &str) -> Option<&str> {
    auth_header.strip_prefix("Bearer ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_bearer_token_valid() {
        let header = "Bearer eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.test.sig";
        let token = extract_bearer_token(header);
        assert_eq!(token, Some("eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.test.sig"));
    }

    #[test]
    fn extract_bearer_token_missing_prefix() {
        assert!(extract_bearer_token("Basic abc123").is_none());
        assert!(extract_bearer_token("").is_none());
        assert!(extract_bearer_token("bearer lowercase").is_none());
    }

    #[test]
    fn bot_framework_claims_deserialization() {
        let json = r#"{
            "iss": "https://api.botframework.com",
            "aud": "app-id-123",
            "exp": 9999999999,
            "nbf": 1000000000,
            "serviceurl": "https://smba.trafficmanager.net/teams/"
        }"#;
        let claims: BotFrameworkClaims = serde_json::from_str(json).unwrap();
        assert_eq!(claims.iss, "https://api.botframework.com");
        assert_eq!(claims.aud, "app-id-123");
        assert_eq!(
            claims.service_url.as_deref(),
            Some("https://smba.trafficmanager.net/teams/")
        );
    }
}
