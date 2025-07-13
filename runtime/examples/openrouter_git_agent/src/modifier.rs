//! File Modifier - Handles actual file modifications based on plans

use anyhow::Result;
use std::path::Path;
use tracing::{info, warn, error};
use crate::planner::{ExecutionPlan, ExecutionStep, ActionType};
use crate::git_tools::{GitRepository, FileChange, ChangeType};

/// Handles file modifications with safety checks and backups
pub struct FileModifier {
    backup_enabled: bool,
    validation_enabled: bool,
    git_repo: GitRepository,
}

impl FileModifier {
    pub fn new(backup_enabled: bool, validation_enabled: bool, git_repo: GitRepository) -> Self {
        Self {
            backup_enabled,
            validation_enabled,
            git_repo,
        }
    }

    /// Apply changes from an ExecutionPlan
    pub async fn apply_changes(&self, plan: &ExecutionPlan) -> Result<Vec<ModificationResult>> {
        info!("Starting to apply execution plan with {} steps", plan.steps.len());
        let mut results = Vec::new();
        let mut backup_branch = None;

        // Create backup branch if enabled
        if self.backup_enabled {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs();
            let branch_name = format!("backup-{}", timestamp);
            
            match self.git_repo.create_feature_branch(&branch_name).await {
                Ok(branch) => {
                    backup_branch = Some(branch);
                    info!("Created backup branch: {}", branch_name);
                }
                Err(e) => {
                    warn!("Failed to create backup branch: {}", e);
                }
            }
        }

        // Apply each step in the plan
        for (i, step) in plan.steps.iter().enumerate() {
            info!("Executing step {}/{}: {}", i + 1, plan.steps.len(), step.description);
            
            match self.apply_step(step).await {
                Ok(mut step_results) => {
                    for result in &mut step_results {
                        if let Some(ref backup) = backup_branch {
                            result.backup_id = Some(backup.clone());
                        }
                    }
                    results.extend(step_results);
                }
                Err(e) => {
                    error!("Failed to execute step {}: {}", i + 1, e);
                    results.push(ModificationResult {
                        file_path: step.files.first().cloned().unwrap_or_default(),
                        success: false,
                        backup_id: backup_branch.clone(),
                        error: Some(e.to_string()),
                    });
                }
            }
        }

        // Commit changes if any were successful
        let successful_files: Vec<String> = results
            .iter()
            .filter(|r| r.success)
            .map(|r| r.file_path.clone())
            .collect();

        if !successful_files.is_empty() {
            let commit_message = format!("Applied execution plan: {} files modified", successful_files.len());
            match self.git_repo.commit_changes(&commit_message, &successful_files).await {
                Ok(commit_id) => {
                    info!("Changes committed: {}", commit_id);
                }
                Err(e) => {
                    warn!("Failed to commit changes: {}", e);
                }
            }
        }

        info!("Execution plan completed: {}/{} steps successful",
              results.iter().filter(|r| r.success).count(),
              results.len());

        Ok(results)
    }

    /// Apply a single execution step
    async fn apply_step(&self, step: &ExecutionStep) -> Result<Vec<ModificationResult>> {
        let mut results = Vec::new();

        match step.action_type {
            ActionType::CreateFile => {
                for file_path in &step.files {
                    let change = FileChange {
                        file_path: file_path.clone(),
                        change_type: ChangeType::Create,
                        content: Some(format!("// TODO: Implement {}\n", step.description)),
                        line_range: None,
                    };
                    
                    match self.apply_file_change(&change).await {
                        Ok(result) => results.push(result),
                        Err(e) => results.push(ModificationResult {
                            file_path: file_path.clone(),
                            success: false,
                            backup_id: None,
                            error: Some(e.to_string()),
                        }),
                    }
                }
            }
            ActionType::ModifyFile => {
                for file_path in &step.files {
                    let change = FileChange {
                        file_path: file_path.clone(),
                        change_type: ChangeType::Modify,
                        content: Some(format!("// Modified: {}\n", step.description)),
                        line_range: None,
                    };
                    
                    match self.apply_file_change(&change).await {
                        Ok(result) => results.push(result),
                        Err(e) => results.push(ModificationResult {
                            file_path: file_path.clone(),
                            success: false,
                            backup_id: None,
                            error: Some(e.to_string()),
                        }),
                    }
                }
            }
            ActionType::DeleteFile => {
                for file_path in &step.files {
                    let change = FileChange {
                        file_path: file_path.clone(),
                        change_type: ChangeType::Delete,
                        content: None,
                        line_range: None,
                    };
                    
                    match self.apply_file_change(&change).await {
                        Ok(result) => results.push(result),
                        Err(e) => results.push(ModificationResult {
                            file_path: file_path.clone(),
                            success: false,
                            backup_id: None,
                            error: Some(e.to_string()),
                        }),
                    }
                }
            }
            ActionType::RunCommand => {
                // For now, just log that we would run a command
                info!("Would run command for step: {}", step.description);
                results.push(ModificationResult {
                    file_path: "command".to_string(),
                    success: true,
                    backup_id: None,
                    error: None,
                });
            }
            ActionType::Validate => {
                // For now, just assume validation passes
                info!("Validation step: {}", step.description);
                results.push(ModificationResult {
                    file_path: "validation".to_string(),
                    success: true,
                    backup_id: None,
                    error: None,
                });
            }
        }

        Ok(results)
    }

