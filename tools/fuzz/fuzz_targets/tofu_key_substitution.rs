#![no_main]

use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use symbi_runtime::integrations::{KeyStoreConfig, LocalKeyStore, PinnedKey};

#[derive(Arbitrary, Debug)]
struct Input {
    identifier: String,
    original_key: Vec<u8>,
    candidate_key: Vec<u8>,
    mutate_candidate: bool,
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

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fuzz_target!(|input: Input| {
    let identifier = clamp(input.identifier, 64, "provider.example.com");
    let original_material = if input.original_key.is_empty() {
        vec![0_u8]
    } else {
        input.original_key
    };

    let dir = tempfile::tempdir().expect("tempdir");
    let config = KeyStoreConfig {
        store_path: dir.path().join("keys.json"),
        create_if_missing: true,
        file_permissions: Some(0o600),
    };
    let store = LocalKeyStore::with_config(config).expect("keystore");

    let pinned_original = PinnedKey::new(
        identifier.clone(),
        hex::encode(&original_material),
        "ES256".to_string(),
        sha256_hex(&original_material),
    );
    store.pin_key(pinned_original.clone()).expect("first pin");

    // Determine whether this second key is a substitution attempt.
    let candidate_material = if input.mutate_candidate {
        if input.candidate_key.is_empty() {
            vec![1_u8]
        } else {
            input.candidate_key
        }
    } else {
        original_material.clone()
    };

    let candidate = PinnedKey::new(
        identifier.clone(),
        hex::encode(&candidate_material),
        "ES256".to_string(),
        sha256_hex(&candidate_material),
    );

    let second_pin = store.pin_key(candidate.clone());

    if input.mutate_candidate && candidate_material != original_material {
        assert!(second_pin.is_err());
        assert!(!store
            .verify_key(&identifier, &candidate.public_key, &candidate.fingerprint)
            .expect("verify key"));
    } else {
        assert!(second_pin.is_ok());
        assert!(store
            .verify_key(&identifier, &candidate.public_key, &candidate.fingerprint)
            .expect("verify key"));
    }
});
