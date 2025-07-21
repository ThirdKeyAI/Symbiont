//! Local Key Store for Trust-On-First-Use (TOFU) mechanism
//! 
//! Provides secure storage and management of public keys for tool providers
//! to prevent man-in-the-middle and key substitution attacks.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;
use std::sync::{Arc, RwLock};
use serde_json;

use super::types::{KeyStoreConfig, KeyStoreError, PinnedKey};

/// Thread-safe local key store implementing TOFU mechanism
#[derive(Debug)]
pub struct LocalKeyStore {
    /// Configuration for the key store
    config: KeyStoreConfig,
    /// In-memory cache of pinned keys
    keys: Arc<RwLock<HashMap<String, PinnedKey>>>,
}

impl LocalKeyStore {
    /// Create a new key store with default configuration
    pub fn new() -> Result<Self, KeyStoreError> {
        Self::with_config(KeyStoreConfig::default())
    }

    /// Create a new key store with custom configuration
    pub fn with_config(config: KeyStoreConfig) -> Result<Self, KeyStoreError> {
        let store = Self {
            config,
            keys: Arc::new(RwLock::new(HashMap::new())),
        };
        
        // Load existing keys from file
        store.load_from_file()?;
        
        Ok(store)
    }

    /// Pin a new key for the given identifier (TOFU logic)
    /// Returns an error if a different key is already pinned for this identifier
    pub fn pin_key(&self, key: PinnedKey) -> Result<(), KeyStoreError> {
        let mut keys = self.keys.write().map_err(|e| KeyStoreError::IoError {
            reason: format!("Failed to acquire write lock: {}", e),
        })?;

        // Check if key already exists for this identifier
        if let Some(existing_key) = keys.get(&key.identifier) {
            // TOFU: If key already exists, it must match exactly
            if existing_key.public_key != key.public_key || 
               existing_key.fingerprint != key.fingerprint {
                return Err(KeyStoreError::KeyMismatch {
                    identifier: key.identifier.clone(),
                });
            }
            // Key already exists and matches - this is fine
            return Ok(());
        }

        // Pin the new key
        keys.insert(key.identifier.clone(), key);
        drop(keys); // Release lock before file I/O

        // Persist to file
        self.save_to_file()
    }

    /// Retrieve a pinned key by identifier
    pub fn get_key(&self, identifier: &str) -> Result<PinnedKey, KeyStoreError> {
        let keys = self.keys.read().map_err(|e| KeyStoreError::IoError {
            reason: format!("Failed to acquire read lock: {}", e),
        })?;

        keys.get(identifier)
            .cloned()
            .ok_or_else(|| KeyStoreError::KeyNotFound {
                identifier: identifier.to_string(),
            })
    }

    /// Check if a key is pinned for the given identifier
    pub fn has_key(&self, identifier: &str) -> Result<bool, KeyStoreError> {
        let keys = self.keys.read().map_err(|e| KeyStoreError::IoError {
            reason: format!("Failed to acquire read lock: {}", e),
        })?;

        Ok(keys.contains_key(identifier))
    }

    /// Remove a pinned key by identifier
    pub fn remove_key(&self, identifier: &str) -> Result<Option<PinnedKey>, KeyStoreError> {
        let mut keys = self.keys.write().map_err(|e| KeyStoreError::IoError {
            reason: format!("Failed to acquire write lock: {}", e),
        })?;

        let removed_key = keys.remove(identifier);
        drop(keys); // Release lock before file I/O

        if removed_key.is_some() {
            self.save_to_file()?;
        }

        Ok(removed_key)
    }

    /// List all pinned key identifiers
    pub fn list_identifiers(&self) -> Result<Vec<String>, KeyStoreError> {
        let keys = self.keys.read().map_err(|e| KeyStoreError::IoError {
            reason: format!("Failed to acquire read lock: {}", e),
        })?;

        Ok(keys.keys().cloned().collect())
    }

    /// Get all pinned keys
    pub fn list_keys(&self) -> Result<Vec<PinnedKey>, KeyStoreError> {
        let keys = self.keys.read().map_err(|e| KeyStoreError::IoError {
            reason: format!("Failed to acquire read lock: {}", e),
        })?;

        Ok(keys.values().cloned().collect())
    }

    /// Clear all pinned keys
    pub fn clear(&self) -> Result<(), KeyStoreError> {
        let mut keys = self.keys.write().map_err(|e| KeyStoreError::IoError {
            reason: format!("Failed to acquire write lock: {}", e),
        })?;

        keys.clear();
        drop(keys); // Release lock before file I/O

        self.save_to_file()
    }

    /// Verify that a key matches the pinned key for an identifier
    pub fn verify_key(&self, identifier: &str, public_key: &str, fingerprint: &str) -> Result<bool, KeyStoreError> {
        let pinned_key = self.get_key(identifier)?;
        Ok(pinned_key.public_key == public_key && pinned_key.fingerprint == fingerprint)
    }

