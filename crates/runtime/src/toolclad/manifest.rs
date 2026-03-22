//! ToolClad manifest parsing
//!
//! Parses `.clad.toml` files into typed `Manifest` structs.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// HTTP backend configuration for oneshot API tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpDef {
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    pub body_template: Option<String>,
    #[serde(default)]
    pub success_status: Vec<u16>,
    #[serde(default)]
    pub error_status: Vec<u16>,
}

/// MCP proxy backend configuration for governed MCP tool passthrough.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpProxyDef {
    /// Named MCP server connection (from symbiont.toml).
    pub server: String,
    /// Upstream MCP tool name to invoke.
    pub tool: String,
    /// Field mapping from manifest args to upstream tool args.
    #[serde(default)]
    pub field_map: HashMap<String, String>,
}

/// A parsed ToolClad manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub tool: ToolMeta,
    #[serde(default)]
    pub args: HashMap<String, ArgDef>,
    #[serde(default)]
    pub command: CommandDef,
    pub output: OutputDef,
    /// HTTP backend configuration (oneshot API tools).
    pub http: Option<HttpDef>,
    /// MCP proxy backend (for governed MCP tool passthrough).
    pub mcp: Option<McpProxyDef>,
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

/// Browser mode configuration for headless or live browser sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserDef {
    #[serde(default = "default_browser_engine")]
    pub engine: String,
    #[serde(default = "default_true")]
    pub headless: bool,
    /// "launch" (spawn headless) or "live" (attach to running Chrome).
    #[serde(default = "default_connect")]
    pub connect: String,
    /// "accessibility_tree" | "html" | "text" — default content extraction mode.
    #[serde(default = "default_extract_mode")]
    pub extract_mode: String,
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
    "cdp".to_string()
}
fn default_connect() -> String {
    "launch".to_string()
}
fn default_extract_mode() -> String {
    "accessibility_tree".to_string()
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

/// Load custom type definitions from `toolclad.toml` at the project root.
///
/// The file uses `[types.*]` sections where each entry has a `base` field
/// (mapped to `type_name`) and other `ArgDef` fields. For example:
///
/// ```toml
/// [types.service_protocol]
/// base = "enum"
/// allowed = ["ssh", "ftp", "http"]
/// ```
pub fn load_custom_types(project_dir: &Path) -> HashMap<String, ArgDef> {
    let path = project_dir.join("toolclad.toml");
    if !path.exists() {
        return HashMap::new();
    }
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("  Warning: failed to read {}: {}", path.display(), e);
            return HashMap::new();
        }
    };
    let table: toml::Value = match toml::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("  Warning: failed to parse {}: {}", path.display(), e);
            return HashMap::new();
        }
    };
    let types_table = match table.get("types").and_then(|t| t.as_table()) {
        Some(t) => t,
        None => return HashMap::new(),
    };
    let mut result = HashMap::new();
    for (name, value) in types_table {
        let tbl = match value.as_table() {
            Some(t) => t,
            None => continue,
        };
        let base = match tbl.get("base").and_then(|b| b.as_str()) {
            Some(b) => b.to_string(),
            None => {
                eprintln!(
                    "  Warning: custom type '{}' missing 'base' field, skipping",
                    name
                );
                continue;
            }
        };
        // Build an ArgDef from the table, using `base` as the type_name
        let allowed = tbl.get("allowed").and_then(|a| {
            a.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
        });
        let pattern = tbl
            .get("pattern")
            .and_then(|p| p.as_str())
            .map(String::from);
        let min = tbl.get("min").and_then(|v| v.as_integer());
        let max = tbl.get("max").and_then(|v| v.as_integer());
        let clamp = tbl.get("clamp").and_then(|v| v.as_bool()).unwrap_or(false);
        let schemes = tbl.get("schemes").and_then(|s| {
            s.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
        });
        let scope_check = tbl
            .get("scope_check")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let description = tbl
            .get("description")
            .and_then(|d| d.as_str())
            .unwrap_or("")
            .to_string();

        result.insert(
            name.clone(),
            ArgDef {
                position: 0,
                required: false,
                type_name: base,
                description,
                allowed,
                default: None,
                pattern,
                sanitize: None,
                min,
                max,
                clamp,
                schemes,
                scope_check,
            },
        );
    }
    result
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
        assert!(manifest.mcp.is_none());
        assert!(manifest.http.is_none());
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

    #[test]
    fn test_parse_mcp_proxy_manifest() {
        let toml_str = r#"
[tool]
name = "governed_search"
version = "1.0.0"
description = "Search via governed MCP proxy"
mode = "oneshot"

[tool.cedar]
resource = "Tool::Search"
action = "execute_search"

[args.query]
position = 1
required = true
type = "string"
description = "Search query"

[args.max_results]
position = 2
required = false
type = "integer"
description = "Maximum results to return"
default = 10

[mcp]
server = "brave-search"
tool = "brave_web_search"

[mcp.field_map]
query = "q"
max_results = "count"

[output]
format = "json"

[output.schema]
type = "object"
"#;
        let manifest: Manifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.tool.name, "governed_search");
        let mcp = manifest.mcp.as_ref().unwrap();
        assert_eq!(mcp.server, "brave-search");
        assert_eq!(mcp.tool, "brave_web_search");
        assert_eq!(mcp.field_map.get("query").unwrap(), "q");
        assert_eq!(mcp.field_map.get("max_results").unwrap(), "count");
    }

    #[test]
    fn test_parse_mcp_proxy_no_field_map() {
        let toml_str = r#"
[tool]
name = "passthrough_tool"
version = "1.0.0"
description = "Direct passthrough to MCP tool"

[args.input]
position = 1
required = true
type = "string"
description = "Input value"

[mcp]
server = "my-server"
tool = "upstream_tool"

[output]
format = "json"

[output.schema]
type = "object"
"#;
        let manifest: Manifest = toml::from_str(toml_str).unwrap();
        let mcp = manifest.mcp.as_ref().unwrap();
        assert_eq!(mcp.server, "my-server");
        assert_eq!(mcp.tool, "upstream_tool");
        assert!(mcp.field_map.is_empty());
    }
}
