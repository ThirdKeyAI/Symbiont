//! Swappable provider for signature operations.
//!
//! The default routes to `ed25519-dalek` (the crate the runtime has always
//! used). The `fips` feature routes the same operations to
//! `aws-lc-rs`, which links AWS-LC-FIPS. Outputs are byte-identical across
//! providers — Ed25519 signing is deterministic (RFC 8032) — enforced by
//! `tests/crypto_provider_vectors.rs`.
//!
//! Key types stay `ed25519_dalek` structs (as byte containers); the *operations*
//! are what route through the provider.

#[derive(Debug, thiserror::Error)]
pub enum CryptoProviderError {
    #[error("signature verification failed")]
    BadSignature,
}

#[cfg(not(feature = "fips"))]
mod imp {
    use super::CryptoProviderError;
    use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

    pub fn ed25519_sign(key: &SigningKey, msg: &[u8]) -> [u8; 64] {
        key.sign(msg).to_bytes()
    }

    pub fn ed25519_verify(
        public: &[u8; 32],
        msg: &[u8],
        sig: &[u8; 64],
    ) -> Result<(), CryptoProviderError> {
        let vk = VerifyingKey::from_bytes(public).map_err(|_| CryptoProviderError::BadSignature)?;
        vk.verify(msg, &Signature::from_bytes(sig))
            .map_err(|_| CryptoProviderError::BadSignature)
    }
}

#[cfg(feature = "fips")]
mod imp {
    use super::CryptoProviderError;
    use aws_lc_rs::signature::{Ed25519KeyPair, UnparsedPublicKey, ED25519};
    use ed25519_dalek::SigningKey;

    pub fn ed25519_sign(key: &SigningKey, msg: &[u8]) -> [u8; 64] {
        let kp = Ed25519KeyPair::from_seed_unchecked(&key.to_bytes())
            .expect("SigningKey always yields a valid 32-byte seed");
        let mut out = [0u8; 64];
        out.copy_from_slice(kp.sign(msg).as_ref());
        out
    }

    pub fn ed25519_verify(
        public: &[u8; 32],
        msg: &[u8],
        sig: &[u8; 64],
    ) -> Result<(), CryptoProviderError> {
        UnparsedPublicKey::new(&ED25519, public)
            .verify(msg, sig)
            .map_err(|_| CryptoProviderError::BadSignature)
    }
}

pub use imp::{ed25519_sign, ed25519_verify};

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[42u8; 32])
    }

    #[test]
    fn provider_sign_matches_dalek_direct() {
        let k = key();
        let via_provider = ed25519_sign(&k, b"chain-hash");
        let direct = k.sign(b"chain-hash").to_bytes();
        assert_eq!(via_provider, direct);
    }

    #[test]
    fn provider_verify_roundtrip_and_reject() {
        let k = key();
        let public = k.verifying_key().to_bytes();
        let sig = ed25519_sign(&k, b"chain-hash");
        assert!(ed25519_verify(&public, b"chain-hash", &sig).is_ok());
        assert!(ed25519_verify(&public, b"tampered", &sig).is_err());
        let mut bad = sig;
        bad[0] ^= 1;
        assert!(ed25519_verify(&public, b"chain-hash", &bad).is_err());
    }
}