    /// Load keys from the store file
    fn load_from_file(&self) -> Result<(), KeyStoreError> {
        if !self.config.store_path.exists() {
            if self.config.create_if_missing {
                // Create parent directories if they don't exist
                if let Some(parent) = self.config.store_path.parent() {
                    fs::create_dir_all(parent).map_err(|e| KeyStoreError::IoError {
                        reason: format!("Failed to create parent directories: {}", e),
                    })?;
                }
                // Create empty store file
                self.save_to_file()?;
                return Ok(());
            } else {
                return Err(KeyStoreError::StoreFileNotFound {
                    path: self.config.store_path.display().to_string(),
                });
            }
        }

        let file = File::open(&self.config.store_path).map_err(|e| KeyStoreError::ReadFailed {
            reason: format!("Failed to open store file: {}", e),
        })?;

        let reader = BufReader::new(file);
        let loaded_keys: HashMap<String, PinnedKey> = serde_json::from_reader(reader)
            .map_err(|e| KeyStoreError::SerializationFailed {
                reason: format!("Failed to deserialize keys: {}", e),
            })?;

        let mut keys = self.keys.write().map_err(|e| KeyStoreError::IoError {
            reason: format!("Failed to acquire write lock: {}", e),
        })?;

        *keys = loaded_keys;

        Ok(())
    }

    /// Save keys to the store file
    fn save_to_file(&self) -> Result<(), KeyStoreError> {
        let keys = self.keys.read().map_err(|e| KeyStoreError::IoError {
            reason: format!("Failed to acquire read lock: {}", e),
        })?;

        // Create parent directories if they don't exist
        if let Some(parent) = self.config.store_path.parent() {
            fs::create_dir_all(parent).map_err(|e| KeyStoreError::WriteFailed {
                reason: format!("Failed to create parent directories: {}", e),
            })?;
        }

        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.config.store_path)
            .map_err(|e| KeyStoreError::WriteFailed {
                reason: format!("Failed to open store file for writing: {}", e),
            })?;

        // Set file permissions on Unix systems
        #[cfg(unix)]
        if let Some(permissions) = self.config.file_permissions {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = file.metadata()
                .map_err(|e| KeyStoreError::WriteFailed {
                    reason: format!("Failed to get file metadata: {}", e),
                })?
                .permissions();
            perms.set_mode(permissions);
            fs::set_permissions(&self.config.store_path, perms)
                .map_err(|e| KeyStoreError::PermissionDenied {
                    reason: format!("Failed to set file permissions: {}", e),
                })?;
        }

        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, &*keys)
            .map_err(|e| KeyStoreError::SerializationFailed {
                reason: format!("Failed to serialize keys: {}", e),
            })?;

        writer.flush().map_err(|e| KeyStoreError::WriteFailed {
            reason: format!("Failed to flush writer: {}", e),
        })?;

        Ok(())
    }

    /// Get the path to the store file
    pub fn store_path(&self) -> &Path {
        &self.config.store_path
    }
}

impl Default for LocalKeyStore {
    fn default() -> Self {
        Self::new().expect("Failed to create default key store")
    }
}

// Implement Clone for LocalKeyStore to allow sharing between threads
impl Clone for LocalKeyStore {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            keys: Arc::clone(&self.keys),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::collections::HashMap;

    fn create_test_key_store() -> (LocalKeyStore, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let store_path = temp_dir.path().join("test_keys.json");
        
        let config = KeyStoreConfig {
            store_path,
            create_if_missing: true,
            file_permissions: Some(0o600),
        };
        
        let store = LocalKeyStore::with_config(config).unwrap();
        (store, temp_dir)
    }

    fn create_test_key(identifier: &str) -> PinnedKey {
        PinnedKey::new(
            identifier.to_string(),
            format!("public_key_for_{}", identifier),
            "Ed25519".to_string(),
            format!("fingerprint_for_{}", identifier),
        )
    }

    #[test]
    fn test_pin_and_get_key() {
        let (store, _temp_dir) = create_test_key_store();
        let key = create_test_key("example.com");

        // Pin the key
        store.pin_key(key.clone()).unwrap();

        // Retrieve the key
        let retrieved_key = store.get_key("example.com").unwrap();
        assert_eq!(retrieved_key.identifier, key.identifier);
        assert_eq!(retrieved_key.public_key, key.public_key);
        assert_eq!(retrieved_key.fingerprint, key.fingerprint);
    }

    #[test]
    fn test_tofu_key_mismatch() {
        let (store, _temp_dir) = create_test_key_store();
        let key1 = create_test_key("example.com");
        let mut key2 = create_test_key("example.com");
        key2.public_key = "different_public_key".to_string();

        // Pin the first key
        store.pin_key(key1).unwrap();

        // Try to pin a different key for the same identifier
        let result = store.pin_key(key2);
        assert!(matches!(result, Err(KeyStoreError::KeyMismatch { .. })));
    }

