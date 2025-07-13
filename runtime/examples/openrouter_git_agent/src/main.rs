use anyhow::Result;
use clap::{Parser, ValueEnum};
use std::path::PathBuf;
use tracing::{info, Level};

mod config;
mod openrouter;
mod git_tools;
pub mod planner;
pub mod modifier;
pub mod workflow;
pub mod validator;

use config::AgentConfig;
use openrouter::OpenRouterClient;

#[derive(Parser)]
#[command(name = "openrouter_git_agent")]
#[command(about = "AI-powered Git repository modification agent")]
struct Cli {
    /// Repository URL to work with
    #[arg(short, long)]
    repo: String,
    
    /// Natural language instruction for what to do with the repository
    prompt: String,
    
    /// Autonomy level for applying changes
    #[arg(short, long, default_value = "auto-backup")]
    autonomy: AutonomyLevel,
    
    /// Dry run mode - generate plan only, no changes
    #[arg(long)]
    dry_run: bool,
    
    /// Configuration file path
    #[arg(short, long, default_value = "config.toml")]
    config: PathBuf,
    
    /// Skip clarification questions (use AI best judgment)
    #[arg(long)]
    skip_clarification: bool,
}

#[derive(Debug, Clone, ValueEnum)]
enum AutonomyLevel {
    Ask,         // Ask before each change
    AutoBackup,  // Auto-backup, ask for big features
    AutoCommit,  // Auto-commit everything
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();
    
    info!("Starting OpenRouter Git Agent");
    
    // Load configuration
    let config = if cli.config.exists() {
        AgentConfig::load_from_file(&cli.config)?
    } else {
        info!("Config file not found, using defaults. Create {} to customize settings.", cli.config.display());
        AgentConfig::default()
    };
    
    // Initialize components
    let openrouter_client = OpenRouterClient::new(config.openrouter.clone())?;
    
    // Test connection
    if let Err(e) = openrouter_client.test_connection().await {
        eprintln!("❌ Failed to connect to OpenRouter: {}", e);
        eprintln!("Please check your API key and internet connection.");
        std::process::exit(1);
    }
    
    // Initialize planner with OpenRouter client
    let planner = planner::PromptPlanner::new(openrouter_client);
    
    // Create GitRepository for the target repo
    let git_repo = git_tools::GitRepository::new(cli.repo.clone(), config.git.clone())?;
    
    // Initialize modifier and validator
    let modifier = modifier::FileModifier::new(true, true, git_repo);
    let validator = validator::ChangeValidator::new(true, true, true);
    
    // Convert CLI autonomy level to workflow autonomy level
    let autonomy_level = match cli.autonomy {
        AutonomyLevel::Ask => workflow::AutonomyLevel::Ask,
        AutonomyLevel::AutoBackup => workflow::AutonomyLevel::AutoBackup,
        AutonomyLevel::AutoCommit => workflow::AutonomyLevel::AutoCommit,
    };
    
    // Create workflow orchestrator
    let workflow_orchestrator = workflow::WorkflowOrchestrator::new(
        planner,
        modifier,
        validator,
        autonomy_level,
    );
    
    // Create natural language request
    let nl_request = workflow::NLRequest {
        prompt: cli.prompt,
        repo_path: cli.repo,
        dry_run: cli.dry_run,
    };
    
    // Execute the workflow
    match workflow_orchestrator.execute_natural_language_request(&nl_request).await {
        Ok(result) => {
            if result.success {
                println!("\n✅ {}", result.summary);
            } else {
                println!("\n❌ Workflow failed: {}", result.summary);
                for error in &result.errors {
                    println!("   Error: {}", error);
                }
            }
        }
        Err(e) => {
            eprintln!("❌ Workflow execution failed: {}", e);
            std::process::exit(1);
        }
    }
    
    Ok(())
}
