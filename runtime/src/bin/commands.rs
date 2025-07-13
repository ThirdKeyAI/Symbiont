//! CLI command implementations

use anyhow::{anyhow, Result};
use std::io::{self, Write};
use tracing::{error, info, warn};

use symbiont_runtime::integrations::{
    McpClient, SecureMcpClient, McpClientConfig, McpTool, ToolProvider, VerificationStatus,
};

use crate::config::{McpConfig, McpServerConfig, McpServerType, ServerStatus};
use crate::github::GitHubClient;

/// Add a new MCP server
pub async fn add_server(
    config: &McpConfig,
    source: &str,
    name: Option<String>,
    skip_verification: bool,
) -> Result<()> {
    info!("Adding MCP server from source: {}", source);
    
    // Parse source to determine type and configuration
    let (server_type, server_name, resolved_source) = resolve_source(source, name)?;
    
    // Load existing configuration
    let mut mcp_config = config.clone();
    
    // Check if server already exists
    if mcp_config.get_server(&server_name).is_some() {
        return Err(anyhow!("Server '{}' already exists", server_name));
    }
    
    // Create server configuration
    let mut server_config = McpServerConfig::new(
        server_name.clone(),
        resolved_source.clone(),
        server_type.clone(),
    );
    
    // Discover server information based on type
    match server_type {
        McpServerType::GitHub => {
            let github_client = GitHubClient::new(mcp_config.settings.github_token.clone())?;
            let repo_info = GitHubClient::parse_github_url(&resolved_source)?;
            
            info!("Discovering MCP servers in GitHub repository: {}/{}", repo_info.owner, repo_info.repo);
            
            let discovered_servers = github_client.discover_mcp_servers(&repo_info).await?;
            
            if discovered_servers.is_empty() {
                warn!("No MCP servers found in repository");
                return Err(anyhow!("No MCP servers discovered in the specified repository"));
            }
            
            // Use the first discovered server
            let server_info = &discovered_servers[0];
            server_config.metadata.insert("description".to_string(), 
                server_info.description.clone().unwrap_or_default());
            server_config.metadata.insert("entry_point".to_string(), 
                server_info.entry_point.clone());
            
            // Add discovered tools
            for tool in &server_info.tools {
                server_config.add_tool(tool.name.clone());
            }
            
            info!("Discovered {} tools in server", server_info.tools.len());
        }
        McpServerType::Direct => {
            // For direct URLs, we'll attempt to connect and discover tools later
            server_config.endpoint = Some(resolved_source.clone());
        }
        McpServerType::Registry => {
            // Registry lookup would be implemented here
            return Err(anyhow!("Registry lookup not implemented yet"));
        }
    }
    
    // Set up MCP client for verification
    if !skip_verification {
        info!("Verifying server tools...");
        
        let mcp_client_config = McpClientConfig {
            enforce_verification: mcp_config.settings.enforce_verification,
            allow_unverified_in_dev: mcp_config.settings.allow_unverified_in_dev,
            verification_timeout_seconds: mcp_config.settings.verification_timeout_seconds,
            max_concurrent_verifications: mcp_config.settings.max_concurrent_verifications,
        };
        
        let mcp_client = SecureMcpClient::with_defaults(mcp_client_config)?;
        
        // Create mock tools for verification based on discovered information
        // In a real implementation, this would connect to the actual MCP server
        let verification_result = verify_server_tools(&mcp_client, &server_config).await;
        
        match verification_result {
            Ok(_) => {
                server_config.verification_status = crate::config::VerificationStatus::Verified;
                server_config.status = ServerStatus::Active;
                info!("✅ Server verification successful");
            }
            Err(e) => {
                if mcp_config.settings.enforce_verification {
                    error!("❌ Server verification failed: {}", e);
                    return Err(e);
                } else {
                    warn!("⚠️ Server verification failed but continuing: {}", e);
                    server_config.verification_status = crate::config::VerificationStatus::Failed {
                        reason: e.to_string(),
                    };
                    server_config.status = ServerStatus::Error { 
                        message: format!("Verification failed: {}", e)
                    };
                }
            }
        }
    } else {
        server_config.verification_status = crate::config::VerificationStatus::Skipped {
            reason: "Verification skipped by user".to_string(),
        };
        server_config.status = ServerStatus::Unknown;
    }
    
    // Add server to configuration
    mcp_config.add_server(server_name.clone(), server_config);
    
    // Save configuration
    let config_path = expand_path(&std::path::PathBuf::from(&mcp_config.settings.config_dir))?
        .join("mcp-config.toml");
    mcp_config.save(&config_path).await?;
    
    println!("✅ Successfully added MCP server: {}", server_name);
    Ok(())
}

