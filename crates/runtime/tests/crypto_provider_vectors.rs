//! Golden vectors for the crypto_provider seam.
//!
//! Ed25519 signing is deterministic (RFC 8032), so for a fixed seed and message
//! every conforming provider MUST produce these exact bytes. Run under BOTH:
//!   cargo test -p symbi-runtime --test crypto_provider_vectors
//!   cargo test -p symbi-runtime --features fips --test crypto_provider_vectors
//! A mismatch means the active provider is not interchangeable with the
//! default — which would silently fork every signed audit chain.

use ed25519_dalek::SigningKey;
use symbi_runtime::crypto_provider::{ed25519_sign, ed25519_verify};

const SEED: [u8; 32] = [42u8; 32];
const MSG: &[u8] = b"symbiont-crypto-provider-golden-v1";
// Generated once under the default (dalek) provider; MUST never change.
const GOLDEN_PUBLIC: &str = "197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61";
const GOLDEN_SIG: &str = "607899e36b1579da38466a02832d5c2719864bec5b437d3ad3799d27eda94175fb3082a0a29381ea3610b78aba5056578400f6e782fb8040d2c2d228248ebc0d";

#[test]
fn active_provider_matches_golden_vector() {
    let key = SigningKey::from_bytes(&SEED);
    assert_eq!(
        hex::encode(key.verifying_key().to_bytes()),
        GOLDEN_PUBLIC,
        "public key derivation diverged"
    );
    assert_eq!(
        hex::encode(ed25519_sign(&key, MSG)),
        GOLDEN_SIG,
        "signature diverged from the committed golden vector"
    );
}

#[test]
fn active_provider_verifies_golden_signature() {
    let mut public = [0u8; 32];
    hex::decode_to_slice(GOLDEN_PUBLIC, &mut public).unwrap();
    let mut sig = [0u8; 64];
    hex::decode_to_slice(GOLDEN_SIG, &mut sig).unwrap();
    assert!(ed25519_verify(&public, MSG, &sig).is_ok());
}
