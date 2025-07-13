use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentConfig {
    pub openrouter: OpenRouterConfig,
    pub symbiont: SymbiontConfig,
    pub git: GitConfig,
    pub security: SecurityConfig,
    pub workflow: WorkflowConfig,
    pub validation: ValidationConfig,
    pub safety: SafetyConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenRouterConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SymbiontConfig {
    pub context_storage_path: PathBuf,
    pub qdrant_url: String,
    pub collection_name: String,
    pub vector_dimension: usize,
    pub enable_compression: bool,
    pub max_context_size_mb: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GitConfig {
    pub clone_base_path: PathBuf,
    pub max_file_size_mb: usize,
    pub allowed_extensions: Vec<String>,
    pub ignore_patterns: Vec<String>,
    pub max_files_per_repo: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SecurityConfig {
    pub enable_schemapin: bool,
    pub schemapin_binary_path: Option<PathBuf>,
    pub policy_file: Option<PathBuf>,
    pub enable_sandbox: bool,
    pub sandbox_tier: String,
    pub trusted_domains: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WorkflowConfig {
    pub default_autonomy_level: String,
    pub enable_backups: bool,
    pub backup_directory: PathBuf,
    pub confirmation_timeout_seconds: u64,
    pub max_retry_attempts: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ValidationConfig {
    pub enable_syntax_checks: bool,
    pub enable_dependency_checks: bool,
    pub enable_impact_analysis: bool,
    pub validation_timeout_seconds: u64,
    pub strict_mode: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SafetyConfig {
    pub enable_safety_checks: bool,
    pub risk_threshold: String,
    pub protected_patterns: Vec<String>,
    pub dangerous_operations: Vec<String>,
    pub require_confirmation_for_high_risk: bool,
}

impl AgentConfig {
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: AgentConfig = toml::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }
    
    pub async fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = tokio::fs::read_to_string(path).await?;
        let config: AgentConfig = toml::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }
    
    pub async fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        tokio::fs::write(path, content).await?;
        Ok(())
    }
    
    pub fn default() -> Self {
        Self {
            openrouter: OpenRouterConfig {
                api_key: std::env::var("OPENROUTER_API_KEY")
                    .unwrap_or_else(|_| "your_api_key_here".to_string()),
                base_url: "https://openrouter.ai/api/v1".to_string(),
                model: "anthropic/claude-3.5-sonnet".to_string(),
                max_tokens: Some(4000),
                temperature: Some(0.1),
                timeout_seconds: Some(60),
            },
            symbiont: SymbiontConfig {
                context_storage_path: PathBuf::from("./agent_contexts"),
                qdrant_url: "http://localhost:6333".to_string(),
                collection_name: "openrouter_git_agent".to_string(),
                vector_dimension: 384,
                enable_compression: true,
                max_context_size_mb: 100,
            },
            git: GitConfig {
                clone_base_path: PathBuf::from("./repositories"),
                max_file_size_mb: 5,
                allowed_extensions: vec![
                    "rs".to_string(),
                    "py".to_string(),
                    "js".to_string(),
                    "ts".to_string(),
                    "go".to_string(),
                    "java".to_string(),
                    "cpp".to_string(),
                    "c".to_string(),
                    "h".to_string(),
                    "md".to_string(),
                    "txt".to_string(),
                    "toml".to_string(),
                    "yaml".to_string(),
                    "yml".to_string(),
                    "json".to_string(),
                ],
                ignore_patterns: vec![
                    "target/".to_string(),
                    "node_modules/".to_string(),
                    ".git/".to_string(),
                    "*.lock".to_string(),
                    "*.log".to_string(),
                ],
                max_files_per_repo: 1000,
            },
            security: SecurityConfig {
                enable_schemapin: false,  // Disabled by default for simpler setup
                schemapin_binary_path: Some(PathBuf::from("/usr/local/bin/schemapin-cli")),
                policy_file: Some(PathBuf::from("security_policies.yaml")),
                enable_sandbox: false,  // Disabled by default for simpler setup
                sandbox_tier: "Tier2".to_string(),
                trusted_domains: vec![
                    "github.com".to_string(),
                    "gitlab.com".to_string(),
                ],
            },
            workflow: WorkflowConfig {
                default_autonomy_level: "auto-backup".to_string(),
                enable_backups: true,
                backup_directory: PathBuf::from("./backups"),
                confirmation_timeout_seconds: 30,
                max_retry_attempts: 3,
            },
            validation: ValidationConfig {
                enable_syntax_checks: true,
                enable_dependency_checks: true,
                enable_impact_analysis: true,
                validation_timeout_seconds: 60,
                strict_mode: false,
            },
            safety: SafetyConfig {
                enable_safety_checks: true,
                risk_threshold: "medium".to_string(),
                protected_patterns: vec![
                    "*.env".to_string(),
                    "*.key".to_string(),
                    "*.pem".to_string(),
                    "password".to_string(),
                    "secret".to_string(),
                ],
                dangerous_operations: vec![
                    "rm -rf".to_string(),
                    "format".to_string(),
                    "delete".to_string(),
                ],
                require_confirmation_for_high_risk: true,
            },
        }
    }
    
    fn validate(&self) -> Result<()> {
        // Skip validation for default values to allow easier setup
        if self.openrouter.api_key.is_empty() {
            anyhow::bail!("OpenRouter API key must be set");
        }
        
        if !self.openrouter.base_url.starts_with("http") {
            anyhow::bail!("OpenRouter base URL must be a valid HTTP(S) URL");
        }
        
        // Validate Symbiont configuration
        if self.symbiont.vector_dimension == 0 {
            anyhow::bail!("Vector dimension must be greater than 0");
        }
        
        if self.symbiont.max_context_size_mb == 0 {
            anyhow::bail!("Max context size must be greater than 0");
        }
        
        // Validate Git configuration
        if self.git.max_file_size_mb == 0 {
            anyhow::bail!("Max file size must be greater than 0");
        }
        
        if self.git.max_files_per_repo == 0 {
            anyhow::bail!("Max files per repo must be greater than 0");
        }
        
        Ok(())
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    
    #[tokio::test]
    async fn test_config_roundtrip() {
        let config = AgentConfig::default();
        let temp_dir = tempdir().unwrap();
        let config_path = temp_dir.path().join("test_config.toml");
        
        // Save and load config
        config.save(&config_path).await.unwrap();
        let loaded_config = AgentConfig::load(&config_path).await.unwrap();
        
        assert_eq!(config.openrouter.model, loaded_config.openrouter.model);
        assert_eq!(config.symbiont.vector_dimension, loaded_config.symbiont.vector_dimension);
    }
    
    #[test]
    fn test_config_validation() {
        let mut config = AgentConfig::default();
        
        // Test with empty API key
        config.openrouter.api_key = "".to_string();
        assert!(config.validate().is_err());
        
        // Reset and test vector dimension
        config = AgentConfig::default();
        config.symbiont.vector_dimension = 0;
        assert!(config.validate().is_err());
    }
}