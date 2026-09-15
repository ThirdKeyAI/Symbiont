//! Registry mapping MCP server names (referenced from ToolClad `[mcp]`
//! manifests) to a stdio launch spec. Loaded from `./mcp-config.toml`
//! (per-project) or `~/.symbiont/mcp-config.toml` (user default).

use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct StdioServerSpec {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// HTTPS discovery URL for the server's SchemaPin public key (PEM).
    /// Enforcement requires this URL or an operator-provisioned public key.
    /// Missing trust configuration blocks invocation.
    #[serde(default)]
    pub public_key_url: Option<String>,
    /// Optional operator-provisioned public trust anchor for offline verification.
    /// It is never taken from the server's response or passed to its environment.
    #[serde(default)]
    pub public_key_pem: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryFile {
    #[serde(default)]
    servers: HashMap<String, StdioServerSpec>,
}

#[derive(Debug, Clone, Default)]
pub struct McpServerRegistry {
    servers: HashMap<String, StdioServerSpec>,
}

impl McpServerRegistry {
    /// Use the project registry, or the user registry if no project registry
    /// exists. An invalid project registry must not fall back to a different
    /// server configuration with potentially broader authority.
    pub fn load() -> Result<Self, String> {
        Self::load_from_paths(&Self::candidate_paths())
    }

    fn load_from_paths(paths: &[PathBuf]) -> Result<Self, String> {
        for path in paths {
            match std::fs::symlink_metadata(path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(format!(
                        "Cannot inspect MCP registry {}: {error}",
                        path.display()
                    ))
                }
                Ok(_) => {}
            }
            let contents = std::fs::read_to_string(path)
                .map_err(|error| format!("Cannot read MCP registry {}: {error}", path.display()))?;
            return Self::from_toml_str(&contents)
                .map_err(|error| format!("Invalid MCP registry {}: {error}", path.display()));
        }
        Ok(Self::default())
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

    #[test]
    fn invalid_project_registry_cannot_fall_back_to_user_servers() {
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("project.toml");
        let fallback = dir.path().join("user.toml");
        std::fs::write(&fallback, "[servers.fallback]\ncommand = 'fixture'").unwrap();
        let paths = [local.clone(), fallback];
        assert!(McpServerRegistry::load_from_paths(&paths)
            .unwrap()
            .get("fallback")
            .is_some());
        for content in [
            "not valid TOML",
            "[servers.local]\ncommand = 'fixture'\npublic_key_urll = 'typo'",
        ] {
            std::fs::write(&local, content).unwrap();
            assert!(McpServerRegistry::load_from_paths(&paths).is_err());
        }
        std::fs::write(&local, "[servers.local]\ncommand = 'fixture'").unwrap();
        let registry = McpServerRegistry::load_from_paths(&paths).unwrap();
        assert!(registry.get("local").is_some());
        assert!(registry.get("fallback").is_none());
        std::fs::remove_file(&local).unwrap();
        std::fs::create_dir(&local).unwrap();
        assert!(McpServerRegistry::load_from_paths(&paths).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn dangling_registry_symlink_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp-config.toml");
        std::os::unix::fs::symlink(dir.path().join("missing"), &path).unwrap();
        assert!(McpServerRegistry::load_from_paths(&[path]).is_err());
    }
}