    #[test]
    fn test_tofu_same_key_twice() {
        let (store, _temp_dir) = create_test_key_store();
        let key = create_test_key("example.com");

        // Pin the key twice - should succeed
        store.pin_key(key.clone()).unwrap();
        store.pin_key(key).unwrap();
    }

    #[test]
    fn test_has_key() {
        let (store, _temp_dir) = create_test_key_store();
        let key = create_test_key("example.com");

        assert!(!store.has_key("example.com").unwrap());
        store.pin_key(key).unwrap();
        assert!(store.has_key("example.com").unwrap());
    }

    #[test]
    fn test_remove_key() {
        let (store, _temp_dir) = create_test_key_store();
        let key = create_test_key("example.com");

        store.pin_key(key.clone()).unwrap();
        assert!(store.has_key("example.com").unwrap());

        let removed_key = store.remove_key("example.com").unwrap();
        assert!(removed_key.is_some());
        assert_eq!(removed_key.unwrap().identifier, key.identifier);
        assert!(!store.has_key("example.com").unwrap());
    }

    #[test]
    fn test_list_keys() {
        let (store, _temp_dir) = create_test_key_store();
        let key1 = create_test_key("example.com");
        let key2 = create_test_key("test.org");

        store.pin_key(key1).unwrap();
        store.pin_key(key2).unwrap();

        let identifiers = store.list_identifiers().unwrap();
        assert_eq!(identifiers.len(), 2);
        assert!(identifiers.contains(&"example.com".to_string()));
        assert!(identifiers.contains(&"test.org".to_string()));

        let keys = store.list_keys().unwrap();
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn test_clear() {
        let (store, _temp_dir) = create_test_key_store();
        let key1 = create_test_key("example.com");
        let key2 = create_test_key("test.org");

        store.pin_key(key1).unwrap();
        store.pin_key(key2).unwrap();
        assert_eq!(store.list_identifiers().unwrap().len(), 2);

        store.clear().unwrap();
        assert_eq!(store.list_identifiers().unwrap().len(), 0);
    }

    #[test]
    fn test_verify_key() {
        let (store, _temp_dir) = create_test_key_store();
        let key = create_test_key("example.com");

        store.pin_key(key.clone()).unwrap();

        // Verify with correct key
        assert!(store.verify_key("example.com", &key.public_key, &key.fingerprint).unwrap());

        // Verify with incorrect key
        assert!(!store.verify_key("example.com", "wrong_key", &key.fingerprint).unwrap());
        assert!(!store.verify_key("example.com", &key.public_key, "wrong_fingerprint").unwrap());
    }

    #[test]
    fn test_persistence() {
        let temp_dir = TempDir::new().unwrap();
        let store_path = temp_dir.path().join("test_keys.json");
        
        let config = KeyStoreConfig {
            store_path: store_path.clone(),
            create_if_missing: true,
            file_permissions: Some(0o600),
        };

        // Create store and add a key
        {
            let store = LocalKeyStore::with_config(config.clone()).unwrap();
            let key = create_test_key("example.com");
            store.pin_key(key).unwrap();
        }

        // Create new store instance and verify key persisted
        {
            let store = LocalKeyStore::with_config(config).unwrap();
            assert!(store.has_key("example.com").unwrap());
            let retrieved_key = store.get_key("example.com").unwrap();
            assert_eq!(retrieved_key.identifier, "example.com");
        }
    }

    #[test]
    fn test_key_not_found() {
        let (store, _temp_dir) = create_test_key_store();
        
        let result = store.get_key("nonexistent.com");
        assert!(matches!(result, Err(KeyStoreError::KeyNotFound { .. })));
    }

    #[test]
    fn test_pinned_key_with_metadata() {
        let (store, _temp_dir) = create_test_key_store();
        
        let mut metadata = HashMap::new();
        metadata.insert("version".to_string(), serde_json::Value::String("1.0.0".to_string()));
        metadata.insert("source".to_string(), serde_json::Value::String("test".to_string()));
        
        let key = PinnedKey::with_metadata(
            "example.com".to_string(),
            "test_public_key".to_string(),
            "Ed25519".to_string(),
            "test_fingerprint".to_string(),
            metadata.clone(),
        );

        store.pin_key(key).unwrap();
        
        let retrieved_key = store.get_key("example.com").unwrap();
        assert!(retrieved_key.metadata.is_some());
        let retrieved_metadata = retrieved_key.metadata.unwrap();
        assert_eq!(retrieved_metadata.get("version"), metadata.get("version"));
        assert_eq!(retrieved_metadata.get("source"), metadata.get("source"));
    }
}