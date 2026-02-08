//! AgentPin Verifier
//!
//! Provides trait and implementations for verifying AgentPin credentials.

use async_trait::async_trait;

use agentpin::verification::{VerificationResult as ApVerificationResult, VerifierConfig};

use super::key_store::AgentPinKeyStore;
use super::types::{AgentPinConfig, AgentPinError, AgentVerificationResult};

/// Trait for verifying AgentPin credentials
#[async_trait]
pub trait AgentPinVerifier: Send + Sync {
    /// Verify a JWT credential and return the verification result
    async fn verify_credential(&self, jwt: &str) -> Result<AgentVerificationResult, AgentPinError>;
}

/// Default verifier that delegates to the agentpin crate's online verification
pub struct DefaultAgentPinVerifier {
    config: AgentPinConfig,
    key_store: AgentPinKeyStore,
}

impl DefaultAgentPinVerifier {
    /// Create a new verifier from configuration
    pub fn new(config: AgentPinConfig) -> Result<Self, AgentPinError> {
        let key_store = AgentPinKeyStore::new(&config.key_store_path).map_err(|e| {
            AgentPinError::KeyStoreError {
                reason: e.to_string(),
            }
        })?;

        Ok(Self { config, key_store })
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

        let result = agentpin::verification::verify_credential(
            jwt,
            &mut pin_store,
            audience,
            &verifier_config,
        )
        .await;

        // Persist updated pin store (may have new TOFU entries)
        if let Err(e) = self.key_store.save_pin_store(&pin_store) {
            tracing::warn!("Failed to persist AgentPin key store: {}", e);
        }

        Ok(Self::convert_result(&result))
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
}
