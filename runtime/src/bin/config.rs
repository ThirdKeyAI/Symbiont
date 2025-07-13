//! Configuration management for MCP servers

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub struct McpConfig {
    /// Global MCP settings
    pub settings: McpSettings,
    /// Registered MCP servers
    pub servers: HashMap<String, McpServerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpSettings {
    /// Default verification enforcement
    pub enforce_verification: bool,
    /// Allow unverified tools in development mode
    pub allow_unverified_in_dev: bool,
    /// Verification timeout in seconds
    pub verification_timeout_seconds: u64,
    /// Maximum concurrent verifications
    pub max_concurrent_verifications: usize,
    /// GitHub token for API access
    pub github_token: Option<String>,
    /// Configuration directory
    pub config_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Server name
    pub name: String,
    /// Server source (GitHub URL, direct URL, etc.)
    pub source: String,
    /// Server type
    pub server_type: McpServerType,
    /// Connection endpoint
    pub endpoint: Option<String>,
    /// Last verification status
    pub verification_status: VerificationStatus,
    /// Server status
    pub status: ServerStatus,
    /// Last updated timestamp
    pub updated_at: DateTime<Utc>,
    /// Registered tools
    pub tools: Vec<String>,
    /// Server metadata
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum McpServerType {
    GitHub,
    Direct,
    Registry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VerificationStatus {
    Pending,
    Verified,
    Failed { reason: String },
    Skipped { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerStatus {
    Active,
    Inactive,
    Error { message: String },
    Unknown,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            enforce_verification: true,
            allow_unverified_in_dev: false,
            verification_timeout_seconds: 30,
            max_concurrent_verifications: 5,
            github_token: std::env::var("GITHUB_TOKEN").ok(),
            config_dir: "~/.symbiont".to_string(),
        }
    }
}


impl McpConfig {
    /// Load configuration from file or create default
    pub async fn load_or_create<P: AsRef<Path>>(config_path: P) -> Result<Self> {
        let path = config_path.as_ref();
        
        if path.exists() {
            Self::load(path).await
        } else {
            let config = Self::default();
            config.save(path).await?;
            Ok(config)
        }
    }
    
    /// Load configuration from file
    pub async fn load<P: AsRef<Path>>(config_path: P) -> Result<Self> {
        let content = fs::read_to_string(config_path).await?;
        let config: McpConfig = toml::from_str(&content)?;
        Ok(config)
    }
    
    /// Save configuration to file
    pub async fn save<P: AsRef<Path>>(&self, config_path: P) -> Result<()> {
        let path = config_path.as_ref();
        
        // Create parent directory if it doesn't exist
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        
        let content = toml::to_string_pretty(self)?;
        fs::write(path, content).await?;
        Ok(())
    }
    
    /// Add a new server configuration
    pub fn add_server(&mut self, name: String, config: McpServerConfig) {
        self.servers.insert(name, config);
    }
    
    /// Remove server configuration
    pub fn remove_server(&mut self, name: &str) -> Option<McpServerConfig> {
        self.servers.remove(name)
    }
    
    /// Get server configuration
    pub fn get_server(&self, name: &str) -> Option<&McpServerConfig> {
        self.servers.get(name)
    }
    
    /// Get mutable server configuration
    pub fn get_server_mut(&mut self, name: &str) -> Option<&mut McpServerConfig> {
        self.servers.get_mut(name)
    }
    
    /// List all server names
    pub fn list_servers(&self) -> Vec<&String> {
        self.servers.keys().collect()
    }
    
    /// Update server status
    pub fn update_server_status(&mut self, name: &str, status: ServerStatus) {
        if let Some(server) = self.servers.get_mut(name) {
            server.status = status;
            server.updated_at = Utc::now();
        }
    }
    
    /// Update server verification status
    pub fn update_verification_status(&mut self, name: &str, status: VerificationStatus) {
        if let Some(server) = self.servers.get_mut(name) {
            server.verification_status = status;
            server.updated_at = Utc::now();
        }
    }
}

impl McpServerConfig {
    /// Create new server configuration
    pub fn new(
        name: String,
        source: String,
        server_type: McpServerType,
    ) -> Self {
        Self {
            name,
            source,
            server_type,
            endpoint: None,
            verification_status: VerificationStatus::Pending,
            status: ServerStatus::Unknown,
            updated_at: Utc::now(),
            tools: Vec::new(),
            metadata: HashMap::new(),
        }
    }
    
    /// Check if server is active
    pub fn is_active(&self) -> bool {
        matches!(self.status, ServerStatus::Active)
    }
    
    /// Check if server is verified
    pub fn is_verified(&self) -> bool {
        matches!(self.verification_status, VerificationStatus::Verified)
    }
    
    /// Add tool to server
    pub fn add_tool(&mut self, tool_name: String) {
        if !self.tools.contains(&tool_name) {
            self.tools.push(tool_name);
        }
    }
    
    /// Remove tool from server
    pub fn remove_tool(&mut self, tool_name: &str) {
        self.tools.retain(|t| t != tool_name);
    }
}

impl std::fmt::Display for VerificationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerificationStatus::Pending => write!(f, "Pending"),
            VerificationStatus::Verified => write!(f, "Verified"),
            VerificationStatus::Failed { reason } => write!(f, "Failed: {}", reason),
            VerificationStatus::Skipped { reason } => write!(f, "Skipped: {}", reason),
        }
    }
}

impl std::fmt::Display for ServerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerStatus::Active => write!(f, "Active"),
            ServerStatus::Inactive => write!(f, "Inactive"),
            ServerStatus::Error { message } => write!(f, "Error: {}", message),
            ServerStatus::Unknown => write!(f, "Unknown"),
        }
    }
}