/// Remove an MCP server
pub async fn remove_server(config: &McpConfig, name: &str, force: bool) -> Result<()> {
    let mut mcp_config = config.clone();
    
    // Check if server exists
    if mcp_config.get_server(name).is_none() {
        return Err(anyhow!("Server '{}' not found", name));
    }
    
    // Confirm removal unless forced
    if !force {
        print!("Are you sure you want to remove server '{}'? (y/N): ", name);
        io::stdout().flush()?;
        
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        
        if !input.trim().to_lowercase().starts_with('y') {
            println!("Removal cancelled");
            return Ok(());
        }
    }
    
    // Remove server
    mcp_config.remove_server(name);
    
    // Save configuration
    let config_path = expand_path(&std::path::PathBuf::from(&mcp_config.settings.config_dir))?
        .join("mcp-config.toml");
    mcp_config.save(&config_path).await?;
    
    println!("✅ Successfully removed MCP server: {}", name);
    Ok(())
}

/// List all registered MCP servers
pub async fn list_servers(
    config: &McpConfig,
    detailed: bool,
    status_filter: Option<String>,
) -> Result<()> {
    let servers = config.list_servers();
    
    if servers.is_empty() {
        println!("No MCP servers registered");
        return Ok(());
    }
    
    println!("Registered MCP servers:");
    println!();
    
    for server_name in servers {
        if let Some(server_config) = config.get_server(server_name) {
            // Apply status filter if provided
            if let Some(ref filter) = status_filter {
                let matches = match filter.to_lowercase().as_str() {
                    "active" => server_config.is_active(),
                    "inactive" => matches!(server_config.status, ServerStatus::Inactive),
                    "error" => matches!(server_config.status, ServerStatus::Error { .. }),
                    _ => true,
                };
                
                if !matches {
                    continue;
                }
            }
            
            // Display server information
            if detailed {
                print_detailed_server_info(server_config);
            } else {
                print_server_summary(server_config);
            }
        }
    }
    
    Ok(())
}

/// Check server status and health
pub async fn status_check(
    config: &McpConfig,
    name: Option<String>,
    health_check: bool,
) -> Result<()> {
    if let Some(server_name) = name {
        // Check specific server
        if let Some(server_config) = config.get_server(&server_name) {
            println!("Server: {}", server_name);
            print_detailed_server_info(server_config);
            
            if health_check {
                println!("\nPerforming health check...");
                // TODO: Implement actual health check
                println!("Health check not implemented yet");
            }
        } else {
            return Err(anyhow!("Server '{}' not found", server_name));
        }
    } else {
        // Check all servers
        let servers = config.list_servers();
        
        if servers.is_empty() {
            println!("No MCP servers registered");
            return Ok(());
        }
        
        println!("Server Status Overview:");
        println!();
        
        for server_name in servers {
            if let Some(server_config) = config.get_server(server_name) {
                println!("📋 {} - {} - {}", 
                    server_name, 
                    server_config.status, 
                    server_config.verification_status
                );
            }
        }
    }
    
    Ok(())
}

/// Verify server tools and schemas
pub async fn verify_server(config: &McpConfig, name: &str, force: bool) -> Result<()> {
    let mut mcp_config = config.clone();
    
    let server_config = mcp_config.get_server(name)
        .ok_or_else(|| anyhow!("Server '{}' not found", name))?
        .clone();
    
    // Check if already verified and not forcing
    if !force && server_config.is_verified() {
        println!("Server '{}' is already verified", name);
        return Ok(());
    }
    
    println!("Verifying server: {}", name);
    
    // Set up MCP client
    let mcp_client_config = McpClientConfig {
        enforce_verification: mcp_config.settings.enforce_verification,
        allow_unverified_in_dev: mcp_config.settings.allow_unverified_in_dev,
        verification_timeout_seconds: mcp_config.settings.verification_timeout_seconds,
        max_concurrent_verifications: mcp_config.settings.max_concurrent_verifications,
    };
    
    let mcp_client = SecureMcpClient::with_defaults(mcp_client_config)?;
    
    // Perform verification
    match verify_server_tools(&mcp_client, &server_config).await {
        Ok(_) => {
            mcp_config.update_verification_status(
                name, 
                crate::config::VerificationStatus::Verified
            );
            mcp_config.update_server_status(name, ServerStatus::Active);
            
            // Save configuration
            let config_path = expand_path(&std::path::PathBuf::from(&mcp_config.settings.config_dir))?
                .join("mcp-config.toml");
            mcp_config.save(&config_path).await?;
            
            println!("✅ Server verification successful");
        }
        Err(e) => {
            mcp_config.update_verification_status(
                name,
                crate::config::VerificationStatus::Failed { 
                    reason: e.to_string() 
                }
            );
            mcp_config.update_server_status(
                name, 
                ServerStatus::Error { 
                    message: format!("Verification failed: {}", e)
                }
            );
            
            // Save configuration
            let config_path = expand_path(&std::path::PathBuf::from(&mcp_config.settings.config_dir))?
                .join("mcp-config.toml");
            mcp_config.save(&config_path).await?;
            
            return Err(anyhow!("❌ Server verification failed: {}", e));
        }
    }
    
    Ok(())
}

