//! AgentPin Integration Types
//!
//! Configuration, error, and result types for the AgentPin integration.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

/// How discovery documents are resolved.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DiscoveryMode {
    /// Standard `.well-known` HTTPS fetch (default).
    #[default]
    WellKnown,
    /// Pre-shared trust bundle loaded from a file.
    Bundle,
    /// Local filesystem directory containing `{domain}.json` files.
    Local,
    /// Chain: try sync resolvers (bundle → local) then fall back to async.
    Chain,
}

/// AgentPin integration configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPinConfig {
    /// Whether AgentPin verification is enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Path to the TOFU key pin store file
    #[serde(default = "default_key_store_path")]
    pub key_store_path: PathBuf,
    /// Path to the discovery document cache directory
    #[serde(default = "default_discovery_cache_path")]
    pub discovery_cache_path: PathBuf,
    /// Discovery document cache TTL in seconds
    #[serde(default = "default_cache_ttl_secs")]
    pub cache_ttl_secs: u64,
    /// Maximum allowed clock skew in seconds
    #[serde(default = "default_clock_skew_secs")]
    pub clock_skew_secs: i64,
    /// Maximum allowed credential TTL in seconds
    #[serde(default = "default_max_ttl_secs")]
    pub max_ttl_secs: i64,
    /// Expected audience claim (this service's domain)
    pub audience: Option<String>,
    /// How discovery documents are obtained
    #[serde(default)]
    pub discovery_mode: DiscoveryMode,
    /// Path to a trust bundle JSON file (used when discovery_mode = bundle or chain)
    pub trust_bundle_path: Option<PathBuf>,
    /// Path to a directory of discovery docs (used when discovery_mode = local or chain)
    pub local_discovery_dir: Option<PathBuf>,
    /// Path to a directory of revocation docs (used when discovery_mode = local or chain)
    pub local_revocation_dir: Option<PathBuf>,
}

fn default_enabled() -> bool {
    false
}

fn default_key_store_path() -> PathBuf {
    let mut p = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    p.push(".symbiont");
    p.push("agentpin_keys.json");
    p
}

fn default_discovery_cache_path() -> PathBuf {
    let mut p = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    p.push(".symbiont");
    p.push("agentpin_discovery");
    p
}

fn default_cache_ttl_secs() -> u64 {
    3600
}

fn default_clock_skew_secs() -> i64 {
    60
}

fn default_max_ttl_secs() -> i64 {
    86400
}

impl Default for AgentPinConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            key_store_path: default_key_store_path(),
            discovery_cache_path: default_discovery_cache_path(),
            cache_ttl_secs: default_cache_ttl_secs(),
            clock_skew_secs: default_clock_skew_secs(),
            max_ttl_secs: default_max_ttl_secs(),
            audience: None,
            discovery_mode: DiscoveryMode::default(),
            trust_bundle_path: None,
            local_discovery_dir: None,
            local_revocation_dir: None,
        }
    }
}

/// Errors from AgentPin operations
#[derive(Error, Debug, Clone)]
pub enum AgentPinError {
    #[error("Credential verification failed: {reason}")]
    VerificationFailed { reason: String },

    #[error("Discovery document fetch failed for {domain}: {reason}")]
    DiscoveryFetchFailed { domain: String, reason: String },

    #[error("Key store error: {reason}")]
    KeyStoreError { reason: String },

    #[error("Configuration error: {reason}")]
    ConfigError { reason: String },

    #[error("IO error: {reason}")]
    IoError { reason: String },

    #[error("Credential expired")]
    CredentialExpired,

    #[error("Agent not found in discovery: {agent_id}")]
    AgentNotFound { agent_id: String },

    #[error("Key pin mismatch for domain: {domain}")]
    KeyPinMismatch { domain: String },
}

/// Result of verifying an AgentPin credential
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentVerificationResult {
    /// Whether verification succeeded
    pub valid: bool,
    /// Agent ID from the credential
    pub agent_id: Option<String>,
    /// Issuer domain from the credential
    pub issuer: Option<String>,
    /// Capabilities granted by the credential
    pub capabilities: Vec<String>,
    /// Whether the delegation chain was verified
    pub delegation_verified: Option<bool>,
    /// Error message if verification failed
    pub error_message: Option<String>,
    /// Warnings generated during verification
    pub warnings: Vec<String>,
}

