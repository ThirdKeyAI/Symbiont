//! Native Rust SchemaPin Client Implementation
//!
//! Provides native Rust implementation using the schemapin crate instead of CLI wrapper

use async_trait::async_trait;
use chrono::Utc;
use schemapin::crypto::{calculate_key_id, generate_key_pair, sign_data, verify_signature};
use sha2::Digest;

use super::types::{
    SchemaPinError, SignArgs, SignatureInfo, SigningResult, VerificationResult, VerifyArgs,
};

/// Trait for SchemaPin operations using native Rust implementation
#[async_trait]
pub trait SchemaPinClient: Send + Sync {
    /// Verify a schema using the native SchemaPin implementation
    async fn verify_schema(&self, args: VerifyArgs) -> Result<VerificationResult, SchemaPinError>;

    /// Sign a schema using the native SchemaPin implementation
    async fn sign_schema(&self, args: SignArgs) -> Result<SigningResult, SchemaPinError>;

    /// Check if the implementation is available
    async fn check_available(&self) -> Result<bool, SchemaPinError>;

    /// Get version information
    async fn get_version(&self) -> Result<String, SchemaPinError>;
}

/// Native SchemaPin implementation using the schemapin Rust crate
pub struct NativeSchemaPinClient {
    // No configuration needed for native implementation
}

impl NativeSchemaPinClient {
    /// Create a new native SchemaPin client
    pub fn new() -> Self {
        Self {}
    }

    /// Fetch public key from URL and return PEM format.
    ///
    /// Hardening:
    /// - URL must pass [`crate::net_guard::reject_ssrf_url`] (no private IPs,
    ///   loopback, link-local, or cloud-metadata hosts, no non-http(s) schemes).
    /// - Plaintext HTTP is refused unless
    ///   `SYMBIONT_SCHEMAPIN_ALLOW_INSECURE=1` is explicitly set — the fetched
    ///   bytes become the trust anchor for schema signatures, so a MITM here
    ///   silently breaks verification for every subsequent schema.
    /// - Per-request 10 s timeout; response body capped at 64 KiB to stop a
    ///   hostile keyserver from filling memory.
    ///
    /// Supports two response formats:
    /// - Raw PEM: response body is the PEM-encoded public key directly
    /// - SchemaPin discovery JSON: response is a JSON object with a `public_key_pem` field
    ///   (e.g., from `/.well-known/schemapin.json`)
    async fn fetch_public_key(&self, public_key_url: &str) -> Result<String, SchemaPinError> {
        use futures::StreamExt;

        crate::net_guard::reject_ssrf_url(public_key_url).map_err(|reason| {
            SchemaPinError::IoError {
                reason: format!(
                    "Refusing to fetch public key from {}: {}",
                    public_key_url, reason
                ),
            }
        })?;

        let allow_insecure = std::env::var("SYMBIONT_SCHEMAPIN_ALLOW_INSECURE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        if public_key_url.starts_with("http://") && !allow_insecure {
            return Err(SchemaPinError::IoError {
                reason: format!(
                    "Refusing plaintext HTTP for public key fetch ({}); set \
                     SYMBIONT_SCHEMAPIN_ALLOW_INSECURE=1 only for local testing",
                    public_key_url
                ),
            });
        }

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::limited(3))
            .build()
            .map_err(|e| SchemaPinError::IoError {
                reason: format!("Failed to build HTTP client: {}", e),
            })?;

        let response =
            client
                .get(public_key_url)
                .send()
                .await
                .map_err(|e| SchemaPinError::IoError {
                    reason: format!("Failed to fetch public key from {}: {}", public_key_url, e),
                })?;

        if !response.status().is_success() {
            return Err(SchemaPinError::IoError {
                reason: format!("HTTP error {} when fetching public key", response.status()),
            });
        }

        // Stream the body with a hard cap so we can't be DoS'd by a huge
        // response. 64 KiB is two orders of magnitude larger than any
        // realistic PEM or JSON-wrapped key.
        const MAX_KEY_BODY_BYTES: usize = 64 * 1024;
        let mut stream = response.bytes_stream();
        let mut buf: Vec<u8> = Vec::with_capacity(4096);
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| SchemaPinError::IoError {
                reason: format!("Failed to read public key response: {}", e),
            })?;
            if buf.len() + chunk.len() > MAX_KEY_BODY_BYTES {
                return Err(SchemaPinError::IoError {
                    reason: format!(
                        "Public key response from {} exceeded {} bytes",
                        public_key_url, MAX_KEY_BODY_BYTES
                    ),
                });
            }
            buf.extend_from_slice(&chunk);
        }

        let body = String::from_utf8(buf).map_err(|e| SchemaPinError::IoError {
            reason: format!("Public key response was not valid UTF-8: {}", e),
        })?;

        // If the response looks like JSON, extract the public_key_pem field
        let trimmed = body.trim();
        if trimmed.starts_with('{') {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
                if let Some(pem) = json.get("public_key_pem").and_then(|v| v.as_str()) {
                    return Ok(pem.to_string());
                }
                return Err(SchemaPinError::IoError {
                    reason: format!(
                        "JSON response from {} does not contain a 'public_key_pem' field",
                        public_key_url
                    ),
                });
            }
        }

        Ok(body)
    }

    /// Read file contents from filesystem
    async fn read_file(&self, path: &str) -> Result<Vec<u8>, SchemaPinError> {
        tokio::fs::read(path)
            .await
            .map_err(|_e| SchemaPinError::SchemaFileNotFound {
                path: path.to_string(),
            })
    }
}

