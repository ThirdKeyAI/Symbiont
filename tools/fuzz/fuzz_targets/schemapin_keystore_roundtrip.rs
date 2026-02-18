#![no_main]

//! Fuzz target: SchemaPin LocalKeyStore round-trip operations
//!
//! Exercises the key store with arbitrary sequences of operations and
//! validates core TOFU invariants:
//!   - Pin + verify round-trip must succeed
//!   - Key substitution for an already-pinned identifier must be rejected
//!   - Remove + has_key must return false
//!   - No operation sequence should cause a panic

use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use std::collections::HashMap;
use symbi_runtime::integrations::{KeyStoreConfig, LocalKeyStore, PinnedKey};

/// A single key store operation driven by the fuzzer.
#[derive(Arbitrary, Debug)]
enum KeyStoreOp {
    Pin {
        identifier: String,
        public_key: String,
        algorithm: String,
        fingerprint: String,
    },
    Verify {
        identifier: String,
        public_key: String,
        fingerprint: String,
    },
    Get(String),
    Remove(String),
    HasKey(String),
    ListKeys,
    Clear,
}

/// Top-level fuzz input: a sequence of operations to replay.
#[derive(Arbitrary, Debug)]
struct Input {
    operations: Vec<KeyStoreOp>,
}

/// Clamp a string to `max` bytes, respecting char boundaries.
/// Returns `fallback` when the string is empty.
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
    // Limit the operation count to avoid unbounded execution time.
    let ops = if input.operations.len() > 64 {
        &input.operations[..64]
    } else {
        &input.operations
    };

    // ── 1. Create a tempdir-backed store ─────────────────────────────────
    let dir = tempfile::tempdir().expect("tempdir");
    let config = KeyStoreConfig {
        store_path: dir.path().join("keys.json"),
        create_if_missing: true,
        file_permissions: Some(0o600),
    };
    let store = LocalKeyStore::with_config(config).expect("keystore");

    // Shadow state: track what we believe the store contains so we can
    // assert invariants after each operation.
    //   key: identifier -> (public_key, fingerprint)
    let mut shadow: HashMap<String, (String, String)> = HashMap::new();

    // ── 2. Execute operations in sequence ────────────────────────────────
    for op in ops {
        match op {
            KeyStoreOp::Pin {
                identifier,
                public_key,
                algorithm,
                fingerprint,
            } => {
                let id = clamp(identifier.clone(), 128, "id");
                let pk = clamp(public_key.clone(), 512, "pk");
                let alg = clamp(algorithm.clone(), 32, "ES256");
                let fp = clamp(fingerprint.clone(), 256, "fp");

                let pinned = PinnedKey::new(id.clone(), pk.clone(), alg, fp.clone());
                let result = store.pin_key(pinned);

                match shadow.get(&id) {
                    Some((existing_pk, existing_fp)) => {
                        if *existing_pk == pk && *existing_fp == fp {
                            // Same key material -- pin should succeed.
                            assert!(
                                result.is_ok(),
                                "Re-pinning identical key must succeed"
                            );
                        } else {
                            // Different key material -- TOFU must reject.
                            assert!(
                                result.is_err(),
                                "Pinning different key for existing id must fail (TOFU)"
                            );
                        }
                    }
                    None => {
                        // First pin for this id -- should succeed.
                        if result.is_ok() {
                            shadow.insert(id, (pk, fp));
                        }
                        // We tolerate errors (e.g., I/O) but they must not panic.
                    }
                }
            }

            KeyStoreOp::Verify {
                identifier,
                public_key,
                fingerprint,
            } => {
                let id = clamp(identifier.clone(), 128, "id");
                let pk = clamp(public_key.clone(), 512, "pk");
                let fp = clamp(fingerprint.clone(), 256, "fp");

                let result = store.verify_key(&id, &pk, &fp);

                match shadow.get(&id) {
                    Some((pinned_pk, pinned_fp)) => {
                        // Key exists in our shadow -- verify should return Ok.
                        let verified = result.expect("verify_key must not panic for pinned id");
                        if *pinned_pk == pk && *pinned_fp == fp {
                            assert!(verified, "verify_key must return true for matching key");
                        } else {
                            assert!(
                                !verified,
                                "verify_key must return false for non-matching key"
                            );
                        }
                    }
                    None => {
                        // No key pinned -- expect KeyNotFound error.
                        assert!(
                            result.is_err(),
                            "verify_key for non-existent id must return Err"
                        );
                    }
                }
            }

            KeyStoreOp::Get(identifier) => {
                let id = clamp(identifier.clone(), 128, "id");
                let result = store.get_key(&id);

                match shadow.get(&id) {
                    Some((pk, fp)) => {
                        let key = result.expect("get_key must succeed for pinned id");
                        assert_eq!(key.public_key, *pk, "get_key public_key mismatch");
                        assert_eq!(key.fingerprint, *fp, "get_key fingerprint mismatch");
                    }
                    None => {
                        assert!(result.is_err(), "get_key for absent id must return Err");
                    }
                }
            }

            KeyStoreOp::Remove(identifier) => {
                let id = clamp(identifier.clone(), 128, "id");
                let result = store.remove_key(&id);

                // remove_key returns Result<Option<PinnedKey>>.
                // It should not panic regardless of input.
                if result.is_ok() {
                    shadow.remove(&id);

                    // After removal, has_key must return false.
                    assert!(
                        !store.has_key(&id).unwrap_or(false),
                        "has_key must be false after removal"
                    );
                }
            }

            KeyStoreOp::HasKey(identifier) => {
                let id = clamp(identifier.clone(), 128, "id");
                let result = store.has_key(&id);

                if let Ok(has) = result {
                    let expected = shadow.contains_key(&id);
                    assert_eq!(
                        has, expected,
                        "has_key mismatch: store says {}, shadow says {}",
                        has, expected
                    );
                }
            }

            KeyStoreOp::ListKeys => {
                let result = store.list_keys();
                if let Ok(keys) = result {
                    assert_eq!(
                        keys.len(),
                        shadow.len(),
                        "list_keys count mismatch: store={}, shadow={}",
                        keys.len(),
                        shadow.len()
                    );
                }
            }

            KeyStoreOp::Clear => {
                let result = store.clear();
                if result.is_ok() {
                    shadow.clear();

                    // After clear, the store must be empty.
                    let ids = store.list_identifiers().unwrap_or_default();
                    assert!(ids.is_empty(), "list_identifiers must be empty after clear");
                }
            }
        }
    }
});
