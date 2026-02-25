#![no_main]

//! Fuzz target for the API key store.
//!
//! Exercises:
//! - Loading key stores from malformed JSON
//! - Key validation with corrupt Argon2 hashes
//! - Roundtrip: hash_key → validate_key
//! - Edge cases: empty keys, very long keys, revoked keys

use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use serde_json;
use symbi_runtime::api::api_keys::{ApiKeyRecord, ApiKeyStore};
use tempfile::NamedTempFile;
use std::io::Write;

#[derive(Arbitrary, Debug)]
struct Input {
    mode: KeyStoreFuzzMode,
}

#[derive(Arbitrary, Debug)]
enum KeyStoreFuzzMode {
    /// Load from arbitrary JSON string (may be malformed).
    LoadMalformedJson { json: String },
    /// Roundtrip: hash a key, create a store with it, then validate.
    HashAndValidate {
        raw_key: String,
        key_id: String,
        description: String,
    },
    /// Validate against corrupt/arbitrary hash strings.
    CorruptHash {
        raw_key: String,
        corrupt_hash: String,
        key_id: String,
    },
    /// Validate with revoked key.
    RevokedKey {
        raw_key: String,
        key_id: String,
    },
    /// Validate with wrong key (must fail).
    WrongKey {
        correct_key: String,
        wrong_key: String,
        key_id: String,
    },
    /// Empty store validation.
    EmptyStore {
        raw_key: String,
    },
    /// Multiple records with agent scoping.
    ScopedValidation {
        raw_key: String,
        key_id: String,
        agents: Vec<String>,
    },
}