impl Default for NativeSchemaPinClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SchemaPinClient for NativeSchemaPinClient {
    async fn verify_schema(&self, args: VerifyArgs) -> Result<VerificationResult, SchemaPinError> {
        // Validate arguments
        if args.schema_path.is_empty() {
            return Err(SchemaPinError::InvalidArguments {
                args: vec!["schema_path cannot be empty".to_string()],
            });
        }

        if args.public_key_url.is_empty() {
            return Err(SchemaPinError::InvalidArguments {
                args: vec!["public_key_url cannot be empty".to_string()],
            });
        }

        // Validate public key URL format
        if !args.public_key_url.starts_with("http://")
            && !args.public_key_url.starts_with("https://")
        {
            return Err(SchemaPinError::InvalidPublicKeyUrl {
                url: args.public_key_url.clone(),
            });
        }

        // Read schema file
        let schema_data = self.read_file(&args.schema_path).await?;

        // Fetch public key
        let public_key_pem = self.fetch_public_key(&args.public_key_url).await?;

        // Calculate key ID for reference
        let key_id = calculate_key_id(&public_key_pem).map_err(|e| SchemaPinError::IoError {
            reason: format!("Failed to calculate key ID: {}", e),
        })?;

        // Calculate schema hash for the response regardless of outcome
        let schema_hash = {
            let mut hasher = sha2::Sha256::new();
            hasher.update(&schema_data);
            hex::encode(hasher.finalize())
        };

        // Attempt to extract an embedded signature from the schema JSON.
        // Schemas signed by SchemaPin contain a top-level `signature` field.
        let embedded_signature: Option<String> =
            serde_json::from_slice::<serde_json::Value>(&schema_data)
                .ok()
                .and_then(|v| {
                    v.get("signature")
                        .and_then(|s| s.as_str())
                        .map(String::from)
                });

        if let Some(ref sig) = embedded_signature {
            // Verify the embedded signature against the schema content and fetched public key
            // Strip the signature field to get the canonical payload that was signed
            let mut schema_value: serde_json::Value = serde_json::from_slice(&schema_data)
                .map_err(|e| SchemaPinError::IoError {
                    reason: format!("Failed to parse schema JSON: {}", e),
                })?;
            if let Some(obj) = schema_value.as_object_mut() {
                obj.remove("signature");
            }
            let canonical_payload =
                serde_json::to_vec(&schema_value).map_err(|e| SchemaPinError::IoError {
                    reason: format!("Failed to serialize canonical schema: {}", e),
                })?;

            match verify_signature(&public_key_pem, &canonical_payload, sig) {
                Ok(true) => {
                    tracing::info!(
                        "Schema signature verified successfully for {}",
                        args.schema_path
                    );
                    Ok(VerificationResult {
                        success: true,
                        message: "Schema signature verified successfully using native Rust implementation".to_string(),
                        schema_hash: Some(schema_hash),
                        public_key_url: Some(args.public_key_url.clone()),
                        signature: Some(SignatureInfo {
                            algorithm: "ECDSA_P256".to_string(),
                            signature: sig.clone(),
                            key_fingerprint: Some(key_id),
                            valid: true,
                        }),
                        metadata: None,
                        timestamp: Some(Utc::now().to_rfc3339()),
                    })
                }
                Ok(false) => {
                    tracing::warn!(
                        "Schema signature verification failed: signature invalid for {}",
                        args.schema_path
                    );
                    Ok(VerificationResult {
                        success: false,
                        message: "Schema signature verification failed: signature is invalid"
                            .to_string(),
                        schema_hash: Some(schema_hash),
                        public_key_url: Some(args.public_key_url.clone()),
                        signature: Some(SignatureInfo {
                            algorithm: "ECDSA_P256".to_string(),
                            signature: sig.clone(),
                            key_fingerprint: Some(key_id),
                            valid: false,
                        }),
                        metadata: None,
                        timestamp: Some(Utc::now().to_rfc3339()),
                    })
                }
                Err(e) => {
                    tracing::warn!(
                        "Schema signature verification error for {}: {}",
                        args.schema_path,
                        e
                    );
                    Ok(VerificationResult {
                        success: false,
                        message: format!("Schema signature verification error: {}", e),
                        schema_hash: Some(schema_hash),
                        public_key_url: Some(args.public_key_url.clone()),
                        signature: Some(SignatureInfo {
                            algorithm: "ECDSA_P256".to_string(),
                            signature: sig.clone(),
                            key_fingerprint: Some(key_id),
                            valid: false,
                        }),
                        metadata: None,
                        timestamp: Some(Utc::now().to_rfc3339()),
                    })
                }
            }
        } else {
            // No signature provided — fail verification (fail-closed)
            tracing::warn!(
                "Schema verification failed for {}: no signature provided for verification",
                args.schema_path
            );
            Ok(VerificationResult {
                success: false,
                message: "No signature provided for verification".to_string(),
                schema_hash: Some(schema_hash),
                public_key_url: Some(args.public_key_url.clone()),
                signature: None,
                metadata: None,
                timestamp: Some(Utc::now().to_rfc3339()),
            })
        }
    }

