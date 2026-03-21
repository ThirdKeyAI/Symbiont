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
    #[serde(default)]
    pub command: CommandDef,
    pub output: OutputDef,
    /// Session mode configuration (for interactive CLI tools).
    pub session: Option<SessionDef>,
    /// Browser mode configuration (for headless browser sessions).
    pub browser: Option<BrowserDef>,
}

/// Tool metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolMeta {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub binary: String,
    pub description: String,
    /// Execution mode: "oneshot" (default), "session", or "browser".
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    #[serde(default = "default_risk_tier")]
    pub risk_tier: String,
    #[serde(default)]
    pub human_approval: bool,
    pub cedar: Option<CedarMeta>,
    pub evidence: Option<EvidenceMeta>,
}

fn default_mode() -> String {
    "oneshot".to_string()
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

/// Command construction definition (oneshot mode).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

// ---- Session Mode Types ----

/// Session mode configuration for interactive CLI tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDef {
    /// Command to start the tool process.
    pub startup_command: String,
    /// Regex pattern matching the tool's ready prompt.
    pub ready_pattern: String,
    #[serde(default = "default_timeout")]
    pub startup_timeout_seconds: u64,
    #[serde(default = "default_session_idle")]
    pub idle_timeout_seconds: u64,
    #[serde(default = "default_session_timeout")]
    pub session_timeout_seconds: u64,
    #[serde(default = "default_max_interactions")]
    pub max_interactions: u32,
    /// Per-interaction settings.
    pub interaction: Option<SessionInteractionDef>,
    /// Allowed session commands (the allow-list).
    #[serde(default)]
    pub commands: HashMap<String, SessionCommandDef>,
}

fn default_session_idle() -> u64 {
    300
}
fn default_session_timeout() -> u64 {
    1800
}
fn default_max_interactions() -> u32 {
    100
}

/// Session interaction configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInteractionDef {
    #[serde(default)]
    pub input_sanitize: Vec<String>,
    #[serde(default = "default_output_max")]
    pub output_max_bytes: u64,
    #[serde(default = "default_output_wait")]
    pub output_wait_ms: u64,
}

fn default_output_max() -> u64 {
    1_048_576
}
fn default_output_wait() -> u64 {
    2000
}

/// A declared session command (becomes an MCP tool).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCommandDef {
    /// Regex pattern the command must match.
    pub pattern: String,
    pub description: String,
    #[serde(default = "default_risk_tier")]
    pub risk_tier: String,
    #[serde(default)]
    pub human_approval: bool,
    /// If true, extract target from command for scope checking.
    #[serde(default)]
    pub extract_target: bool,
    /// Optional command-specific args.
    #[serde(default)]
    pub args: HashMap<String, ArgDef>,
}

// ---- Browser Mode Types ----

/// Browser mode configuration for headless browser sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserDef {
    #[serde(default = "default_browser_engine")]
    pub engine: String,
    #[serde(default = "default_true")]
    pub headless: bool,
    #[serde(default = "default_timeout")]
    pub startup_timeout_seconds: u64,
    #[serde(default = "default_session_timeout")]
    pub session_timeout_seconds: u64,
    #[serde(default = "default_session_idle")]
    pub idle_timeout_seconds: u64,
    #[serde(default = "default_max_interactions")]
    pub max_interactions: u32,
    /// URL scope enforcement.
    pub scope: Option<BrowserScopeDef>,
    /// Allowed browser commands.
    #[serde(default)]
    pub commands: HashMap<String, BrowserCommandDef>,
    /// State inference configuration.
    pub state: Option<BrowserStateDef>,
}

fn default_browser_engine() -> String {
    "playwright".to_string()
}

/// Browser URL scope enforcement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserScopeDef {
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    #[serde(default)]
    pub blocked_domains: Vec<String>,
    #[serde(default)]
    pub allow_external: bool,
}

/// A declared browser command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserCommandDef {
    pub description: String,
    #[serde(default = "default_risk_tier")]
    pub risk_tier: String,
    #[serde(default)]
    pub human_approval: bool,
    /// Command-specific args.
    #[serde(default)]
    pub args: HashMap<String, ArgDef>,
}

/// Browser state inference fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserStateDef {
    #[serde(default)]
    pub fields: Vec<String>,
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
        assert_eq!(manifest.tool.mode, "oneshot");
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
