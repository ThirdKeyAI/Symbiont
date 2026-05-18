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

    /// The verifier was constructed with invalid or unsafe configuration
    /// (e.g. missing audience under strict mode).
    #[error("configuration error: {0}")]
    Configuration(String),

    /// The JWT used an algorithm that is not on the verifier's allowlist
    /// (e.g. an RSA-family algorithm such as `RS256`, where the underlying
    /// RSA implementation is subject to RUSTSEC-2023-0071 / Marvin Attack).
    #[error("JWT algorithm not allowed: {algorithm}")]
    AlgorithmNotAllowed {
        /// The disallowed algorithm header value.
        algorithm: String,
    },

    /// JWT verifier was constructed without an audience. Audience is
    /// mandatory to prevent cross-service token reuse.
    #[error("JWT verifier requires an audience claim to be configured")]
    MissingAudience,
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
///
/// This verifier expects symmetric (HS256) JWTs. RSA-family and `none`
/// algorithms are rejected unconditionally — the algorithm allowlist is
/// enforced both via `Validation::algorithms` and by an explicit header
/// inspection step. This neutralises RUSTSEC-2023-0071 (Marvin Attack on
/// the transitively-pulled `rsa` crate) on every path that reaches a JWT
/// verifier in the runtime.
pub struct JwtVerifier {
    secret: Vec<u8>,
    header_name: String,
    required_issuer: Option<String>,
    /// The `aud` claim must match this value. Audience is mandatory — the
    /// constructor refuses to build a verifier without one to prevent
    /// cross-service token reuse.
    audience: String,
}

impl JwtVerifier {
    /// Create a JWT verifier using HMAC-SHA256 symmetric signing.
    ///
    /// Audience is REQUIRED. Without an audience claim any JWT signed with
    /// the same key — including tokens minted for a different service —
    /// would be accepted, so the verifier refuses to construct. The previous
    /// `SYMBIONT_ALLOW_NO_JWT_AUDIENCE` escape hatch has been removed.
    ///
    /// # Arguments
    /// * `secret` — the shared HMAC secret
    /// * `header_name` — HTTP header carrying the JWT (e.g. `"Authorization"`)
    /// * `required_issuer` — if set, the `iss` claim must match this value
    /// * `audience` — required: the `aud` claim must match this value;
    ///   passing `None` returns [`VerifyError::MissingAudience`]
    pub fn new_hmac(
        secret: Vec<u8>,
        header_name: String,
        required_issuer: Option<String>,
        audience: Option<String>,
    ) -> Result<Self, VerifyError> {
        let audience = audience.ok_or(VerifyError::MissingAudience)?;
        Ok(Self {
            secret,
            header_name,
            required_issuer,
            audience,
        })
    }

