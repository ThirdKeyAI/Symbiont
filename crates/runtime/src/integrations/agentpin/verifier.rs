//! AgentPin Verifier
//!
//! Provides trait and implementations for verifying AgentPin credentials.

use std::fs;

use async_trait::async_trait;

use agentpin::resolver::{
    ChainResolver, DiscoveryResolver, LocalFileResolver, TrustBundleResolver,
};
use agentpin::types::bundle::TrustBundle;
use agentpin::verification::{VerificationResult as ApVerificationResult, VerifierConfig};

use super::discovery::DiscoveryCache;
use super::key_store::AgentPinKeyStore;
use super::types::{AgentPinConfig, AgentPinError, AgentVerificationResult, DiscoveryMode};

/// Trait for verifying AgentPin credentials
#[async_trait]
pub trait AgentPinVerifier: Send + Sync {
    /// Verify a JWT credential and return the verification result
    async fn verify_credential(&self, jwt: &str) -> Result<AgentVerificationResult, AgentPinError>;
}

/// Default verifier that delegates to the agentpin crate's verification engine.
///
/// Dispatches to the appropriate resolver based on `DiscoveryMode`.
pub struct DefaultAgentPinVerifier {
    config: AgentPinConfig,
    key_store: AgentPinKeyStore,
    sync_resolver: Option<Box<dyn DiscoveryResolver>>,
}

impl DefaultAgentPinVerifier {
    /// Create a new verifier from configuration.
    ///
    /// Pre-loads trust bundle and builds resolvers based on `discovery_mode`.
    pub fn new(config: AgentPinConfig) -> Result<Self, AgentPinError> {
        let key_store = AgentPinKeyStore::new(&config.key_store_path).map_err(|e| {
            AgentPinError::KeyStoreError {
                reason: e.to_string(),
            }
        })?;

        let sync_resolver = Self::build_sync_resolver(&config)?;

        Ok(Self {
            config,
            key_store,
            sync_resolver,
        })
    }

    /// Build a sync resolver from config, if applicable.
    fn build_sync_resolver(
        config: &AgentPinConfig,
    ) -> Result<Option<Box<dyn DiscoveryResolver>>, AgentPinError> {
        match config.discovery_mode {
            DiscoveryMode::Bundle => {
                let path = config.trust_bundle_path.as_ref().ok_or_else(|| {
                    AgentPinError::ConfigError {
                        reason: "trust_bundle_path required for bundle mode".to_string(),
                    }
                })?;
                let json = fs::read_to_string(path).map_err(|e| AgentPinError::IoError {
                    reason: format!("Failed to read trust bundle: {}", e),
                })?;
                let bundle: TrustBundle =
                    serde_json::from_str(&json).map_err(|e| AgentPinError::ConfigError {
                        reason: format!("Invalid trust bundle JSON: {}", e),
                    })?;
                Ok(Some(Box::new(TrustBundleResolver::new(&bundle))))
            }
            DiscoveryMode::Local => {
                let dir = config.local_discovery_dir.as_ref().ok_or_else(|| {
                    AgentPinError::ConfigError {
                        reason: "local_discovery_dir required for local mode".to_string(),
                    }
                })?;
                Ok(Some(Box::new(LocalFileResolver::new(
                    dir,
                    config.local_revocation_dir.as_deref(),
                ))))
            }
            DiscoveryMode::Chain => {
                let mut resolvers: Vec<Box<dyn DiscoveryResolver>> = Vec::new();

                if let Some(ref path) = config.trust_bundle_path {
                    if let Ok(json) = fs::read_to_string(path) {
                        if let Ok(bundle) = serde_json::from_str::<TrustBundle>(&json) {
                            resolvers.push(Box::new(TrustBundleResolver::new(&bundle)));
                        }
                    }
                }

                if let Some(ref dir) = config.local_discovery_dir {
                    resolvers.push(Box::new(LocalFileResolver::new(
                        dir,
                        config.local_revocation_dir.as_deref(),
                    )));
                }

                if resolvers.is_empty() {
                    // Chain with no sync resolvers — fall back to WellKnown
                    Ok(None)
                } else {
                    Ok(Some(Box::new(ChainResolver::new(resolvers))))
                }
            }
            DiscoveryMode::WellKnown => Ok(None),
        }
    }

