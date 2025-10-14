//! Prompt Planner - Analyzes natural language prompts and creates execution plans

use anyhow::Result;
use tracing::info;
use crate::openrouter::OpenRouterClient;

/// Represents an interpreted instruction from natural language
pub struct Interpretation {
    pub intent: Intent,
    pub scope: Scope,
    pub complexity: Complexity,
    pub files_affected: Vec<String>,
    pub dependencies: Vec<String>,
    pub risks: Vec<String>,
}

/// The primary intent category of the instruction
#[derive(Debug, Clone)]
pub enum Intent {
    Create,
    Modify,
    Delete,
    Refactor,
    Debug,
    Test,
    Document,
    Optimize,
}

/// Scope of changes required
#[derive(Debug, Clone)]
pub enum Scope {
    File,
    Module,
    Package,
    Repository,
}

/// Complexity assessment of the task
#[derive(Debug, Clone)]
pub enum Complexity {
    Simple,
    Medium,
    Complex,
    Unsafe,
}

/// Main prompt planner that converts natural language to structured plans
pub struct PromptPlanner {
    openrouter_client: OpenRouterClient,
}

impl PromptPlanner {
    pub fn new(openrouter_client: OpenRouterClient) -> Self {
        Self { openrouter_client }
    }

    /// Generate an execution plan from a natural language prompt and repository context
    pub async fn generate_plan(&self, prompt: &str, repo_path: &str) -> Result<ExecutionPlan> {
        info!("Generating execution plan for prompt: {}", prompt);
        
        // 1. Gather repository context
        let repo_context = self.gather_repository_context(repo_path).await?;
        
        // 2. Use OpenRouter to generate the execution plan
        let execution_plan = self.openrouter_client.generate_execution_plan(prompt, &repo_context).await?;
        
        info!("Generated execution plan with {} steps", execution_plan.steps.len());
        Ok(execution_plan)
    }

    pub async fn interpret_prompt(&self, prompt: &str, _repo_context: &str) -> Result<Interpretation> {
        // Basic interpretation logic - in a real system this would be more sophisticated
        let intent = self.classify_intent(prompt);
        let scope = self.determine_scope(prompt);
        let complexity = self.assess_complexity(prompt);
        
        Ok(Interpretation {
            intent,
            scope,
            complexity,
            files_affected: vec![],
            dependencies: vec![],
            risks: vec![],
        })
    }

    pub async fn clarify_ambiguity(&self, interpretation: &Interpretation) -> Result<Vec<String>> {
        let mut questions = Vec::new();
        
        match interpretation.complexity {
            Complexity::Complex | Complexity::Unsafe => {
                questions.push("This operation appears complex. Would you like to proceed step by step?".to_string());
            }
            _ => {}
        }
        
        if interpretation.files_affected.is_empty() {
            questions.push("Which specific files should be modified?".to_string());
        }
        
        Ok(questions)
    }

    async fn gather_repository_context(&self, repo_path: &str) -> Result<String> {
        info!("Gathering repository context from: {}", repo_path);
        
        // Create a simple context from repository structure
        let context = if std::path::Path::new(repo_path).exists() {
            // For local repositories, create a basic file listing
            self.create_local_repo_context(repo_path).await?
        } else {
            // For URLs, we'd need to clone or analyze remotely
            format!("Repository URL: {}\nNote: Remote repository context gathering not yet implemented", repo_path)
        };
        
        Ok(context)
    }

    async fn create_local_repo_context(&self, repo_path: &str) -> Result<String> {
        let mut context = String::new();
        context.push_str(&format!("Repository Path: {}\n\n", repo_path));
        
        // Get basic file structure
        if let Ok(entries) = std::fs::read_dir(repo_path) {
            context.push_str("Files and Directories:\n");
            for entry in entries.flatten() {
                let file_name = entry.file_name();
                let name = file_name.to_string_lossy();
                if !name.starts_with('.') {  // Skip hidden files
                    let file_type = if entry.path().is_dir() { "DIR" } else { "FILE" };
                    context.push_str(&format!("  {} {}\n", file_type, name));
                }
            }
        }
        
        // Look for important files
        context.push_str("\nKey Files Found:\n");
        for important_file in ["README.md", "Cargo.toml", "package.json", "main.rs", "lib.rs"] {
            let file_path = std::path::Path::new(repo_path).join(important_file);
            if file_path.exists() {
                context.push_str(&format!("  ✓ {}\n", important_file));
                
                // For small files, include a snippet
                if let Ok(content) = std::fs::read_to_string(&file_path) {
                    if content.len() < 500 {
                        context.push_str(&format!("    Content snippet: {}\n",
                            content.lines().take(5).collect::<Vec<_>>().join(" | ")));
                    }
                }
            }
        }
        
        Ok(context)
    }

    fn classify_intent(&self, prompt: &str) -> Intent {
        let prompt_lower = prompt.to_lowercase();
        
        if prompt_lower.contains("create") || prompt_lower.contains("add") || prompt_lower.contains("new") {
            Intent::Create
        } else if prompt_lower.contains("delete") || prompt_lower.contains("remove") {
            Intent::Delete
        } else if prompt_lower.contains("refactor") || prompt_lower.contains("reorganize") {
            Intent::Refactor
        } else if prompt_lower.contains("test") {
            Intent::Test
        } else if prompt_lower.contains("document") || prompt_lower.contains("doc") {
            Intent::Document
        } else if prompt_lower.contains("optimize") || prompt_lower.contains("performance") {
            Intent::Optimize
        } else if prompt_lower.contains("fix") || prompt_lower.contains("debug") {
            Intent::Debug
        } else {
            Intent::Modify
        }
    }

    fn determine_scope(&self, prompt: &str) -> Scope {
        let prompt_lower = prompt.to_lowercase();
        
        if prompt_lower.contains("entire") || prompt_lower.contains("whole") || prompt_lower.contains("all") {
            Scope::Repository
        } else if prompt_lower.contains("package") || prompt_lower.contains("module") {
            Scope::Package
        } else if prompt_lower.contains("function") || prompt_lower.contains("specific") {
            Scope::File
        } else {
            Scope::Module
        }
    }

    fn assess_complexity(&self, prompt: &str) -> Complexity {
        let prompt_lower = prompt.to_lowercase();
        
        if prompt_lower.contains("unsafe") || prompt_lower.contains("dangerous") {
            Complexity::Unsafe
        } else if prompt_lower.contains("complex") || prompt_lower.contains("major") ||
                  prompt_lower.contains("breaking") || prompt_lower.len() > 200 {
            Complexity::Complex
        } else if prompt_lower.contains("simple") || prompt_lower.contains("small") {
            Complexity::Simple
        } else {
            Complexity::Medium
        }
    }
}

/// Represents a complete execution plan
pub struct ExecutionPlan {
    pub steps: Vec<ExecutionStep>,
    pub estimated_duration: std::time::Duration,
    pub risk_level: RiskLevel,
}

/// Individual step in an execution plan
pub struct ExecutionStep {
    pub description: String,
    pub action_type: ActionType,
    pub files: Vec<String>,
    pub requires_confirmation: bool,
}

/// Type of action to be performed
#[derive(Debug, Clone)]
pub enum ActionType {
    CreateFile,
    ModifyFile,
    DeleteFile,
    RunCommand,
    Validate,
}

/// Risk assessment levels
#[derive(Debug, Clone)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}