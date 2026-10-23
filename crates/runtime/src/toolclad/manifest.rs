//! ToolClad manifest parsing and load-time validation.
//!
//! A manifest is a security contract, so an unrecognized key is a load error,
//! never a comment: every struct here denies unknown fields. The accepted key
//! set is the ToolClad reference allowlist plus Symbiont's extensions
//! (`[source]`, `[filesystem]`, `feeds_decision`, `finalize`,
//! `[browser.network]`, `[session] sentinel_command`, and the `literal_text` /
//! `agent_summary` argument types). A control Symbiont cannot honor makes the
//! manifest fail to load instead of degrading to "off".

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use super::command;
use super::validator;

/// HTTP backend configuration for oneshot API tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct McpProxyDef {
    /// Named MCP server connection (from the project MCP registry).
    pub server: String,
    /// Upstream MCP tool name to invoke.
    pub tool: String,
    /// Field mapping from manifest args to upstream tool args.
    #[serde(default)]
    pub field_map: HashMap<String, String>,
}

/// A parsed ToolClad manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub tool: ToolMeta,
    /// Per-operation file capabilities, enforced by the embedding runtime.
    pub filesystem: Option<crate::sandbox::files::FileAccess>,
    /// Fixed read-only source broker; mutually exclusive with executable backends.
    pub source: Option<crate::sandbox::source::SourceQuery>,
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
#[serde(deny_unknown_fields)]
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
    /// ToolClad dispatch mode. Symbiont executes every manifest it loads, so
    /// only `"exec"` is accepted; validator-only `"callback"` is refused.
    #[serde(default = "default_dispatch")]
    pub dispatch: String,
    /// The tool reaches the network from its arguments. Makes
    /// `block_internal` mandatory on every network-target argument and, for
    /// sessions, is the only way to get sandbox network access.
    #[serde(default)]
    pub network_egress: bool,
    /// Permit an interpreter (`sh`, `python`, `env`, ...) as `argv[0]`.
    #[serde(default)]
    pub allow_interpreter: bool,
    pub cedar: Option<CedarMeta>,
    pub evidence: Option<EvidenceMeta>,
    /// Resource limits, enforced by tightening the selected sandbox boundary.
    pub limits: Option<ToolLimits>,
}

fn default_mode() -> String {
    "oneshot".to_string()
}
fn default_dispatch() -> String {
    "exec".to_string()
}
fn default_timeout() -> u64 {
    30
}
fn default_risk_tier() -> String {
    "low".to_string()
}

/// `[tool.limits]`. Each declared limit is applied to the selected boundary;
/// a boundary that cannot express a declared limit refuses the call.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolLimits {
    pub nproc: Option<u64>,
    pub nofile: Option<u64>,
    pub fsize_bytes: Option<u64>,
    pub address_space_bytes: Option<u64>,
    pub cpu_seconds: Option<u64>,
}

/// Cedar policy metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CedarMeta {
    pub resource: String,
    pub action: String,
}

/// Evidence capture configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceMeta {
    /// Per-call directory under the evidence root. Only `{evidence_dir}` and
    /// `{scan_id}` may appear, and it must start with `{evidence_dir}/`.
    #[serde(default = "default_evidence_dir")]
    pub output_dir: String,
    #[serde(default = "default_true")]
    pub capture: bool,
    #[serde(default = "default_hash")]
    pub hash: String,
    /// Browser screenshots; accepted for format compatibility (the browser
    /// backend is not executable).
    #[serde(default)]
    pub screenshots: bool,
}

fn default_evidence_dir() -> String {
    "{evidence_dir}/{scan_id}".to_string()
}
fn default_true() -> bool {
    true
}
fn default_hash() -> String {
    "sha256".to_string()
}

/// Argument definition.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArgDef {
    #[serde(default)]
    pub position: u32,
    #[serde(default)]
    pub required: bool,
    #[serde(rename = "type")]
    pub type_name: String,
    #[serde(default)]
    pub description: String,
    pub allowed: Option<Vec<String>>,
    pub default: Option<toml::Value>,
    /// Anchored: compiled as `^(?:pattern)$`.
    pub pattern: Option<String>,
    /// Legacy opt-in to the shell-metacharacter blocklist (always applied
    /// to string-derived types). Only `"injection"` is accepted.
    pub sanitize: Option<Vec<String>>,
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub min_float: Option<f64>,
    pub max_float: Option<f64>,
    #[serde(default)]
    pub clamp: bool,
    /// `url` scheme allowlist; defaults to http/https.
    pub schemes: Option<Vec<String>>,
    #[serde(default)]
    pub scope_check: bool,
    /// Refuse loopback, private and other non-public targets, resolving a
    /// hostname and checking every answer (failure is a refusal).
    #[serde(default)]
    pub block_internal: bool,
    /// Permit a value starting with `-` or `+`.
    #[serde(default)]
    pub allow_leading_dash: bool,
    /// Permit U+0020 inside the value (declared `exec` form only).
    #[serde(default)]
    pub allow_whitespace: bool,
    /// True if this argument's value feeds a privileged downstream decision
    /// (routing, escalation, authorization). Free-text args with this flag
    /// set are flagged by the ToolClad lint — use an enum + Cedar grounding
    /// (see `crate::toolclad::decision`).
    #[serde(default)]
    pub feeds_decision: bool,
}

/// Command construction definition (oneshot mode).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandDef {
    /// Canonical argv form; each element is exactly one argv entry.
    pub exec: Option<Vec<String>>,
    /// Deprecated string form, tokenized into `exec` semantics at load.
    pub template: Option<String>,
    pub executor: Option<String>,
    #[serde(default)]
    pub defaults: HashMap<String, toml::Value>,
    #[serde(default)]
    pub mappings: HashMap<String, HashMap<String, String>>,
    #[serde(default)]
    pub conditionals: HashMap<String, ConditionalDef>,
    /// Only `"--"`: inserted before the first agent-controlled argv entry.
    pub end_of_options: Option<String>,
}

