//! File-backed TOFU Key Pin Store for AgentPin
//!
//! Wraps agentpin::pinning::KeyPinStore with file-backed persistence
//! at `~/.symbiont/agentpin_keys.json` with 0o600 permissions.

use agentpin::pinning::KeyPinStore;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};

use super::types::AgentPinError;

/// File-backed key pin store for TOFU key pinning
#[derive(Debug, Clone)]
pub struct AgentPinKeyStore {
    /// Path to the key store JSON file
    store_path: PathBuf,
}

impl AgentPinKeyStore {
    /// Create a new key store backed by the given file path.
    /// Creates parent directories and an empty store if it doesn't exist.
    pub fn new(store_path: &Path) -> Result<Self, AgentPinError> {
        // Create parent directories
        if let Some(parent) = store_path.parent() {
            fs::create_dir_all(parent).map_err(|e| AgentPinError::IoError {
                reason: format!("Failed to create key store directory: {}", e),
            })?;
        }

        let ks = Self {
            store_path: store_path.to_path_buf(),
        };

        // Initialize empty store if file doesn't exist
        if !store_path.exists() {
            let empty = KeyPinStore::new();
            ks.save_pin_store(&empty)?;
        }

        Ok(ks)
    }

    /// Load the pin store from disk
    pub fn load_pin_store(&self) -> Result<KeyPinStore, AgentPinError> {
        if !self.store_path.exists() {
            return Ok(KeyPinStore::new());
        }

        let file = File::open(&self.store_path).map_err(|e| AgentPinError::IoError {
            reason: format!("Failed to open key store: {}", e),
        })?;

        let reader = BufReader::new(file);
        let json: String =
            serde_json::from_reader(reader).map_err(|e| AgentPinError::KeyStoreError {
                reason: format!("Failed to deserialize key store: {}", e),
            })?;

        KeyPinStore::from_json(&json).map_err(|e| AgentPinError::KeyStoreError {
            reason: format!("Failed to parse key store: {}", e),
        })
    }

    /// Save the pin store to disk with restricted permissions
    pub fn save_pin_store(&self, store: &KeyPinStore) -> Result<(), AgentPinError> {
        let json = store.to_json().map_err(|e| AgentPinError::KeyStoreError {
            reason: format!("Failed to serialize key store: {}", e),
        })?;

        // Create parent directories
        if let Some(parent) = self.store_path.parent() {
            fs::create_dir_all(parent).map_err(|e| AgentPinError::IoError {
                reason: format!("Failed to create key store directory: {}", e),
            })?;
        }

        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.store_path)
            .map_err(|e| AgentPinError::IoError {
                reason: format!("Failed to open key store for writing: {}", e),
            })?;

        // Set restrictive permissions on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            fs::set_permissions(&self.store_path, perms).map_err(|e| AgentPinError::IoError {
                reason: format!("Failed to set key store permissions: {}", e),
            })?;
        }

        // Write as a JSON string value so we can round-trip through KeyPinStore::from_json
        serde_json::to_writer_pretty(&mut file, &json).map_err(|e| AgentPinError::IoError {
            reason: format!("Failed to write key store: {}", e),
        })?;

        file.flush().map_err(|e| AgentPinError::IoError {
            reason: format!("Failed to flush key store: {}", e),
        })?;

        Ok(())
    }

    /// Get the path to the store file
    pub fn store_path(&self) -> &Path {
        &self.store_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_create_new_key_store() {
        let temp_dir = TempDir::new().unwrap();
        let store_path = temp_dir.path().join("agentpin_keys.json");

        let ks = AgentPinKeyStore::new(&store_path).unwrap();
        assert!(ks.store_path().exists());
    }

    #[test]
    fn test_load_empty_store() {
        let temp_dir = TempDir::new().unwrap();
        let store_path = temp_dir.path().join("agentpin_keys.json");

        let ks = AgentPinKeyStore::new(&store_path).unwrap();
        let pin_store = ks.load_pin_store().unwrap();

        // A fresh pin store should have no domains
        assert!(pin_store.get_domain("anything.com").is_none());
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let store_path = temp_dir.path().join("agentpin_keys.json");

        let ks = AgentPinKeyStore::new(&store_path).unwrap();
        let store = KeyPinStore::new();

        ks.save_pin_store(&store).unwrap();
        let loaded = ks.load_pin_store().unwrap();

        // Both should serialize to the same JSON
        assert_eq!(store.to_json().unwrap(), loaded.to_json().unwrap());
    }

    #[test]
    fn test_creates_parent_directories() {
        let temp_dir = TempDir::new().unwrap();
        let store_path = temp_dir.path().join("nested").join("dir").join("keys.json");

        let ks = AgentPinKeyStore::new(&store_path).unwrap();
        assert!(ks.store_path().exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = TempDir::new().unwrap();
        let store_path = temp_dir.path().join("agentpin_keys.json");

        let ks = AgentPinKeyStore::new(&store_path).unwrap();
        let store = KeyPinStore::new();
        ks.save_pin_store(&store).unwrap();

        let metadata = fs::metadata(&store_path).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn test_load_nonexistent_returns_empty() {
        let temp_dir = TempDir::new().unwrap();
        let store_path = temp_dir.path().join("does_not_exist.json");

        // Don't create via new(), just construct directly
        let ks = AgentPinKeyStore {
            store_path: store_path.clone(),
        };

        let pin_store = ks.load_pin_store().unwrap();
        assert!(pin_store.get_domain("anything").is_none());
    }
}
