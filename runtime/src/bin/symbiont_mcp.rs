//! Symbiont MCP Server Management CLI
//! 
//! Provides command-line interface for managing MCP servers with integrated
//! security verification and tool discovery.

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::{info, Level};

mod commands;
mod config;
mod github;
mod registry;

use commands::*;

#[derive(Parser)]
#[command(name = "symbiont-mcp")]
#[command(about = "Symbiont MCP Server Management CLI")]
#[command(version = "0.1.0")]
struct Cli {
    /// Configuration file path
    #[arg(short, long, default_value = "~/.symbiont/mcp-config.toml")]
    config: PathBuf,
    
    /// Verbose output
    #[arg(short, long)]
    verbose: bool,
    
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a new MCP server
    Add {
        /// GitHub repository URL or server endpoint
        source: String,
        /// Optional server name (auto-generated if not provided)
        #[arg(short, long)]
        name: Option<String>,
        /// Skip verification (development only)
        #[arg(long)]
        skip_verification: bool,
    },
    /// Remove an MCP server
    Remove {
        /// Server name to remove
        name: String,
        /// Force removal without confirmation
        #[arg(short, long)]
        force: bool,
    },
    /// List all registered MCP servers
    List {
        /// Show detailed information
        #[arg(short, long)]
        detailed: bool,
        /// Filter by status (active, inactive, error)
        #[arg(short, long)]
        status: Option<String>,
    },
    /// Check server status and health
    Status {
        /// Specific server name (all servers if not provided)
        name: Option<String>,
        /// Run health check
        #[arg(short, long)]
        health_check: bool,
    },
    /// Verify server tools and schemas
    Verify {
        /// Server name to verify
        name: String,
        /// Force re-verification
        #[arg(short, long)]
        force: bool,
    },
    /// Update server configuration
    Update {
        /// Server name to update
        name: String,
        /// New source URL
        #[arg(short, long)]
        source: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    
    // Initialize logging
    let level = if cli.verbose { Level::DEBUG } else { Level::INFO };
    tracing_subscriber::fmt()
        .with_max_level(level)
        .init();
    
    info!("Starting Symbiont MCP CLI");
    
    // Load configuration
    let config_path = expand_path(&cli.config)?;
    let config = config::McpConfig::load_or_create(&config_path).await?;
    
    // Execute command
    match cli.command {
        Commands::Add { source, name, skip_verification } => {
            add_server(&config, &source, name, skip_verification).await
        }
        Commands::Remove { name, force } => {
            remove_server(&config, &name, force).await
        }
        Commands::List { detailed, status } => {
            list_servers(&config, detailed, status).await
        }
        Commands::Status { name, health_check } => {
            status_check(&config, name, health_check).await
        }
        Commands::Verify { name, force } => {
            verify_server(&config, &name, force).await
        }
        Commands::Update { name, source } => {
            update_server(&config, &name, source).await
        }
    }
}

/// Expand ~ in path to home directory
fn expand_path(path: &PathBuf) -> Result<PathBuf> {
    let path_str = path.to_string_lossy();
    if path_str.starts_with('~') {
        let home = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
        let relative = &path_str[1..];
        if relative.starts_with('/') {
            Ok(home.join(&relative[1..]))
        } else {
            Ok(home.join(relative))
        }
    } else {
        Ok(path.clone())
    }
}