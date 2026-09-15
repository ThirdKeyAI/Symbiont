//! Verify Bot Framework signatures and bind the inbound activity to its service URL.
use crate::{error::ChannelAdapterError, transport};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};

const OPENID_METADATA_URL: &str =
    "https://login.botframework.com/v1/.well-known/openidconfiguration";
const BOT_FRAMEWORK_ISSUER: &str = "https://api.botframework.com";

#[derive(Debug, Serialize, Deserialize)]
pub struct BotFrameworkClaims {
    pub iss: String,
    pub aud: String,
    pub exp: usize,
    pub nbf: usize,
    #[serde(rename = "serviceurl", alias = "serviceUrl")]
    pub service_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenIdConfig {
    jwks_uri: String,
    issuer: String,
    id_token_signing_alg_values_supported: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct JwksResponse {
    keys: Vec<JwkKey>,
}

#[derive(Debug, Deserialize)]
struct JwkKey {
    kty: String,
    kid: Option<String>,
    n: Option<String>,
    e: Option<String>,
    alg: Option<String>,
    #[serde(rename = "use")]
    key_use: Option<String>,
    #[serde(default)]
    endorsements: Vec<String>,
}

fn rejected(message: &str) -> ChannelAdapterError {
    ChannelAdapterError::Auth(message.into())
}

/// The legacy bypass argument is retained to reject old insecure configurations.
/// Every accepted request requires a current RSA signature from a Teams-endorsed key.
pub async fn validate_bot_framework_token(
    token: &str,
    client_id: &str,
    skip_jwks_verification: bool,
) -> Result<BotFrameworkClaims, ChannelAdapterError> {
    if skip_jwks_verification {
        return Err(rejected("Teams signature verification cannot be disabled"));
    }
    if token.len() > 16 * 1024 || client_id.is_empty() {
        return Err(rejected("invalid Teams token or audience"));
    }
    let header = decode_header(token).map_err(|_| rejected("invalid JWT header"))?;
    if header.alg != Algorithm::RS256 {
        return Err(rejected("Teams tokens require RS256"));
    }
    let kid = header
        .kid
        .filter(|id| !id.is_empty() && id.len() <= 256)
        .ok_or_else(|| rejected("JWT missing or invalid key ID"))?;
    let client = transport::client()?;
    let response = client
        .get(OPENID_METADATA_URL)
        .send()
        .await
        .map_err(|_| rejected("failed to fetch Bot Framework metadata"))?;
    let metadata: OpenIdConfig = transport::read_json(response, transport::METADATA_LIMIT).await?;
    if metadata.issuer != BOT_FRAMEWORK_ISSUER
        || !metadata
            .id_token_signing_alg_values_supported
            .iter()
            .any(|alg| alg == "RS256")
    {
        return Err(rejected(
            "untrusted Bot Framework issuer or signing algorithm",
        ));
    }
    let keys_url = transport::base_url(&metadata.jwks_uri, false)?;
    if keys_url.host_str() != Some("login.botframework.com")
        || keys_url.port_or_known_default() != Some(443)
    {
        return Err(rejected(
            "JWKS must remain on the Bot Framework metadata authority",
        ));
    }
    let response = client
        .get(keys_url)
        .send()
        .await
        .map_err(|_| rejected("failed to fetch Bot Framework keys"))?;
    let jwks: JwksResponse = transport::read_json(response, transport::METADATA_LIMIT).await?;
    let mut matching = jwks
        .keys
        .iter()
        .filter(|key| key.kid.as_deref() == Some(&kid));
    let key = matching
        .next()
        .ok_or_else(|| rejected("no matching Bot Framework key"))?;
    if matching.next().is_some()
        || key.kty != "RSA"
        || key.alg.as_deref().is_some_and(|alg| alg != "RS256")
        || key.key_use.as_deref().is_some_and(|usage| usage != "sig")
        || !key.endorsements.iter().any(|channel| channel == "msteams")
    {
        return Err(rejected(
            "ambiguous, invalid or unendorsed Teams signing key",
        ));
    }
    let n = key
        .n
        .as_deref()
        .ok_or_else(|| rejected("missing RSA modulus"))?;
    let e = key
        .e
        .as_deref()
        .ok_or_else(|| rejected("missing RSA exponent"))?;
    let key =
        DecodingKey::from_rsa_components(n, e).map_err(|_| rejected("invalid RSA components"))?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_required_spec_claims(&["exp", "nbf", "iss", "aud"]);
    validation.set_audience(&[client_id]);
    validation.set_issuer(&[BOT_FRAMEWORK_ISSUER]);
    validation.validate_nbf = true;
    validation.leeway = 300;
    let claims = decode::<BotFrameworkClaims>(token, &key, &validation)
        .map_err(|_| rejected("Teams signature or claims validation failed"))?
        .claims;
    if claims.nbf >= claims.exp {
        return Err(rejected("invalid Teams token validity interval"));
    }
    transport::base_url(
        claims
            .service_url
            .as_deref()
            .ok_or_else(|| rejected("missing signed service URL"))?,
        false,
    )?;
    Ok(claims)
}

pub(super) fn validate_activity(
    claims: &BotFrameworkClaims,
    activity: &super::events::Activity,
) -> Result<(), ChannelAdapterError> {
    if activity.channel_id.as_deref() != Some("msteams") {
        return Err(rejected("activity is not a Teams channel"));
    }
    let service = claims
        .service_url
        .as_deref()
        .ok_or_else(|| rejected("missing signed service URL"))?;
    transport::base_url(service, false)?;
    if activity.service_url.as_deref() != Some(service) {
        return Err(rejected("activity service URL does not match signed claim"));
    }
    Ok(())
}

pub fn extract_bearer_token(auth_header: &str) -> Option<&str> {
    auth_header.strip_prefix("Bearer ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn verification_bypass_and_oversized_tokens_are_refused_before_network() {
        assert!(validate_bot_framework_token("", "fixture", true)
            .await
            .is_err());
        assert!(
            validate_bot_framework_token(&"x".repeat(16 * 1024 + 1), "fixture", false)
                .await
                .is_err()
        );
    }

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
