//! Local encrypted secrets store.
//!
//! Secrets are stored at `.symbi/secrets.enc` using AES-256-GCM.
//! The encryption key comes from the `SYMBIONT_MASTER_KEY` env var,
//! or a generated random key stored in `.symbi/master.key` (600 perms).
//!
//! Format:
//!   [12 bytes nonce] [ciphertext] [16 bytes auth tag]
//!
//! Plaintext is a JSON map of key -> value pairs.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;

const SECRETS_FILE: &str = ".symbi/secrets.enc";
const KEY_FILE: &str = ".symbi/master.key";

fn symbi_dir() -> PathBuf {
    PathBuf::from(".symbi")
}

fn secrets_path() -> PathBuf {
    PathBuf::from(SECRETS_FILE)
}

fn key_path() -> PathBuf {
    PathBuf::from(KEY_FILE)
}

/// Derive a 32-byte key from a passphrase via SHA-256.
fn derive_key(passphrase: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(passphrase.as_bytes());
    let result = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&result);
    key
}

/// Get or create the master encryption key.
fn get_or_create_key() -> Result<[u8; 32]> {
    // First check env var
    if let Ok(passphrase) = std::env::var("SYMBIONT_MASTER_KEY") {
        return Ok(derive_key(&passphrase));
    }

    // Otherwise use or create the local key file
    let path = key_path();
    if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        return Ok(derive_key(content.trim()));
    }

    // Generate a new random passphrase
    use aes_gcm::aead::rand_core::{OsRng, RngCore};
    let mut passphrase_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut passphrase_bytes);
    let passphrase =
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, passphrase_bytes);

    std::fs::create_dir_all(symbi_dir())?;
    std::fs::write(&path, &passphrase)?;

    // Set restrictive permissions (Unix)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path)?.permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&path, perms)?;
    }

    Ok(derive_key(&passphrase))
}

/// Load all secrets from disk (empty map if file doesn't exist).
fn load_secrets() -> Result<HashMap<String, String>> {
    let path = secrets_path();
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let key_bytes = get_or_create_key()?;
    let cipher = Aes256Gcm::new_from_slice(&key_bytes)
        .map_err(|e| anyhow!("Failed to init cipher: {}", e))?;

    let encrypted = std::fs::read(&path)?;
    if encrypted.len() < 12 {
        return Err(anyhow!("Secrets file is corrupted (too short)"));
    }

    let (nonce_bytes, ciphertext) = encrypted.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| anyhow!("Failed to decrypt secrets: {}", e))?;

    let json = String::from_utf8(plaintext)?;
    let map: HashMap<String, String> = serde_json::from_str(&json)?;
    Ok(map)
}

/// Save the secrets map to disk (encrypted).
fn save_secrets(secrets: &HashMap<String, String>) -> Result<()> {
    use aes_gcm::aead::rand_core::{OsRng, RngCore};

    let key_bytes = get_or_create_key()?;
    let cipher = Aes256Gcm::new_from_slice(&key_bytes)
        .map_err(|e| anyhow!("Failed to init cipher: {}", e))?;

    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let json = serde_json::to_string(secrets)?;
    let ciphertext = cipher
        .encrypt(nonce, json.as_bytes())
        .map_err(|e| anyhow!("Failed to encrypt secrets: {}", e))?;

    let mut output = nonce_bytes.to_vec();
    output.extend_from_slice(&ciphertext);

    std::fs::create_dir_all(symbi_dir())?;
    std::fs::write(secrets_path(), output)?;

    // Set restrictive permissions
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(secrets_path())?.permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(secrets_path(), perms)?;
    }

    Ok(())
}

/// Set a secret value.
pub fn set_secret(key: &str, value: &str) -> Result<()> {
    let mut secrets = load_secrets()?;
    secrets.insert(key.to_string(), value.to_string());
    save_secrets(&secrets)
}

/// Get a secret value.
pub fn get_secret(key: &str) -> Result<Option<String>> {
    let secrets = load_secrets()?;
    Ok(secrets.get(key).cloned())
}

/// Delete a secret.
pub fn delete_secret(key: &str) -> Result<bool> {
    let mut secrets = load_secrets()?;
    let existed = secrets.remove(key).is_some();
    if existed {
        save_secrets(&secrets)?;
    }
    Ok(existed)
}

/// List all secret keys (values never returned in bulk).
pub fn list_secrets() -> Result<Vec<String>> {
    let secrets = load_secrets()?;
    let mut keys: Vec<String> = secrets.keys().cloned().collect();
    keys.sort();
    Ok(keys)
}

/// Get all secrets as env vars (for injection into deployed containers).
pub fn all_as_env() -> Result<HashMap<String, String>> {
    load_secrets()
}

/// Clear all secrets (for testing or reset).
#[cfg(test)]
fn clear_all() -> Result<()> {
    if secrets_path().exists() {
        std::fs::remove_file(secrets_path())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn in_temp_dir<F: FnOnce()>(f: F) {
        let tmp = tempfile::tempdir().unwrap();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        // Use a fixed passphrase for deterministic tests
        std::env::set_var("SYMBIONT_MASTER_KEY", "test-passphrase-for-unit-tests");
        f();
        std::env::remove_var("SYMBIONT_MASTER_KEY");
        std::env::set_current_dir(orig).unwrap();
    }

    #[test]
    #[serial]
    fn test_set_and_get() {
        in_temp_dir(|| {
            set_secret("API_KEY", "secret-value").unwrap();
            let retrieved = get_secret("API_KEY").unwrap();
            assert_eq!(retrieved, Some("secret-value".to_string()));
        });
    }

    #[test]
    #[serial]
    fn test_list() {
        in_temp_dir(|| {
            set_secret("FOO", "1").unwrap();
            set_secret("BAR", "2").unwrap();
            let mut keys = list_secrets().unwrap();
            keys.sort();
            assert_eq!(keys, vec!["BAR".to_string(), "FOO".to_string()]);
        });
    }

    #[test]
    #[serial]
    fn test_delete() {
        in_temp_dir(|| {
            set_secret("TEMP", "value").unwrap();
            assert!(delete_secret("TEMP").unwrap());
            assert!(!delete_secret("TEMP").unwrap()); // already gone
            assert_eq!(get_secret("TEMP").unwrap(), None);
        });
    }

    #[test]
    #[serial]
    fn test_encryption_roundtrip() {
        in_temp_dir(|| {
            set_secret("SENSITIVE", "don't leak this").unwrap();

            // Read raw file — should NOT contain plaintext
            let raw = std::fs::read(secrets_path()).unwrap();
            let as_str = String::from_utf8_lossy(&raw);
            assert!(!as_str.contains("don't leak this"));

            // But get_secret should return it
            assert_eq!(
                get_secret("SENSITIVE").unwrap(),
                Some("don't leak this".to_string())
            );
        });
    }

    #[test]
    #[serial]
    fn test_missing_file_returns_empty() {
        in_temp_dir(|| {
            let _ = clear_all();
            assert_eq!(list_secrets().unwrap(), Vec::<String>::new());
        });
    }
}
