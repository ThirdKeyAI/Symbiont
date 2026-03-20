//! ToolClad manifest parsing
//!
//! Parses `.clad.toml` files into typed `Manifest` structs.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// A parsed ToolClad manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub tool: ToolMeta,
    #[serde(default)]
    pub args: HashMap<String, ArgDef>,
    pub command: CommandDef,
    pub output: OutputDef,
}

/// Tool metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolMeta {
    pub name: String,
    pub version: String,
    pub binary: String,
    pub description: String,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    #[serde(default = "default_risk_tier")]
    pub risk_tier: String,
    #[serde(default)]
    pub human_approval: bool,
    pub cedar: Option<CedarMeta>,
    pub evidence: Option<EvidenceMeta>,
}

fn default_timeout() -> u64 {
    30
}
fn default_risk_tier() -> String {
    "low".to_string()
}

/// Cedar policy metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CedarMeta {
    pub resource: String,
    pub action: String,
}

/// Evidence capture configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceMeta {
    pub output_dir: String,
    #[serde(default = "default_true")]
    pub capture: bool,
    #[serde(default = "default_hash")]
    pub hash: String,
}

fn default_true() -> bool {
    true
}
fn default_hash() -> String {
    "sha256".to_string()
}

/// Argument definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArgDef {
    pub position: u32,
    #[serde(default)]
    pub required: bool,
    #[serde(rename = "type")]
    pub type_name: String,
    #[serde(default)]
    pub description: String,
    pub allowed: Option<Vec<String>>,
    pub default: Option<toml::Value>,
    pub pattern: Option<String>,
    pub sanitize: Option<Vec<String>>,
    pub min: Option<i64>,
    pub max: Option<i64>,
    #[serde(default)]
    pub clamp: bool,
    pub schemes: Option<Vec<String>>,
    #[serde(default)]
    pub scope_check: bool,
}

/// Command construction definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandDef {
    pub template: Option<String>,
    pub executor: Option<String>,
    #[serde(default)]
    pub defaults: HashMap<String, toml::Value>,
    #[serde(default)]
    pub mappings: HashMap<String, HashMap<String, String>>,
    #[serde(default)]
    pub conditionals: HashMap<String, ConditionalDef>,
}

/// Conditional command fragment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConditionalDef {
    pub when: String,
    pub template: String,
}

/// Output configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputDef {
    pub format: String,
    pub parser: Option<String>,
    #[serde(default = "default_true")]
    pub envelope: bool,
    #[serde(default)]
    pub schema: serde_json::Value,
}

/// Load a single manifest from a `.clad.toml` file.
pub fn load_manifest(path: &Path) -> Result<Manifest, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    toml::from_str(&content).map_err(|e| format!("Failed to parse {}: {}", path.display(), e))
}

/// Load all manifests from a directory.
pub fn load_manifests_from_dir(dir: &Path) -> Vec<(String, Manifest)> {
    let mut manifests = Vec::new();
    if !dir.exists() || !dir.is_dir() {
        return manifests;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "toml").unwrap_or(false)
                && path
                    .file_name()
                    .map(|n| n.to_string_lossy().ends_with(".clad.toml"))
                    .unwrap_or(false)
            {
                match load_manifest(&path) {
                    Ok(manifest) => {
                        let name = manifest.tool.name.clone();
                        manifests.push((name, manifest));
                    }
                    Err(e) => {
                        eprintln!("  ⚠ Failed to load {}: {}", path.display(), e);
                    }
                }
            }
        }
    }
    manifests
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_manifest() {
        let toml_str = r#"
[tool]
name = "test_tool"
version = "1.0.0"
binary = "echo"
description = "A test tool"

[args.message]
position = 1
required = true
type = "string"
description = "Message to echo"

[command]
template = "echo {message}"

[output]
format = "text"

[output.schema]
type = "object"
"#;
        let manifest: Manifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.tool.name, "test_tool");
        assert_eq!(manifest.tool.binary, "echo");
        assert!(manifest.args.contains_key("message"));
        assert_eq!(manifest.args["message"].type_name, "string");
        assert_eq!(
            manifest.command.template,
            Some("echo {message}".to_string())
        );
    }

    #[test]
    fn test_parse_manifest_with_mappings() {
        let toml_str = r#"
[tool]
name = "nmap"
version = "1.0.0"
binary = "nmap"
description = "Scanner"

[args.target]
position = 1
required = true
type = "scope_target"

[args.scan_type]
position = 2
required = true
type = "enum"
allowed = ["ping", "service"]

[command]
template = "nmap {_scan_flags} {target}"

[command.mappings.scan_type]
ping = "-sn"
service = "-sT -sV"

[output]
format = "text"

[output.schema]
type = "object"
"#;
        let manifest: Manifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.command.mappings["scan_type"]["ping"], "-sn");
    }
}