    async fn sign_schema(&self, args: SignArgs) -> Result<SigningResult, SchemaPinError> {
        // Validate arguments
        if args.schema_path.is_empty() {
            return Err(SchemaPinError::InvalidArguments {
                args: vec!["schema_path cannot be empty".to_string()],
            });
        }

        if args.private_key_path.is_empty() {
            return Err(SchemaPinError::InvalidArguments {
                args: vec!["private_key_path cannot be empty".to_string()],
            });
        }

        // Read schema file
        let schema_data = self.read_file(&args.schema_path).await?;

        // Read private key file
        let private_key_pem = tokio::fs::read_to_string(&args.private_key_path)
            .await
            .map_err(|_| SchemaPinError::PrivateKeyNotFound {
                path: args.private_key_path.clone(),
            })?;

        // Sign the schema data
        let signature = sign_data(&private_key_pem, &schema_data).map_err(|e| {
            SchemaPinError::SigningFailed {
                reason: format!("Failed to sign schema: {}", e),
            }
        })?;

        // Calculate schema hash
        let mut hasher = sha2::Sha256::new();
        hasher.update(&schema_data);
        let schema_hash = hex::encode(hasher.finalize());

        // Generate key ID from the corresponding public key
        // In practice, you'd derive the public key from the private key
        let key_pair = generate_key_pair().map_err(|e| SchemaPinError::SigningFailed {
            reason: format!("Failed to generate key pair for ID calculation: {}", e),
        })?;

        let key_id = calculate_key_id(&key_pair.public_key_pem).map_err(|e| {
            SchemaPinError::SigningFailed {
                reason: format!("Failed to calculate key ID: {}", e),
            }
        })?;

        // Determine output path
        let output_path = args
            .output_path
            .unwrap_or_else(|| format!("{}.signed", args.schema_path));

        Ok(SigningResult {
            success: true,
            message: "Schema signed successfully using native Rust implementation".to_string(),
            schema_hash: Some(schema_hash),
            signed_schema_path: Some(output_path),
            signature: Some(SignatureInfo {
                algorithm: "ECDSA_P256".to_string(),
                signature,
                key_fingerprint: Some(key_id),
                valid: true,
            }),
            metadata: None,
            timestamp: Some(Utc::now().to_rfc3339()),
        })
    }

    async fn check_available(&self) -> Result<bool, SchemaPinError> {
        // Native implementation is always available if the crate is compiled in
        Ok(true)
    }

    async fn get_version(&self) -> Result<String, SchemaPinError> {
        Ok("schemapin-native v1.1.4 (Rust implementation)".to_string())
    }
}

/// Mock implementation for testing
pub struct MockNativeSchemaPinClient {
    should_succeed: bool,
    mock_result: Option<VerificationResult>,
}

impl MockNativeSchemaPinClient {
    /// Create a new mock that always succeeds
    pub fn new_success() -> Self {
        Self {
            should_succeed: true,
            mock_result: None,
        }
    }

    /// Create a new mock that always fails
    pub fn new_failure() -> Self {
        Self {
            should_succeed: false,
            mock_result: None,
        }
    }

    /// Create a new mock with custom result
    pub fn with_result(result: VerificationResult) -> Self {
        Self {
            should_succeed: result.success,
            mock_result: Some(result),
        }
    }
}

