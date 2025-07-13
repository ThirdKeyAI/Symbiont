use anyhow::Result;
use git2::{Repository, RepositoryOpenFlags};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tracing::{info, warn};

use crate::config::GitConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub path: String,
    pub content: String,
    pub size: usize,
    pub extension: Option<String>,
    pub last_modified: Option<SystemTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryInfo {
    pub url: String,
    pub local_path: PathBuf,
    pub description: Option<String>,
    pub languages: Vec<String>,
    pub files: Vec<FileInfo>,
    pub total_size: usize,
    pub file_count: usize,
}

pub struct GitRepository {
    pub url: String,
    pub local_path: PathBuf,
    pub config: GitConfig,
}

impl GitRepository {
    pub fn new(url: String, config: GitConfig) -> Result<Self> {
        // Create a unique directory name based on the repository URL
        let repo_name = Self::extract_repo_name(&url);
        let local_path = config.clone_base_path.join(&repo_name);
        
        Ok(Self {
            url,
            local_path,
            config,
        })
    }

    fn extract_repo_name(url: &str) -> String {
        url.split('/')
            .last()
            .unwrap_or("unknown")
            .trim_end_matches(".git")
            .to_string()
    }

    pub async fn clone_or_update(&self) -> Result<Repository> {
        if self.local_path.exists() {
            info!("Repository already exists, opening: {:?}", self.local_path);
            let repo = Repository::open_ext(
                &self.local_path,
                RepositoryOpenFlags::empty(),
                &[] as &[&std::ffi::OsStr],
            )?;
            
            // Try to update (fetch latest changes)
            if let Ok(mut remote) = repo.find_remote("origin") {
                if let Err(e) = remote.fetch(&["refs/heads/*:refs/remotes/origin/*"], None, None) {
                    warn!("Failed to fetch updates: {}", e);
                }
            }
            
            Ok(repo)
        } else {
            info!("Cloning repository from: {}", self.url);
            std::fs::create_dir_all(&self.local_path)?;
            
            let repo = Repository::clone(&self.url, &self.local_path)?;
            info!("Successfully cloned to: {:?}", self.local_path);
            Ok(repo)
        }
    }

    pub async fn analyze_repository(&self) -> Result<RepositoryInfo> {
        let repo = self.clone_or_update().await?;
        
        info!("Analyzing repository structure");
        
        let mut files = Vec::new();
        let mut total_size = 0;
        let mut languages = std::collections::HashSet::new();
        
        // Walk through the repository files
        self.scan_directory(&self.local_path, &mut files, &mut total_size, &mut languages)?;
        
        // Get repository description if available
        let description = self.get_repository_description(&repo);
        
        let repo_info = RepositoryInfo {
            url: self.url.clone(),
            local_path: self.local_path.clone(),
            description,
            languages: languages.into_iter().collect(),
            file_count: files.len(),
            total_size,
            files,
        };
        
        info!("Repository analysis complete: {} files, {} bytes", repo_info.file_count, repo_info.total_size);
        Ok(repo_info)
    }

    fn scan_directory(
        &self,
        dir: &Path,
        files: &mut Vec<FileInfo>,
        total_size: &mut usize,
        languages: &mut std::collections::HashSet<String>,
    ) -> Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            
            // Skip hidden files and excluded directories
            if let Some(file_name) = path.file_name() {
                let file_name_str = file_name.to_string_lossy();
                if file_name_str.starts_with('.') {
                    continue;
                }
                
                if path.is_dir() {
                    if self.config.ignore_patterns.contains(&file_name_str.to_string()) {
                        continue;
                    }
                    // Recursively scan subdirectories
                    self.scan_directory(&path, files, total_size, languages)?;
                    continue;
                }
            }
            
            if path.is_file() {
                if let Ok(file_info) = self.process_file(&path, languages) {
                    *total_size += file_info.size;
                    files.push(file_info);
                }
            }
        }
        
        Ok(())
    }

    fn process_file(
        &self,
        file_path: &Path,
        languages: &mut std::collections::HashSet<String>,
    ) -> Result<FileInfo> {
        // Get file extension
        let extension = file_path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|s| s.to_string());
        
        // Check if we should include this file
        if let Some(ref ext) = extension {
            if !self.config.allowed_extensions.contains(ext) {
                anyhow::bail!("File extension {} not included", ext);
            }
            languages.insert(ext.clone());
        }
        
        // Get file metadata
        let metadata = std::fs::metadata(file_path)?;
        let size = metadata.len() as usize;
        
        // Check file size limit
        if size > (self.config.max_file_size_mb * 1024 * 1024) {
            anyhow::bail!("File too large: {} bytes", size);
        }
        
        // Read file content
        let content = std::fs::read_to_string(file_path)
            .unwrap_or_else(|_| "[Binary file or read error]".to_string());
        
        // Get relative path from repository root
        let relative_path = file_path
            .strip_prefix(&self.local_path)
            .unwrap_or(file_path)
            .to_string_lossy()
            .to_string();
        
        Ok(FileInfo {
            path: relative_path,
            content,
            size,
            extension,
            last_modified: metadata.modified().ok(),
        })
    }

    fn get_repository_description(&self, repo: &Repository) -> Option<String> {
        // Try to get description from various sources
        
        // Check for README files
        for readme_name in &["README.md", "README.txt", "README.rst", "README"] {
            let readme_path = self.local_path.join(readme_name);
            if readme_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&readme_path) {
                    // Return first paragraph as description
                    return content
                        .lines()
                        .find(|line| !line.trim().is_empty() && !line.starts_with('#'))
                        .map(|s| s.trim().to_string());
                }
            }
        }
        
        // Try to get from Git config
        if let Ok(config) = repo.config() {
            if let Ok(description) = config.get_string("remote.origin.description") {
                return Some(description);
            }
        }
        
        None
    }

    pub fn cleanup(&self) -> Result<()> {
        if self.local_path.exists() {
            info!("Cleaning up repository: {:?}", self.local_path);
            std::fs::remove_dir_all(&self.local_path)?;
        }
        Ok(())
    }
}

impl Drop for GitRepository {
    fn drop(&mut self) {
        // Optionally cleanup on drop - disabled by default to allow reuse
        // let _ = self.cleanup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn create_test_config() -> GitConfig {
        let temp_dir = tempdir().unwrap();
        GitConfig {
            clone_base_path: temp_dir.path().to_path_buf(),
            max_file_size_mb: 1, // 1MB
            allowed_extensions: vec!["rs".to_string(), "md".to_string()],
            ignore_patterns: vec![".git".to_string()],
            max_files_per_repo: 1000,
        }
    }

    #[test]
    fn test_extract_repo_name() {
        assert_eq!(
            GitRepository::extract_repo_name("https://github.com/user/repo.git"),
            "repo"
        );
        assert_eq!(
            GitRepository::extract_repo_name("https://github.com/user/repo"),
            "repo"
        );
    }

    #[tokio::test]
    async fn test_repository_creation() {
        let config = create_test_config();
        let repo = GitRepository::new(
            "https://github.com/rust-lang/mdBook".to_string(),
            config,
        );
        
        assert!(repo.is_ok());
        let repo = repo.unwrap();
        assert!(repo.local_path.to_string_lossy().contains("mdBook"));
    }
}