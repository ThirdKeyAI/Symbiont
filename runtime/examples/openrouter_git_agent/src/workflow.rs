//! Workflow Orchestrator - Coordinates the entire modification workflow

use anyhow::Result;
use tracing::info;
use crate::planner::{ExecutionPlan, PromptPlanner};
use crate::modifier::FileModifier;
use crate::validator::ChangeValidator;

/// Request structure for natural language processing
pub struct NLRequest {
    pub prompt: String,
    pub repo_path: String,
    pub dry_run: bool,
}

/// Main workflow orchestrator that coordinates all components
pub struct WorkflowOrchestrator {
    planner: PromptPlanner,
    modifier: FileModifier,
    validator: ChangeValidator,
    autonomy_level: AutonomyLevel,
}

/// Autonomy levels for the workflow
#[derive(Debug, Clone)]
pub enum AutonomyLevel {
    Ask,
    AutoBackup,
    AutoCommit,
}

impl WorkflowOrchestrator {
    pub fn new(
        planner: PromptPlanner,
        modifier: FileModifier,
        validator: ChangeValidator,
        autonomy_level: AutonomyLevel,
    ) -> Self {
        Self {
            planner,
            modifier,
            validator,
            autonomy_level,
        }
    }

    /// Main entry point for processing natural language requests
    pub async fn execute_natural_language_request(&self, request: &NLRequest) -> Result<WorkflowResult> {
        info!("Processing natural language request: {}", request.prompt);
        
        // 1. Generate execution plan from the prompt
        let execution_plan = self.planner.generate_plan(&request.prompt, &request.repo_path).await?;
        
        // 2. Pretty-print the execution plan
        self.print_execution_plan(&execution_plan);
        
        // 3. If dry run, just show the plan
        if request.dry_run {
            return Ok(WorkflowResult {
                success: true,
                changes_applied: 0,
                backups_created: vec![],
                errors: vec![],
                summary: format!("DRY RUN: Generated execution plan with {} steps", execution_plan.steps.len()),
            });
        }
        
        // 4. Ask for user approval unless we're in auto mode
        let should_proceed = match self.autonomy_level {
            AutonomyLevel::Ask => {
                self.request_user_confirmation(&execution_plan).await?
            }
            AutonomyLevel::AutoBackup | AutonomyLevel::AutoCommit => true,
        };
        
        if !should_proceed {
            return Ok(WorkflowResult {
                success: false,
                changes_applied: 0,
                backups_created: vec![],
                errors: vec!["User declined to proceed".to_string()],
                summary: "Operation cancelled by user".to_string(),
            });
        }
        
        // 5. Execute the plan
        info!("User approved execution, proceeding with plan");
        let modification_results = self.modifier.apply_changes(&execution_plan).await?;
        
        // 6. Generate summary
        let successful_changes = modification_results.iter().filter(|r| r.success).count();
        let failed_changes = modification_results.len() - successful_changes;
        let backups_created: Vec<String> = modification_results
            .iter()
            .filter_map(|r| r.backup_id.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        let errors: Vec<String> = modification_results
            .iter()
            .filter_map(|r| r.error.clone())
            .collect();
        
        let summary = if failed_changes > 0 {
            format!("Completed with {} successful and {} failed modifications",
                   successful_changes, failed_changes)
        } else {
            format!("Successfully completed all {} modifications", successful_changes)
        };
        
        // 7. Print results
        self.print_modification_results(&modification_results);
        
        Ok(WorkflowResult {
            success: failed_changes == 0,
            changes_applied: successful_changes,
            backups_created,
            errors,
            summary,
        })
    }

    pub async fn execute_workflow(
        &self,
        prompt: &str,
        repo_path: &str,
        dry_run: bool,
    ) -> Result<WorkflowResult> {
        let request = NLRequest {
            prompt: prompt.to_string(),
            repo_path: repo_path.to_string(),
            dry_run,
        };
        self.execute_natural_language_request(&request).await
    }

    pub async fn request_user_confirmation(&self, _plan: &ExecutionPlan) -> Result<bool> {
        println!("\n🤔 Please review the execution plan above.");
        println!("Do you want to proceed? (y/N): ");
        
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        
        Ok(input.trim().to_lowercase() == "y" || input.trim().to_lowercase() == "yes")
    }

    pub async fn handle_error(&self, error: &anyhow::Error, context: &str) -> Result<RecoveryAction> {
        println!("❌ Error in {}: {}", context, error);
        println!("How would you like to proceed?");
        println!("1. Retry");
        println!("2. Skip this step");
        println!("3. Abort");
        println!("4. Manual intervention required");
        
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        
        match input.trim() {
            "1" => Ok(RecoveryAction::Retry),
            "2" => Ok(RecoveryAction::Skip),
            "3" => Ok(RecoveryAction::Abort),
            "4" => Ok(RecoveryAction::Manual),
            _ => Ok(RecoveryAction::Abort),
        }
    }

    pub async fn generate_summary(&self, results: &[ModificationResult]) -> Result<String> {
        let successful = results.iter().filter(|r| r.success).count();
        let failed = results.len() - successful;
        
        Ok(format!(
            "Summary: {} successful modifications, {} failed",
            successful, failed
        ))
    }

    fn print_execution_plan(&self, plan: &ExecutionPlan) {
        println!("\n📋 Execution Plan");
        println!("================");
        println!("Risk Level: {:?}", plan.risk_level);
        println!("Estimated Duration: {:?}", plan.estimated_duration);
        println!("Steps: {}", plan.steps.len());
        println!();
        
        for (i, step) in plan.steps.iter().enumerate() {
            println!("{}. {}", i + 1, step.description);
            println!("   Action: {:?}", step.action_type);
            if !step.files.is_empty() {
                println!("   Files: {}", step.files.join(", "));
            }
            if step.requires_confirmation {
                println!("   ⚠️  Requires user confirmation");
            }
            println!();
        }
    }

    fn print_modification_results(&self, results: &[ModificationResult]) {
        println!("\n📊 Modification Results");
        println!("======================");
        
        let successful = results.iter().filter(|r| r.success).count();
        let failed = results.len() - successful;
        
        println!("Total: {} | Successful: {} | Failed: {}", results.len(), successful, failed);
        println!();
        
        for result in results {
            let status_icon = if result.success { "✅" } else { "❌" };
            println!("{} {}", status_icon, result.file_path);
            
            if let Some(ref error) = result.error {
                println!("   Error: {}", error);
            }
            
            if let Some(ref backup_id) = result.backup_id {
                println!("   Backup: {}", backup_id);
            }
        }
        println!();
    }
}

/// Result of the entire workflow execution
pub struct WorkflowResult {
    pub success: bool,
    pub changes_applied: usize,
    pub backups_created: Vec<String>,
    pub errors: Vec<String>,
    pub summary: String,
}

/// Actions that can be taken for error recovery
#[derive(Debug, Clone)]
pub enum RecoveryAction {
    Retry,
    Skip,
    Abort,
    Manual,
}

/// Result of individual modifications (re-export for convenience)
pub use crate::modifier::ModificationResult;