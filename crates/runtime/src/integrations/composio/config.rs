//! Composio MCP configuration types and loader
//!
//! Defines the TOML-deserializable configuration for `~/.symbiont/mcp-config.toml`,
//! supporting both Composio SSE servers and stdio-based MCP servers.

use serde::{Deserialize, Serialize};
use std::path::Path;

use super::error::ComposioError;

/// Top-level MCP configuration file structure
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct McpConfigFile {
    /// Global Composio settings (API key, base URL)
    pub composio: Option<ComposioGlobalConfig>,
    /// List of configured MCP servers
    #[serde(default)]
    pub mcp_servers: Vec<McpServerEntry>,
}

/// Global Composio configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ComposioGlobalConfig {
    /// API key — literal value or `env:VAR_NAME` to read from environment
    pub api_key: String,
    /// Base URL for Composio MCP endpoints
    #[serde(default = "default_base_url")]
    pub base_url: String,
}

fn default_base_url() -> String {
    "https://backend.composio.dev".to_string()
}

/// An individual MCP server entry, tagged by transport type
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum McpServerEntry {
    /// Composio-hosted MCP server accessed via SSE
    #[serde(rename = "composio")]
    Composio {
        name: String,
        server_id: String,
        user_id: String,
        /// Optional direct MCP URL (overrides base_url + server_id + user_id)
        #[serde(default)]
        url: Option<String>,
        #[serde(default)]
        policy: Option<ServerPolicy>,
    },
    /// Local stdio-based MCP server
    #[serde(rename = "stdio")]
    Stdio {
        name: String,
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        policy: Option<ServerPolicy>,
    },
}

impl McpServerEntry {
    /// Returns the name of this server entry
    pub fn name(&self) -> &str {
        match self {
            McpServerEntry::Composio { name, .. } => name,
            McpServerEntry::Stdio { name, .. } => name,
        }
    }

    /// Returns the policy for this server entry, if any
    pub fn policy(&self) -> Option<&ServerPolicy> {
        match self {
            McpServerEntry::Composio { policy, .. } => policy.as_ref(),
            McpServerEntry::Stdio { policy, .. } => policy.as_ref(),
        }
    }
}

/// Per-server policy controlling which tools are exposed and how
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ServerPolicy {
    /// Glob patterns for allowed tool names (e.g. `["GITHUB_*"]`)
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Glob patterns for tools requiring user approval before invocation
    #[serde(default)]
    pub require_approval: Vec<String>,
    /// Audit logging level: "none", "basic", or "full"
    #[serde(default = "default_audit_level")]
    pub audit_level: String,
    /// Rate limit: maximum tool calls per minute
    pub max_calls_per_minute: Option<u32>,
}

fn default_audit_level() -> String {
    "basic".to_string()
}

/// Resolve a secret value that may reference an environment variable.
///
/// If the value starts with `env:`, the remainder is treated as an
/// environment variable name and its value is returned. Otherwise
/// the literal string is returned.
pub fn resolve_secret(value: &str) -> Result<String, ComposioError> {
    if let Some(var_name) = value.strip_prefix("env:") {
        std::env::var(var_name).map_err(|_| ComposioError::ConfigError {
            reason: format!("environment variable '{}' not set", var_name),
        })
    } else {
        Ok(value.to_string())
    }
}

