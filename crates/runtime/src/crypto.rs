//! Cryptographic utilities for Symbiont
//!
//! This module provides encryption and decryption capabilities using industry-standard
//! algorithms like AES-256-GCM for symmetric encryption and Argon2 for key derivation.

use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
};
use argon2::{
    password_hash::{rand_core::RngCore, PasswordHasher, SaltString},
    Argon2,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

/// Errors that can occur during cryptographic operations
#[derive(Debug, Error)]
pub enum CryptoError {
    /// Invalid key format or length
    #[error("Invalid key: {message}")]
    InvalidKey { message: String },

    /// Encryption operation failed
    #[error("Encryption failed: {message}")]
    EncryptionFailed { message: String },

    /// Decryption operation failed
    #[error("Decryption failed: {message}")]
    DecryptionFailed { message: String },

    /// Key derivation failed
    #[error("Key derivation failed: {message}")]
    KeyDerivationFailed { message: String },

    /// Invalid ciphertext format
    #[error("Invalid ciphertext format: {message}")]
    InvalidCiphertext { message: String },

    /// Base64 encoding/decoding error
    #[error("Base64 error: {message}")]
    Base64Error { message: String },
}

/// Encrypted data container
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedData {
    /// Base64-encoded ciphertext
    pub ciphertext: String,
    /// Base64-encoded nonce/IV
    pub nonce: String,
    /// Base64-encoded salt used for key derivation
    pub salt: String,
    /// Algorithm used for encryption
    pub algorithm: String,
    /// Key derivation function used
    pub kdf: String,
}

impl fmt::Display for EncryptedData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EncryptedData(algorithm={})", self.algorithm)
    }
}

/// AES-256-GCM encryption/decryption utilities
pub struct Aes256GcmCrypto;

impl Default for Aes256GcmCrypto {
    fn default() -> Self {
        Self::new()
    }
}

impl Aes256GcmCrypto {
    /// Create a new Aes256GcmCrypto instance
    pub fn new() -> Self {
        Self
    }

    /// Encrypt data using AES-256-GCM with a direct key (for CLI usage)
    pub fn encrypt(&self, plaintext: &[u8], key: &str) -> Result<Vec<u8>, CryptoError> {
        // Decode the base64 key
        let key_bytes = BASE64.decode(key).map_err(|e| {
            CryptoError::InvalidKey {
                message: format!("Invalid base64 key: {}", e),
            }
        })?;

        if key_bytes.len() != 32 {
            return Err(CryptoError::InvalidKey {
                message: "Key must be 32 bytes".to_string(),
            });
        }

        let cipher_key = Key::<Aes256Gcm>::from_slice(&key_bytes);
        let cipher = Aes256Gcm::new(cipher_key);

        // Generate random nonce
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);

        // Encrypt
        let ciphertext = cipher.encrypt(&nonce, plaintext).map_err(|e| {
            CryptoError::EncryptionFailed {
                message: e.to_string(),
            }
        })?;

        // Combine nonce + ciphertext
        let mut result = Vec::with_capacity(12 + ciphertext.len());
        result.extend_from_slice(&nonce);
        result.extend_from_slice(&ciphertext);

