#![no_main]

use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use symbi_runtime::integrations::{KeyStoreConfig, LocalKeyStore, PinnedKey};

#[derive(Arbitrary, Debug)]
struct Input {
    /// Identifier for the pinned key (e.g., domain name)
    identifier: String,
    /// Arbitrary hex-like garbage for the public key
    public_key: String,
    /// Should be "ES256" or garbage
    algorithm: String,
    /// Arbitrary garbage fingerprint
    fingerprint: String,
    /// The public key to try verifying against
    verify_public_key: String,
    /// The fingerprint to try verifying against
    verify_fingerprint: String,
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

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fuzz_target!(|input: Input| {
    // ── 1. Clamp all strings to reasonable lengths ──────────────────────
    let identifier = clamp(input.identifier, 64, "provider.example.com");
    let public_key = clamp(input.public_key, 256, "deadbeef");
    let algorithm = clamp(input.algorithm, 32, "ES256");
    let fingerprint = clamp(input.fingerprint, 128, "0000");
    let verify_public_key = clamp(input.verify_public_key, 256, "cafebabe");
    let verify_fingerprint = clamp(input.verify_fingerprint, 128, "ffff");

    // ── 2. Create a tempdir and LocalKeyStore ──────────────────────────
    let dir = tempfile::tempdir().expect("tempdir");
    let config = KeyStoreConfig {
        store_path: dir.path().join("keys.json"),
        create_if_missing: true,
        file_permissions: Some(0o600),
    };
    let store = LocalKeyStore::with_config(config).expect("keystore");

    // ── 3. Build a PinnedKey from the fuzzed fields ────────────────────
    let pinned = PinnedKey::new(
        identifier.clone(),
        public_key.clone(),
        algorithm.clone(),
        fingerprint.clone(),
    );

    // ── 4. Pin the key — must not panic (errors are fine) ──────────────
    let pin_result = store.pin_key(pinned);
    if pin_result.is_err() {
        // Invalid input was correctly rejected — nothing more to check.
        return;
    }

    // ── 5. Verify with the fuzzed verify_* fields ──────────────────────
    let verify_result = store
        .verify_key(&identifier, &verify_public_key, &verify_fingerprint)
        .expect("verify_key must not panic");

    if verify_public_key == public_key && verify_fingerprint == fingerprint {
        // Exact match — verification MUST succeed.
        assert!(
            verify_result,
            "verify_key returned false for matching key+fingerprint"
        );
    } else {
        // At least one field differs — garbage must NEVER pass.
        assert!(
            !verify_result,
            "verify_key returned true for NON-matching input! \
             public_key match: {}, fingerprint match: {}",
            verify_public_key == public_key,
            verify_fingerprint == fingerprint,
        );
    }

    // ── 6. Key-substitution: second pin with same id, different data ───
    //       Derive deterministically different key material so the
    //       second key is guaranteed to differ from the first.
    let different_public_key = sha256_hex(public_key.as_bytes());
    let different_fingerprint = sha256_hex(fingerprint.as_bytes());

    // Only test substitution when the derived values actually differ
    // (they will in practice because sha256 of X != X for short hex strings,
    //  but guard anyway).
    if different_public_key != public_key || different_fingerprint != fingerprint {
        let substitute = PinnedKey::new(
            identifier.clone(),
            different_public_key,
            algorithm,
            different_fingerprint,
        );
        let second_pin = store.pin_key(substitute);
        assert!(
            second_pin.is_err(),
            "Second pin with different key material must be rejected (TOFU violation)"
        );
    }
});