/// Load the MCP configuration file from the given path.
///
/// Defaults to `~/.symbiont/mcp-config.toml` if no path is provided.
/// Returns an empty default config if the file does not exist.
pub fn load_mcp_config(path: Option<&Path>) -> Result<McpConfigFile, ComposioError> {
    let config_path = match path {
        Some(p) => p.to_path_buf(),
        None => {
            let home = dirs::home_dir().ok_or_else(|| ComposioError::ConfigError {
                reason: "could not determine home directory".to_string(),
            })?;
            home.join(".symbiont").join("mcp-config.toml")
        }
    };

    if !config_path.exists() {
        return Ok(McpConfigFile::default());
    }

    let content =
        std::fs::read_to_string(&config_path).map_err(|e| ComposioError::ConfigError {
            reason: format!("failed to read {}: {}", config_path.display(), e),
        })?;

    toml::from_str(&content).map_err(|e| ComposioError::ConfigError {
        reason: format!("failed to parse {}: {}", config_path.display(), e),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_parse_minimal_toml() {
        let toml_str = r#"
[composio]
api_key = "test-key-123"
"#;
        let config: McpConfigFile = toml::from_str(toml_str).unwrap();
        let composio = config.composio.unwrap();
        assert_eq!(composio.api_key, "test-key-123");
        assert_eq!(composio.base_url, "https://backend.composio.dev");
        assert!(config.mcp_servers.is_empty());
    }

    #[test]
    fn test_parse_full_toml_with_both_server_types() {
        let toml_str = r#"
[composio]
api_key = "env:COMPOSIO_API_KEY"
base_url = "https://custom.composio.dev"

[[mcp_servers]]
type = "composio"
name = "github"
server_id = "srv_github_123"
user_id = "user_456"

[mcp_servers.policy]
allowed_tools = ["GITHUB_*"]
require_approval = ["GITHUB_DELETE_*"]
audit_level = "full"
max_calls_per_minute = 60

[[mcp_servers]]
type = "stdio"
name = "local-tools"
command = "/usr/local/bin/mcp-server"
args = ["--port", "8080"]
"#;
        let config: McpConfigFile = toml::from_str(toml_str).unwrap();
        let composio = config.composio.unwrap();
        assert_eq!(composio.api_key, "env:COMPOSIO_API_KEY");
        assert_eq!(composio.base_url, "https://custom.composio.dev");
        assert_eq!(config.mcp_servers.len(), 2);

        match &config.mcp_servers[0] {
            McpServerEntry::Composio {
                name,
                server_id,
                user_id,
                policy,
                ..
            } => {
                assert_eq!(name, "github");
                assert_eq!(server_id, "srv_github_123");
                assert_eq!(user_id, "user_456");
                let p = policy.as_ref().unwrap();
                assert_eq!(p.allowed_tools, vec!["GITHUB_*"]);
                assert_eq!(p.require_approval, vec!["GITHUB_DELETE_*"]);
                assert_eq!(p.audit_level, "full");
                assert_eq!(p.max_calls_per_minute, Some(60));
            }
            _ => panic!("expected Composio server entry"),
        }

        match &config.mcp_servers[1] {
            McpServerEntry::Stdio {
                name,
                command,
                args,
                policy,
            } => {
                assert_eq!(name, "local-tools");
                assert_eq!(command, "/usr/local/bin/mcp-server");
                assert_eq!(args, &["--port", "8080"]);
                assert!(policy.is_none());
            }
            _ => panic!("expected Stdio server entry"),
        }
    }

    #[test]
    fn test_resolve_secret_env_var() {
        std::env::set_var("TEST_COMPOSIO_KEY_12345", "secret-value");
        let result = resolve_secret("env:TEST_COMPOSIO_KEY_12345").unwrap();
        assert_eq!(result, "secret-value");
        std::env::remove_var("TEST_COMPOSIO_KEY_12345");
    }

    #[test]
    fn test_resolve_secret_literal() {
        let result = resolve_secret("literal-key").unwrap();
        assert_eq!(result, "literal-key");
    }

    #[test]
    fn test_resolve_secret_missing_env_var() {
        let result = resolve_secret("env:NONEXISTENT_VAR_COMPOSIO_XYZ");
        assert!(result.is_err());
    }

    #[test]
    fn test_load_missing_file_returns_empty() {
        let config =
            load_mcp_config(Some(Path::new("/tmp/nonexistent-composio-config.toml"))).unwrap();
        assert!(config.composio.is_none());
        assert!(config.mcp_servers.is_empty());
    }

    #[test]
    fn test_load_invalid_toml_errors() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "this is not [valid toml {{{{").unwrap();
        let result = load_mcp_config(Some(f.path()));
        assert!(result.is_err());
    }

    #[test]
    fn test_policy_defaults() {
        let toml_str = r#"
[[mcp_servers]]
type = "composio"
name = "test"
server_id = "srv_1"
user_id = "usr_1"

[mcp_servers.policy]
"#;
        let config: McpConfigFile = toml::from_str(toml_str).unwrap();
        match &config.mcp_servers[0] {
            McpServerEntry::Composio { policy, .. } => {
                let p = policy.as_ref().unwrap();
                assert!(p.allowed_tools.is_empty());
                assert!(p.require_approval.is_empty());
                assert_eq!(p.audit_level, "basic");
                assert!(p.max_calls_per_minute.is_none());
            }
            _ => panic!("expected Composio entry"),
        }
    }

    #[test]
    fn test_load_valid_file() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
[composio]
api_key = "test-key"

[[mcp_servers]]
type = "composio"
name = "github"
server_id = "srv_1"
user_id = "usr_1"
"#
        )
        .unwrap();
        let config = load_mcp_config(Some(f.path())).unwrap();
        assert!(config.composio.is_some());
        assert_eq!(config.mcp_servers.len(), 1);
    }

    #[test]
    fn test_server_entry_name_accessor() {
        let entry = McpServerEntry::Composio {
            name: "github".to_string(),
            server_id: "srv_1".to_string(),
            user_id: "usr_1".to_string(),
            url: None,
            policy: None,
        };
        assert_eq!(entry.name(), "github");

        let entry = McpServerEntry::Stdio {
            name: "local".to_string(),
            command: "cmd".to_string(),
            args: vec![],
            policy: None,
        };
        assert_eq!(entry.name(), "local");
    }
}
