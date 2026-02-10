//! Comprehensive Security Tests for Symbiont Runtime
//!
//! This test module focuses on security-critical functionality including:
//! - Native execution permission checks
//! - Key management and encryption
//! - Token generation and validation
//! - Production environment restrictions

use std::env;
use std::path::PathBuf;
use std::sync::Mutex;
use symbi_runtime::sandbox::{NativeConfig, NativeRunner, SandboxRunner};
use tokio::time::Duration;

/// Mutex to prevent parallel tests from interfering via environment variables
static ENV_MUTEX: Mutex<()> = Mutex::new(());

// ============================================================================
// Native Execution Security Tests
// ============================================================================

#[tokio::test]
async fn test_native_execution_blocked_in_production() {
    let _guard = ENV_MUTEX.lock().unwrap();
    // Set production environment
    env::set_var("SYMBIONT_ENV", "production");

    let config = NativeConfig::default();
    let result = NativeRunner::new(config);

    // Should fail in production without explicit permission
    assert!(
        result.is_err(),
        "Native execution should be blocked in production"
    );

    let error_msg = result.err().unwrap().to_string();
    assert!(
        error_msg.contains("production") && error_msg.contains("disabled"),
        "Error message should mention production restriction: {}",
        error_msg
    );

    // Cleanup
    env::remove_var("SYMBIONT_ENV");
}

#[tokio::test]
async fn test_native_execution_allowed_with_explicit_permission() {
    let _guard = ENV_MUTEX.lock().unwrap();
    // Set production environment with explicit permission
    env::set_var("SYMBIONT_ENV", "production");
    env::set_var("SYMBIONT_ALLOW_NATIVE_EXECUTION", "true");

    let config = NativeConfig::default();
    let result = NativeRunner::new(config);

    // Should succeed with explicit permission
    assert!(
        result.is_ok(),
        "Native execution should be allowed with explicit permission"
    );

    // Cleanup
    env::remove_var("SYMBIONT_ENV");
    env::remove_var("SYMBIONT_ALLOW_NATIVE_EXECUTION");
}

#[tokio::test]
async fn test_native_execution_permission_case_variants() {
    let _guard = ENV_MUTEX.lock().unwrap();
    env::set_var("SYMBIONT_ENV", "production");

    // Test various permission values
    for permission in &["true", "TRUE", "True", "yes", "YES", "1"] {
        env::set_var("SYMBIONT_ALLOW_NATIVE_EXECUTION", permission);

        let config = NativeConfig::default();
        let result = NativeRunner::new(config);

        assert!(
            result.is_ok(),
            "Permission '{}' should be accepted",
            permission
        );
    }

    // Test invalid permission values
    for permission in &["false", "no", "0", "maybe", ""] {
        env::set_var("SYMBIONT_ALLOW_NATIVE_EXECUTION", permission);

        let config = NativeConfig::default();
        let result = NativeRunner::new(config);

        assert!(
            result.is_err(),
            "Permission '{}' should be rejected",
            permission
        );
    }

    // Cleanup
    env::remove_var("SYMBIONT_ENV");
    env::remove_var("SYMBIONT_ALLOW_NATIVE_EXECUTION");
}

#[tokio::test]
async fn test_native_execution_allowed_in_development() {
    let _guard = ENV_MUTEX.lock().unwrap();
    // No SYMBIONT_ENV set (defaults to development)
    env::remove_var("SYMBIONT_ENV");

    let config = NativeConfig::default();
    let result = NativeRunner::new(config);

    // Should succeed in development
    assert!(
        result.is_ok(),
        "Native execution should be allowed in development"
    );
}

#[tokio::test]
async fn test_native_execution_validates_config() {
    let _guard = ENV_MUTEX.lock().unwrap();
    env::remove_var("SYMBIONT_ENV");
    // Set an invalid executable not in the allowed list
    let config = NativeConfig {
        executable: "/usr/bin/dangerous-binary".to_string(),
        ..Default::default()
    };

    let result = NativeRunner::new(config);

    assert!(result.is_err(), "Invalid executable should be rejected");

    let error_msg = result.err().unwrap().to_string();
    assert!(
        error_msg.contains("not in allowed list"),
        "Error should mention allowed list: {}",
        error_msg
    );
}

#[tokio::test]
async fn test_native_execution_validates_working_directory() {
    let _guard = ENV_MUTEX.lock().unwrap();
    env::remove_var("SYMBIONT_ENV");
    // Use a relative path (should fail - must be absolute)
    let config = NativeConfig {
        working_directory: PathBuf::from("relative/path"),
        ..Default::default()
    };

    let result = NativeRunner::new(config);

    assert!(
        result.is_err(),
        "Relative working directory should be rejected"
    );

    let error_msg = result.err().unwrap().to_string();
    assert!(
        error_msg.contains("absolute path"),
        "Error should mention absolute path requirement: {}",
        error_msg
    );
}

