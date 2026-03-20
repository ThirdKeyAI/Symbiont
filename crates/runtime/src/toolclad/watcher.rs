//! Hot-reload watcher for ToolClad manifests (development mode only)
//!
//! Polls the tools/ directory for changes and reloads manifests.
//! Uses simple file modification time comparison — no extra dependencies.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

use super::manifest::{load_manifest, Manifest};

/// Shared manifest registry that can be updated by the watcher.
pub type ManifestRegistry = Arc<RwLock<HashMap<String, Manifest>>>;

/// Create a new manifest registry from an initial set of manifests.
pub fn create_registry(manifests: Vec<(String, Manifest)>) -> ManifestRegistry {
    Arc::new(RwLock::new(manifests.into_iter().collect()))
}

/// Start a background polling watcher for the tools directory.
/// Only call this in development mode.
pub fn start_watcher(registry: ManifestRegistry, tools_dir: PathBuf, poll_interval: Duration) {
    std::thread::spawn(move || {
        let mut last_seen: HashMap<PathBuf, SystemTime> = HashMap::new();

        // Initialize with current state
        if let Ok(entries) = std::fs::read_dir(&tools_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if is_clad_toml(&path) {
                    if let Ok(meta) = std::fs::metadata(&path) {
                        if let Ok(modified) = meta.modified() {
                            last_seen.insert(path, modified);
                        }
                    }
                }
            }
        }

        loop {
            std::thread::sleep(poll_interval);

            let mut current: HashMap<PathBuf, SystemTime> = HashMap::new();
            if let Ok(entries) = std::fs::read_dir(&tools_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if is_clad_toml(&path) {
                        if let Ok(meta) = std::fs::metadata(&path) {
                            if let Ok(modified) = meta.modified() {
                                current.insert(path, modified);
                            }
                        }
                    }
                }
            }

            // Check for new or modified files
            for (path, modified) in &current {
                let needs_reload = match last_seen.get(path) {
                    Some(prev) => modified > prev,
                    None => true, // New file
                };

                if needs_reload {
                    match load_manifest(path) {
                        Ok(manifest) => {
                            let name = manifest.tool.name.clone();
                            if let Ok(mut reg) = registry.write() {
                                reg.insert(name.clone(), manifest);
                                eprintln!(
                                    "→ Hot-reloaded tools/{}",
                                    path.file_name().unwrap_or_default().to_string_lossy()
                                );
                            }
                        }
                        Err(e) => {
                            eprintln!("⚠ Failed to reload {}: {}", path.display(), e);
                        }
                    }
                }
            }

            // Check for deleted files
            let removed: Vec<PathBuf> = last_seen
                .keys()
                .filter(|k| !current.contains_key(*k))
                .cloned()
                .collect();

            for path in &removed {
                // Try to figure out which tool name this was
                if let Ok(reg) = registry.read() {
                    let name_to_remove: Option<String> = reg.iter().find_map(|(name, _)| {
                        let expected = tools_dir.join(format!("{}.clad.toml", name));
                        if expected == *path {
                            Some(name.clone())
                        } else {
                            None
                        }
                    });
                    drop(reg);
                    if let Some(name) = name_to_remove {
                        if let Ok(mut reg) = registry.write() {
                            reg.remove(&name);
                            eprintln!("→ Removed tool: {}", name);
                        }
                    }
                }
            }

            last_seen = current;
        }
    });
}

fn is_clad_toml(path: &Path) -> bool {
    path.file_name()
        .map(|n| n.to_string_lossy().ends_with(".clad.toml"))
        .unwrap_or(false)
}