impl AgentVerificationResult {
    /// Create a successful verification result
    pub fn success(agent_id: String, issuer: String, capabilities: Vec<String>) -> Self {
        Self {
            valid: true,
            agent_id: Some(agent_id),
            issuer: Some(issuer),
            capabilities,
            delegation_verified: None,
            error_message: None,
            warnings: vec![],
        }
    }

    /// Create a failed verification result
    pub fn failure(error_message: String) -> Self {
        Self {
            valid: false,
            agent_id: None,
            issuer: None,
            capabilities: vec![],
            delegation_verified: None,
            error_message: Some(error_message),
            warnings: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AgentPinConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.cache_ttl_secs, 3600);
        assert_eq!(config.clock_skew_secs, 60);
        assert_eq!(config.max_ttl_secs, 86400);
        assert!(config.audience.is_none());
        assert!(config
            .key_store_path
            .to_string_lossy()
            .contains("agentpin_keys.json"));
        assert_eq!(config.discovery_mode, DiscoveryMode::WellKnown);
        assert!(config.trust_bundle_path.is_none());
        assert!(config.local_discovery_dir.is_none());
        assert!(config.local_revocation_dir.is_none());
    }

    #[test]
    fn test_config_serialization_roundtrip() {
        let config = AgentPinConfig {
            enabled: true,
            audience: Some("example.com".to_string()),
            ..Default::default()
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: AgentPinConfig = serde_json::from_str(&json).unwrap();

        assert!(deserialized.enabled);
        assert_eq!(deserialized.audience, Some("example.com".to_string()));
        assert_eq!(deserialized.cache_ttl_secs, config.cache_ttl_secs);
    }

    #[test]
    fn test_verification_result_success() {
        let result = AgentVerificationResult::success(
            "agent-001".to_string(),
            "maker.example.com".to_string(),
            vec!["execute:code".to_string()],
        );
        assert!(result.valid);
        assert_eq!(result.agent_id, Some("agent-001".to_string()));
        assert_eq!(result.issuer, Some("maker.example.com".to_string()));
        assert_eq!(result.capabilities.len(), 1);
        assert!(result.error_message.is_none());
    }

    #[test]
    fn test_verification_result_failure() {
        let result = AgentVerificationResult::failure("signature invalid".to_string());
        assert!(!result.valid);
        assert!(result.agent_id.is_none());
        assert_eq!(result.error_message, Some("signature invalid".to_string()));
    }

    #[test]
    fn test_verification_result_serialization() {
        let result = AgentVerificationResult::success(
            "agent-001".to_string(),
            "maker.example.com".to_string(),
            vec!["read:data".to_string()],
        );
        let json = serde_json::to_string(&result).unwrap();
        let deserialized: AgentVerificationResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.valid, result.valid);
        assert_eq!(deserialized.agent_id, result.agent_id);
    }

    #[test]
    fn test_discovery_mode_serde() {
        assert_eq!(
            serde_json::to_string(&DiscoveryMode::WellKnown).unwrap(),
            "\"wellknown\""
        );
        assert_eq!(
            serde_json::to_string(&DiscoveryMode::Bundle).unwrap(),
            "\"bundle\""
        );
        assert_eq!(
            serde_json::to_string(&DiscoveryMode::Local).unwrap(),
            "\"local\""
        );
        assert_eq!(
            serde_json::to_string(&DiscoveryMode::Chain).unwrap(),
            "\"chain\""
        );
    }

    #[test]
    fn test_agentpin_error_display() {
        let err = AgentPinError::VerificationFailed {
            reason: "bad sig".to_string(),
        };
        assert!(err.to_string().contains("bad sig"));

        let err = AgentPinError::KeyPinMismatch {
            domain: "evil.com".to_string(),
        };
        assert!(err.to_string().contains("evil.com"));
    }
}
