//! Webhook signature verification for inbound HTTP requests.
//!
//! Provides a [`SignatureVerifier`] trait with implementations for HMAC-SHA256
//! and JWT-based verification, plus [`WebhookProvider`] presets for GitHub,
//! Stripe, Slack, and custom webhook sources.

use async_trait::async_trait;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use thiserror::Error;

type HmacSha256 = Hmac<Sha256>;

/// Errors that can occur during webhook signature verification.
#[derive(Debug, Error)]
pub enum VerifyError {
    /// A required header was not present in the request.
    #[error("missing header: {0}")]
    MissingHeader(String),

    /// The signature header value could not be parsed or decoded.
    #[error("invalid signature: {0}")]
    InvalidSignature(String),

    /// The computed signature did not match the provided signature.
    #[error("verification failed: {0}")]
    VerificationFailed(String),
}

/// Trait for verifying webhook request signatures.
///
/// Implementations inspect request headers and body to verify authenticity.
#[async_trait]
pub trait SignatureVerifier: Send + Sync {
    /// Verify that the request is authentic.
    ///
    /// # Arguments
    /// * `headers` — request headers as `(name, value)` pairs
    /// * `body` — raw request body bytes
    async fn verify(&self, headers: &[(String, String)], body: &[u8]) -> Result<(), VerifyError>;
}

/// HMAC-SHA256 signature verifier.
///
/// Computes `HMAC-SHA256(secret, body)` and compares it (in constant time)
/// against the signature found in the configured header.
pub struct HmacVerifier {
    secret: Vec<u8>,
    header_name: String,
    prefix: Option<String>,
}

impl HmacVerifier {
    /// Create a new HMAC verifier.
    ///
    /// # Arguments
    /// * `secret` — the shared HMAC secret
    /// * `header_name` — HTTP header that carries the signature
    /// * `prefix` — optional prefix on the header value (e.g. `"sha256="`)
    pub fn new(secret: Vec<u8>, header_name: String, prefix: Option<String>) -> Self {
        Self {
            secret,
            header_name,
            prefix,
        }
    }

    /// Find a header value by name (case-insensitive).
    fn find_header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
        let name_lower = name.to_lowercase();
        headers
            .iter()
            .find(|(k, _)| k.to_lowercase() == name_lower)
            .map(|(_, v)| v.as_str())
    }
}

#[async_trait]
impl SignatureVerifier for HmacVerifier {
    async fn verify(&self, headers: &[(String, String)], body: &[u8]) -> Result<(), VerifyError> {
        let header_value = Self::find_header(headers, &self.header_name)
            .ok_or_else(|| VerifyError::MissingHeader(self.header_name.clone()))?;

        // Strip prefix if configured
        let signature_hex = match &self.prefix {
            Some(prefix) => header_value.strip_prefix(prefix.as_str()).ok_or_else(|| {
                VerifyError::InvalidSignature(format!(
                    "header value does not start with expected prefix '{}'",
                    prefix
                ))
            })?,
            None => header_value,
        };

        // Decode the hex signature from the header
        let provided_sig = hex::decode(signature_hex).map_err(|e| {
            VerifyError::InvalidSignature(format!("failed to decode hex signature: {}", e))
        })?;

        // Compute HMAC-SHA256
        let mut mac = HmacSha256::new_from_slice(&self.secret)
            .map_err(|e| VerifyError::VerificationFailed(format!("HMAC init failed: {}", e)))?;
        mac.update(body);
        let computed = mac.finalize().into_bytes();

        // Constant-time comparison
        if computed.as_slice().ct_eq(&provided_sig).unwrap_u8() != 1 {
            return Err(VerifyError::VerificationFailed(
                "signature mismatch".to_string(),
            ));
        }

        Ok(())
    }
}

/// JWT signature verifier (HMAC-SHA256 symmetric).
///
/// Extracts a JWT from a request header, strips an optional `Bearer ` prefix,
/// and validates it using the `jsonwebtoken` crate.
pub struct JwtVerifier {
    secret: Vec<u8>,
    header_name: String,
    required_issuer: Option<String>,
}