    /// Convert agentpin crate's VerificationResult to our integration type
    fn convert_result(result: &ApVerificationResult) -> AgentVerificationResult {
        if result.valid {
            let capabilities = result
                .capabilities
                .as_ref()
                .map(|caps| caps.iter().map(|c| c.to_string()).collect())
                .unwrap_or_default();

            AgentVerificationResult {
                valid: true,
                agent_id: result.agent_id.clone(),
                issuer: result.issuer.clone(),
                capabilities,
                delegation_verified: result.delegation_verified,
                error_message: None,
                warnings: result.warnings.clone(),
            }
        } else {
            AgentVerificationResult {
                valid: false,
                agent_id: result.agent_id.clone(),
                issuer: result.issuer.clone(),
                capabilities: vec![],
                delegation_verified: None,
                error_message: result.error_message.clone(),
                warnings: result.warnings.clone(),
            }
        }
    }
}

#[async_trait]
impl AgentPinVerifier for DefaultAgentPinVerifier {
    async fn verify_credential(&self, jwt: &str) -> Result<AgentVerificationResult, AgentPinError> {
        let verifier_config = VerifierConfig {
            clock_skew_secs: self.config.clock_skew_secs,
            max_ttl_secs: self.config.max_ttl_secs,
        };

        let audience = self.config.audience.as_deref();

        let mut pin_store = self.key_store.load_pin_store()?;

        let result = if let Some(ref resolver) = self.sync_resolver {
            // Use the sync resolver (bundle, local, or chain)
            agentpin::verification::verify_credential_with_resolver(
                jwt,
                resolver.as_ref(),
                &mut pin_store,
                audience,
                &verifier_config,
            )
        } else {
            // Fall back to online WellKnown resolution
            agentpin::verification::verify_credential(
                jwt,
                &mut pin_store,
                audience,
                &verifier_config,
            )
            .await
        };

        // Persist updated pin store (may have new TOFU entries)
        if let Err(e) = self.key_store.save_pin_store(&pin_store) {
            tracing::warn!("Failed to persist AgentPin key store: {}", e);
        }

        Ok(Self::convert_result(&result))
    }
}

/// Caching wrapper around a [`DiscoveryResolver`] that checks the
/// [`DiscoveryCache`] before delegating.
pub struct CachingResolver<R: DiscoveryResolver> {
    inner: R,
    cache: DiscoveryCache,
}

impl<R: DiscoveryResolver> CachingResolver<R> {
    pub fn new(inner: R, cache: DiscoveryCache) -> Self {
        Self { inner, cache }
    }
}

impl<R: DiscoveryResolver> DiscoveryResolver for CachingResolver<R> {
    fn resolve_discovery(
        &self,
        domain: &str,
    ) -> Result<agentpin::types::discovery::DiscoveryDocument, agentpin::error::Error> {
        if let Some(cached) = self.cache.get(domain) {
            return Ok(cached);
        }

        let doc = self.inner.resolve_discovery(domain)?;

        // Best-effort cache write
        let _ = self.cache.put(domain, &doc);

        Ok(doc)
    }

    fn resolve_revocation(
        &self,
        domain: &str,
        discovery: &agentpin::types::discovery::DiscoveryDocument,
    ) -> Result<Option<agentpin::types::revocation::RevocationDocument>, agentpin::error::Error>
    {
        self.inner.resolve_revocation(domain, discovery)
    }
}

/// Mock verifier for testing
pub struct MockAgentPinVerifier {
    should_succeed: bool,
    mock_agent_id: String,
    mock_issuer: String,
    mock_capabilities: Vec<String>,
}