        Ok(result)
    }

    /// Decrypt data using AES-256-GCM with a direct key (for CLI usage)
    pub fn decrypt(&self, encrypted_data: &[u8], key: &str) -> Result<Vec<u8>, CryptoError> {
        if encrypted_data.len() < 12 {
            return Err(CryptoError::InvalidCiphertext {
                message: "Encrypted data too short".to_string(),
            });
        }

        // Decode the base64 key
        let key_bytes = BASE64.decode(key).map_err(|e| {
            CryptoError::InvalidKey {
                message: format!("Invalid base64 key: {}", e),
            }
        })?;

        if key_bytes.len() != 32 {
            return Err(CryptoError::InvalidKey {
                message: "Key must be 32 bytes".to_string(),
            });
        }

        let cipher_key = Key::<Aes256Gcm>::from_slice(&key_bytes);
        let cipher = Aes256Gcm::new(cipher_key);

        // Extract nonce and ciphertext
        let (nonce_bytes, ciphertext) = encrypted_data.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);

        // Decrypt
        let plaintext = cipher.decrypt(nonce, ciphertext).map_err(|e| {
            CryptoError::DecryptionFailed {
                message: e.to_string(),
            }
        })?;

        Ok(plaintext)
    }

    /// Encrypt data using AES-256-GCM with Argon2 key derivation (original method)
    pub fn encrypt_with_password(plaintext: &[u8], password: &str) -> Result<EncryptedData, CryptoError> {
        // Generate random salt
        let mut salt = [0u8; 32];
        OsRng.fill_bytes(&mut salt);
        let salt_string = SaltString::encode_b64(&salt)
            .map_err(|e| CryptoError::KeyDerivationFailed {
                message: e.to_string(),
            })?;

        // Derive key using Argon2
        let argon2 = Argon2::default();
        let password_hash = argon2
            .hash_password(password.as_bytes(), &salt_string)
            .map_err(|e| CryptoError::KeyDerivationFailed {
                message: e.to_string(),
            })?;

        // Extract the hash bytes for the encryption key
        let hash_binding = password_hash.hash.unwrap();
        let key_bytes = hash_binding.as_bytes();
        if key_bytes.len() < 32 {
            return Err(CryptoError::InvalidKey {
                message: "Derived key too short".to_string(),
            });
        }

        let key = Key::<Aes256Gcm>::from_slice(&key_bytes[..32]);
        let cipher = Aes256Gcm::new(key);

        // Generate random nonce
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);

        // Encrypt
        let ciphertext = cipher.encrypt(&nonce, plaintext).map_err(|e| {
            CryptoError::EncryptionFailed {
                message: e.to_string(),
            }
        })?;

        Ok(EncryptedData {
            ciphertext: BASE64.encode(&ciphertext),
            nonce: BASE64.encode(nonce),
            salt: BASE64.encode(salt),
            algorithm: "AES-256-GCM".to_string(),
            kdf: "Argon2".to_string(),
        })
    }

    /// Decrypt data using AES-256-GCM with Argon2 key derivation (static method)
    pub fn decrypt_with_password(encrypted_data: &EncryptedData, password: &str) -> Result<Vec<u8>, CryptoError> {
        // Decode base64 components
        let ciphertext = BASE64.decode(&encrypted_data.ciphertext).map_err(|e| {
            CryptoError::Base64Error {
                message: e.to_string(),
            }
        })?;

        let nonce_bytes = BASE64.decode(&encrypted_data.nonce).map_err(|e| {
            CryptoError::Base64Error {
                message: e.to_string(),
            }
        })?;

        let salt = BASE64.decode(&encrypted_data.salt).map_err(|e| {
            CryptoError::Base64Error {
                message: e.to_string(),
            }
        })?;

        // Reconstruct salt string
        let salt_string = SaltString::encode_b64(&salt).map_err(|e| {
            CryptoError::KeyDerivationFailed {
                message: e.to_string(),
            }
        })?;

        // Derive key using the same parameters
        let argon2 = Argon2::default();
        let password_hash = argon2
            .hash_password(password.as_bytes(), &salt_string)
            .map_err(|e| CryptoError::KeyDerivationFailed {
                message: e.to_string(),
            })?;

        let hash_binding = password_hash.hash.unwrap();
        let key_bytes = hash_binding.as_bytes();
        if key_bytes.len() < 32 {
            return Err(CryptoError::InvalidKey {
                message: "Derived key too short".to_string(),
            });
        }

        let key = Key::<Aes256Gcm>::from_slice(&key_bytes[..32]);
        let cipher = Aes256Gcm::new(key);

        // Create nonce
        if nonce_bytes.len() != 12 {
            return Err(CryptoError::InvalidCiphertext {
                message: "Invalid nonce length".to_string(),
            });
        }
        let nonce = Nonce::from_slice(&nonce_bytes);

        // Decrypt
        let plaintext = cipher.decrypt(nonce, ciphertext.as_ref()).map_err(|e| {
            CryptoError::DecryptionFailed {
                message: e.to_string(),
            }
        })?;

        Ok(plaintext)
    }
}

/// Utilities for key management
pub struct KeyUtils;

impl Default for KeyUtils {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyUtils {
    /// Create a new KeyUtils instance
    pub fn new() -> Self {
        Self
    }

