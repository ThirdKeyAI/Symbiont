#![no_main]

//! Fuzz target for the crypto module (AES-256-GCM).
//!
//! Exercises:
//! - Password-based encrypt → decrypt roundtrip
//! - Malformed EncryptedData (bad base64, truncated nonce, wrong algorithm)
//! - Direct key encrypt → decrypt roundtrip
//! - Garbage ciphertext decryption (must fail gracefully, never panic)

use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use symbi_runtime::crypto::{Aes256GcmCrypto, EncryptedData, KeyUtils};

#[derive(Arbitrary, Debug)]
struct Input {
    mode: CryptoFuzzMode,
}

#[derive(Arbitrary, Debug)]
enum CryptoFuzzMode {
    /// Roundtrip: encrypt then decrypt with same password.
    PasswordRoundtrip {
        plaintext: Vec<u8>,
        password: String,
    },
    /// Decrypt malformed EncryptedData.
    MalformedDecrypt {
        ciphertext: String,
        nonce: String,
        salt: String,
        algorithm: String,
        kdf: String,
        password: String,
    },
    /// Direct key encrypt → decrypt roundtrip.
    DirectKeyRoundtrip {
        plaintext: Vec<u8>,
        key: String,
    },
    /// Decrypt garbage bytes with direct key.
    GarbageDecrypt {
        data: Vec<u8>,
        key: String,
    },
    /// Wrong password decryption (must fail).
    WrongPassword {
        plaintext: Vec<u8>,
        correct_password: String,
        wrong_password: String,
    },
}

fn clamp_bytes(mut v: Vec<u8>, max: usize) -> Vec<u8> {
    v.truncate(max);
    v
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
        CryptoFuzzMode::PasswordRoundtrip { plaintext, password } => {
            let plaintext = clamp_bytes(plaintext, 4096);
            let password = clamp(password, 128, "fuzz-password");

            // Encrypt must not panic.
            let encrypted = match Aes256GcmCrypto::encrypt_with_password(&plaintext, &password) {
                Ok(e) => e,
                Err(_) => return,
            };

            // Decrypt with same password must succeed and match.
            let decrypted = Aes256GcmCrypto::decrypt_with_password(&encrypted, &password);
            assert!(
                decrypted.is_ok(),
                "roundtrip decrypt must succeed",
            );
            assert_eq!(
                decrypted.unwrap(),
                plaintext,
                "roundtrip must produce original plaintext",
            );
        }

        CryptoFuzzMode::MalformedDecrypt {
            ciphertext, nonce, salt, algorithm, kdf, password,
        } => {
            let data = EncryptedData {
                ciphertext: clamp(ciphertext, 1024, "AAAA"),
                nonce: clamp(nonce, 64, "AAAA"),
                salt: clamp(salt, 128, "AAAA"),
                algorithm: clamp(algorithm, 32, "AES-256-GCM"),
                kdf: clamp(kdf, 32, "Argon2"),
            };
            let password = clamp(password, 128, "fuzz-password");

            // Must not panic — errors are expected.
            let _ = Aes256GcmCrypto::decrypt_with_password(&data, &password);
        }

        CryptoFuzzMode::DirectKeyRoundtrip { plaintext, key } => {
            let plaintext = clamp_bytes(plaintext, 4096);
            let key = clamp(key, 128, "");

            // If key is empty, generate one.
            let key = if key.is_empty() {
                KeyUtils::new().generate_key()
            } else {
                key
            };

            let crypto = Aes256GcmCrypto::new();

            // Encrypt must not panic.
            let encrypted = match crypto.encrypt(&plaintext, &key) {
                Ok(e) => e,
                Err(_) => return,
            };

            // Decrypt with same key must succeed and match.
            let decrypted = crypto.decrypt(&encrypted, &key);
            assert!(
                decrypted.is_ok(),
                "direct key roundtrip decrypt must succeed",
            );
            assert_eq!(
                decrypted.unwrap(),
                plaintext,
                "direct key roundtrip must produce original plaintext",
            );
        }

        CryptoFuzzMode::GarbageDecrypt { data, key } => {
            let data = clamp_bytes(data, 4096);
            let key = clamp(key, 128, "garbage-key");

            let crypto = Aes256GcmCrypto::new();

            // Must not panic — errors are expected.
            let _ = crypto.decrypt(&data, &key);
        }

        CryptoFuzzMode::WrongPassword { plaintext, correct_password, wrong_password } => {
            let plaintext = clamp_bytes(plaintext, 1024);
            let correct = clamp(correct_password, 128, "correct-pw");
            let wrong = clamp(wrong_password, 128, "wrong-pw");

            // Skip if passwords happen to match.
            if correct == wrong {
                return;
            }

            let encrypted = match Aes256GcmCrypto::encrypt_with_password(&plaintext, &correct) {
                Ok(e) => e,
                Err(_) => return,
            };

            // Wrong password must fail.
            let result = Aes256GcmCrypto::decrypt_with_password(&encrypted, &wrong);
            assert!(
                result.is_err(),
                "wrong password must fail decryption",
            );
        }
    }
});