impl JwtVerifier {
    /// Create a JWT verifier using HMAC-SHA256 symmetric signing.
    ///
    /// # Arguments
    /// * `secret` — the shared HMAC secret
    /// * `header_name` — HTTP header carrying the JWT (e.g. `"Authorization"`)
    /// * `required_issuer` — if set, the `iss` claim must match this value
    pub fn new_hmac(secret: Vec<u8>, header_name: String, required_issuer: Option<String>) -> Self {
        Self {
            secret,
            header_name,
            required_issuer,
        }
    }

    /// Find a header value by name (case-insensitive).
    fn find_header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
        let name_lower = name.to_lowercase();
        headers
            .iter()
            .find(|(k, _)| k.to_lowercase() == name_lower)
            .map(|(_, v)| v.as_str())
    }
}

/// JWT claims used for validation.
#[derive(Debug, serde::Deserialize)]
struct JwtClaims {
    #[serde(default)]
    iss: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    exp: Option<u64>,
}

#[async_trait]
impl SignatureVerifier for JwtVerifier {
    async fn verify(&self, headers: &[(String, String)], _body: &[u8]) -> Result<(), VerifyError> {
        let header_value = Self::find_header(headers, &self.header_name)
            .ok_or_else(|| VerifyError::MissingHeader(self.header_name.clone()))?;

        // Strip "Bearer " prefix if present
        let token = header_value.strip_prefix("Bearer ").unwrap_or(header_value);

        let decoding_key = jsonwebtoken::DecodingKey::from_secret(&self.secret);

        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.required_spec_claims = std::collections::HashSet::new();

        if let Some(ref issuer) = self.required_issuer {
            validation.set_issuer(&[issuer]);
        } else {
            validation.validate_aud = false;
        }
        // Always skip audience validation for webhook JWTs
        validation.validate_aud = false;

        let token_data = jsonwebtoken::decode::<JwtClaims>(token, &decoding_key, &validation)
            .map_err(|e| {
                VerifyError::VerificationFailed(format!("JWT validation failed: {}", e))
            })?;

        // If we required an issuer and it wasn't checked by the library, double-check
        if let Some(ref required) = self.required_issuer {
            match &token_data.claims.iss {
                Some(iss) if iss == required => {}
                Some(iss) => {
                    return Err(VerifyError::VerificationFailed(format!(
                        "issuer mismatch: expected '{}', got '{}'",
                        required, iss
                    )));
                }
                None => {
                    return Err(VerifyError::VerificationFailed(
                        "missing iss claim".to_string(),
                    ));
                }
            }
        }

        Ok(())
    }
}

/// Pre-configured webhook providers.
///
/// Each variant knows the header name and prefix conventions for a particular
/// webhook source and can produce a ready-to-use [`SignatureVerifier`].
pub enum WebhookProvider {
    /// GitHub webhook — `X-Hub-Signature-256` header, `sha256=` prefix.
    GitHub,
    /// Stripe webhook — `Stripe-Signature` header, no prefix.
    Stripe,
    /// Slack Events API — `X-Slack-Signature` header, `v0=` prefix.
    Slack,
    /// Custom webhook — `X-Signature` header, no prefix.
    Custom,
}

