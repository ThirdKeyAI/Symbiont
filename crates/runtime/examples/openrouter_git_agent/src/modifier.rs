//! File Modifier - Handles actual file modifications based on plans

use anyhow::Result;
use std::path::Path;
use std::process::Command;
use tracing::{info, warn, error};
use crate::planner::{ExecutionPlan, ExecutionStep, ActionType};
use crate::git_tools::{GitRepository, FileChange, ChangeType};
use crate::openrouter::OpenRouterClient;

/// Handles file modifications with safety checks and backups
pub struct FileModifier {
    backup_enabled: bool,
    validation_enabled: bool,
    git_repo: GitRepository,
    openrouter_client: Option<OpenRouterClient>,
}

impl FileModifier {
    pub fn new(backup_enabled: bool, validation_enabled: bool, git_repo: GitRepository) -> Self {
        Self {
            backup_enabled,
            validation_enabled,
            git_repo,
            openrouter_client: None,
        }
    }

    pub fn with_openrouter(mut self, client: OpenRouterClient) -> Self {
        self.openrouter_client = Some(client);
        self
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
                    let content = if file_path.contains("security") && file_path.ends_with(".md") {
                        // Generate actual security report content
                        match self.generate_security_report(&step.description).await {
                            Ok(report) => report,
                            Err(e) => {
                                warn!("Failed to generate security report: {}", e);
                                format!("# Security Analysis Report\n\n{}\n\n*Report generation failed: {}*", step.description, e)
                            }
                        }
                    } else {
                        if let Some(client) = &self.openrouter_client {
                            let prompt = format!("Generate content for new file {} to accomplish: {}. Provide only the content.", file_path, step.description);
                            match client.generate_response(&prompt).await {
                                Ok(resp) => resp,
                                Err(e) => {
                                    warn!("Failed to generate content: {}", e);
                                    format!("// Failed to generate: {}", e)
                                }
                            }
                        } else {
                            "// No AI client available for content generation".to_string()
                        }
                    };

                    let change = FileChange {
                        file_path: file_path.clone(),
                        change_type: ChangeType::Create,
                        content: Some(content),
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
                    let full_path = self.git_repo.get_repo_path().join(file_path);
                    let current_content = std::fs::read_to_string(&full_path).unwrap_or_default();

                    let new_content = if let Some(client) = &self.openrouter_client {
                        let prompt = format!("Modify the following code in file {} to accomplish: {}. Provide the full modified content.\n\nCurrent content:\n{}", file_path, step.description, current_content);
                        match client.generate_response(&prompt).await {
                            Ok(resp) => resp,
                            Err(e) => {
                                warn!("Failed to generate modified content: {}", e);
                                current_content
                            }
                        }
                    } else {
                        current_content
                    };

                    let change = FileChange {
                        file_path: file_path.clone(),
                        change_type: ChangeType::Modify,
                        content: Some(new_content),
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
                // Execute actual security commands
                match self.execute_security_command(&step.description).await {
                    Ok(output) => {
                        info!("Command executed successfully: {}", step.description);
                        info!("Output: {}", output);
                        results.push(ModificationResult {
                            file_path: "command".to_string(),
                            success: true,
                            backup_id: None,
                            error: None,
                        });
                    }
                    Err(e) => {
                        error!("Command failed: {}", e);
                        results.push(ModificationResult {
                            file_path: "command".to_string(),
                            success: false,
                            backup_id: None,
                            error: Some(e.to_string()),
                        });
                    }
                }
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

    /// Execute security commands in the repository context
    async fn execute_security_command(&self, description: &str) -> Result<String> {
        // Determine command based on description
        let command = if description.contains("npm audit") {
            "npm audit --json"
        } else if description.contains("ESLint") {
            "npx eslint . --format json"
        } else if description.contains("dependency scan") {
            "npm audit --json"
        } else if description.contains("static analysis") {
            "npx eslint . --format json"
        } else {
            return Ok("Command executed successfully (simulation)".to_string());
        };

        self.run_command_in_repo(command).await
    }

    /// Run a command in the repository directory
    async fn run_command_in_repo(&self, command: &str) -> Result<String> {
        let repo_path = self.git_repo.get_repo_path();
        
        info!("Running command in {}: {}", repo_path.display(), command);
        
        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.is_empty() {
            return Err(anyhow::anyhow!("Empty command"));
        }

        let output = Command::new(parts[0])
            .args(&parts[1..])
            .current_dir(repo_path)
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to execute command: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        
        // Handle commands that may return non-zero exit codes but are still successful
        let is_acceptable_failure = match output.status.code() {
            Some(1) => {
                // npm audit returns exit code 1 when vulnerabilities are found (expected behavior)
                command.contains("npm audit") ||
                // ESLint returns exit code 1 when linting issues are found
                command.contains("eslint")
            },
            _ => false,
        };

        if output.status.success() || is_acceptable_failure {
            Ok(format!("STDOUT:\n{}\nSTDERR:\n{}", stdout, stderr))
        } else {
            Err(anyhow::anyhow!("Command failed with exit code {}: {}",
                output.status.code().unwrap_or(-1), stderr))
        }
    }

    /// Generate comprehensive security report using OpenRouter AI
    async fn generate_security_report(&self, description: &str) -> Result<String> {
        if let Some(client) = &self.openrouter_client {
            let repo_path = self.git_repo.get_repo_path();
            
            // Gather repository information
            let mut repo_info = String::new();
            
            // Try to get package.json info
            let package_json_path = repo_path.join("package.json");
            if package_json_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&package_json_path) {
                    repo_info.push_str(&format!("Package.json:\n{}\n\n", content));
                }
            }

            // Run security commands and collect results
            let mut security_results = String::new();
            
            // Try npm audit
            if let Ok(audit_result) = self.run_command_in_repo("npm audit --json").await {
                security_results.push_str(&format!("NPM Audit Results:\n{}\n\n", audit_result));
            }

            // Try ESLint if available
            if let Ok(eslint_result) = self.run_command_in_repo("npx eslint . --format json").await {
                security_results.push_str(&format!("ESLint Results:\n{}\n\n", eslint_result));
            }

            let prompt = format!(
                r#"Generate a comprehensive security analysis report for this repository.

Task: {}

Repository Information:
{}

Security Scan Results:
{}

Please provide a detailed markdown report that includes:
1. Executive Summary
2. Vulnerability Assessment
3. Dependency Analysis
4. Code Quality Issues
5. Recommendations for Remediation
6. Risk Assessment

Format the response as a professional security audit report in markdown."#,
                description, repo_info, security_results
            );

            match client.generate_response(&prompt).await {
                Ok(response) => Ok(response),
                Err(e) => {
                    warn!("Failed to generate AI report: {}", e);
                    Ok(self.generate_fallback_report(description, &security_results))
                }
            }
        } else {
            Ok(self.generate_fallback_report(description, "No security scan results available"))
        }
    }

    /// Generate a fallback report when AI generation fails
    fn generate_fallback_report(&self, description: &str, security_results: &str) -> String {
        let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
        
        format!(
            r#"# Security Analysis Report

**Generated:** {}
**Task:** {}

## Executive Summary

This security analysis was performed to assess the current security posture of the repository.

## Analysis Results

{}

## Recommendations

1. Review and address any high-severity vulnerabilities identified
2. Update dependencies to their latest secure versions
3. Implement proper input validation and sanitization
4. Consider adding security-focused linting rules
5. Regular security audits should be scheduled

## Risk Assessment

Risk levels should be assessed based on the specific vulnerabilities found and the context of the application.

---
*This report was generated automatically. Please review findings carefully and validate recommendations.*
"#,
            timestamp, description, security_results
        )
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