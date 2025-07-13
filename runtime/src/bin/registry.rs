//! MCP Server Registry for persistence and management

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;

use crate::config::{McpServerConfig, ServerStatus, VerificationStatus};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpRegistry {
    /// Registry metadata
    pub metadata: RegistryMetadata,
    /// Registered servers
    pub servers: HashMap<String, McpServerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryMetadata {
    /// Registry version
    pub version: String,
    /// Creation timestamp
    pub created_at: String,
    /// Last updated timestamp
    pub updated_at: String,
    /// Total number of servers
    pub server_count: usize,
}

impl Default for McpRegistry {
    fn default() -> Self {
        Self {
            metadata: RegistryMetadata {
                version: "1.0.0".to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
                updated_at: chrono::Utc::now().to_rfc3339(),
                server_count: 0,
            },
            servers: HashMap::new(),
        }
    }
}

impl McpRegistry {
    /// Create a new registry
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Load registry from file
    pub async fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(path).await?;
        let registry: McpRegistry = serde_json::from_str(&content)?;
        Ok(registry)
    }
    
    /// Save registry to file
    pub async fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let path = path.as_ref();
        
        // Create parent directory if it doesn't exist
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        
        let content = serde_json::to_string_pretty(self)?;
        fs::write(path, content).await?;
        Ok(())
    }
    
    /// Add server to registry
    pub fn add_server(&mut self, name: String, config: McpServerConfig) {
        self.servers.insert(name, config);
        self.update_metadata();
    }
    
    /// Remove server from registry
    pub fn remove_server(&mut self, name: &str) -> Option<McpServerConfig> {
        let result = self.servers.remove(name);
        self.update_metadata();
        result
    }
    
    /// Get server from registry
    pub fn get_server(&self, name: &str) -> Option<&McpServerConfig> {
        self.servers.get(name)
    }
    
    /// Get mutable server from registry
    pub fn get_server_mut(&mut self, name: &str) -> Option<&mut McpServerConfig> {
        self.servers.get_mut(name)
    }
    
    /// List all server names
    pub fn list_servers(&self) -> Vec<&String> {
        self.servers.keys().collect()
    }
    
    /// Get servers by status
    pub fn get_servers_by_status(&self, status: &ServerStatus) -> Vec<&McpServerConfig> {
        self.servers
            .values()
            .filter(|server| std::mem::discriminant(&server.status) == std::mem::discriminant(status))
            .collect()
    }
    
    /// Get servers by verification status
    pub fn get_servers_by_verification(&self, verification: &VerificationStatus) -> Vec<&McpServerConfig> {
        self.servers
            .values()
            .filter(|server| std::mem::discriminant(&server.verification_status) == std::mem::discriminant(verification))
            .collect()
    }
    
    /// Update server status
    pub fn update_server_status(&mut self, name: &str, status: ServerStatus) {
        if let Some(server) = self.servers.get_mut(name) {
            server.status = status;
            server.updated_at = chrono::Utc::now();
        }
        self.update_metadata();
    }
    
    /// Update server verification status
    pub fn update_verification_status(&mut self, name: &str, verification: VerificationStatus) {
        if let Some(server) = self.servers.get_mut(name) {
            server.verification_status = verification;
            server.updated_at = chrono::Utc::now();
        }
        self.update_metadata();
    }
    
    /// Get registry statistics
    pub fn get_stats(&self) -> RegistryStats {
        let total = self.servers.len();
        let active = self.servers.values().filter(|s| s.is_active()).count();
        let verified = self.servers.values().filter(|s| s.is_verified()).count();
        let with_errors = self.servers.values()
            .filter(|s| matches!(s.status, ServerStatus::Error { .. }))
            .count();
        
        RegistryStats {
            total_servers: total,
            active_servers: active,
            verified_servers: verified,
            servers_with_errors: with_errors,
            total_tools: self.servers.values().map(|s| s.tools.len()).sum(),
        }
    }
    
    /// Update metadata
    fn update_metadata(&mut self) {
        self.metadata.updated_at = chrono::Utc::now().to_rfc3339();
        self.metadata.server_count = self.servers.len();
    }
    
    /// Validate registry integrity
    pub fn validate(&self) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        
        // Check for duplicate sources
        let mut sources = std::collections::HashSet::new();
        for (name, server) in &self.servers {
            if !sources.insert(&server.source) {
                errors.push(ValidationError::DuplicateSource {
                    server_name: name.clone(),
                    source: server.source.clone(),
                });
            }
        }
        
        // Check for invalid server names
        for name in self.servers.keys() {
            if name.is_empty() || name.contains(' ') {
                errors.push(ValidationError::InvalidServerName {
                    server_name: name.clone(),
                });
            }
        }
        
        // Check metadata consistency
        if self.metadata.server_count != self.servers.len() {
            errors.push(ValidationError::MetadataInconsistency {
                expected: self.servers.len(),
                actual: self.metadata.server_count,
            });
        }
        
        errors
    }
    
    /// Cleanup old entries
    pub fn cleanup_old_entries(&mut self, max_age_days: u64) {
        let cutoff = chrono::Utc::now() - chrono::Duration::days(max_age_days as i64);
        
        let to_remove: Vec<String> = self.servers
            .iter()
            .filter(|(_, server)| {
                server.updated_at < cutoff &&
                matches!(server.status, ServerStatus::Error { .. } | ServerStatus::Inactive)
            })
            .map(|(name, _)| name.clone())
            .collect();
        
        for name in to_remove {
            self.remove_server(&name);
        }
    }
}