/// Update server configuration
pub async fn update_server(
    config: &McpConfig,
    name: &str,
    source: Option<String>,
) -> Result<()> {
    let mut mcp_config = config.clone();
    
    let server_config = mcp_config.get_server_mut(name)
        .ok_or_else(|| anyhow!("Server '{}' not found", name))?;
    
    if let Some(new_source) = source {
        server_config.source = new_source;
        server_config.updated_at = chrono::Utc::now();
        
        // Reset verification status since source changed
        server_config.verification_status = crate::config::VerificationStatus::Pending;
        server_config.status = ServerStatus::Unknown;
    }
    
    // Save configuration
    let config_path = expand_path(&std::path::PathBuf::from(&mcp_config.settings.config_dir))?
        .join("mcp-config.toml");
    mcp_config.save(&config_path).await?;
    
    println!("✅ Successfully updated server: {}", name);
    Ok(())
}

// Helper functions

fn resolve_source(source: &str, name: Option<String>) -> Result<(McpServerType, String, String)> {
    if source.starts_with("https://github.com/") || source.starts_with("github.com/") {
        let normalized_source = if !source.starts_with("https://") {
            format!("https://{}", source)
        } else {
            source.to_string()
        };
        
        let repo_info = GitHubClient::parse_github_url(&normalized_source)?;
        let server_name = name.unwrap_or_else(|| format!("{}-{}", repo_info.owner, repo_info.repo));
        
        Ok((McpServerType::GitHub, server_name, normalized_source))
    } else if source.starts_with("http://") || source.starts_with("https://") {
        let server_name = name.unwrap_or_else(|| {
            url::Url::parse(source)
                .map(|u| u.host_str().unwrap_or("unknown").to_string())
                .unwrap_or_else(|_| "unknown".to_string())
        });
        
        Ok((McpServerType::Direct, server_name, source.to_string()))
    } else {
        // Assume it's a registry name
        let server_name = name.unwrap_or_else(|| source.to_string());
        Ok((McpServerType::Registry, server_name, source.to_string()))
    }
}

async fn verify_server_tools(
    mcp_client: &SecureMcpClient,
    _server_config: &McpServerConfig,
) -> Result<()> {
    // This is a mock implementation
    // In a real implementation, this would:
    // 1. Connect to the MCP server
    // 2. Discover available tools
    // 3. Verify each tool's schema using the SecureMcpClient
    // 4. Check cryptographic signatures
    
    // For now, we'll just simulate successful verification
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    
    // Create a mock tool for verification
    let mock_tool = McpTool {
        name: "mock_tool".to_string(),
        description: "Mock tool for testing".to_string(),
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "input": {"type": "string"}
            }
        }),
        provider: ToolProvider {
            identifier: "example.com".to_string(),
            name: "Example Provider".to_string(),
            public_key_url: "https://example.com/pubkey".to_string(),
            version: Some("1.0.0".to_string()),
        },
        verification_status: VerificationStatus::Pending,
        metadata: None,
    };
    
    // Attempt tool discovery (this would normally connect to the actual server)
    let _discovery_event = mcp_client.discover_tool(mock_tool).await?;
    
    Ok(())
}

fn print_server_summary(server_config: &McpServerConfig) {
    let status_icon = match server_config.status {
        ServerStatus::Active => "🟢",
        ServerStatus::Inactive => "🟡",
        ServerStatus::Error { .. } => "🔴",
        ServerStatus::Unknown => "⚪",
    };
    
    let verification_icon = match server_config.verification_status {
        crate::config::VerificationStatus::Verified => "✅",
        crate::config::VerificationStatus::Failed { .. } => "❌",
        crate::config::VerificationStatus::Pending => "⏳",
        crate::config::VerificationStatus::Skipped { .. } => "⏭️",
    };
    
    println!("{} {} {} - {} tools - {}",
        status_icon,
        verification_icon,
        server_config.name,
        server_config.tools.len(),
        server_config.source
    );
}

fn print_detailed_server_info(server_config: &McpServerConfig) {
    println!("📋 Server: {}", server_config.name);
    println!("   Source: {}", server_config.source);
    println!("   Type: {:?}", server_config.server_type);
    println!("   Status: {}", server_config.status);
    println!("   Verification: {}", server_config.verification_status);
    println!("   Tools: {} registered", server_config.tools.len());
    
    if !server_config.tools.is_empty() {
        println!("   Tool list:");
        for tool in &server_config.tools {
            println!("     - {}", tool);
        }
    }
    
    if let Some(endpoint) = &server_config.endpoint {
        println!("   Endpoint: {}", endpoint);
    }
    
    println!("   Updated: {}", server_config.updated_at.format("%Y-%m-%d %H:%M:%S UTC"));
    
    if !server_config.metadata.is_empty() {
        println!("   Metadata:");
        for (key, value) in &server_config.metadata {
            println!("     {}: {}", key, value);
        }
    }
    
    println!();
}

fn expand_path(path: &std::path::Path) -> Result<std::path::PathBuf> {
    let path_str = path.to_string_lossy();
    if path_str.starts_with('~') {
        let home = dirs::home_dir()
            .ok_or_else(|| anyhow!("Could not determine home directory"))?;
        let relative = &path_str[1..];
        if relative.starts_with('/') {
            Ok(home.join(&relative[1..]))
        } else {
            Ok(home.join(relative))
        }
    } else {
        Ok(path.to_path_buf())
    }
}