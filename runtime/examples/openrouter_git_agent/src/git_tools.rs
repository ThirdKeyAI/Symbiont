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
            .next_back()
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

    /// Get the repository path for command execution
    pub fn get_repo_path(&self) -> &Path {
        &self.local_path
    }
}

/// Represents a file change operation
#[derive(Clone)]
pub struct FileChange {
    pub file_path: String,
    pub change_type: ChangeType,
    pub content: Option<String>,
    pub line_range: Option<(usize, usize)>,
}

/// Type of change to apply to a file
#[derive(Debug, Clone)]
pub enum ChangeType {
    Create,
    Modify,
    Delete,
    Append,
    Insert,
    Replace,
}

impl GitRepository {
    /// Apply file changes to the repository
    pub async fn apply_changes(&self, changes: &[FileChange]) -> Result<Vec<String>> {
        let mut modified_files = Vec::new();

        for change in changes {
            let file_path = self.local_path.join(&change.file_path);
            
            match change.change_type {
                ChangeType::Create => {
                    if let Some(content) = &change.content {
                        self.write_file(&file_path, content).await?;
                        modified_files.push(change.file_path.clone());
                    }
                }
                ChangeType::Modify | ChangeType::Replace => {
                    if let Some(content) = &change.content {
                        self.write_file(&file_path, content).await?;
                        modified_files.push(change.file_path.clone());
                    }
                }
                ChangeType::Delete => {
                    if file_path.exists() {
                        std::fs::remove_file(&file_path)?;
                        modified_files.push(change.file_path.clone());
                    }
                }
                ChangeType::Append => {
                    if let Some(content) = &change.content {
                        let mut existing_content = if file_path.exists() {
                            std::fs::read_to_string(&file_path)?
                        } else {
                            String::new()
                        };
                        existing_content.push_str(content);
                        self.write_file(&file_path, &existing_content).await?;
                        modified_files.push(change.file_path.clone());
                    }
                }
                ChangeType::Insert => {
                    if let Some(content) = &change.content {
                        if let Some((start_line, _)) = change.line_range {
                            let existing_content = if file_path.exists() {
                                std::fs::read_to_string(&file_path)?
                            } else {
                                String::new()
                            };
                            let mut lines: Vec<&str> = existing_content.lines().collect();
                            let insert_pos = start_line.min(lines.len());
                            lines.insert(insert_pos, content);
                            let new_content = lines.join("\n");
                            self.write_file(&file_path, &new_content).await?;
                            modified_files.push(change.file_path.clone());
                        }
                    }
                }
            }
        }

        Ok(modified_files)
    }

    /// Create a backup branch for safety
    pub async fn create_backup_branch(&self, branch_name: &str) -> Result<String> {
        self.create_feature_branch(branch_name).await
    }