/// Conditional command fragment.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConditionalDef {
    pub when: String,
    pub template: String,
}

/// Output configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputDef {
    pub format: String,
    pub parser: Option<String>,
    #[serde(default = "default_true")]
    pub envelope: bool,
    #[serde(default)]
    pub schema: serde_json::Value,
    /// Per-stream capture cap (default 8 MiB, at most 1 GiB).
    #[serde(default = "default_max_bytes")]
    pub max_bytes: u64,
}

pub const DEFAULT_OUTPUT_MAX_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_OUTPUT_BYTES: u64 = 1_073_741_824;

fn default_max_bytes() -> u64 {
    DEFAULT_OUTPUT_MAX_BYTES
}

// ---- Session Mode Types ----

/// Session mode configuration for interactive CLI tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionDef {
    /// Command to start the tool process.
    pub startup_command: String,
    /// Regex pattern matching the tool's ready prompt.
    pub ready_pattern: String,
    /// Symbiont extension: a command line, containing `{sentinel}`, that makes
    /// the far side echo a fresh per-interaction sentinel (for example
    /// `\echo {sentinel}` for psql). The prompt is accepted only at the start
    /// of a line after the sentinel line. psql gets this by default.
    pub sentinel_command: Option<String>,
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
    /// Accepted by the format but refused at load: a child session may never
    /// be registered from tool output (ToolClad S7).
    pub child_detection: Option<toml::Value>,
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
#[serde(deny_unknown_fields)]
pub struct SessionInteractionDef {
    /// Sink-specific input policies: `injection` (shell-metacharacter
    /// blocklist), `psql_meta` (no backslash meta-commands), `msf_meta`
    /// (console verb denylist). Unknown values are a load error.
    #[serde(default)]
    pub input_sanitize: Vec<String>,
    #[serde(default = "default_output_max")]
    pub output_max_bytes: u64,
    #[serde(default = "default_output_wait")]
    pub output_wait_ms: u64,
}

/// Accepted `[session.interaction] input_sanitize` values.
pub const SESSION_SANITIZERS: &[&str] = &["injection", "psql_meta", "msf_meta"];

fn default_output_max() -> u64 {
    1_048_576
}
fn default_output_wait() -> u64 {
    2000
}

/// A declared session command (becomes an MCP tool).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionCommandDef {
    /// Regex pattern the command must match (anchored).
    pub pattern: String,
    pub description: String,
    #[serde(default = "default_risk_tier")]
    pub risk_tier: String,
    #[serde(default)]
    pub human_approval: bool,
    /// Close the worker after its response and publish declared new output.
    #[serde(default)]
    pub finalize: bool,
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
#[serde(deny_unknown_fields)]
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
    /// Explicit request capabilities, separate from domain matching.
    pub network: Option<BrowserNetworkDef>,
    /// Allowed browser commands.
    #[serde(default)]
    pub commands: HashMap<String, BrowserCommandDef>,
    /// State inference configuration.
    pub state: Option<BrowserStateDef>,
    /// ToolClad `[browser.defaults]`.
    pub defaults: Option<BrowserDefaultsDef>,
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

/// ToolClad `[browser.defaults]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserDefaultsDef {
    pub extract_mode: Option<String>,
}

/// Browser request capabilities. Private destinations require exact literal
/// origins; a domain allowlist alone does not authorize private network access.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserNetworkDef {
    #[serde(default = "default_browser_methods")]
    pub allowed_methods: Vec<String>,
    #[serde(default)]
    pub private_origins: Vec<String>,
}

fn default_browser_methods() -> Vec<String> {
    vec!["GET".into(), "HEAD".into()]
}

impl Default for BrowserNetworkDef {
    fn default() -> Self {
        Self {
            allowed_methods: default_browser_methods(),
            private_origins: Vec::new(),
        }
    }
}

/// Browser URL scope enforcement.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct BrowserStateDef {
    #[serde(default)]
    pub fields: Vec<String>,
}

// ---- Load-time policy tables (mirrors the ToolClad reference) ----

/// Binaries that connect to a destination derived from their arguments. The
/// ToolClad reference list plus `dig` and `whois`, which the reference omits:
/// both emit network traffic selected by their argument, so a target they
/// receive gets the same egress policy.
pub const EGRESS_BINARIES: &[&str] = &[
    "curl", "wget", "nmap", "nc", "ncat", "netcat", "masscan", "telnet", "ssh", "ping", "fping",
    "httpie", "http", "wfuzz", "ffuf", "hydra", "nikto", "dig", "whois",
];

/// Session binaries whose commands can name a network destination.
pub const NETWORK_SESSION_BINARIES: &[&str] = &["msfconsole"];

/// Argument types that name a network destination.
pub const NETWORK_TARGET_TYPES: &[&str] = &["scope_target", "url", "ip_address", "cidr"];

/// Refused as `argv[0]` unless `[tool] allow_interpreter = true`.
pub const INTERPRETERS: &[&str] = &[
    "sh", "bash", "zsh", "dash", "ksh", "csh", "tcsh", "fish", "ash", "busybox", "env", "eval",
    "xargs", "python", "python2", "python3", "perl", "ruby", "node", "deno", "bun", "lua", "php",
    "awk",
];

pub const RISK_TIERS: &[&str] = &["info", "low", "medium", "high", "critical"];

pub const BUILTIN_PARSERS: &[&str] = &[
    "builtin:json",
    "builtin:jsonl",
    "builtin:csv",
    "builtin:xml",
    "builtin:text",
];

pub(crate) fn basename(s: &str) -> String {
    s.rsplit(['/', '\\'])
        .next()
        .unwrap_or(s)
        .to_ascii_lowercase()
}

