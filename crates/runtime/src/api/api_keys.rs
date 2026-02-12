//! Per-agent API key authentication
//!
//! File-backed API key store with Argon2 hashing. Falls back to the legacy
//! `SYMBIONT_API_TOKEN` environment variable when no key store is configured.

#[cfg(feature = "http-api")]
use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};

#[cfg(feature = "http-api")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "http-api")]
use std::path::Path;

/// A single API key record stored on disk
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyRecord {
    /// Unique identifier for this key (e.g. "prod-agent-1")
    pub key_id: String,
    /// Argon2 hash of the raw API key
    pub key_hash: String,
    /// Optional list of agent IDs this key is scoped to (None = all agents)
    pub agent_scope: Option<Vec<String>>,
    /// Human-readable description
    pub description: String,
    /// ISO-8601 creation timestamp
    pub created_at: String,
    /// Whether this key has been revoked
    #[serde(default)]
    pub revoked: bool,
}

/// Returned on successful key validation
#[cfg(feature = "http-api")]
#[derive(Debug, Clone)]
pub struct ValidatedKey {
    /// The key_id that matched
    pub key_id: String,
    /// Agent scope (if any)
    pub agent_scope: Option<Vec<String>>,
}

/// File-backed API key store
#[cfg(feature = "http-api")]
pub struct ApiKeyStore {
    records: Vec<ApiKeyRecord>,
}

#[cfg(feature = "http-api")]
impl ApiKeyStore {
    /// Load API key records from a JSON file.
    ///
    /// The file should contain a JSON array of `ApiKeyRecord` objects.
    /// Returns an empty store if the file does not exist.
    pub fn load_from_file(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            tracing::info!(
                "API keys file not found at {} — empty store",
                path.display()
            );
            return Ok(Self {
                records: Vec::new(),
            });
        }

        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read API keys file: {}", e))?;

        let records: Vec<ApiKeyRecord> = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse API keys JSON: {}", e))?;

        tracing::info!("Loaded {} API key record(s)", records.len());
        Ok(Self { records })
    }

    /// Create an empty store (for testing)
    pub fn empty() -> Self {
        Self {
            records: Vec::new(),
        }
    }

    /// Validate a raw API key against all non-revoked records.
    ///
    /// Returns `Some(ValidatedKey)` on the first matching record, or `None`.
    pub fn validate_key(&self, raw_key: &str) -> Option<ValidatedKey> {
        let argon2 = Argon2::default();

        for record in &self.records {
            if record.revoked {
                continue;
            }

            let parsed_hash = match PasswordHash::new(&record.key_hash) {
                Ok(h) => h,
                Err(_) => continue,
            };

            if argon2
                .verify_password(raw_key.as_bytes(), &parsed_hash)
                .is_ok()
            {
                return Some(ValidatedKey {
                    key_id: record.key_id.clone(),
                    agent_scope: record.agent_scope.clone(),
                });
            }
        }

        None
    }

    /// Hash a raw API key with Argon2 (utility for key provisioning).
    pub fn hash_key(raw_key: &str) -> Result<String, String> {
        let salt = SaltString::generate(&mut rand::thread_rng());
        let argon2 = Argon2::default();
        let hash = argon2
            .hash_password(raw_key.as_bytes(), &salt)
            .map_err(|e| format!("Failed to hash key: {}", e))?;
        Ok(hash.to_string())
    }

    /// Returns true if the store has any records
    pub fn has_records(&self) -> bool {
        !self.records.is_empty()
    }
}

#[cfg(all(test, feature = "http-api"))]
mod tests {
    use super::*;

    #[test]
    fn test_hash_and_verify() {
        let raw_key = "sk-test-super-secret-key-12345";
        let hash = ApiKeyStore::hash_key(raw_key).unwrap();

        let store = ApiKeyStore {
            records: vec![ApiKeyRecord {
                key_id: "test-key".to_string(),
                key_hash: hash,
                agent_scope: None,
                description: "Test key".to_string(),
                created_at: "2024-01-01T00:00:00Z".to_string(),
                revoked: false,
            }],
        };

        let result = store.validate_key(raw_key);
        assert!(result.is_some());
        assert_eq!(result.unwrap().key_id, "test-key");
    }

    #[test]
    fn test_revoked_key_rejected() {
        let raw_key = "sk-revoked-key-12345";
        let hash = ApiKeyStore::hash_key(raw_key).unwrap();

        let store = ApiKeyStore {
            records: vec![ApiKeyRecord {
                key_id: "revoked-key".to_string(),
                key_hash: hash,
                agent_scope: Some(vec!["agent-1".to_string()]),
                description: "Revoked key".to_string(),
                created_at: "2024-01-01T00:00:00Z".to_string(),
                revoked: true,
            }],
        };

        assert!(store.validate_key(raw_key).is_none());
    }

    #[test]
    fn test_wrong_key_rejected() {
        let raw_key = "sk-correct-key";
        let hash = ApiKeyStore::hash_key(raw_key).unwrap();

        let store = ApiKeyStore {
            records: vec![ApiKeyRecord {
                key_id: "test-key".to_string(),
                key_hash: hash,
                agent_scope: None,
                description: "Test key".to_string(),
                created_at: "2024-01-01T00:00:00Z".to_string(),
                revoked: false,
            }],
        };

        assert!(store.validate_key("sk-wrong-key").is_none());
    }

    #[test]
    fn test_empty_store() {
        let store = ApiKeyStore::empty();
        assert!(store.validate_key("any-key").is_none());
        assert!(!store.has_records());
    }

    #[test]
    fn test_nonexistent_file() {
        let store =
            ApiKeyStore::load_from_file(Path::new("/tmp/nonexistent-api-keys-12345.json")).unwrap();
        assert!(!store.has_records());
    }
}