#[derive(Debug, Clone)]
pub struct RegistryStats {
    pub total_servers: usize,
    pub active_servers: usize,
    pub verified_servers: usize,
    pub servers_with_errors: usize,
    pub total_tools: usize,
}

#[derive(Debug, Clone)]
pub enum ValidationError {
    DuplicateSource {
        server_name: String,
        source: String,
    },
    InvalidServerName {
        server_name: String,
    },
    MetadataInconsistency {
        expected: usize,
        actual: usize,
    },
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidationError::DuplicateSource { server_name, source } => {
                write!(f, "Duplicate source '{}' found for server '{}'", source, server_name)
            }
            ValidationError::InvalidServerName { server_name } => {
                write!(f, "Invalid server name: '{}'", server_name)
            }
            ValidationError::MetadataInconsistency { expected, actual } => {
                write!(f, "Metadata inconsistency: expected {} servers, found {}", expected, actual)
            }
        }
    }
}

impl std::error::Error for ValidationError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{McpServerType, VerificationStatus};
    
    #[test]
    fn test_registry_creation() {
        let registry = McpRegistry::new();
        assert_eq!(registry.servers.len(), 0);
        assert_eq!(registry.metadata.server_count, 0);
    }
    
    #[test]
    fn test_add_remove_server() {
        let mut registry = McpRegistry::new();
        let server_config = McpServerConfig::new(
            "test-server".to_string(),
            "https://github.com/test/repo".to_string(),
            McpServerType::GitHub,
        );
        
        registry.add_server("test-server".to_string(), server_config.clone());
        assert_eq!(registry.servers.len(), 1);
        assert_eq!(registry.metadata.server_count, 1);
        
        let removed = registry.remove_server("test-server");
        assert!(removed.is_some());
        assert_eq!(registry.servers.len(), 0);
        assert_eq!(registry.metadata.server_count, 0);
    }
    
    #[test]
    fn test_validation() {
        let mut registry = McpRegistry::new();
        
        // Add server with duplicate source
        let server1 = McpServerConfig::new(
            "server1".to_string(),
            "https://github.com/test/repo".to_string(),
            McpServerType::GitHub,
        );
        let server2 = McpServerConfig::new(
            "server2".to_string(),
            "https://github.com/test/repo".to_string(), // Same source
            McpServerType::GitHub,
        );
        
        registry.add_server("server1".to_string(), server1);
        registry.add_server("server2".to_string(), server2);
        
        let errors = registry.validate();
        assert!(!errors.is_empty());
        assert!(matches!(errors[0], ValidationError::DuplicateSource { .. }));
    }
    
    #[test]
    fn test_stats() {
        let mut registry = McpRegistry::new();
        let mut server_config = McpServerConfig::new(
            "test-server".to_string(),
            "https://github.com/test/repo".to_string(),
            McpServerType::GitHub,
        );
        
        server_config.status = ServerStatus::Active;
        server_config.verification_status = VerificationStatus::Verified;
        server_config.add_tool("tool1".to_string());
        server_config.add_tool("tool2".to_string());
        
        registry.add_server("test-server".to_string(), server_config);
        
        let stats = registry.get_stats();
        assert_eq!(stats.total_servers, 1);
        assert_eq!(stats.active_servers, 1);
        assert_eq!(stats.verified_servers, 1);
        assert_eq!(stats.total_tools, 2);
    }
}