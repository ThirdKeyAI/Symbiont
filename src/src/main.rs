use clap::{Parser, Subcommand};
use std::process;

#[derive(Parser)]
#[command(name = "symbi")]
#[command(about = "Symbi - Unified DSL and Runtime for AI-native programming")]
#[command(version = "0.1.0")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// DSL operations
    Dsl {
        /// DSL command arguments
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// Runtime operations
    Runtime {
        /// Runtime command arguments
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// MCP server operations
    Mcp {
        /// MCP command arguments
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Dsl { args } => {
            // Launch the DSL binary
            let mut cmd = std::process::Command::new("symbi-dsl");
            cmd.args(&args);
            
            let status = cmd.status().unwrap_or_else(|_| {
                eprintln!("Failed to execute symbi-dsl");
                process::exit(1);
            });
            
            if !status.success() {
                process::exit(status.code().unwrap_or(1));
            }
        }
        Commands::Runtime { args: _ } => {
            // For now, print a message since we don't have a standalone runtime binary
            eprintln!("Runtime functionality will be integrated directly into symbi in future versions");
            eprintln!("Use 'symbi mcp' for MCP server functionality");
            process::exit(1);
        }
        Commands::Mcp { args } => {
            // Launch the MCP binary
            let mut cmd = std::process::Command::new("symbi-mcp");
            cmd.args(&args);
            
            let status = cmd.status().unwrap_or_else(|_| {
                eprintln!("Failed to execute symbi-mcp");
                process::exit(1);
            });
            
            if !status.success() {
                process::exit(status.code().unwrap_or(1));
            }
        }
    }
}