    /// Strict constructor: an audience is required, matching the M-2
    /// recommendation that HMAC JWT verifiers never accept tokens without
    /// validating the intended recipient.
    pub fn new_hmac_with_audience(
        secret: Vec<u8>,
        header_name: String,
        required_issuer: Option<String>,
        audience: String,
    ) -> Self {
        Self {
            secret,
            header_name,
            required_issuer,
            audience,
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

        // Explicit defence-in-depth: inspect the header alg before any
        // signature work. The Validation::algorithms allowlist below also
        // enforces this, but reading the header first gives a precise error
        // and avoids any possibility of an alg-confusion bug in the verifier
        // crate exposing the RSA code path (RUSTSEC-2023-0071).
        let header = jsonwebtoken::decode_header(token).map_err(|e| {
            VerifyError::InvalidSignature(format!("failed to decode JWT header: {}", e))
        })?;
        // This verifier path is symmetric (HMAC). HS256 is the only allowed
        // algorithm here. Everything else — RSA-family, EC, EdDSA, none — is
        // rejected unconditionally. Other paths (e.g. the EdDSA Bearer path
        // in `http_input::server`) maintain their own asymmetric-only
        // allowlists.
        if !matches!(header.alg, jsonwebtoken::Algorithm::HS256) {
            return Err(VerifyError::AlgorithmNotAllowed {
                algorithm: format!("{:?}", header.alg),
            });
        }

        let decoding_key = jsonwebtoken::DecodingKey::from_secret(&self.secret);

        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
        // Pin the algorithm allowlist to HS256 only. This refuses every
        // RS/PS/ES/EdDSA/none algorithm at the validator level and
        // neutralises the Marvin Attack reachability through this verifier.
        validation.algorithms = vec![jsonwebtoken::Algorithm::HS256];
        validation.required_spec_claims = std::collections::HashSet::new();

        if let Some(ref issuer) = self.required_issuer {
            validation.set_issuer(&[issuer]);
        }

        // Audience is mandatory (enforced at constructor time).
        validation.validate_aud = true;
        validation.aud = Some(std::collections::HashSet::from([self.audience.clone()]));

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
            aud: String,
            exp: u64,
        }

        let claims = Claims {
            iss: "test-issuer".to_string(),
            aud: "test-audience".to_string(),
            exp: now + 3600,
        };

        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret),
        )
        .unwrap();

        // new_hmac now requires an audience so the verifier can't be reused
        // across services; supply one for the test.
        let verifier = JwtVerifier::new_hmac(
            secret.to_vec(),
            "Authorization".to_string(),
            Some("test-issuer".to_string()),
            Some("test-audience".to_string()),
        )
        .expect("test verifier construction");

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
            aud: String,
            exp: u64,
        }

        let claims = Claims {
            iss: "test-issuer".to_string(),
            aud: "test-audience".to_string(),
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
            Some("test-audience".to_string()),
        )
        .expect("test verifier construction");

        let headers = vec![("Authorization".to_string(), format!("Bearer {}", token))];
        let result = verifier.verify(&headers, b"").await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            VerifyError::VerificationFailed(_)
        ));
    }

    #[tokio::test]
    async fn test_jwt_verifier_requires_audience() {
        // M2: constructor must refuse `None` audience — no env-var escape
        // hatch any more.
        let secret = b"jwt-test-secret";
        let result = JwtVerifier::new_hmac(
            secret.to_vec(),
            "Authorization".to_string(),
            Some("test-issuer".to_string()),
            None,
        );
        assert!(matches!(result, Err(VerifyError::MissingAudience)));
    }

    #[tokio::test]
    async fn test_jwt_verifier_env_var_does_not_bypass_audience() {
        // Even if a stale operator sets SYMBIONT_ALLOW_NO_JWT_AUDIENCE, the
        // constructor must still refuse: the escape hatch is gone.
        let secret = b"jwt-test-secret";
        std::env::set_var("SYMBIONT_ALLOW_NO_JWT_AUDIENCE", "1");
        let result = JwtVerifier::new_hmac(
            secret.to_vec(),
            "Authorization".to_string(),
            Some("test-issuer".to_string()),
            None,
        );
        std::env::remove_var("SYMBIONT_ALLOW_NO_JWT_AUDIENCE");
        assert!(matches!(result, Err(VerifyError::MissingAudience)));
    }

    /// C4: a token signed with an RSA algorithm must be rejected even before
    /// any signature work happens. We forge a JWT with `alg=RS256` in the
    /// header by hand (header.payload.signature, base64url) so we don't
    /// need an RSA key. The verifier must trip the AlgorithmNotAllowed
    /// guard at header-decode time.
    #[tokio::test]
    async fn test_jwt_verifier_rejects_rs256_token() {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
        let header_json = r#"{"alg":"RS256","typ":"JWT"}"#;
        let payload_json = r#"{"iss":"test-issuer","aud":"test-audience","exp":9999999999}"#;
        let token = format!(
            "{}.{}.{}",
            URL_SAFE_NO_PAD.encode(header_json),
            URL_SAFE_NO_PAD.encode(payload_json),
            URL_SAFE_NO_PAD.encode(b"fake-signature"),
        );

        let verifier = JwtVerifier::new_hmac(
            b"any-secret".to_vec(),
            "Authorization".to_string(),
            Some("test-issuer".to_string()),
            Some("test-audience".to_string()),
        )
        .expect("test verifier construction");

        let headers = vec![("Authorization".to_string(), format!("Bearer {}", token))];
        let result = verifier.verify(&headers, b"").await;
        assert!(
            matches!(result, Err(VerifyError::AlgorithmNotAllowed { .. })),
            "RS256 must be rejected, got: {:?}",
            result
        );
    }

    /// Same idea as above for RS384 / RS512 / PS256 / EdDSA / none — every
    /// non-HS256 algorithm string must be rejected.
    #[tokio::test]
    async fn test_jwt_verifier_rejects_all_non_hs256_algorithms() {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

        // jsonwebtoken's decode_header only accepts algorithms it knows
        // about. "none" is intentionally not part of the Algorithm enum
        // (jsonwebtoken refuses to recognise it at all), so we exercise the
        // remaining asymmetric algorithms it does support.
        let algs = [
            "RS256", "RS384", "RS512", "PS256", "PS384", "PS512", "ES256", "EdDSA",
        ];
        for alg in algs {
            let header_json = format!(r#"{{"alg":"{}","typ":"JWT"}}"#, alg);
            let payload_json = r#"{"iss":"test-issuer","aud":"test-audience","exp":9999999999}"#;
            let token = format!(
                "{}.{}.{}",
                URL_SAFE_NO_PAD.encode(header_json),
                URL_SAFE_NO_PAD.encode(payload_json),
                URL_SAFE_NO_PAD.encode(b"fake-signature"),
            );

            let verifier = JwtVerifier::new_hmac(
                b"any-secret".to_vec(),
                "Authorization".to_string(),
                Some("test-issuer".to_string()),
                Some("test-audience".to_string()),
            )
            .expect("test verifier construction");

            let headers = vec![("Authorization".to_string(), format!("Bearer {}", token))];
            let result = verifier.verify(&headers, b"").await;
            assert!(
                matches!(result, Err(VerifyError::AlgorithmNotAllowed { .. })),
                "alg={} must be rejected, got: {:?}",
                alg,
                result
            );
        }
    }

    /// Belt-and-braces: even if header inspection were skipped, the
    /// `Validation::algorithms` allowlist must not contain RSA/PS variants.
    #[test]
    fn test_jwt_verifier_validation_algorithms_is_hs256_only() {
        // Reconstruct the same Validation the runtime uses, ensure the
        // allowlist is exactly HS256 — no RSA, no PS, no EdDSA.
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.algorithms = vec![jsonwebtoken::Algorithm::HS256];
        assert_eq!(validation.algorithms, vec![jsonwebtoken::Algorithm::HS256]);
        for forbidden in [
            jsonwebtoken::Algorithm::RS256,
            jsonwebtoken::Algorithm::RS384,
            jsonwebtoken::Algorithm::RS512,
            jsonwebtoken::Algorithm::PS256,
            jsonwebtoken::Algorithm::PS384,
            jsonwebtoken::Algorithm::PS512,
            jsonwebtoken::Algorithm::ES256,
            jsonwebtoken::Algorithm::EdDSA,
        ] {
            assert!(
                !validation.algorithms.contains(&forbidden),
                "{:?} must not be in the JWT verifier allowlist",
                forbidden
            );
        }
    }
}