fn clamp(mut s: String, max: usize, fallback: &str) -> String {
    if s.is_empty() {
        return fallback.to_string();
    }
    if s.len() > max {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

fuzz_target!(|input: Input| {
    match input.mode {
        KeyStoreFuzzMode::LoadMalformedJson { json } => {
            let json = clamp(json, 8192, "{}");

            // Write to temp file and try to load.
            let mut tmp = match NamedTempFile::new() {
                Ok(f) => f,
                Err(_) => return,
            };
            if tmp.write_all(json.as_bytes()).is_err() {
                return;
            }

            // Must not panic on any JSON input.
            let _ = ApiKeyStore::load_from_file(tmp.path());
        }

        KeyStoreFuzzMode::HashAndValidate {
            raw_key, key_id, description,
        } => {
            let raw_key = clamp(raw_key, 256, "test-api-key-1234");
            let key_id = clamp(key_id, 64, "key-001");
            let description = clamp(description, 128, "fuzz test key");

            // Hash the key.
            let hash = match ApiKeyStore::hash_key(&raw_key) {
                Ok(h) => h,
                Err(_) => return,
            };

            // Build a store with this record.
            let records = vec![ApiKeyRecord {
                key_id: key_id.clone(),
                key_hash: hash,
                agent_scope: None,
                description,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                revoked: false,
            }];

            let json = serde_json::to_string(&records).unwrap();
            let mut tmp = match NamedTempFile::new() {
                Ok(f) => f,
                Err(_) => return,
            };
            if tmp.write_all(json.as_bytes()).is_err() {
                return;
            }

            let store = match ApiKeyStore::load_from_file(tmp.path()) {
                Ok(s) => s,
                Err(_) => return,
            };

            // Roundtrip: correct key must validate.
            let validated = store.validate_key(&raw_key);
            assert!(
                validated.is_some(),
                "roundtrip key validation must succeed",
            );
            assert_eq!(validated.unwrap().key_id, key_id);
        }

        KeyStoreFuzzMode::CorruptHash {
            raw_key, corrupt_hash, key_id,
        } => {
            let raw_key = clamp(raw_key, 256, "test-key");
            let corrupt_hash = clamp(corrupt_hash, 512, "$argon2id$garbage");
            let key_id = clamp(key_id, 64, "key-001");

            let records = vec![ApiKeyRecord {
                key_id,
                key_hash: corrupt_hash,
                agent_scope: None,
                description: "corrupt hash test".to_string(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                revoked: false,
            }];

            let json = serde_json::to_string(&records).unwrap();
            let mut tmp = match NamedTempFile::new() {
                Ok(f) => f,
                Err(_) => return,
            };
            if tmp.write_all(json.as_bytes()).is_err() {
                return;
            }

            let store = match ApiKeyStore::load_from_file(tmp.path()) {
                Ok(s) => s,
                Err(_) => return,
            };

            // Must not panic — corrupt hash should just fail validation.
            let _ = store.validate_key(&raw_key);
        }

        KeyStoreFuzzMode::RevokedKey { raw_key, key_id } => {
            let raw_key = clamp(raw_key, 256, "test-key");
            let key_id = clamp(key_id, 64, "key-001");

            let hash = match ApiKeyStore::hash_key(&raw_key) {
                Ok(h) => h,
                Err(_) => return,
            };

            let records = vec![ApiKeyRecord {
                key_id,
                key_hash: hash,
                agent_scope: None,
                description: "revoked key".to_string(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                revoked: true,
            }];

            let json = serde_json::to_string(&records).unwrap();
            let mut tmp = match NamedTempFile::new() {
                Ok(f) => f,
                Err(_) => return,
            };
            if tmp.write_all(json.as_bytes()).is_err() {
                return;
            }

            let store = match ApiKeyStore::load_from_file(tmp.path()) {
                Ok(s) => s,
                Err(_) => return,
            };

            // Revoked keys must not validate.
            assert!(
                store.validate_key(&raw_key).is_none(),
                "revoked key must not validate",
            );
        }

        KeyStoreFuzzMode::WrongKey { correct_key, wrong_key, key_id } => {
            let correct = clamp(correct_key, 256, "correct-key");
            let wrong = clamp(wrong_key, 256, "wrong-key");
            let key_id = clamp(key_id, 64, "key-001");

            if correct == wrong {
                return;
            }

            let hash = match ApiKeyStore::hash_key(&correct) {
                Ok(h) => h,
                Err(_) => return,
            };

            let records = vec![ApiKeyRecord {
                key_id,
                key_hash: hash,
                agent_scope: None,
                description: "test".to_string(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                revoked: false,
            }];

            let json = serde_json::to_string(&records).unwrap();
            let mut tmp = match NamedTempFile::new() {
                Ok(f) => f,
                Err(_) => return,
            };
            if tmp.write_all(json.as_bytes()).is_err() {
                return;
            }

            let store = match ApiKeyStore::load_from_file(tmp.path()) {
                Ok(s) => s,
                Err(_) => return,
            };

            // Wrong key must not validate.
            assert!(
                store.validate_key(&wrong).is_none(),
                "wrong key must not validate",
            );
        }

        KeyStoreFuzzMode::EmptyStore { raw_key } => {
            let raw_key = clamp(raw_key, 256, "test-key");
            let store = ApiKeyStore::empty();

            // Empty store must return None for any key.
            assert!(
                store.validate_key(&raw_key).is_none(),
                "empty store must return None",
            );
        }

        KeyStoreFuzzMode::ScopedValidation { raw_key, key_id, agents } => {
            let raw_key = clamp(raw_key, 256, "test-key");
            let key_id = clamp(key_id, 64, "key-001");
            let agents: Vec<String> = agents
                .into_iter()
                .take(4)
                .map(|a| clamp(a, 64, "agent"))
                .collect();

            let hash = match ApiKeyStore::hash_key(&raw_key) {
                Ok(h) => h,
                Err(_) => return,
            };

            let scope = if agents.is_empty() { None } else { Some(agents.clone()) };

            let records = vec![ApiKeyRecord {
                key_id: key_id.clone(),
                key_hash: hash,
                agent_scope: scope,
                description: "scoped key".to_string(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                revoked: false,
            }];

            let json = serde_json::to_string(&records).unwrap();
            let mut tmp = match NamedTempFile::new() {
                Ok(f) => f,
                Err(_) => return,
            };
            if tmp.write_all(json.as_bytes()).is_err() {
                return;
            }

            let store = match ApiKeyStore::load_from_file(tmp.path()) {
                Ok(s) => s,
                Err(_) => return,
            };

            // Key must validate and preserve scope.
            let validated = store.validate_key(&raw_key);
            assert!(validated.is_some(), "scoped key must validate");
            let v = validated.unwrap();
            assert_eq!(v.key_id, key_id);
            if !agents.is_empty() {
                assert_eq!(v.agent_scope, Some(agents));
            }
        }
    }
});
