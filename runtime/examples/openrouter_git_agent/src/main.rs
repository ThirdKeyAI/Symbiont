use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::{info, Level};

mod config;
mod openrouter;
mod git_tools;

use config::AgentConfig;
use openrouter::OpenRouterClient;
use git_tools::GitRepository;

#[derive(Parser)]
#[command(name = "openrouter_git_agent")]
#[command(about = "An AI agent for analyzing Git repositories using OpenRouter")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    
    #[arg(short, long, default_value = "config.toml")]
    config: PathBuf,
}

#[derive(Subcommand)]
enum Commands {
    /// Analyze a Git repository
    Analyze {
        /// Repository URL to analyze
        #[arg(short, long)]
        repo: String,
        
        /// Query to ask about the repository
        #[arg(short, long, default_value = "What is this repository about?")]
        query: String,
    },
    
    /// Show configuration status
    Status,
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
    
    match cli.command {
        Commands::Analyze { repo, query } => {
            analyze_repository(config, repo, query).await?;
        }
        Commands::Status => {
            show_status(config).await?;
        }
    }
    
    Ok(())
}

async fn analyze_repository(config: AgentConfig, repo_url: String, query: String) -> Result<()> {
    info!("Analyzing repository: {}", repo_url);
    
    // Initialize OpenRouter client
    let openrouter = OpenRouterClient::new(config.openrouter.clone())?;
    
    // Clone and analyze the repository
    let git_repo = GitRepository::new(repo_url.clone(), config.git.clone())?;
    let repo_info = git_repo.analyze_repository().await?;
    
    info!("Found {} files in repository", repo_info.files.len());
    
    // Prepare context from repository files
    let mut context = String::new();
    context.push_str(&format!("Repository: {}\n", repo_url));
    context.push_str(&format!("Description: {}\n", repo_info.description.unwrap_or_else(|| "No description available".to_string())));
    context.push_str(&format!("Total files: {}\n", repo_info.files.len()));
    context.push_str(&format!("Languages: {}\n", repo_info.languages.join(", ")));
    context.push_str("\nKey files:\n");
    
    // Include content from key files (limit to avoid token limits)
    for file in repo_info.files.iter().take(10) {
        if file.content.len() > 2000 {
            context.push_str(&format!("File: {} ({}...)\n", file.path, &file.content[..2000]));
        } else {
            context.push_str(&format!("File: {} ({})\n", file.path, file.content));
        }
    }
    
    // Ask OpenRouter to analyze the repository
    let analysis = openrouter.analyze_code(&context, &repo_url, &query).await?;
    
    println!("\n=== Repository Analysis ===");
    println!("Repository: {}", repo_url);
    println!("Query: {}", query);
    println!("\nAnalysis:");
    println!("{}", analysis);
    println!("\n=== End Analysis ===");
    
    Ok(())
}

async fn show_status(config: AgentConfig) -> Result<()> {
    println!("=== OpenRouter Git Agent Status ===");
    println!("OpenRouter API Key: {}", if config.openrouter.api_key.is_empty() { "Not configured" } else { "Configured" });
    println!("Model: {}", config.openrouter.model);
    println!("Max tokens: {:?}", config.openrouter.max_tokens);
    println!("Clone directory: {}", config.git.clone_base_path.display());
    println!("Max file size: {} MB", config.git.max_file_size_mb);
    println!("Included extensions: {}", config.git.allowed_extensions.join(", "));
    
    // Test OpenRouter connection
    if !config.openrouter.api_key.is_empty() {
        let client = OpenRouterClient::new(config.openrouter)?;
        match client.test_connection().await {
            Ok(_) => println!("OpenRouter connection: OK"),
            Err(e) => println!("OpenRouter connection: ERROR - {}", e),
        }
    }
    
    Ok(())
}