    /// Apply a single file change
    async fn apply_file_change(&self, change: &FileChange) -> Result<ModificationResult> {
        // Validate syntax if enabled
        if self.validation_enabled {
            let file_path = Path::new(&change.file_path);
            if file_path.exists() {
                match self.validate_syntax(file_path).await {
                    Ok(validation) => {
                        if !validation.is_valid {
                            return Ok(ModificationResult {
                                file_path: change.file_path.clone(),
                                success: false,
                                backup_id: None,
                                error: Some(format!("Validation failed: {}", validation.errors.join(", "))),
                            });
                        }
                    }
                    Err(e) => {
                        warn!("Validation error for {}: {}", change.file_path, e);
                    }
                }
            }
        }

        // Apply the change through git_tools
        match self.git_repo.apply_changes(&[change.clone()]).await {
            Ok(modified_files) => {
                if modified_files.contains(&change.file_path) {
                    Ok(ModificationResult {
                        file_path: change.file_path.clone(),
                        success: true,
                        backup_id: None,
                        error: None,
                    })
                } else {
                    Ok(ModificationResult {
                        file_path: change.file_path.clone(),
                        success: false,
                        backup_id: None,
                        error: Some("File was not modified".to_string()),
                    })
                }
            }
            Err(e) => Ok(ModificationResult {
                file_path: change.file_path.clone(),
                success: false,
                backup_id: None,
                error: Some(e.to_string()),
            }),
        }
    }

    pub async fn create_backup(&self, _file_path: &Path) -> Result<String> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        let backup_id = format!("backup-{}", timestamp);
        self.git_repo.create_backup_branch(&backup_id).await
    }

    pub async fn restore_backup(&self, backup_id: &str) -> Result<()> {
        self.git_repo.restore_from_backup(backup_id).await
    }

    pub async fn validate_syntax(&self, file_path: &Path) -> Result<ValidationResult> {
        // Basic syntax validation - in a real system this would be more sophisticated
        if !file_path.exists() {
            return Ok(ValidationResult {
                is_valid: false,
                errors: vec!["File does not exist".to_string()],
                warnings: vec![],
            });
        }

        // Try to read the file
        match std::fs::read_to_string(file_path) {
            Ok(_content) => {
                // For now, just assume all files are valid
                // In a real system, you'd run language-specific linters
                Ok(ValidationResult {
                    is_valid: true,
                    errors: vec![],
                    warnings: vec![],
                })
            }
            Err(e) => Ok(ValidationResult {
                is_valid: false,
                errors: vec![format!("Failed to read file: {}", e)],
                warnings: vec![],
            }),
        }
    }

    pub async fn preview_changes(&self, plan: &ExecutionPlan) -> Result<String> {
        let mut preview = String::new();
        preview.push_str("=== EXECUTION PLAN PREVIEW ===\n\n");
        
        for (i, step) in plan.steps.iter().enumerate() {
            preview.push_str(&format!("Step {}: {}\n", i + 1, step.description));
            preview.push_str(&format!("  Action: {:?}\n", step.action_type));
            if !step.files.is_empty() {
                preview.push_str(&format!("  Files: {}\n", step.files.join(", ")));
            }
            if step.requires_confirmation {
                preview.push_str("  ⚠️  Requires confirmation\n");
            }
            preview.push('\n');
        }
        
        Ok(preview)
    }
}

/// Result of a file modification operation
pub struct ModificationResult {
    pub file_path: String,
    pub success: bool,
    pub backup_id: Option<String>,
    pub error: Option<String>,
}

/// Result of syntax validation
pub struct ValidationResult {
    pub is_valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}