impl MockAgentPinVerifier {
    /// Create a mock verifier that always succeeds
    pub fn new_success() -> Self {
        Self {
            should_succeed: true,
            mock_agent_id: "mock-agent-001".to_string(),
            mock_issuer: "mock.example.com".to_string(),
            mock_capabilities: vec!["execute:*".to_string()],
        }
    }

    /// Create a mock verifier that always fails
    pub fn new_failure() -> Self {
        Self {
            should_succeed: false,
            mock_agent_id: String::new(),
            mock_issuer: String::new(),
            mock_capabilities: vec![],
        }
    }

    /// Create a mock with custom identity
    pub fn with_identity(agent_id: String, issuer: String, capabilities: Vec<String>) -> Self {
        Self {
            should_succeed: true,
            mock_agent_id: agent_id,
            mock_issuer: issuer,
            mock_capabilities: capabilities,
        }
    }
}

#[async_trait]
impl AgentPinVerifier for MockAgentPinVerifier {
    async fn verify_credential(
        &self,
        _jwt: &str,
    ) -> Result<AgentVerificationResult, AgentPinError> {
        if self.should_succeed {
            Ok(AgentVerificationResult::success(
                self.mock_agent_id.clone(),
                self.mock_issuer.clone(),
                self.mock_capabilities.clone(),
            ))
        } else {
            Ok(AgentVerificationResult::failure(
                "Mock verification failed".to_string(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_verifier_success() {
        let verifier = MockAgentPinVerifier::new_success();
        let result = verifier.verify_credential("dummy.jwt.token").await.unwrap();
        assert!(result.valid);
        assert_eq!(result.agent_id, Some("mock-agent-001".to_string()));
        assert_eq!(result.issuer, Some("mock.example.com".to_string()));
        assert!(!result.capabilities.is_empty());
    }

    #[tokio::test]
    async fn test_mock_verifier_failure() {
        let verifier = MockAgentPinVerifier::new_failure();
        let result = verifier.verify_credential("dummy.jwt.token").await.unwrap();
        assert!(!result.valid);
        assert!(result.error_message.is_some());
    }

    #[tokio::test]
    async fn test_mock_verifier_custom_identity() {
        let verifier = MockAgentPinVerifier::with_identity(
            "custom-agent".to_string(),
            "custom.example.com".to_string(),
            vec!["read:data".to_string(), "write:data".to_string()],
        );
        let result = verifier.verify_credential("dummy.jwt.token").await.unwrap();
        assert!(result.valid);
        assert_eq!(result.agent_id, Some("custom-agent".to_string()));
        assert_eq!(result.issuer, Some("custom.example.com".to_string()));
        assert_eq!(result.capabilities.len(), 2);
    }

    #[test]
    fn test_caching_resolver() {
        use agentpin::types::bundle::TrustBundle;

        let temp_dir = tempfile::tempdir().unwrap();
        let cache = DiscoveryCache::new(temp_dir.path(), 3600).unwrap();

        let doc = agentpin::discovery::build_discovery_document(
            "cached.example.com",
            agentpin::types::discovery::EntityType::Maker,
            vec![agentpin::jwk::Jwk {
                kid: "k1".to_string(),
                kty: "EC".to_string(),
                crv: "P-256".to_string(),
                x: "x".to_string(),
                y: "y".to_string(),
                use_: "sig".to_string(),
                key_ops: None,
                exp: None,
            }],
            vec![],
            2,
            "2026-02-10T00:00:00Z",
        );

        let bundle = TrustBundle {
            agentpin_bundle_version: "0.1".to_string(),
            created_at: "2026-02-10T00:00:00Z".to_string(),
            documents: vec![doc],
            revocations: vec![],
        };
        let inner = TrustBundleResolver::new(&bundle);
        let resolver = CachingResolver::new(inner, cache);

        // First call: miss, delegates to inner
        let resolved = resolver.resolve_discovery("cached.example.com").unwrap();
        assert_eq!(resolved.entity, "cached.example.com");

        // Second call: should be cached
        let resolved2 = resolver.resolve_discovery("cached.example.com").unwrap();
        assert_eq!(resolved2.entity, "cached.example.com");
    }
}