/// Rank of a risk tier (unknown sorts highest so it is never treated as low).
pub fn tier_rank(tier: &str) -> u8 {
    match tier {
        "info" => 0,
        "low" => 1,
        "medium" => 2,
        "high" => 3,
        _ => 4,
    }
}

impl Manifest {
    pub(crate) fn validate_source_backend(&self) -> Result<(), String> {
        if self.source.is_some()
            && (self.tool.mode != "oneshot"
                || self.http.is_some()
                || self.mcp.is_some()
                || self.session.is_some()
                || self.browser.is_some()
                || self.filesystem.is_some()
                || !self.tool.binary.is_empty()
                || self.command.template.is_some()
                || self.command.exec.is_some()
                || self.command.executor.is_some()
                || !self.command.defaults.is_empty()
                || !self.command.mappings.is_empty()
                || !self.command.conditionals.is_empty()
                || self.output.format != "json"
                || self.output.parser.is_some())
        {
            return Err("source queries require an exclusive oneshot source backend with JSON output and no parser".into());
        }
        Ok(())
    }

    /// Every argument with its reporting name: top-level args, then session
    /// and browser command args by key path, sorted for stable errors.
    pub fn all_arg_defs(&self) -> Vec<(String, &ArgDef)> {
        let mut out: Vec<(String, &ArgDef)> =
            self.args.iter().map(|(n, d)| (n.clone(), d)).collect();
        if let Some(session) = &self.session {
            for (cmd, def) in &session.commands {
                for (n, d) in &def.args {
                    out.push((format!("session.commands.{cmd}.args.{n}"), d));
                }
            }
        }
        if let Some(browser) = &self.browser {
            for (cmd, def) in &browser.commands {
                for (n, d) in &def.args {
                    out.push((format!("browser.commands.{cmd}.args.{n}"), d));
                }
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// The executable a session starts, lowercased basename.
    pub fn session_binary(&self) -> Option<String> {
        let session = self.session.as_ref()?;
        let argv = shlex::split(&session.startup_command)?;
        argv.first().map(|a| basename(a))
    }

    /// True if the command dispatches a known egress binary.
    fn invokes_egress_binary(&self) -> bool {
        let mut names = Vec::new();
        if !self.tool.binary.is_empty() {
            names.push(basename(&self.tool.binary));
        }
        if let Ok(Some(exec)) = command::compiled_exec(self) {
            if let Some(first) = exec.first() {
                names.push(basename(first));
            }
        }
        if let Some(bin) = self.session_binary() {
            names.push(bin);
        }
        names.iter().any(|n| EGRESS_BINARIES.contains(&n.as_str()))
    }

    /// The manifest reaches the network from its arguments.
    pub fn is_egress(&self) -> bool {
        self.tool.network_egress
            || self.http.is_some()
            || self.browser.is_some()
            || self.invokes_egress_binary()
    }

    /// A session whose commands can name a network destination.
    pub fn is_network_session(&self) -> bool {
        self.session_binary().is_some_and(|b| {
            EGRESS_BINARIES.contains(&b.as_str()) || NETWORK_SESSION_BINARIES.contains(&b.as_str())
        })
    }

    /// The effective risk tier of a call: the manifest tier raised to a
    /// session or browser command's own tier.
    pub fn effective_risk_tier(&self, command: Option<&str>) -> String {
        let command_tier = command.and_then(|c| {
            self.session
                .as_ref()
                .and_then(|s| s.commands.get(c))
                .map(|d| d.risk_tier.clone())
                .or_else(|| {
                    self.browser
                        .as_ref()
                        .and_then(|b| b.commands.get(c))
                        .map(|d| d.risk_tier.clone())
                })
        });
        match command_tier {
            Some(t) if tier_rank(&t) > tier_rank(&self.tool.risk_tier) => t,
            _ => self.tool.risk_tier.clone(),
        }
    }

    /// Full load-time validation. Every loader and every executor runs this,
    /// so a manifest constructed in code gets the same checks as one read
    /// from disk.
    pub fn validate(&self, custom_types: &HashMap<String, ArgDef>) -> Result<(), String> {
        self.validate_source_backend()?;
        if !matches!(self.tool.mode.as_str(), "oneshot" | "session" | "browser") {
            return Err(format!(
                "invalid mode '{}', must be one of: oneshot, session, browser",
                self.tool.mode
            ));
        }
        if self.tool.dispatch != "exec" {
            return Err(format!(
                "dispatch '{}' is not supported: Symbiont executes every manifest it loads (use dispatch = \"exec\")",
                self.tool.dispatch
            ));
        }
        if self.tool.timeout_seconds == 0 {
            return Err("timeout_seconds must be greater than zero".into());
        }
        if !RISK_TIERS.contains(&self.tool.risk_tier.as_str()) {
            return Err(format!(
                "invalid risk_tier '{}', must be one of: {}",
                self.tool.risk_tier,
                RISK_TIERS.join(", ")
            ));
        }
        self.check_argv0()?;
        self.check_output()?;
        self.check_evidence()?;
        self.check_limits()?;
        self.check_http()?;
        self.check_args(custom_types)?;
        self.egress_lint(custom_types)?;
        self.check_mappings()?;
        command::check_command(self)?;
        self.check_session()?;
        self.check_browser()?;
        Ok(())
    }

    fn check_argv0(&self) -> Result<(), String> {
        let interpreter = |argv0: &str| -> Result<(), String> {
            if !self.tool.allow_interpreter && INTERPRETERS.contains(&basename(argv0).as_str()) {
                return Err(format!(
                    "command argv[0] '{argv0}' is an interpreter; set allow_interpreter = true to permit it"
                ));
            }
            Ok(())
        };
        if let Some(exe) = &self.command.executor {
            return interpreter(exe);
        }
        let Some(exec) = command::compiled_exec(self)? else {
            return Ok(());
        };
        let Some(argv0) = exec.first() else {
            return Ok(());
        };
        interpreter(argv0)?;
        let binary = &self.tool.binary;
        if !binary.is_empty()
            && argv0.rsplit('/').next().unwrap_or(argv0)
                != binary.rsplit('/').next().unwrap_or(binary)
        {
            return Err(format!(
                "command argv[0] '{argv0}' does not match tool.binary '{binary}'"
            ));
        }
        Ok(())
    }

    fn check_output(&self) -> Result<(), String> {
        if let Some(parser) = &self.output.parser {
            if !BUILTIN_PARSERS.contains(&parser.as_str()) {
                // Symbiont keeps operator-allowlisted external parsers, which
                // run inside the command boundary. Anything else is refused at
                // load rather than ignored.
                let path = parser.strip_prefix("custom:").map(str::trim).ok_or_else(|| {
                    format!(
                        "unsupported output parser '{parser}': use one of {} or custom:<absolute path> \
                         listed in SYMBIONT_TOOLCLAD_ALLOWED_PARSERS",
                        BUILTIN_PARSERS.join(", ")
                    )
                })?;
                if !Path::new(path).is_absolute() {
                    return Err(format!(
                        "custom output parser '{path}' must be an absolute path"
                    ));
                }
            }
        }
        if self.output.max_bytes == 0 || self.output.max_bytes > MAX_OUTPUT_BYTES {
            return Err(format!(
                "output.max_bytes must be an integer from 1 to {MAX_OUTPUT_BYTES} (got {})",
                self.output.max_bytes
            ));
        }
        Ok(())
    }

    fn check_evidence(&self) -> Result<(), String> {
        let Some(evidence) = &self.tool.evidence else {
            return Ok(());
        };
        if evidence.hash != "sha256" {
            return Err(format!(
                "tool.evidence.hash '{}' is not supported (use sha256)",
                evidence.hash
            ));
        }
        super::evidence::relative_dir(&evidence.output_dir, "check").map(|_| ())
    }

    fn check_limits(&self) -> Result<(), String> {
        if let Some(limits) = &self.tool.limits {
            for (name, value) in [
                ("nproc", limits.nproc),
                ("nofile", limits.nofile),
                ("fsize_bytes", limits.fsize_bytes),
                ("address_space_bytes", limits.address_space_bytes),
                ("cpu_seconds", limits.cpu_seconds),
            ] {
                if value == Some(0) {
                    return Err(format!("tool.limits.{name} must be greater than zero"));
                }
            }
        }
        Ok(())
    }

    fn check_http(&self) -> Result<(), String> {
        let Some(http) = &self.http else {
            return Ok(());
        };
        let host = super::executor::http_url_literal_host(&http.url)?;
        for name in http.headers.keys() {
            super::executor::check_header_name(name)?;
        }
        static ARG_PLACEHOLDER: std::sync::LazyLock<regex::Regex> =
            std::sync::LazyLock::new(|| {
                regex::Regex::new(r"\{([a-zA-Z0-9_]+)\}").expect("pattern")
            });
        for template in std::iter::once(&http.url)
            .chain(http.headers.values())
            .chain(http.body_template.iter())
        {
            for capture in ARG_PLACEHOLDER.captures_iter(template) {
                if !self.args.contains_key(&capture[1]) {
                    return Err(format!(
                        "unresolved placeholder '{{{}}}' in [http]",
                        &capture[1]
                    ));
                }
            }
        }
        let block_internal = self.all_arg_defs().iter().any(|(_, d)| d.block_internal);
        let bare = host
            .strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
            .unwrap_or(&host);
        if let Some(ip) = super::hostpolicy::parse_canonical_ip(bare) {
            super::hostpolicy::enforce_ip_policy("http.url", &ip, block_internal)?;
        } else if block_internal {
            if super::hostpolicy::is_internal_name(bare) {
                return Err(format!(
                    "http.url host '{bare}' is an internal name (blocked by block_internal policy)"
                ));
            }
            if let Some(ip) = super::hostpolicy::embedded_ip_literal(bare) {
                super::hostpolicy::enforce_ip_policy("http.url", &std::net::IpAddr::V4(ip), true)?;
            }
        }
        Ok(())
    }

    fn check_args(&self, custom_types: &HashMap<String, ArgDef>) -> Result<(), String> {
        for (name, def) in self.all_arg_defs() {
            let base = validator::base_type(def, Some(custom_types))
                .map_err(|e| format!("argument '{name}': {e}"))?;
            let resolved = validator::resolve(def, Some(custom_types))
                .map_err(|e| format!("argument '{name}': {e}"))?;
            if base == "enum" && resolved.allowed.is_none() {
                return Err(format!(
                    "argument '{name}' is type 'enum' but has no 'allowed' list"
                ));
            }
            if def.allowed.is_some() && base != "enum" {
                return Err(format!(
                    "argument '{name}' sets 'allowed' but is type '{}'; only enum honors it",
                    def.type_name
                ));
            }
            if base == "regex_match" && resolved.pattern.is_none() {
                return Err(format!(
                    "argument '{name}' is type 'regex_match' but has no 'pattern'"
                ));
            }
            if let Some(pattern) = &resolved.pattern {
                regex::Regex::new(&validator::anchor_pattern(pattern))
                    .map_err(|e| format!("argument '{name}' has an invalid pattern: {e}"))?;
            }
            if def.scope_check && !NETWORK_TARGET_TYPES.contains(&base.as_str()) {
                return Err(format!(
                    "argument '{name}' scope_check is not supported for type '{}'",
                    def.type_name
                ));
            }
            if def.block_internal && !NETWORK_TARGET_TYPES.contains(&base.as_str()) {
                return Err(format!(
                    "argument '{name}' block_internal is not supported for type '{}'",
                    def.type_name
                ));
            }
            if let Some(sanitize) = &def.sanitize {
                if let Some(bad) = sanitize.iter().find(|s| s.as_str() != "injection") {
                    return Err(format!(
                        "argument '{name}' sanitize value '{bad}' is not supported (only \"injection\")"
                    ));
                }
            }
            if let Some(schemes) = &def.schemes {
                if base != "url" {
                    return Err(format!(
                        "argument '{name}' schemes is only supported for type 'url'"
                    ));
                }
                if schemes.is_empty() {
                    return Err(format!("argument '{name}' schemes must not be empty"));
                }
            }
            if let (Some(lo), Some(hi)) = (resolved.min, resolved.max) {
                if lo > hi {
                    return Err(format!("argument '{name}' has min greater than max"));
                }
            }
            if let (Some(lo), Some(hi)) = (resolved.min_float, resolved.max_float) {
                if !(lo.is_finite() && hi.is_finite()) || lo > hi {
                    return Err(format!(
                        "argument '{name}' has an invalid min_float/max_float"
                    ));
                }
            }
            if let Some(default) = &def.default {
                let value = command::toml_value_to_string(default);
                // Empty optional defaults mean "unset" and are not validated.
                if !(value.is_empty() && !def.required) {
                    let mut strict = def.clone();
                    strict.scope_check = false;
                    validator::validate_arg_offline(
                        &strict,
                        &value,
                        Some(custom_types),
                        command::exec_was_declared(self),
                    )
                    .map_err(|e| {
                        format!("argument '{name}' has an invalid default '{value}': {e}")
                    })?;
                }
            }
        }
        Ok(())
    }

    /// Egress safety: in an egress manifest every network-target argument
    /// must set `block_internal = true`, and a custom type over a
    /// network-target base is refused (it cannot carry the flag).
    fn egress_lint(&self, custom_types: &HashMap<String, ArgDef>) -> Result<(), String> {
        if !self.is_egress() {
            return Ok(());
        }
        for (name, def) in self.all_arg_defs() {
            if NETWORK_TARGET_TYPES.contains(&def.type_name.as_str()) {
                if !def.block_internal {
                    return Err(format!(
                        "argument '{name}' is a network target for an egress tool but block_internal is not set; \
                         set block_internal = true to default-deny internal/SSRF targets"
                    ));
                }
            } else if custom_types.contains_key(&def.type_name) {
                let base = validator::base_type(def, Some(custom_types))?;
                if NETWORK_TARGET_TYPES.contains(&base.as_str()) {
                    return Err(format!(
                        "argument '{name}' uses custom type '{}' as a network target; \
                         custom types cannot carry block_internal — use the built-in {base} type",
                        def.type_name
                    ));
                }
            }
        }
        Ok(())
    }

    fn check_mappings(&self) -> Result<(), String> {
        for (arg, table) in &self.command.mappings {
            let def = self
                .args
                .get(arg)
                .ok_or_else(|| format!("mapping references unknown argument '{arg}'"))?;
            if def.type_name == "enum" {
                if let Some(allowed) = &def.allowed {
                    let mut missing: Vec<&str> = allowed
                        .iter()
                        .filter(|v| !v.is_empty() && !table.contains_key(*v))
                        .map(String::as_str)
                        .collect();
                    missing.sort_unstable();
                    if !missing.is_empty() {
                        return Err(format!(
                            "mapping for enum argument '{arg}' does not cover allowed values: [{}] (map a value to \"\" for no flags)",
                            missing.join(", ")
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn check_session(&self) -> Result<(), String> {
        let Some(session) = &self.session else {
            if self.tool.mode == "session" {
                return Err("mode = \"session\" requires a [session] section".into());
            }
            return Ok(());
        };
        if self.tool.mode != "session" {
            return Err("a [session] section requires mode = \"session\"".into());
        }
        if session.child_detection.is_some() {
            return Err(
                "session.child_detection is not supported: a child session is never registered from tool output (ToolClad S7)"
                    .into(),
            );
        }
        regex::Regex::new(&format!(r"\A(?:{})\z", session.ready_pattern))
            .map_err(|e| format!("invalid session ready_pattern: {e}"))?;
        if let Some(sentinel) = &session.sentinel_command {
            if sentinel.matches("{sentinel}").count() != 1 || sentinel.chars().any(char::is_control)
            {
                return Err(
                    "session.sentinel_command must contain {sentinel} exactly once and no control characters"
                        .into(),
                );
            }
        }
        if let Some(interaction) = &session.interaction {
            if let Some(bad) = interaction
                .input_sanitize
                .iter()
                .find(|s| !SESSION_SANITIZERS.contains(&s.as_str()))
            {
                return Err(format!(
                    "session.interaction.input_sanitize value '{bad}' is not supported (use {})",
                    SESSION_SANITIZERS.join(", ")
                ));
            }
        }
        let network = self.is_network_session();
        for (name, command) in &session.commands {
            regex::Regex::new(&format!(r"\A(?:{})\z", command.pattern))
                .map_err(|e| format!("invalid pattern for session command '{name}': {e}"))?;
            if !RISK_TIERS.contains(&command.risk_tier.as_str()) {
                return Err(format!(
                    "session command '{name}' has invalid risk_tier '{}'",
                    command.risk_tier
                ));
            }
            if command.extract_target && !command.pattern.contains("(?P<target>") {
                return Err(format!(
                    "session command '{name}' sets extract_target but its pattern has no (?P<target>...) group"
                ));
            }
            if network
                && !command.extract_target
                && !command.human_approval
                && !self.tool.human_approval
            {
                return Err(format!(
                    "session command '{name}' runs in a network-capable session; set extract_target = true \
                     (with a (?P<target>...) group) or human_approval = true"
                ));
            }
        }
        Ok(())
    }

    fn check_browser(&self) -> Result<(), String> {
        if let Some(browser) = &self.browser {
            for (name, command) in &browser.commands {
                if !RISK_TIERS.contains(&command.risk_tier.as_str()) {
                    return Err(format!(
                        "browser command '{name}' has invalid risk_tier '{}'",
                        command.risk_tier
                    ));
                }
            }
        } else if self.tool.mode == "browser" {
            return Err("mode = \"browser\" requires a [browser] section".into());
        }
        Ok(())
    }
}

/// Authoring lints over a manifest that already validates: advice for
/// reviewers of generated or hand-written manifests, not load errors.
pub fn lint(manifest: &Manifest) -> Vec<String> {
    let mut warnings = Vec::new();
    if manifest.tool.allow_interpreter {
        warnings.push(
            "allow_interpreter = true: an interpreter turns argument values into code; \
             require human approval and review every argument"
                .to_string(),
        );
    }
    for (name, def) in manifest.all_arg_defs() {
        if def.type_name == "url" && !def.scope_check {
            warnings.push(format!(
                "argument '{name}' is a url without scope_check = true; destinations are not limited to the project scope"
            ));
        }
    }
    if manifest.command.end_of_options.is_none() {
        if let Ok(Some(tokens)) = command::compiled_exec(manifest) {
            let mut fenced = false;
            for (i, token) in tokens.iter().enumerate().skip(1) {
                if token == "--" {
                    fenced = true;
                }
                let Some(name) = token
                    .strip_prefix('{')
                    .and_then(|t| t.strip_suffix('}'))
                    .filter(|n| manifest.args.contains_key(*n))
                else {
                    continue;
                };
                let after_option = tokens[i - 1].starts_with('-') && tokens[i - 1] != "--";
                let text_type = validator::base_type(&manifest.args[name], None)
                    .map(|b| validator::STRING_DERIVED_TYPES.contains(&b.as_str()))
                    .unwrap_or(true);
                if !fenced && !after_option && text_type {
                    warnings.push(format!(
                        "argument '{name}' reaches argv as an operand without `--`; set [command] end_of_options = \"--\" if the tool honors it"
                    ));
                }
            }
        }
    }
    warnings
}

/// Parse and validate a manifest from TOML text.
pub fn parse_manifest(
    text: &str,
    custom_types: &HashMap<String, ArgDef>,
) -> Result<Manifest, String> {
    let manifest: Manifest = toml::from_str(text).map_err(|e| e.to_string())?;
    manifest.validate(custom_types)?;
    if manifest.command.template.is_some() && manifest.command.exec.is_none() {
        tracing::warn!(
            tool = %manifest.tool.name,
            "[command] template is deprecated; use exec = [...]"
        );
    }
    Ok(manifest)
}

/// Load a single manifest from a `.clad.toml` file (no project custom types).
pub fn load_manifest(path: &Path) -> Result<Manifest, String> {
    load_manifest_with_types(path, &HashMap::new())
}

/// Load a single manifest with the project's custom types.
pub fn load_manifest_with_types(
    path: &Path,
    custom_types: &HashMap<String, ArgDef>,
) -> Result<Manifest, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    parse_manifest(&content, custom_types)
        .map_err(|e| format!("Failed to load {}: {}", path.display(), e))
}

/// Load all manifests from a directory. Any manifest that fails to load fails
/// the whole directory: a tool the operator shipped must never silently
/// disappear from the runtime. A missing directory is an empty tool set.
pub fn load_manifests_from_dir(dir: &Path) -> Result<Vec<(String, Manifest)>, String> {
    let project = dir
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let custom_types = load_custom_types(project)?;
    load_manifests_from_dir_with_types(dir, &custom_types)
}

/// [`load_manifests_from_dir`] with explicit custom types.
pub fn load_manifests_from_dir_with_types(
    dir: &Path,
    custom_types: &HashMap<String, ArgDef>,
) -> Result<Vec<(String, Manifest)>, String> {
    let mut manifests: Vec<(String, Manifest)> = Vec::new();
    if !dir.is_dir() {
        return Ok(manifests);
    }
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("Cannot read {}: {e}", dir.display()))?;
    let mut paths: Vec<_> = entries
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<_, _>>()
        .map_err(|e| format!("Cannot read {}: {e}", dir.display()))?;
    paths.sort();
    let mut errors = Vec::new();
    for path in paths {
        if !path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().ends_with(".clad.toml"))
        {
            continue;
        }
        match load_manifest_with_types(&path, custom_types) {
            Ok(manifest) => {
                let name = manifest.tool.name.clone();
                if manifests.iter().any(|(n, _)| *n == name) {
                    errors.push(format!("{}: duplicate tool name '{name}'", path.display()));
                } else {
                    manifests.push((name, manifest));
                }
            }
            Err(e) => errors.push(e),
        }
    }
    if errors.is_empty() {
        Ok(manifests)
    } else {
        for error in &errors {
            tracing::error!(error = %error, "ToolClad manifest failed to load");
        }
        Err(format!(
            "ToolClad manifests failed to load: {}",
            errors.join("; ")
        ))
    }
}

/// Keys a `[types.<name>]` table in `toolclad.toml` accepts: the ToolClad
/// reference set plus Symbiont's `clamp`, `schemes`, `scope_check` and
/// `description`.
const CUSTOM_TYPE_KEYS: &[&str] = &[
    "base",
    "allowed",
    "pattern",
    "min",
    "max",
    "min_float",
    "max_float",
    "clamp",
    "schemes",
    "scope_check",
    "description",
];

/// Load custom type definitions from `toolclad.toml` at the project root.
///
/// Strict: the only top-level key is `types`, each type accepts only
/// [`CUSTOM_TYPE_KEYS`], `base` is required, and a custom type may not shadow
/// a built-in type. A missing file means no custom types.
///
/// ```toml
/// [types.service_protocol]
/// base = "enum"
/// allowed = ["ssh", "ftp", "http"]
/// ```
pub fn load_custom_types(project_dir: &Path) -> Result<HashMap<String, ArgDef>, String> {
    let path = project_dir.join("toolclad.toml");
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(e) => return Err(format!("Cannot read {}: {e}", path.display())),
    };
    parse_custom_types(&content).map_err(|e| format!("{}: {e}", path.display()))
}

/// [`load_custom_types`] from TOML text.
pub fn parse_custom_types(content: &str) -> Result<HashMap<String, ArgDef>, String> {
    let table: toml::Table = toml::from_str(content).map_err(|e| e.to_string())?;
    let mut result = HashMap::new();
    for (key, value) in table {
        if key != "types" {
            return Err(format!("unknown key '{key}'"));
        }
        let types = value.as_table().ok_or("'types' must be a table")?;
        for (name, value) in types {
            let mut fields = value
                .as_table()
                .ok_or_else(|| format!("'types.{name}' must be a table"))?
                .clone();
            if let Some(bad) = fields
                .keys()
                .find(|k| !CUSTOM_TYPE_KEYS.contains(&k.as_str()))
            {
                return Err(format!("unknown key 'types.{name}.{bad}'"));
            }
            if validator::is_builtin_type(name) {
                return Err(format!("custom type '{name}' shadows a built-in type"));
            }
            let base = fields
                .remove("base")
                .ok_or_else(|| format!("custom type '{name}' missing 'base' field"))?;
            fields.insert("type".into(), base);
            let def: ArgDef = toml::Value::Table(fields)
                .try_into()
                .map_err(|e| format!("custom type '{name}': {e}"))?;
            result.insert(name.clone(), def);
        }
    }
    // Every alias must resolve to a built-in base without a cycle.
    for (name, def) in &result {
        validator::base_type(def, Some(&result))
            .map_err(|e| format!("custom type '{name}': {e}"))?;
        if let Some(pattern) = &def.pattern {
            regex::Regex::new(&validator::anchor_pattern(pattern))
                .map_err(|e| format!("custom type '{name}' has an invalid pattern: {e}"))?;
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Manifest, String> {
        parse_manifest(text, &HashMap::new())
    }

    const MINIMAL: &str = r#"
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
exec = ["echo", "{message}"]

[output]
format = "text"

[output.schema]
type = "object"
"#;

    #[test]
    fn test_parse_minimal_manifest() {
        let manifest = parse(MINIMAL).unwrap();
        assert_eq!(manifest.tool.name, "test_tool");
        assert_eq!(manifest.tool.binary, "echo");
        assert_eq!(manifest.tool.mode, "oneshot");
        assert_eq!(manifest.args["message"].type_name, "string");
        assert!(manifest.mcp.is_none());
        assert!(manifest.http.is_none());
        assert_eq!(manifest.output.max_bytes, DEFAULT_OUTPUT_MAX_BYTES);
    }

    #[test]
    fn unknown_keys_are_refused_in_every_section() {
        for (section, line) in [
            ("[tool]", "tiemout_seconds = 5"),
            ("[args.message]", "block_internal_typo = true"),
            ("[command]", "shell = true"),
            ("[output]", "max_byte = 1"),
        ] {
            let text = MINIMAL.replacen(section, &format!("{section}\n{line}"), 1);
            let err = parse(&text).unwrap_err();
            assert!(err.contains("unknown field"), "{section}: {err}");
        }
        let text = format!("{MINIMAL}\n[surprise]\nx = 1\n");
        assert!(parse(&text).is_err());
    }

    #[test]
    fn toolclad_keys_are_accepted() {
        let text = r#"
[tool]
name = "t"
version = "1"
binary = "echo"
description = "d"
timeout_seconds = 5
network_egress = false
allow_interpreter = false
dispatch = "exec"

[tool.limits]
nproc = 16
cpu_seconds = 10

[tool.evidence]
output_dir = "{evidence_dir}/{scan_id}-t"
capture = true
hash = "sha256"

[args.n]
position = 1
type = "number"
min_float = 0.0
max_float = 1.0
allow_leading_dash = false

[args.s]
position = 2
type = "string"
allow_whitespace = true

[command]
exec = ["echo", "{n}", "{s}"]
end_of_options = "--"

[output]
format = "text"
max_bytes = 1024
"#;
        let m = parse(text).unwrap();
        assert_eq!(m.tool.limits.unwrap().nproc, Some(16));
        assert_eq!(m.output.max_bytes, 1024);
    }

    #[test]
    fn unsupported_controls_fail_to_load() {
        for (from, to) in [
            ("timeout_seconds = 5", "timeout_seconds = 0"),
            ("dispatch = \"exec\"", "dispatch = \"callback\""),
            ("max_bytes = 1024", "max_bytes = 0"),
            ("hash = \"sha256\"", "hash = \"md5\""),
        ] {
            let text = r#"
[tool]
name = "t"
version = "1"
binary = "echo"
description = "d"
timeout_seconds = 5
dispatch = "exec"
[tool.evidence]
hash = "sha256"
[command]
exec = ["echo"]
[output]
format = "text"
max_bytes = 1024
"#
            .replace(from, to);
            assert!(parse(&text).is_err(), "{to}");
        }
        let custom = MINIMAL.replace(
            "format = \"text\"",
            "format = \"json\"\nparser = \"scripts/parse.py\"",
        );
        assert!(parse(&custom).unwrap_err().contains("parser"));
        let sanitize = MINIMAL.replace(
            "type = \"string\"",
            "type = \"string\"\nsanitize = [\"html\"]",
        );
        assert!(parse(&sanitize).is_err());
    }

    #[test]
    fn interpreter_and_binary_mismatch_are_refused() {
        let shell = MINIMAL
            .replace("binary = \"echo\"", "binary = \"bash\"")
            .replace(
                "exec = [\"echo\", \"{message}\"]",
                "exec = [\"bash\", \"-c\", \"{message}\"]",
            );
        assert!(parse(&shell).unwrap_err().contains("interpreter"));
        let allowed = shell.replace(
            "binary = \"bash\"",
            "binary = \"bash\"\nallow_interpreter = true",
        );
        assert!(parse(&allowed).is_ok());
        let mismatch = MINIMAL.replace(
            "exec = [\"echo\", \"{message}\"]",
            "exec = [\"cat\", \"{message}\"]",
        );
        assert!(parse(&mismatch).unwrap_err().contains("does not match"));
    }

    #[test]
    fn egress_binaries_require_block_internal() {
        let text = r#"
[tool]
name = "fetch"
version = "1"
binary = "curl"
description = "d"
[args.url]
position = 1
required = true
type = "url"
[command]
exec = ["curl", "{url}"]
[output]
format = "text"
"#;
        assert!(parse(text).unwrap_err().contains("block_internal"));
        let fixed = text.replace("type = \"url\"", "type = \"url\"\nblock_internal = true");
        assert!(parse(&fixed).is_ok());
        let dig = text
            .replace("curl", "dig")
            .replace("type = \"url\"", "type = \"scope_target\"");
        assert!(parse(&dig).unwrap_err().contains("block_internal"));
    }

    #[test]
    fn mappings_must_cover_enums() {
        let text = r#"
[tool]
name = "t"
version = "1"
binary = "echo"
description = "d"
[args.kind]
position = 1
required = true
type = "enum"
allowed = ["a", "b"]
[command]
exec = ["echo", "{_kind_flags}"]
[command.mappings.kind]
a = "-a"
[output]
format = "text"
"#;
        assert!(parse(text).unwrap_err().contains("does not cover"));
    }

    #[test]
    fn session_child_detection_and_unknown_sanitizers_are_refused() {
        let base = r#"
[tool]
name = "s"
version = "1"
description = "d"
mode = "session"
[session]
startup_command = "psql"
ready_pattern = "db=> "
[session.commands.select]
pattern = "SELECT .+"
description = "q"
[output]
format = "text"
"#;
        assert!(parse(base).is_ok());
        let child = base.replace(
            "[session.commands.select]",
            "[session.child_detection]\npattern = \"x\"\n[session.commands.select]",
        );
        assert!(parse(&child).unwrap_err().contains("S7"));
        let sanitize = base.replace(
            "[session.commands.select]",
            "[session.interaction]\ninput_sanitize = [\"html\"]\n[session.commands.select]",
        );
        assert!(parse(&sanitize).is_err());
        let msf = base.replace(
            "startup_command = \"psql\"",
            "startup_command = \"msfconsole -q\"",
        );
        assert!(parse(&msf).unwrap_err().contains("network-capable"));
    }

    #[test]
    fn directory_load_failures_are_loud() {
        let dir = tempfile::tempdir().unwrap();
        let tools = dir.path().join("tools");
        std::fs::create_dir(&tools).unwrap();
        std::fs::write(tools.join("good.clad.toml"), MINIMAL).unwrap();
        assert_eq!(load_manifests_from_dir(&tools).unwrap().len(), 1);
        std::fs::write(
            tools.join("bad.clad.toml"),
            MINIMAL.replace("[tool]", "[tool]\ntiemout_seconds = 1"),
        )
        .unwrap();
        let err = load_manifests_from_dir(&tools).unwrap_err();
        assert!(err.contains("bad.clad.toml"), "{err}");
        assert!(load_manifests_from_dir(&dir.path().join("missing"))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn custom_types_are_strict() {
        let types = parse_custom_types(
            "[types.proto]\nbase = \"enum\"\nallowed = [\"ssh\"]\n[types.alias]\nbase = \"proto\"\n",
        )
        .unwrap();
        assert_eq!(types["proto"].type_name, "enum");
        assert!(parse_custom_types("[types.x]\nbase = \"string\"\ntypo = 1\n").is_err());
        assert!(parse_custom_types("[types.string]\nbase = \"string\"\n").is_err());
        assert!(parse_custom_types("[types.x]\nallowed = []\n").is_err());
        assert!(parse_custom_types("[types.a]\nbase = \"b\"\n[types.b]\nbase = \"a\"\n").is_err());
        assert!(parse_custom_types("[other]\n").is_err());
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
        let manifest = parse(toml_str).unwrap();
        let mcp = manifest.mcp.as_ref().unwrap();
        assert_eq!(mcp.server, "brave-search");
        assert_eq!(mcp.tool, "brave_web_search");
        assert_eq!(mcp.field_map.get("query").unwrap(), "q");
        assert_eq!(mcp.field_map.get("max_results").unwrap(), "count");
    }

    #[test]
    fn test_arg_feeds_decision_parses_and_defaults_false() {
        let toml_str = MINIMAL.replace(
            "description = \"Message to echo\"",
            "description = \"Message to echo\"\nfeeds_decision = true",
        );
        let m = parse(&toml_str).unwrap();
        assert!(m.args["message"].feeds_decision);
    }

    #[test]
    fn effective_risk_tier_raises_to_the_command_tier() {
        let text = r#"
[tool]
name = "s"
version = "1"
description = "d"
mode = "session"
risk_tier = "low"
[session]
startup_command = "psql"
ready_pattern = "db=> "
[session.commands.drop]
pattern = "DROP .+"
description = "q"
risk_tier = "critical"
[output]
format = "text"
"#;
        let m = parse(text).unwrap();
        assert_eq!(m.effective_risk_tier(Some("drop")), "critical");
        assert_eq!(m.effective_risk_tier(None), "low");
    }
}
