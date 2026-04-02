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
use std::collections::HashMap;

#[cfg(feature = "http-api")]
use std::path::Path;

/// A single API key record stored on disk
#[cfg(feature = "http-api")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyRecord {
    /// Unique identifier for this key (e.g. "prod-agent-1").
    /// This is also used as the key ID prefix in the `keyid.secret` format.
    pub key_id: String,
    /// Argon2 hash of the raw API key (the secret part only, after the `.`)
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

/// File-backed API key store with O(1) key lookup.
///
/// Keys use the format `keyid.secret`. On validation the key ID prefix is
/// extracted and used for a direct HashMap lookup, so only ONE Argon2 verify
/// is performed regardless of how many keys are stored. Keys without a `.`
/// separator fall back to the legacy O(n) scan for backward compatibility.
#[cfg(feature = "http-api")]
pub struct ApiKeyStore {
    /// Key ID -> record for O(1) lookup
    records_by_id: HashMap<String, ApiKeyRecord>,
    /// Flat list kept for legacy (no-prefix) fallback
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
                records_by_id: HashMap::new(),
                records: Vec::new(),
            });
        }

        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read API keys file: {}", e))?;

        let records: Vec<ApiKeyRecord> = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse API keys JSON: {}", e))?;

        tracing::info!("Loaded {} API key record(s)", records.len());

        let mut records_by_id = HashMap::with_capacity(records.len());
        for record in &records {
            records_by_id.insert(record.key_id.clone(), record.clone());
        }

        Ok(Self {
            records_by_id,
            records,
        })
    }

    /// Create an empty store (for testing)
    pub fn empty() -> Self {
        Self {
            records_by_id: HashMap::new(),
            records: Vec::new(),
        }
    }

    /// Create a store from a vec of records (for testing)
    #[cfg(test)]
    fn from_records(records: Vec<ApiKeyRecord>) -> Self {
        let mut records_by_id = HashMap::with_capacity(records.len());
        for record in &records {
            records_by_id.insert(record.key_id.clone(), record.clone());
        }
        Self {
            records_by_id,
            records,
        }
    }

    /// Validate a raw API key against the store.
    ///
    /// Preferred format: `keyid.secret` — performs O(1) lookup by key ID then
    /// a single Argon2 verify. If the key has no `.` separator, falls back to
    /// the legacy O(n) scan for backward compatibility (with a deprecation
    /// warning).
    pub fn validate_key(&self, raw_key: &str) -> Option<ValidatedKey> {
        // Argon2::default() is correct for verification — verify_password()
        // extracts parameters from the stored PHC hash string automatically.
        let argon2 = Argon2::default();

        // Try the new keyid.secret format: split on the FIRST dot
        if let Some((key_id, secret)) = raw_key.split_once('.') {
            if !key_id.is_empty() && !secret.is_empty() {
                // O(1) lookup by key ID
                if let Some(record) = self.records_by_id.get(key_id) {
                    if record.revoked {
                        return None;
                    }

                    let parsed_hash = match PasswordHash::new(&record.key_hash) {
                        Ok(h) => h,
                        Err(_) => return None,
                    };

                    // Single Argon2 verify — constant-time internally
                    if argon2
                        .verify_password(secret.as_bytes(), &parsed_hash)
                        .is_ok()
                    {
                        return Some(ValidatedKey {
                            key_id: record.key_id.clone(),
                            agent_scope: record.agent_scope.clone(),
                        });
                    }
                }
                // Key ID not found or verify failed
                return None;
            }
        }

        // Legacy fallback: no `.` separator — O(n) scan over all keys.
        // This path exists for backward compatibility with keys that were
        // issued before the keyid.secret format was introduced.
        tracing::warn!(
            "API key without 'keyid.secret' prefix — using legacy O(n) scan. \
             Re-issue keys in 'keyid.secret' format to avoid per-request DoS risk."
        );

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

    /// Hash a raw API key with Argon2id (utility for key provisioning).
    /// Uses Argon2id with 19 MiB memory, 2 iterations, 1 thread (OWASP recommendation).
    pub fn hash_key(raw_key: &str) -> Result<String, String> {
        let salt = SaltString::generate(&mut rand::thread_rng());
        let params = argon2::Params::new(19 * 1024, 2, 1, None)
            .map_err(|e| format!("Invalid Argon2 parameters: {}", e))?;
        let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
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
    fn test_hash_and_verify_legacy() {
        // Legacy format (no keyid prefix) — should still work via O(n) fallback
        let raw_key = "sk-test-super-secret-key-12345";
        let hash = ApiKeyStore::hash_key(raw_key).unwrap();

        let store = ApiKeyStore::from_records(vec![ApiKeyRecord {
            key_id: "test-key".to_string(),
            key_hash: hash,
            agent_scope: None,
            description: "Test key".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            revoked: false,
        }]);

        let result = store.validate_key(raw_key);
        assert!(result.is_some());
        assert_eq!(result.unwrap().key_id, "test-key");
    }

    #[test]
    fn test_prefixed_key_o1_lookup() {
        // New format: keyid.secret — should do O(1) lookup
        let secret = "super-secret-part";
        let hash = ApiKeyStore::hash_key(secret).unwrap();

        let store = ApiKeyStore::from_records(vec![ApiKeyRecord {
            key_id: "sk_abc123".to_string(),
            key_hash: hash,
            agent_scope: Some(vec!["agent-1".to_string()]),
            description: "Prefixed key".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            revoked: false,
        }]);

        // keyid.secret format
        let result = store.validate_key("sk_abc123.super-secret-part");
        assert!(result.is_some());
        let validated = result.unwrap();
        assert_eq!(validated.key_id, "sk_abc123");
        assert_eq!(validated.agent_scope, Some(vec!["agent-1".to_string()]));
    }

    #[test]
    fn test_prefixed_key_wrong_secret() {
        let secret = "correct-secret";
        let hash = ApiKeyStore::hash_key(secret).unwrap();

        let store = ApiKeyStore::from_records(vec![ApiKeyRecord {
            key_id: "sk_abc123".to_string(),
            key_hash: hash,
            agent_scope: None,
            description: "Test".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            revoked: false,
        }]);

        assert!(store.validate_key("sk_abc123.wrong-secret").is_none());
    }

    #[test]
    fn test_prefixed_key_unknown_id() {
        let secret = "some-secret";
        let hash = ApiKeyStore::hash_key(secret).unwrap();

        let store = ApiKeyStore::from_records(vec![ApiKeyRecord {
            key_id: "sk_abc123".to_string(),
            key_hash: hash,
            agent_scope: None,
            description: "Test".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            revoked: false,
        }]);

        // Unknown key ID — should return None without any Argon2 work
        assert!(store.validate_key("sk_unknown.some-secret").is_none());
    }

    #[test]
    fn test_revoked_key_rejected() {
        let secret = "revoked-secret";
        let hash = ApiKeyStore::hash_key(secret).unwrap();

        let store = ApiKeyStore::from_records(vec![ApiKeyRecord {
            key_id: "revoked-key".to_string(),
            key_hash: hash,
            agent_scope: Some(vec!["agent-1".to_string()]),
            description: "Revoked key".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            revoked: true,
        }]);

        // Both prefixed and legacy should be rejected
        assert!(store.validate_key("revoked-key.revoked-secret").is_none());
        assert!(store.validate_key("revoked-secret").is_none());
    }

    #[test]
    fn test_wrong_key_rejected() {
        let raw_key = "sk-correct-key";
        let hash = ApiKeyStore::hash_key(raw_key).unwrap();

        let store = ApiKeyStore::from_records(vec![ApiKeyRecord {
            key_id: "test-key".to_string(),
            key_hash: hash,
            agent_scope: None,
            description: "Test key".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            revoked: false,
        }]);

        assert!(store.validate_key("sk-wrong-key").is_none());
    }

    #[test]
    fn test_empty_store() {
        let store = ApiKeyStore::empty();
        assert!(store.validate_key("any-key").is_none());
        assert!(store.validate_key("prefix.any-key").is_none());
        assert!(!store.has_records());
    }

    #[test]
    fn test_nonexistent_file() {
        let store =
            ApiKeyStore::load_from_file(Path::new("/tmp/nonexistent-api-keys-12345.json")).unwrap();
        assert!(!store.has_records());
    }
}
