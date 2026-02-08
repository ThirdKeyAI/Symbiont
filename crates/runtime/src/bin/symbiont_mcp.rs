//! Symbiont MCP Server Management CLI
//!
//! Provides command-line interface for managing MCP servers with integrated
//! security verification and tool discovery.

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use symbi_runtime::crypto::{Aes256GcmCrypto, KeyUtils};
use tempfile::NamedTempFile;
use tracing::{info, Level};

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
    /// Secrets management commands
    Secrets {
        #[command(subcommand)]
        command: SecretsCommands,
    },
}

#[derive(Subcommand)]
enum SecretsCommands {
    /// Encrypt a plaintext JSON file
    Encrypt {
        /// Input file path
        #[arg(long)]
        r#in: PathBuf,
        /// Output file path
        #[arg(long)]
        out: PathBuf,
    },
    /// Decrypt an encrypted file and print to stdout
    Decrypt {
        /// Input encrypted file path
        #[arg(long)]
        r#in: PathBuf,
    },
    /// Edit an encrypted file in the default editor
    Edit {
        /// File path to edit
        #[arg(long)]
        file: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    let level = if cli.verbose {
        Level::DEBUG
    } else {
        Level::INFO
    };
    tracing_subscriber::fmt().with_max_level(level).init();

    info!("Starting Symbiont MCP CLI");

    // Execute command
    match cli.command {
        Commands::Add {
            source,
            name,
            skip_verification,
        } => {
            println!("Add command not yet implemented");
            println!(
                "Source: {}, Name: {:?}, Skip verification: {}",
                source, name, skip_verification
            );
            Ok(())
        }
        Commands::Remove { name, force } => {
            println!("Remove command not yet implemented");
            println!("Name: {}, Force: {}", name, force);
            Ok(())
        }
        Commands::List { detailed, status } => {
            println!("List command not yet implemented");
            println!("Detailed: {}, Status: {:?}", detailed, status);
            Ok(())
        }
        Commands::Status { name, health_check } => {
            println!("Status command not yet implemented");
            println!("Name: {:?}, Health check: {}", name, health_check);
            Ok(())
        }
        Commands::Verify { name, force } => {
            println!("Verify command not yet implemented");
            println!("Name: {}, Force: {}", name, force);
            Ok(())
        }
        Commands::Update { name, source } => {
            println!("Update command not yet implemented");
            println!("Name: {}, Source: {:?}", name, source);
            Ok(())
        }
        Commands::Secrets { command } => handle_secrets_command(command).await,
    }
}

async fn handle_secrets_command(command: SecretsCommands) -> Result<()> {
    match command {
        SecretsCommands::Encrypt { r#in, out } => encrypt_file(&r#in, &out).await,
        SecretsCommands::Decrypt { r#in } => decrypt_file(&r#in).await,
        SecretsCommands::Edit { file } => edit_encrypted_file(&file).await,
    }
}

async fn encrypt_file(input_path: &PathBuf, output_path: &PathBuf) -> Result<()> {
    // Read the plaintext JSON file
    let plaintext = fs::read_to_string(input_path).map_err(|e| {
        anyhow::anyhow!(
            "Failed to read input file '{}': {}",
            input_path.display(),
            e
        )
    })?;

    // Validate that it's valid JSON
    serde_json::from_str::<serde_json::Value>(&plaintext)
        .map_err(|e| anyhow::anyhow!("Input file is not valid JSON: {}", e))?;

    // Get encryption key
    let key_utils = KeyUtils::new();
    let key = key_utils
        .get_or_create_key()
        .map_err(|e| anyhow::anyhow!("Failed to get encryption key: {}", e))?;

    // Encrypt the data
    let crypto = Aes256GcmCrypto::new();
    let encrypted_data = crypto
        .encrypt(plaintext.as_bytes(), &key)
        .map_err(|e| anyhow::anyhow!("Failed to encrypt data: {}", e))?;

    // Write encrypted data to output file
    fs::write(output_path, encrypted_data).map_err(|e| {
        anyhow::anyhow!(
            "Failed to write encrypted file '{}': {}",
            output_path.display(),
            e
        )
    })?;

    println!(
        "Successfully encrypted '{}' to '{}'",
        input_path.display(),
        output_path.display()
    );
    Ok(())
}

async fn decrypt_file(input_path: &PathBuf) -> Result<()> {
    // Read the encrypted file
    let encrypted_data = fs::read(input_path).map_err(|e| {
        anyhow::anyhow!(
            "Failed to read encrypted file '{}': {}",
            input_path.display(),
            e
        )
    })?;

    // Get decryption key
    let key_utils = KeyUtils::new();
    let key = key_utils
        .get_or_create_key()
        .map_err(|e| anyhow::anyhow!("Failed to get decryption key: {}", e))?;

    // Decrypt the data
    let crypto = Aes256GcmCrypto::new();
    let decrypted_data = crypto
        .decrypt(&encrypted_data, &key)
        .map_err(|e| anyhow::anyhow!("Failed to decrypt data: {}", e))?;

    // Convert to string and validate JSON
    let plaintext = String::from_utf8(decrypted_data)
        .map_err(|e| anyhow::anyhow!("Decrypted data is not valid UTF-8: {}", e))?;

    // Validate JSON before outputting
    serde_json::from_str::<serde_json::Value>(&plaintext)
        .map_err(|e| anyhow::anyhow!("Decrypted data is not valid JSON: {}", e))?;

    // Print decrypted JSON to stdout
    print!("{}", plaintext);
    Ok(())
}

async fn edit_encrypted_file(file_path: &PathBuf) -> Result<()> {
    // Check if file exists
    if !file_path.exists() {
        return Err(anyhow::anyhow!(
            "File '{}' does not exist",
            file_path.display()
        ));
    }

    // Get the editor from environment
    let editor = env::var("EDITOR").unwrap_or_else(|_| "nano".to_string());

    // Read and decrypt the file
    let encrypted_data = fs::read(file_path).map_err(|e| {
        anyhow::anyhow!(
            "Failed to read encrypted file '{}': {}",
            file_path.display(),
            e
        )
    })?;

    let key_utils = KeyUtils::new();
    let key = key_utils
        .get_or_create_key()
        .map_err(|e| anyhow::anyhow!("Failed to get decryption key: {}", e))?;

    let crypto = Aes256GcmCrypto::new();
    let decrypted_data = crypto
        .decrypt(&encrypted_data, &key)
        .map_err(|e| anyhow::anyhow!("Failed to decrypt data: {}", e))?;

    let plaintext = String::from_utf8(decrypted_data)
        .map_err(|e| anyhow::anyhow!("Decrypted data is not valid UTF-8: {}", e))?;

    // Validate JSON
    serde_json::from_str::<serde_json::Value>(&plaintext)
        .map_err(|e| anyhow::anyhow!("Decrypted data is not valid JSON: {}", e))?;

    // Create a temporary file with the decrypted content
    let temp_file = NamedTempFile::new()
        .map_err(|e| anyhow::anyhow!("Failed to create temporary file: {}", e))?;

    fs::write(temp_file.path(), &plaintext)
        .map_err(|e| anyhow::anyhow!("Failed to write to temporary file: {}", e))?;

    // Open the editor
    let status = Command::new(&editor)
        .arg(temp_file.path())
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to execute editor '{}': {}", editor, e))?;

    if !status.success() {
        return Err(anyhow::anyhow!(
            "Editor '{}' exited with non-zero status",
            editor
        ));
    }

    // Read the modified content
    let modified_content = fs::read_to_string(temp_file.path()).map_err(|e| {
        anyhow::anyhow!("Failed to read modified content from temporary file: {}", e)
    })?;

    // Validate JSON
    serde_json::from_str::<serde_json::Value>(&modified_content)
        .map_err(|e| anyhow::anyhow!("Modified content is not valid JSON: {}", e))?;

    // Re-encrypt the content
    let encrypted_data = crypto
        .encrypt(modified_content.as_bytes(), &key)
        .map_err(|e| anyhow::anyhow!("Failed to re-encrypt data: {}", e))?;

    // Write back to the original file
    fs::write(file_path, encrypted_data).map_err(|e| {
        anyhow::anyhow!(
            "Failed to write encrypted file '{}': {}",
            file_path.display(),
            e
        )
    })?;

    println!(
        "Successfully updated encrypted file '{}'",
        file_path.display()
    );
    Ok(())
}