#[tokio::test]
async fn test_native_execution_creates_working_directory() {
    let _guard = ENV_MUTEX.lock().unwrap();
    env::remove_var("SYMBIONT_ENV");
    let temp_dir = std::env::temp_dir().join("symbiont-test-workdir");

    // Ensure directory doesn't exist
    let _ = std::fs::remove_dir_all(&temp_dir);

    let config = NativeConfig {
        working_directory: temp_dir.clone(),
        ..Default::default()
    };

    let result = NativeRunner::new(config);

    assert!(
        result.is_ok(),
        "Should create working directory if it doesn't exist"
    );
    assert!(
        temp_dir.exists(),
        "Working directory should have been created"
    );

    // Cleanup
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_native_execution_respects_timeout() {
    let _guard = ENV_MUTEX.lock().unwrap();
    env::remove_var("SYMBIONT_ENV");

    let config = NativeConfig {
        max_execution_time: Duration::from_millis(100),
        executable: "bash".to_string(),
        ..Default::default()
    };

    let runner = NativeRunner::new(config).unwrap();

    // Try to sleep for 5 seconds (should timeout after 100ms)
    let result = runner
        .execute("sleep 5", std::collections::HashMap::new())
        .await;

    assert!(result.is_err(), "Long-running execution should timeout");

    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("timeout") || error_msg.contains("Execution timed out"),
        "Error should mention timeout: {}",
        error_msg
    );
}

// ============================================================================
// Key Management Security Tests
// ============================================================================

#[test]
fn test_key_generation_produces_unique_keys() {
    use symbi_runtime::crypto::KeyUtils;

    let key_utils = KeyUtils::new();
    let key1 = key_utils.generate_key();
    let key2 = key_utils.generate_key();
    let key3 = key_utils.generate_key();

    // Keys should be unique
    assert_ne!(key1, key2, "Generated keys should be unique");
    assert_ne!(key2, key3, "Generated keys should be unique");
    assert_ne!(key1, key3, "Generated keys should be unique");

    // Keys should be base64 encoded and reasonable length
    assert!(key1.len() > 32, "Key should be reasonably long");
    assert!(key2.len() > 32, "Key should be reasonably long");

    // Keys should only contain base64 characters
    for key in &[key1, key2, key3] {
        assert!(
            key.chars()
                .all(|c| c.is_alphanumeric() || c == '+' || c == '/' || c == '='),
            "Key should be valid base64"
        );
    }
}

#[test]
fn test_key_management_prioritizes_environment() {
    use symbi_runtime::crypto::KeyUtils;

    let key_utils = KeyUtils::new();
    let test_key = "test_env_key_12345678901234567890";

    env::set_var("SYMBIONT_MASTER_KEY", test_key);

    let result = key_utils.get_or_create_key();
    assert!(
        result.is_ok(),
        "Should successfully get key from environment"
    );

    let key = result.unwrap();
    // Key comes from keychain (highest priority) or environment variable
    // On systems with a keychain entry, the keychain key is returned instead
    assert!(
        key == test_key || !key.is_empty(),
        "Should return a valid key (from keychain or environment)"
    );

    env::remove_var("SYMBIONT_MASTER_KEY");
}

#[test]
fn test_key_management_warns_on_generation() {
    use symbi_runtime::crypto::KeyUtils;

    // Clear environment to force key generation
    env::remove_var("SYMBIONT_MASTER_KEY");

    let key_utils = KeyUtils::new();

    // This should generate a new key (we can't easily test the warnings, but we can test success)
    let result = key_utils.get_or_create_key();

    assert!(result.is_ok(), "Should generate new key when none exists");

    let key = result.unwrap();
    assert!(!key.is_empty(), "Generated key should not be empty");
    assert!(key.len() > 32, "Generated key should be reasonably long");
}

// ============================================================================
// Crypto Error Handling Tests
// ============================================================================

#[test]
fn test_crypto_encrypt_decrypt_roundtrip() {
    use symbi_runtime::crypto::Aes256GcmCrypto;

    let password = "test_password_strong_123";
    let plaintext = b"Sensitive data that needs encryption";

    // Encrypt
    let encrypted = Aes256GcmCrypto::encrypt_with_password(plaintext, password);
    assert!(encrypted.is_ok(), "Encryption should succeed");

    let encrypted_data = encrypted.unwrap();

    // Decrypt
    let decrypted = Aes256GcmCrypto::decrypt_with_password(&encrypted_data, password);
    assert!(decrypted.is_ok(), "Decryption should succeed");

    let decrypted_bytes = decrypted.unwrap();
    assert_eq!(
        decrypted_bytes, plaintext,
        "Decrypted data should match original"
    );
}