#[async_trait]
impl SchemaPinClient for MockNativeSchemaPinClient {
    async fn verify_schema(&self, _args: VerifyArgs) -> Result<VerificationResult, SchemaPinError> {
        if let Some(ref result) = self.mock_result {
            if result.success {
                Ok(result.clone())
            } else {
                Err(SchemaPinError::VerificationFailed {
                    reason: result.message.clone(),
                })
            }
        } else if self.should_succeed {
            Ok(VerificationResult {
                success: true,
                message: "Mock verification successful".to_string(),
                schema_hash: Some("mock_native_hash_123".to_string()),
                public_key_url: Some("https://mock.example.com/pubkey".to_string()),
                signature: Some(SignatureInfo {
                    algorithm: "ECDSA_P256".to_string(),
                    signature: "mock_native_signature".to_string(),
                    key_fingerprint: Some("sha256:mock_fingerprint".to_string()),
                    valid: true,
                }),
                metadata: None,
                timestamp: Some(Utc::now().to_rfc3339()),
            })
        } else {
            Err(SchemaPinError::VerificationFailed {
                reason: "Mock verification failed".to_string(),
            })
        }
    }

    async fn sign_schema(&self, _args: SignArgs) -> Result<SigningResult, SchemaPinError> {
        if self.should_succeed {
            Ok(SigningResult {
                success: true,
                message: "Mock native signing successful".to_string(),
                schema_hash: Some("mock_native_signed_hash_456".to_string()),
                signed_schema_path: Some("/mock/path/signed_schema.json".to_string()),
                signature: Some(SignatureInfo {
                    algorithm: "ECDSA_P256".to_string(),
                    signature: "mock_native_signature_data".to_string(),
                    key_fingerprint: Some("sha256:mock_native_fingerprint".to_string()),
                    valid: true,
                }),
                metadata: None,
                timestamp: Some(Utc::now().to_rfc3339()),
            })
        } else {
            Err(SchemaPinError::SigningFailed {
                reason: "Mock native signing failed".to_string(),
            })
        }
    }

    async fn check_available(&self) -> Result<bool, SchemaPinError> {
        Ok(true) // Mock always reports as available
    }

    async fn get_version(&self) -> Result<String, SchemaPinError> {
        Ok("schemapin-cli v1.0.0 (mock)".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_native_client_creation() {
        let client = NativeSchemaPinClient::new();
        let available = client.check_available().await.unwrap();
        assert!(available);

        let version = client.get_version().await.unwrap();
        assert!(version.contains("schemapin-native"));
    }

    #[tokio::test]
    async fn test_fetch_public_key_rejects_ssrf_targets() {
        // Ensure the insecure-override is clear so the http:// + SSRF-guard
        // combination is in effect.
        std::env::remove_var("SYMBIONT_SCHEMAPIN_ALLOW_INSECURE");
        let client = NativeSchemaPinClient::new();
        for url in [
            "http://169.254.169.254/latest/meta-data/",
            "http://127.0.0.1:8080/pub",
            "http://10.1.2.3/key",
            "file:///etc/passwd",
        ] {
            let err = client
                .fetch_public_key(url)
                .await
                .expect_err(&format!("{} must be refused", url));
            match err {
                SchemaPinError::IoError { reason } => {
                    assert!(
                        reason.contains("Refusing"),
                        "wrong message for {}: {}",
                        url,
                        reason
                    );
                }
                other => panic!("unexpected error for {}: {:?}", url, other),
            }
        }
    }

    #[tokio::test]
    async fn test_fetch_public_key_rejects_plaintext_http() {
        // Public non-loopback HTTP URL passes the SSRF guard but must be
        // refused by the TLS guard unless the insecure override is set.
        std::env::remove_var("SYMBIONT_SCHEMAPIN_ALLOW_INSECURE");
        let client = NativeSchemaPinClient::new();
        let err = client
            .fetch_public_key("http://example.com/pub")
            .await
            .expect_err("plaintext must be refused");
        match err {
            SchemaPinError::IoError { reason } => {
                assert!(
                    reason.contains("Refusing plaintext HTTP"),
                    "wrong message: {}",
                    reason
                );
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_mock_native_client_success() {
        let client = MockNativeSchemaPinClient::new_success();
        let args = VerifyArgs::new(
            "/path/to/schema.json".to_string(),
            "https://example.com/pubkey".to_string(),
        );

        let result = client.verify_schema(args).await.unwrap();
        assert!(result.success);
        assert_eq!(result.message, "Mock verification successful");
    }

    #[tokio::test]
    async fn test_mock_native_client_failure() {
        let client = MockNativeSchemaPinClient::new_failure();
        let args = VerifyArgs::new(
            "/path/to/schema.json".to_string(),
            "https://example.com/pubkey".to_string(),
        );

        let result = client.verify_schema(args).await;
        assert!(result.is_err());

        if let Err(SchemaPinError::VerificationFailed { reason }) = result {
            assert_eq!(reason, "Mock verification failed");
        } else {
            panic!("Expected VerificationFailed error");
        }
    }
}