    /// Create a new feature branch
    pub async fn create_feature_branch(&self, branch_name: &str) -> Result<String> {
        // Ensure repository is cloned/accessible first
        let repo = self.clone_or_update().await?;
        
        // Get current HEAD
        let head = repo.head()?;
        let target_commit = head.target().ok_or_else(|| anyhow::anyhow!("Unable to get HEAD commit"))?;
        let commit = repo.find_commit(target_commit)?;
        
        // Create new branch
        let branch = repo.branch(branch_name, &commit, false)?;
        let branch_ref = branch.get().name().ok_or_else(|| anyhow::anyhow!("Unable to get branch name"))?;
        
        // Checkout the new branch
        repo.set_head(branch_ref)?;
        repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))?;
        
        info!("Created and checked out branch: {}", branch_name);
        Ok(branch_name.to_string())
    }

    /// Write content to a file, creating directories as needed
    pub async fn write_file(&self, file_path: &Path, content: &str) -> Result<()> {
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(file_path, content)?;
        Ok(())
    }

    /// Commit changes to the repository
    pub async fn commit_changes(&self, message: &str, files: &[String]) -> Result<String> {
        // Ensure repository is cloned/accessible first
        let repo = self.clone_or_update().await?;
        let mut index = repo.index()?;
        
        // Add files to index
        for file in files {
            index.add_path(Path::new(file))?;
        }
        index.write()?;
        
        // Get tree from index
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        
        // Get signature
        let signature = git2::Signature::now("OpenRouter Git Agent", "agent@example.com")?;
        
        // Get parent commit
        let parent_commit = if let Ok(head) = repo.head() {
            if let Some(target) = head.target() {
                Some(repo.find_commit(target)?)
            } else {
                None
            }
        } else {
            None
        };
        
        // Create commit
        let commit_id = if let Some(parent) = parent_commit {
            repo.commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &[&parent],
            )?
        } else {
            // Initial commit
            repo.commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &[],
            )?
        };
        
        info!("Created commit: {}", commit_id);
        Ok(commit_id.to_string())
    }

    /// Reset to a specific commit
    pub async fn reset_to_commit(&self, commit_id: &str) -> Result<()> {
        let repo = Repository::open(&self.local_path)?;
        let oid = git2::Oid::from_str(commit_id)?;
        let commit = repo.find_commit(oid)?;
        
        repo.reset(
            commit.as_object(),
            git2::ResetType::Hard,
            Some(git2::build::CheckoutBuilder::default().force()),
        )?;
        
        info!("Reset to commit: {}", commit_id);
        Ok(())
    }

    /// Validate repository state
    pub async fn validate_repository_state(&self) -> Result<bool> {
        let repo = Repository::open(&self.local_path)?;
        
        // Check if repository is in a clean state
        let statuses = repo.statuses(None)?;
        let is_clean = statuses.is_empty();
        
        // Check if we're on a valid branch
        let head_valid = repo.head().is_ok();
        
        Ok(is_clean && head_valid)
    }

    /// Get file history
    pub async fn get_file_history(&self, file_path: &str) -> Result<Vec<String>> {
        let repo = Repository::open(&self.local_path)?;
        let mut revwalk = repo.revwalk()?;
        revwalk.push_head()?;
        
        let mut history = Vec::new();
        for oid in revwalk {
            let oid = oid?;
            let commit = repo.find_commit(oid)?;
            
            // Check if this commit affects the file
            if let Ok(tree) = commit.tree() {
                if tree.get_path(Path::new(file_path)).is_ok() {
                    history.push(format!(
                        "{} - {} - {}",
                        oid,
                        commit.author().name().unwrap_or("Unknown"),
                        commit.message().unwrap_or("No message").lines().next().unwrap_or("")
                    ));
                }
            }
        }
        
        Ok(history)
    }

    /// Generate diff for changes
    pub async fn diff_changes(&self, changes: &[FileChange]) -> Result<String> {
        let mut diff_output = String::new();
        
        for change in changes {
            let file_path = self.local_path.join(&change.file_path);
            
            match change.change_type {
                ChangeType::Create => {
                    diff_output.push_str(&format!("--- /dev/null\n+++ {}\n", change.file_path));
                    if let Some(content) = &change.content {
                        for line in content.lines() {
                            diff_output.push_str(&format!("+{}\n", line));
                        }
                    }
                }
                ChangeType::Delete => {
                    diff_output.push_str(&format!("--- {}\n+++ /dev/null\n", change.file_path));
                    if file_path.exists() {
                        let content = std::fs::read_to_string(&file_path)?;
                        for line in content.lines() {
                            diff_output.push_str(&format!("-{}\n", line));
                        }
                    }
                }
                ChangeType::Modify | ChangeType::Replace => {
                    diff_output.push_str(&format!("--- {}\n+++ {}\n", change.file_path, change.file_path));
                    let old_content = if file_path.exists() {
                        std::fs::read_to_string(&file_path)?
                    } else {
                        String::new()
                    };
                    
                    if let Some(new_content) = &change.content {
                        // Simple diff - mark old lines as removed, new as added
                        for line in old_content.lines() {
                            diff_output.push_str(&format!("-{}\n", line));
                        }
                        for line in new_content.lines() {
                            diff_output.push_str(&format!("+{}\n", line));
                        }
                    }
                }
                _ => {
                    diff_output.push_str(&format!("# {} operation on {}\n",
                        match change.change_type {
                            ChangeType::Append => "Append",
                            ChangeType::Insert => "Insert",
                            _ => "Unknown"
                        },
                        change.file_path
                    ));
                }
            }
            diff_output.push('\n');
        }
        
        Ok(diff_output)
    }

    /// Restore from backup branch
    pub async fn restore_from_backup(&self, backup_branch: &str) -> Result<()> {
        let repo = Repository::open(&self.local_path)?;
        
        // Find the backup branch
        let branch = repo.find_branch(backup_branch, git2::BranchType::Local)?;
        let branch_ref = branch.get();
        
        // Get the commit that the branch points to
        let target = branch_ref.target().ok_or_else(|| anyhow::anyhow!("Branch has no target"))?;
        let commit = repo.find_commit(target)?;
        
        // Reset to the backup branch
        repo.reset(
            commit.as_object(),
            git2::ResetType::Hard,
            Some(git2::build::CheckoutBuilder::default().force()),
        )?;
        
        info!("Restored from backup branch: {}", backup_branch);
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