impl WebhookProvider {
    /// Build a [`SignatureVerifier`] for this provider using the given secret.
    pub fn verifier(&self, secret: &[u8]) -> Box<dyn SignatureVerifier> {
        match self {
            WebhookProvider::GitHub => Box::new(HmacVerifier::new(
                secret.to_vec(),
                "X-Hub-Signature-256".to_string(),
                Some("sha256=".to_string()),
            )),
            WebhookProvider::Stripe => Box::new(HmacVerifier::new(
                secret.to_vec(),
                "Stripe-Signature".to_string(),
                None,
            )),
            WebhookProvider::Slack => Box::new(HmacVerifier::new(
                secret.to_vec(),
                "X-Slack-Signature".to_string(),
                Some("v0=".to_string()),
            )),
            WebhookProvider::Custom => Box::new(HmacVerifier::new(
                secret.to_vec(),
                "X-Signature".to_string(),
                None,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    /// Helper: compute HMAC-SHA256 and return hex string.
    fn compute_hmac_hex(secret: &[u8], body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(body);
        hex::encode(mac.finalize().into_bytes())
    }

    #[tokio::test]
    async fn test_hmac_verifier_valid_signature() {
        let secret = b"test-secret";
        let body = b"hello world";
        let sig = compute_hmac_hex(secret, body);

        let verifier = HmacVerifier::new(secret.to_vec(), "X-Signature".to_string(), None);

        let headers = vec![("X-Signature".to_string(), sig)];
        assert!(verifier.verify(&headers, body).await.is_ok());
    }

    #[tokio::test]
    async fn test_hmac_verifier_with_prefix() {
        let secret = b"github-secret";
        let body = b"{\"action\":\"opened\"}";
        let sig = format!("sha256={}", compute_hmac_hex(secret, body));

        let verifier = HmacVerifier::new(
            secret.to_vec(),
            "X-Hub-Signature-256".to_string(),
            Some("sha256=".to_string()),
        );

        let headers = vec![("X-Hub-Signature-256".to_string(), sig)];
        assert!(verifier.verify(&headers, body).await.is_ok());
    }

    #[tokio::test]
    async fn test_hmac_verifier_invalid_signature() {
        let secret = b"test-secret";
        let body = b"hello world";
        let bad_sig = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

        let verifier = HmacVerifier::new(secret.to_vec(), "X-Signature".to_string(), None);

        let headers = vec![("X-Signature".to_string(), bad_sig.to_string())];
        let result = verifier.verify(&headers, body).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            VerifyError::VerificationFailed(_)
        ));
    }

    #[tokio::test]
    async fn test_hmac_verifier_missing_header() {
        let secret = b"test-secret";
        let body = b"hello world";

        let verifier = HmacVerifier::new(secret.to_vec(), "X-Signature".to_string(), None);

        let headers: Vec<(String, String)> = vec![];
        let result = verifier.verify(&headers, body).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), VerifyError::MissingHeader(_)));
    }

    #[tokio::test]
    async fn test_hmac_verifier_case_insensitive_header() {
        let secret = b"test-secret";
        let body = b"payload";
        let sig = compute_hmac_hex(secret, body);

        let verifier = HmacVerifier::new(secret.to_vec(), "X-Signature".to_string(), None);

        // Provide header in lowercase — should still match
        let headers = vec![("x-signature".to_string(), sig)];
        assert!(verifier.verify(&headers, body).await.is_ok());
    }

    #[tokio::test]
    async fn test_github_provider_preset() {
        let secret = b"gh-webhook-secret";
        let body = b"{\"ref\":\"refs/heads/main\"}";
        let sig = format!("sha256={}", compute_hmac_hex(secret, body));

        let verifier = WebhookProvider::GitHub.verifier(secret);

        let headers = vec![("X-Hub-Signature-256".to_string(), sig)];
        assert!(verifier.verify(&headers, body).await.is_ok());
    }

    #[tokio::test]
    async fn test_jwt_verifier_valid_token() {
        use jsonwebtoken::{encode, EncodingKey, Header};

        let secret = b"jwt-test-secret";
        let now = chrono::Utc::now().timestamp() as u64;

        #[derive(serde::Serialize)]
        struct Claims {
            iss: String,
            exp: u64,
        }

        let claims = Claims {
            iss: "test-issuer".to_string(),
            exp: now + 3600,
        };

        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret),
        )
        .unwrap();

        let verifier = JwtVerifier::new_hmac(
            secret.to_vec(),
            "Authorization".to_string(),
            Some("test-issuer".to_string()),
        );

        let headers = vec![("Authorization".to_string(), format!("Bearer {}", token))];
        assert!(verifier.verify(&headers, b"").await.is_ok());
    }

    #[tokio::test]
    async fn test_jwt_verifier_expired_token() {
        use jsonwebtoken::{encode, EncodingKey, Header};

        let secret = b"jwt-test-secret";

        #[derive(serde::Serialize)]
        struct Claims {
            iss: String,
            exp: u64,
        }

        let claims = Claims {
            iss: "test-issuer".to_string(),
            exp: 1_000_000, // long expired
        };

        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret),
        )
        .unwrap();

        let verifier = JwtVerifier::new_hmac(
            secret.to_vec(),
            "Authorization".to_string(),
            Some("test-issuer".to_string()),
        );

        let headers = vec![("Authorization".to_string(), format!("Bearer {}", token))];
        let result = verifier.verify(&headers, b"").await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            VerifyError::VerificationFailed(_)
        ));
    }
}
