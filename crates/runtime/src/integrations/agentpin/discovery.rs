//! Discovery Document Cache for AgentPin
//!
//! Caches discovery documents at `~/.symbiont/agentpin_discovery/` with a
//! configurable TTL to avoid fetching on every verification.

use agentpin::types::discovery::DiscoveryDocument;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::types::AgentPinError;

/// File-backed discovery document cache
#[derive(Debug, Clone)]
pub struct DiscoveryCache {
    /// Directory where cached documents are stored
    cache_dir: PathBuf,
    /// Time-to-live for cached entries
    ttl: Duration,
}

/// A cached discovery document with timestamp
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CachedEntry {
    /// The cached discovery document
    document: DiscoveryDocument,
    /// Unix timestamp when the entry was cached
    cached_at: u64,
}

impl DiscoveryCache {
    /// Create a new discovery cache at the given directory path
    pub fn new(cache_dir: &Path, ttl_secs: u64) -> Result<Self, AgentPinError> {
        fs::create_dir_all(cache_dir).map_err(|e| AgentPinError::IoError {
            reason: format!("Failed to create discovery cache directory: {}", e),
        })?;

        Ok(Self {
            cache_dir: cache_dir.to_path_buf(),
            ttl: Duration::from_secs(ttl_secs),
        })
    }

    /// Get a cached discovery document for the given domain, if it exists and hasn't expired
    pub fn get(&self, domain: &str) -> Option<DiscoveryDocument> {
        let path = self.cache_file_path(domain);
        if !path.exists() {
            return None;
        }

        let file = File::open(&path).ok()?;
        let reader = BufReader::new(file);
        let entry: CachedEntry = serde_json::from_reader(reader).ok()?;

        // Check TTL
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        if now.saturating_sub(entry.cached_at) > self.ttl.as_secs() {
            // Expired - remove the stale file (best effort)
            let _ = fs::remove_file(&path);
            return None;
        }

        Some(entry.document)
    }

    /// Store a discovery document in the cache
    pub fn put(&self, domain: &str, document: &DiscoveryDocument) -> Result<(), AgentPinError> {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let entry = CachedEntry {
            document: document.clone(),
            cached_at: now,
        };

        let path = self.cache_file_path(domain);

        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .map_err(|e| AgentPinError::IoError {
                reason: format!("Failed to open cache file for writing: {}", e),
            })?;

        serde_json::to_writer_pretty(&mut file, &entry).map_err(|e| AgentPinError::IoError {
            reason: format!("Failed to write cache entry: {}", e),
        })?;

        file.flush().map_err(|e| AgentPinError::IoError {
            reason: format!("Failed to flush cache file: {}", e),
        })?;

        Ok(())
    }

    /// Remove a cached entry for a domain
    pub fn invalidate(&self, domain: &str) -> Result<(), AgentPinError> {
        let path = self.cache_file_path(domain);
        if path.exists() {
            fs::remove_file(&path).map_err(|e| AgentPinError::IoError {
                reason: format!("Failed to remove cache entry: {}", e),
            })?;
        }
        Ok(())
    }

    /// Clear all cached entries
    pub fn clear(&self) -> Result<(), AgentPinError> {
        if self.cache_dir.exists() {
            for entry in (fs::read_dir(&self.cache_dir).map_err(|e| AgentPinError::IoError {
                reason: format!("Failed to read cache directory: {}", e),
            })?)
            .flatten()
            {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("json") {
                    let _ = fs::remove_file(&path);
                }
            }
        }
        Ok(())
    }

    /// Compute the cache file path for a domain.
    /// Sanitizes the domain name to produce a safe filename.
    fn cache_file_path(&self, domain: &str) -> PathBuf {
        let safe_name: String = domain
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '.' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.cache_dir.join(format!("{}.json", safe_name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_test_discovery() -> DiscoveryDocument {
        agentpin::discovery::build_discovery_document(
            "test.example.com",
            agentpin::types::discovery::EntityType::Maker,
            vec![],
            vec![],
            3,
            "2026-02-06T00:00:00Z",
        )
    }

    #[test]
    fn test_cache_miss() {
        let temp_dir = TempDir::new().unwrap();
        let cache = DiscoveryCache::new(temp_dir.path(), 3600).unwrap();
        assert!(cache.get("nonexistent.com").is_none());
    }

    #[test]
    fn test_cache_hit() {
        let temp_dir = TempDir::new().unwrap();
        let cache = DiscoveryCache::new(temp_dir.path(), 3600).unwrap();
        let doc = make_test_discovery();

        cache.put("test.example.com", &doc).unwrap();
        let cached = cache.get("test.example.com");
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().entity, "test.example.com");
    }

    #[test]
    fn test_cache_expiry() {
        let temp_dir = TempDir::new().unwrap();
        let cache = DiscoveryCache::new(temp_dir.path(), 3600).unwrap();
        let doc = make_test_discovery();

        // Manually write an entry with an old timestamp
        let path = cache.cache_file_path("test.example.com");
        let entry = super::CachedEntry {
            document: doc,
            cached_at: 0, // Unix epoch = long expired
        };
        let mut file = std::fs::File::create(&path).unwrap();
        serde_json::to_writer(&mut file, &entry).unwrap();

        assert!(cache.get("test.example.com").is_none());
    }

    #[test]
    fn test_cache_invalidate() {
        let temp_dir = TempDir::new().unwrap();
        let cache = DiscoveryCache::new(temp_dir.path(), 3600).unwrap();
        let doc = make_test_discovery();

        cache.put("test.example.com", &doc).unwrap();
        assert!(cache.get("test.example.com").is_some());

        cache.invalidate("test.example.com").unwrap();
        assert!(cache.get("test.example.com").is_none());
    }

    #[test]
    fn test_cache_clear() {
        let temp_dir = TempDir::new().unwrap();
        let cache = DiscoveryCache::new(temp_dir.path(), 3600).unwrap();
        let doc = make_test_discovery();

        cache.put("a.example.com", &doc).unwrap();
        cache.put("b.example.com", &doc).unwrap();

        cache.clear().unwrap();

        assert!(cache.get("a.example.com").is_none());
        assert!(cache.get("b.example.com").is_none());
    }

    #[test]
    fn test_domain_sanitization() {
        let temp_dir = TempDir::new().unwrap();
        let cache = DiscoveryCache::new(temp_dir.path(), 3600).unwrap();
        let doc = make_test_discovery();

        // Domain with odd chars should not break filesystem
        cache.put("weird/domain:8080", &doc).unwrap();
        assert!(cache.get("weird/domain:8080").is_some());
    }
}
