//! Registry mapping MCP server names (referenced from ToolClad `[mcp]`
//! manifests) to a stdio launch spec. Loaded from `./mcp-config.toml`
//! (per-project) or `~/.symbiont/mcp-config.toml` (user default).

use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct StdioServerSpec {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// URL of the server's SchemaPin public key (PEM). Required for this
    /// server's tools to pass verification under enforcement; when unset,
    /// enforced invocation is blocked fail-closed (nothing to verify against).
    #[serde(default)]
    pub public_key_url: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RegistryFile {
    #[serde(default)]
    servers: HashMap<String, StdioServerSpec>,
}

#[derive(Debug, Clone, Default)]
pub struct McpServerRegistry {
    servers: HashMap<String, StdioServerSpec>,
}

impl McpServerRegistry {
    /// Load from `./mcp-config.toml`, falling back to
    /// `~/.symbiont/mcp-config.toml`; empty if neither exists or on read error
    /// (a warning is logged). Never panics.
    pub fn load() -> Self {
        for path in Self::candidate_paths() {
            if path.is_file() {
                match std::fs::read_to_string(&path) {
                    Ok(s) => match Self::from_toml_str(&s) {
                        Ok(reg) => return reg,
                        Err(e) => tracing::warn!("invalid MCP registry {}: {}", path.display(), e),
                    },
                    Err(e) => tracing::warn!("cannot read MCP registry {}: {}", path.display(), e),
                }
            }
        }
        Self::default()
    }

    fn candidate_paths() -> Vec<PathBuf> {
        let mut v = vec![PathBuf::from("mcp-config.toml")];
        if let Some(home) = dirs::home_dir() {
            v.push(home.join(".symbiont").join("mcp-config.toml"));
        }
        v
    }

    pub fn from_toml_str(s: &str) -> Result<Self, String> {
        let file: RegistryFile = toml::from_str(s).map_err(|e| e.to_string())?;
        Ok(Self {
            servers: file.servers,
        })
    }

    pub fn get(&self, server: &str) -> Option<&StdioServerSpec> {
        self.servers.get(server)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_servers_and_looks_up_by_name() {
        let toml = r#"
            [servers.fs]
            command = "mcp-fs"
            args = ["--root", "/tmp"]
            env = { A = "1" }
            [servers.bare]
            command = "bare-server"
        "#;
        let reg = McpServerRegistry::from_toml_str(toml).unwrap();
        let fs = reg.get("fs").expect("fs present");
        assert_eq!(fs.command, "mcp-fs");
        assert_eq!(fs.args, vec!["--root".to_string(), "/tmp".to_string()]);
        assert_eq!(fs.env.get("A").map(String::as_str), Some("1"));
        let bare = reg.get("bare").unwrap();
        assert!(bare.args.is_empty() && bare.env.is_empty());
        assert!(reg.get("missing").is_none());
    }

    #[test]
    fn empty_toml_is_empty_registry() {
        let reg = McpServerRegistry::from_toml_str("").unwrap();
        assert!(reg.get("anything").is_none());
    }
}