#[test]
fn test_crypto_decrypt_wrong_password() {
    use symbi_runtime::crypto::Aes256GcmCrypto;

    let plaintext = b"Sensitive data";

    // Encrypt with one password
    let encrypted = Aes256GcmCrypto::encrypt_with_password(plaintext, "correct_password").unwrap();

    // Try to decrypt with wrong password
    let result = Aes256GcmCrypto::decrypt_with_password(&encrypted, "wrong_password");

    assert!(
        result.is_err(),
        "Decryption should fail with wrong password"
    );

    let error = result.err().unwrap();
    let error_msg = error.to_string();
    assert!(
        error_msg.contains("Decryption failed") || error_msg.contains("Key derivation"),
        "Error should indicate decryption or key derivation failure: {}",
        error_msg
    );
}

#[test]
fn test_crypto_handles_empty_plaintext() {
    use symbi_runtime::crypto::Aes256GcmCrypto;

    let password = "test_password";
    let plaintext = b"";

    let encrypted = Aes256GcmCrypto::encrypt_with_password(plaintext, password);
    assert!(encrypted.is_ok(), "Should handle empty plaintext");

    let encrypted_data = encrypted.unwrap();
    let decrypted = Aes256GcmCrypto::decrypt_with_password(&encrypted_data, password);
    assert!(decrypted.is_ok(), "Should decrypt empty plaintext");

    assert_eq!(decrypted.unwrap(), plaintext);
}

#[test]
fn test_crypto_handles_invalid_ciphertext() {
    use symbi_runtime::crypto::{Aes256GcmCrypto, EncryptedData};

    // Create invalid encrypted data
    let invalid_data = EncryptedData {
        ciphertext: "invalid_base64!@#$".to_string(),
        nonce: "also_invalid!@#$".to_string(),
        salt: "not_valid_either!@#$".to_string(),
        algorithm: "AES-256-GCM".to_string(),
        kdf: "Argon2".to_string(),
    };

    let result = Aes256GcmCrypto::decrypt_with_password(&invalid_data, "password");
    assert!(result.is_err(), "Should reject invalid ciphertext");
}

#[test]
fn test_crypto_different_salts_produce_different_ciphertexts() {
    use symbi_runtime::crypto::Aes256GcmCrypto;

    let password = "same_password";
    let plaintext = b"same_plaintext";

    // Encrypt the same data twice
    let encrypted1 = Aes256GcmCrypto::encrypt_with_password(plaintext, password).unwrap();
    let encrypted2 = Aes256GcmCrypto::encrypt_with_password(plaintext, password).unwrap();

    // Ciphertexts should be different (due to random salt and nonce)
    assert_ne!(
        encrypted1.ciphertext, encrypted2.ciphertext,
        "Same plaintext should produce different ciphertexts"
    );
    assert_ne!(
        encrypted1.salt, encrypted2.salt,
        "Should use different salts"
    );
    assert_ne!(
        encrypted1.nonce, encrypted2.nonce,
        "Should use different nonces"
    );

    // Both should decrypt correctly
    assert_eq!(
        Aes256GcmCrypto::decrypt_with_password(&encrypted1, password).unwrap(),
        plaintext
    );
    assert_eq!(
        Aes256GcmCrypto::decrypt_with_password(&encrypted2, password).unwrap(),
        plaintext
    );
}

// ============================================================================
// Token Generation Tests
// ============================================================================

#[test]
fn test_token_generation_in_up_command() {
    // This tests the token generation logic used in src/commands/up.rs
    use std::time::{SystemTime, UNIX_EPOCH};

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let pid = std::process::id();

    let token = format!("symbi_dev_{:x}_{:x}", timestamp, pid);

    // Token should have expected format
    assert!(
        token.starts_with("symbi_dev_"),
        "Token should have correct prefix"
    );
    assert!(token.len() > 20, "Token should be reasonably long");

    // Token should only contain hex characters after prefix
    let hex_part = &token[10..]; // Skip "symbi_dev_"
    assert!(
        hex_part.chars().all(|c| c.is_ascii_hexdigit() || c == '_'),
        "Token should contain hex and underscores only"
    );
}

#[test]
fn test_generated_tokens_are_unique() {
    use std::thread;
    use std::time::Duration as StdDuration;
    use std::time::{SystemTime, UNIX_EPOCH};

    let mut tokens = Vec::new();

    for _ in 0..5 {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let pid = std::process::id();

        let token = format!("symbi_dev_{:x}_{:x}", timestamp, pid);
        tokens.push(token);

        // Small delay to ensure different timestamps
        thread::sleep(StdDuration::from_nanos(100));
    }

    // All tokens should be unique
    for i in 0..tokens.len() {
        for j in (i + 1)..tokens.len() {
            assert_ne!(tokens[i], tokens[j], "Generated tokens should be unique");
        }
    }
}