    /// Get or create a key, prioritizing keychain, then environment, then generating new
    pub fn get_or_create_key(&self) -> Result<String, CryptoError> {
        // Try keychain first
        if let Ok(key) = self.get_key_from_keychain("symbiont", "secrets") {
            return Ok(key);
        }

        // Try environment variable
        if let Ok(key) = Self::get_key_from_env("SYMBIONT_SECRET_KEY") {
            return Ok(key);
        }

        // Generate a new key and store it in keychain
        let new_key = self.generate_key();
        if let Err(e) = self.store_key_in_keychain("symbiont", "secrets", &new_key) {
            eprintln!("Warning: Failed to store key in keychain: {}", e);
        }
        
        Ok(new_key)
    }

    /// Generate a new random key
    pub fn generate_key(&self) -> String {
        use base64::Engine;
        let mut key_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut key_bytes);
        BASE64.encode(key_bytes)
    }

    /// Store a key in the OS keychain
    #[cfg(feature = "keychain")]
    fn store_key_in_keychain(&self, service: &str, account: &str, key: &str) -> Result<(), CryptoError> {
        use keyring::Entry;
        
        let entry = Entry::new(service, account)
            .map_err(|e| CryptoError::InvalidKey {
                message: format!("Failed to create keychain entry: {}", e),
            })?;

        entry.set_password(key).map_err(|e| CryptoError::InvalidKey {
            message: format!("Failed to store in keychain: {}", e),
        })
    }

    #[cfg(not(feature = "keychain"))]
    fn store_key_in_keychain(&self, _service: &str, _account: &str, _key: &str) -> Result<(), CryptoError> {
        Err(CryptoError::InvalidKey {
            message: "Keychain support not enabled. Compile with 'keychain' feature.".to_string(),
        })
    }

    /// Retrieve a key from environment variable
    pub fn get_key_from_env(env_var: &str) -> Result<String, CryptoError> {
        std::env::var(env_var).map_err(|_| CryptoError::InvalidKey {
            message: format!("Environment variable {} not found", env_var),
        })
    }

    /// Retrieve a key from OS keychain (cross-platform)
    #[cfg(feature = "keychain")]
    pub fn get_key_from_keychain(&self, service: &str, account: &str) -> Result<String, CryptoError> {
        use keyring::Entry;
        
        let entry = Entry::new(service, account)
            .map_err(|e| CryptoError::InvalidKey {
                message: format!("Failed to create keychain entry: {}", e),
            })?;

        entry.get_password().map_err(|e| CryptoError::InvalidKey {
            message: format!("Failed to retrieve from keychain: {}", e),
        })
    }

    #[cfg(not(feature = "keychain"))]
    pub fn get_key_from_keychain(&self, _service: &str, _account: &str) -> Result<String, CryptoError> {
        Err(CryptoError::InvalidKey {
            message: "Keychain support not enabled. Compile with 'keychain' feature.".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let plaintext = b"Hello, world!";
        let password = "test1"; // Test password

        let encrypted = Aes256GcmCrypto::encrypt_with_password(plaintext, password).unwrap();
        let decrypted = Aes256GcmCrypto::decrypt_with_password(&encrypted, password).unwrap();

        assert_eq!(plaintext, decrypted.as_slice());
    }

    #[test]
    fn test_encrypt_decrypt_wrong_password() {
        let plaintext = b"Hello, world!";
        let password = "test1"; // Test password
        let wrong_password = "wrong1"; // Wrong test password

        let encrypted = Aes256GcmCrypto::encrypt_with_password(plaintext, password).unwrap();
        let result = Aes256GcmCrypto::decrypt_with_password(&encrypted, wrong_password);

        assert!(result.is_err());
    }

    #[test]
    fn test_direct_encrypt_decrypt_roundtrip() {
        let plaintext = b"Hello, world!";
        let key_utils = KeyUtils::new();
        let key = key_utils.generate_key();

        let crypto = Aes256GcmCrypto::new();
        let encrypted = crypto.encrypt(plaintext, &key).unwrap();
        let decrypted = crypto.decrypt(&encrypted, &key).unwrap();

        assert_eq!(plaintext, decrypted.as_slice());
    }

    #[test]
    fn test_get_key_from_env() {
        std::env::set_var("TEST_KEY", "test_value");
        let result = KeyUtils::get_key_from_env("TEST_KEY").unwrap();
        assert_eq!(result, "test_value");

        let missing_result = KeyUtils::get_key_from_env("MISSING_KEY");
        assert!(missing_result.is_err());
    }
}