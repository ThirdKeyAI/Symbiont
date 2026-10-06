//! ToolClad executor — bridges ORGA loop to .clad.toml tool manifests
//!
//! Implements the `ActionExecutor` trait: receives tool calls, validates
//! arguments, constructs commands from templates, executes, and returns
//! structured JSON observations.
//!
//! Supports built-in output parsers (json, xml, csv, jsonl, text), custom
//! external parsers, and output schema validation.

use async_trait::async_trait;
use std::collections::HashMap;
use std::time::Duration;

use super::manifest::Manifest;
use super::validator;
use crate::reasoning::circuit_breaker::CircuitBreakerRegistry;
use crate::reasoning::executor::ActionExecutor;
use crate::reasoning::inference::ToolDefinition;
use crate::reasoning::loop_types::{LoopConfig, Observation, ProposedAction};
use crate::reasoning::prepared::{
    canonical_json, digest_json, AuthorizedAction, PreparedAction, ToolContract,
};

use super::manifest::ArgDef;
use crate::sandbox::command::CommandBoundary;
use crate::sandbox::files::FileAccessPlan;
use crate::sandbox::source::SourcePlan;

/// An executor that dispatches tool calls to ToolClad manifests.
/// Handles command, HTTP, MCP, terminal, browser and fixed source-query backends.
struct ToolCladSnapshot {
    executor_id: uuid::Uuid,
    #[cfg(feature = "mcp-client")]
    enforce_mcp_verification: bool,
    manifest: Manifest,
    validated: HashMap<String, String>,
    argv: Option<Vec<String>>,
    boundary: CommandBoundary,
    files: FileAccessPlan,
    source: Option<SourcePlan>,
    #[cfg(feature = "mcp-client")]
    mcp_registry: Option<crate::integrations::mcp::registry::McpServerRegistry>,
}

#[cfg(feature = "mcp-client")]
struct McpDispatchConfig<'a> {
    enforce_verification: bool,
    boundary: &'a CommandBoundary,
    files: &'a FileAccessPlan,
    timeout: Duration,
}

pub struct ToolCladExecutor {
    executor_id: uuid::Uuid,
    command_runs: crate::sandbox::command_cleanup::Registry,
    manifests: HashMap<String, Manifest>,
    tool_defs: Vec<ToolDefinition>,
    custom_types: HashMap<String, ArgDef>,
    /// Manifest versions recorded at construction time for hot-reload detection.
    manifest_versions: HashMap<String, String>,
    /// Session executor for interactive CLI tools.
    session_executor: super::session_executor::SessionExecutor,
    /// Browser executor for CDP-based browser sessions.
    browser_executor: super::browser_executor::BrowserExecutor,
    /// Whether MCP-backed tool calls require SchemaPin verification before
    /// running (fail-closed by default). Disable only for local dev via
    /// [`Self::with_mcp_verification`].
    enforce_mcp_verification: bool,
    /// Scope is loaded once from operator configuration. Retain load errors
    /// so a malformed scope cannot silently turn into unrestricted dispatch.
    scope: Result<Option<super::scope::Scope>, String>,
    boundary: Result<CommandBoundary, String>,
}

impl ToolCladExecutor {
    /// Create an executor from a set of loaded manifests.
    pub fn new(manifests: Vec<(String, Manifest)>) -> Self {
        Self::with_custom_types(manifests, HashMap::new())
    }

    /// Create an executor with custom type definitions loaded from `toolclad.toml`.
    pub fn with_custom_types(
        manifests: Vec<(String, Manifest)>,
        custom_types: HashMap<String, ArgDef>,
    ) -> Self {
        let tool_defs: Vec<ToolDefinition> = manifests
            .iter()
            .flat_map(|(_, m)| generate_tool_definitions(m))
            .collect();
        let manifest_versions: HashMap<String, String> = manifests
            .iter()
            .map(|(name, m)| (name.clone(), m.tool.version.clone()))
            .collect();
        // Create sub-executors for session and browser modes
        let session_manifests: Vec<_> = manifests
            .iter()
            .filter(|(_, m)| m.tool.mode == "session")
            .map(|(n, m)| (n.clone(), m.clone()))
            .collect();
        let browser_manifests: Vec<_> = manifests
            .iter()
            .filter(|(_, m)| m.tool.mode == "browser")
            .map(|(n, m)| (n.clone(), m.clone()))
            .collect();
        let session_executor = super::session_executor::SessionExecutor::new(session_manifests);
        let browser_executor = super::browser_executor::BrowserExecutor::new(browser_manifests);

        let manifest_map: HashMap<String, Manifest> = manifests.into_iter().collect();
        Self {
            executor_id: uuid::Uuid::new_v4(),
            command_runs: Default::default(),
            manifests: manifest_map,
            tool_defs,
            custom_types,
            manifest_versions,
            session_executor,
            browser_executor,
            enforce_mcp_verification: true,
            scope: Ok(None),
            boundary: Ok(CommandBoundary::default()),
        }
    }

    /// Select an operator-owned command boundary. The selection is part of
    /// every prepared call and changes the executor identity.
    pub fn with_command_boundary(mut self, boundary: CommandBoundary) -> Self {
        self.executor_id = uuid::Uuid::new_v4();
        self.session_executor = super::session_executor::SessionExecutor::new(
            self.manifests
                .iter()
                .map(|(name, manifest)| (name.clone(), manifest.clone()))
                .collect(),
        );
        self.boundary = boundary.validate().map(|()| boundary);
        self
    }

    pub fn with_project_sandbox(mut self, project: &std::path::Path) -> Self {
        self.executor_id = uuid::Uuid::new_v4();
        self.session_executor = super::session_executor::SessionExecutor::new(
            self.manifests
                .iter()
                .map(|(name, manifest)| (name.clone(), manifest.clone()))
                .collect(),
        );
        self.boundary = CommandBoundary::load(project);
        self
    }

    /// Explicit development fixture escape hatch; refused in production.
    pub fn with_development_host_execution(self) -> Self {
        self.with_command_boundary(CommandBoundary::development_host())
    }

    /// Toggle SchemaPin verification enforcement for MCP-backed tool calls.
    /// Defaults to `true` (fail-closed); set `false` only for local dev.
    pub fn with_mcp_verification(mut self, enforce: bool) -> Self {
        self.executor_id = uuid::Uuid::new_v4();
        self.enforce_mcp_verification = enforce;
        self
    }

    /// Load project scope for arguments that require a scoped destination.
    pub fn with_project_scope(mut self, project_dir: &std::path::Path) -> Self {
        self.executor_id = uuid::Uuid::new_v4();
        self.scope = super::scope::Scope::load(project_dir);
        self
    }

    pub fn with_scope(mut self, scope: super::scope::Scope) -> Self {
        self.executor_id = uuid::Uuid::new_v4();
        self.scope = scope.validate().map(|()| Some(scope));
        self
    }

    /// Check if this executor handles a given tool name.
    /// Matches both direct tool names and session/browser sub-commands
    /// (e.g., "msfconsole_session" or "msfconsole_session.run").
    pub fn handles(&self, tool_name: &str) -> bool {
        if self.manifests.contains_key(tool_name) {
            return true;
        }
        // Check session and browser executors
        if self.session_executor.handles(tool_name) || self.browser_executor.handles(tool_name) {
            return true;
        }
        // Check for session/browser sub-command pattern: "toolname.command"
        if let Some(base) = tool_name.split('.').next() {
            if let Some(m) = self.manifests.get(base) {
                let cmd = tool_name
                    .strip_prefix(base)
                    .unwrap_or("")
                    .trim_start_matches('.');
                if let Some(session) = &m.session {
                    return session.commands.contains_key(cmd);
                }
                if let Some(browser) = &m.browser {
                    return browser.commands.contains_key(cmd);
                }
            }
        }
        false
    }

    /// Get tool definitions (convenience method that doesn't require importing ActionExecutor).
    pub fn get_tool_definitions(&self) -> Vec<crate::reasoning::inference::ToolDefinition> {
        self.tool_defs.clone()
    }

    /// Number of loaded manifests.
    pub fn count(&self) -> usize {
        self.manifests.len()
    }

    /// Look up the manifest for `name`, check it hasn't been hot-reloaded out
    /// from under the executor, parse `args_json`, and validate each argument
    /// against its `ArgDef`. Shared by the sync `execute_tool` dispatch and
    /// the async MCP-aware dispatch in `execute_actions`.
    fn parse_and_validate(
        &self,
        name: &str,
        args_json: &str,
    ) -> Result<(&Manifest, HashMap<String, String>), String> {
        let manifest = self
            .manifests
            .get(name)
            .ok_or_else(|| format!("No ToolClad manifest for '{}'", name))?;

        // Check manifest version against recorded version (hot-reload detection)
        if let Some(recorded_version) = self.manifest_versions.get(name) {
            if *recorded_version != manifest.tool.version {
                return Err(format!(
                    "Manifest version mismatch for '{}': executor was built with v{} but manifest \
                     is now v{}. The tool definition may have changed — please re-plan.",
                    name, recorded_version, manifest.tool.version
                ));
            }
        }

        Ok((
            manifest,
            self.validate_arguments(name, &manifest.args, args_json)?,
        ))
    }

    fn validate_arguments(
        &self,
        name: &str,
        definitions: &HashMap<String, ArgDef>,
        args_json: &str,
    ) -> Result<HashMap<String, String>, String> {
        // Parse arguments from JSON
        let args: HashMap<String, serde_json::Value> = serde_json::from_str(args_json)
            .map_err(|e| format!("Invalid arguments JSON: {}", e))?;

        for arg_name in args.keys() {
            if !definitions.contains_key(arg_name) {
                return Err(format!("Unknown argument '{arg_name}' for tool '{name}'"));
            }
        }
        let scope = self
            .scope
            .as_ref()
            .map_err(|e| format!("Scope configuration error: {e}"))?;

        // Validate each argument against its definition
        let mut validated: HashMap<String, String> = HashMap::new();
        for (arg_name, arg_def) in definitions {
            let value = if let Some(v) = args.get(arg_name) {
                match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string().trim_matches('"').to_string(),
                }
            } else if arg_def.required {
                return Err(format!("Missing required argument: {}", arg_name));
            } else if let Some(default) = &arg_def.default {
                default.to_string().trim_matches('"').to_string()
            } else {
                String::new()
            };

            if !value.is_empty() || arg_def.required {
                let custom = if self.custom_types.is_empty() {
                    None
                } else {
                    Some(&self.custom_types)
                };
                let cleaned = validator::validate_arg_with_custom(arg_def, &value, custom)
                    .map_err(|e| format!("Validation failed for '{}': {}", arg_name, e))?;
                if validator::requires_scope(arg_def, custom)? {
                    scope
                        .as_ref()
                        .ok_or_else(|| {
                            format!("Argument '{arg_name}' requires a configured project scope")
                        })?
                        .check(&cleaned)
                        .map_err(|e| format!("Scope denied '{arg_name}': {e}"))?;
                }
                validated.insert(arg_name.clone(), cleaned);
            } else {
                validated.insert(arg_name.clone(), value);
            }
        }

        Ok(validated)
    }

    /// Execute a single tool call against a manifest.
    pub fn execute_tool(&self, name: &str, args_json: &str) -> Result<serde_json::Value, String> {
        let (manifest, validated) = self.parse_and_validate(name, args_json)?;
        if manifest.tool.human_approval {
            return Err(
                "tool requires an authorized exact-call approval; use governed dispatch".into(),
            );
        }

        if tokio::runtime::Handle::try_current().is_ok() {
            return Err("tools in an async runtime must use execute_actions".into());
        }
        let boundary = self.boundary.as_ref().map_err(Clone::clone)?;
        let timeout = Duration::from_secs(manifest.tool.timeout_seconds);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        runtime.block_on(async {
            if let Some(source) = Self::prepare_source(manifest, boundary, &validated, timeout)? {
                source
                    .execute_prepared(name, std::time::Instant::now() + timeout)
                    .await
            } else if manifest.mcp.is_some() {
                self.execute_mcp_backend_async(name, manifest, &validated)
                    .await
            } else if manifest.http.is_some() {
                Self::execute_http_backend(name, manifest, &validated, timeout, boundary, None)
                    .await
            } else {
                let argv = build_argv(manifest, &validated)?;
                let files = Self::prepare_files(manifest, boundary, &validated)?;
                Self::execute_shell_backend(name, manifest, &argv, timeout, boundary, &files).await
            }
        })
    }

    fn prepare_source(
        manifest: &Manifest,
        boundary: &CommandBoundary,
        validated: &HashMap<String, String>,
        timeout: Duration,
    ) -> Result<Option<SourcePlan>, String> {
        let Some(query) = &manifest.source else {
            return Ok(None);
        };
        manifest.validate_source_backend()?;
        let deadline = std::time::Instant::now()
            .checked_add(timeout)
            .ok_or("source deadline overflow")?;
        SourcePlan::prepare(boundary, query, validated, deadline).map(Some)
    }

    pub(super) fn prepare_files(
        manifest: &Manifest,
        boundary: &CommandBoundary,
        validated: &HashMap<String, String>,
    ) -> Result<FileAccessPlan, String> {
        if manifest.filesystem.is_some()
            && (!matches!(manifest.tool.mode.as_str(), "oneshot" | "session")
                || manifest.http.is_some()
                || (manifest.tool.mode == "session" && manifest.mcp.is_some()))
        {
            return Err("filesystem grants require a command, MCP tool or terminal session".into());
        }
        if manifest.tool.mode == "session"
            && manifest
                .filesystem
                .as_ref()
                .is_some_and(|f| !f.create.is_empty())
            && !manifest
                .session
                .as_ref()
                .is_some_and(|s| s.commands.values().any(|c| c.finalize))
        {
            return Err("terminal file output requires a declared finalizing command".into());
        }
        FileAccessPlan::prepare(boundary, manifest.filesystem.as_ref(), validated)
    }

    async fn execute_shell_backend(
        name: &str,
        manifest: &Manifest,
        argv: &[String],
        timeout: Duration,
        boundary: &CommandBoundary,
        files: &FileAccessPlan,
    ) -> Result<serde_json::Value, String> {
        // Display only; dispatch uses the immutable argv prepared before policy.
        let command = argv.join(" ");
        files.check_publication_authority()?;
        let start = std::time::Instant::now();
        let staged = files.stage(boundary)?;
        let output = staged
            .boundary
            .execute(argv, timeout.saturating_sub(start.elapsed()))
            .await
            .map_err(|e| format!("Tool '{name}': {e}"))?;
        let duration_ms = start.elapsed().as_millis() as u64;
        let stdout = output.stdout;
        let stderr = output.stderr;

        // Parse output using the manifest's format/parser configuration
        let parsed = parse_output_with_boundary(
            manifest,
            &stdout,
            timeout.saturating_sub(start.elapsed()),
            boundary,
        )
        .await?;

        // Validate parsed output against schema (warnings only, non-fatal)
        let schema_warnings = validate_output_schema(&parsed, &manifest.output.schema);
        let created_files = if output.success {
            if start.elapsed() >= timeout {
                return Err("tool deadline expired before file publication".into());
            }
            staged.publish_recorded().await?
        } else {
            serde_json::json!([])
        };

        // Build evidence envelope
        let scan_id = format!(
            "{}-{}",
            chrono::Utc::now().timestamp(),
            uuid::Uuid::new_v4().as_fields().0
        );
        let status = if output.success { "success" } else { "error" };

        // Hash output for evidence chain
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(stdout.as_bytes());
        let hash = format!("sha256:{}", hex::encode(hasher.finalize()));

        let mut envelope = serde_json::json!({
            "status": status,
            "scan_id": scan_id,
            "tool": name,
            "command": command,
            "duration_ms": duration_ms,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "output_hash": hash,
            "results": parsed,
            "created_files": created_files,
        });

        // Attach stderr and exit_code to the results
        if let Some(obj) = envelope.as_object_mut() {
            if let Some(results) = obj.get_mut("results").and_then(|r| r.as_object_mut()) {
                if !stderr.is_empty() {
                    results.insert(
                        "stderr".to_string(),
                        serde_json::Value::String(stderr.clone()),
                    );
                }
                results.insert("exit_code".to_string(), serde_json::json!(output.exit_code));
            }
        }

        // Attach schema warnings if any
        if !schema_warnings.is_empty() {
            if let Some(obj) = envelope.as_object_mut() {
                obj.insert(
                    "schema_warnings".to_string(),
                    serde_json::json!(schema_warnings),
                );
            }
        }

        Ok(envelope)
    }

    /// Execute an HTTP backend tool.
    async fn execute_http_backend(
        name: &str,
        manifest: &Manifest,
        validated: &HashMap<String, String>,
        timeout: Duration,
        boundary: &CommandBoundary,
        journal: Option<&crate::reasoning::effect_journal::EffectJournal>,
    ) -> Result<serde_json::Value, String> {
        boundary.validate()?;
        if manifest.filesystem.is_some() {
            return Err("HTTP tools cannot declare worker filesystem grants".into());
        }
        let start = std::time::Instant::now();
        let http = manifest.http.as_ref().ok_or("missing HTTP backend")?;
        if timeout.is_zero() {
            return Err(format!("HTTP tool '{name}' timed out before execution"));
        }
        let (method_value, url) = resolve_http_destination(http, validated)?;
        let method = method_value.to_string();
        let client = crate::net_guard::build_ssrf_safe_client(timeout)
            .map_err(|e| format!("Failed to construct HTTP client: {e}"))?;
        let mut request = client.request(method_value, &url);
        for (key, value) in &http.headers {
            request = request.header(key, render_http_template(value, validated)?);
        }
        if let Some(body) = &http.body_template {
            request = request.body(render_http_template(body, validated)?);
        }
        let request = request
            .build()
            .map_err(|e| format!("Invalid HTTP request: {e}"))?;
        let response = super::http_transport::exchange(
            &client,
            request,
            journal,
            start.checked_add(timeout).ok_or("HTTP deadline overflow")?,
            10 * 1024 * 1024,
        )
        .await?;
        let status_code = response.status;
        let response_body = String::from_utf8(response.body)
            .map_err(|e| format!("HTTP response is not valid UTF-8: {e}"))?;
        let is_success = if !http.success_status.is_empty() {
            http.success_status.contains(&status_code)
        } else {
            (200..300).contains(&status_code)
        };

        // Parse response
        let results = parse_output_with_boundary(
            manifest,
            &response_body,
            timeout.saturating_sub(start.elapsed()),
            boundary,
        )
        .await?;
        let duration_ms = start.elapsed().as_millis() as u64;

        let scan_id = format!(
            "{}-{}",
            chrono::Utc::now().timestamp(),
            uuid::Uuid::new_v4().as_fields().0
        );

        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(response_body.as_bytes());
        let hash = format!("sha256:{}", hex::encode(hasher.finalize()));

        Ok(serde_json::json!({
            "status": if is_success { "success" } else { "error" },
            "scan_id": scan_id,
            "tool": name,
            "http_method": method,
            "http_url": url,
            "http_status": status_code,
            "duration_ms": duration_ms,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "output_hash": hash,
            "exit_code": if is_success { 0 } else { status_code as i32 },
            "stderr": "",
            "results": results
        }))
    }

    /// Execute an MCP proxy backend tool by loading the on-disk server
    /// registry (`./mcp-config.toml`) and dispatching to the real stdio MCP
    /// client. See [`Self::execute_mcp_backend_async_with_registry`] for the
    /// injectable-registry variant used by tests.
    #[cfg(feature = "mcp-client")]
    async fn execute_mcp_backend_async(
        &self,
        name: &str,
        manifest: &Manifest,
        validated: &HashMap<String, String>,
    ) -> Result<serde_json::Value, String> {
        let registry = crate::integrations::mcp::registry::McpServerRegistry::load()?;
        self.execute_mcp_backend_async_with_registry(&registry, name, manifest, validated)
            .await
    }

    /// Same as [`Self::execute_mcp_backend_async`] but takes an explicit
    /// registry instead of loading `./mcp-config.toml` from the process's
    /// working directory. `pub` so integration tests can inject an in-memory
    /// registry rather than relying on CWD games.
    #[cfg(feature = "mcp-client")]
    pub async fn execute_mcp_backend_async_with_registry(
        &self,
        registry: &crate::integrations::mcp::registry::McpServerRegistry,
        name: &str,
        manifest: &Manifest,
        validated: &HashMap<String, String>,
    ) -> Result<serde_json::Value, String> {
        let declared = self
            .manifests
            .get(name)
            .ok_or("MCP helper requires a registered manifest")?;
        if declared.tool.human_approval {
            return Err("MCP tool requires an authorized exact-call approval".into());
        }
        if serde_json::to_value(declared).map_err(|e| e.to_string())?
            != serde_json::to_value(manifest).map_err(|e| e.to_string())?
        {
            return Err("MCP helper cannot replace the registered tool contract".into());
        }
        let normalized = self.validate_arguments(
            name,
            &declared.args,
            &serde_json::to_string(validated).map_err(|e| e.to_string())?,
        )?;
        let boundary = self.boundary.as_ref().map_err(Clone::clone)?;
        let files = Self::prepare_files(declared, boundary, &normalized)?;
        self.invoke_mcp_backend_with_registry(
            registry,
            name,
            declared,
            &normalized,
            McpDispatchConfig {
                enforce_verification: self.enforce_mcp_verification,
                boundary,
                files: &files,
                timeout: Duration::from_secs(manifest.tool.timeout_seconds),
            },
        )
        .await
    }

    #[cfg(feature = "mcp-client")]
    async fn invoke_mcp_backend_with_registry(
        &self,
        registry: &crate::integrations::mcp::registry::McpServerRegistry,
        name: &str,
        manifest: &Manifest,
        validated: &HashMap<String, String>,
        dispatch: McpDispatchConfig<'_>,
    ) -> Result<serde_json::Value, String> {
        let mcp = manifest
            .mcp
            .as_ref()
            .ok_or_else(|| "no [mcp] block in manifest".to_string())?;

        let spec = registry.get(&mcp.server).ok_or_else(|| {
            format!(
                "MCP server '{}' not in registry (mcp-config.toml)",
                mcp.server
            )
        })?;

        // Map validated args to upstream tool's expected format
        let upstream_args = map_upstream_args(mcp, &manifest.args, validated);

        let result =
            crate::integrations::mcp::stdio_client::RmcpStdioClient::verified_invoke_with_files(
                spec,
                &mcp.tool,
                upstream_args,
                dispatch.enforce_verification,
                dispatch.timeout,
                dispatch.boundary,
                dispatch.files,
            )
            .await?;

        // Real evidence envelope (replaces the fabricated "delegated" one).
        Ok(serde_json::json!({
            "status": "executed",
            "tool": name,
            "mcp_server": mcp.server,
            "mcp_tool": mcp.tool,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "exit_code": 0,
            "stderr": "",
            "results": result.content,
            "created_files": result.created_files,
        }))
    }

    /// Honest, fail-closed fallback when the crate is built without
    /// `mcp-client`: no fabricated result, just a clear error.
    #[cfg(not(feature = "mcp-client"))]
    async fn execute_mcp_backend_async(
        &self,
        _name: &str,
        _manifest: &Manifest,
        _validated: &HashMap<String, String>,
    ) -> Result<serde_json::Value, String> {
        Err("MCP execution requires the 'mcp-client' feature".to_string())
    }
}

/// Map validated local argument names to the upstream MCP tool's expected
/// argument names via the manifest's `[mcp.field_map]` (identity when a local
/// name has no mapping entry), coercing each value to the JSON type the upstream
/// tool's schema expects based on the manifest arg's declared `type`.
///
/// `parse_and_validate` stringifies every argument (numbers, booleans, arrays,
/// and objects arrive here as their JSON text), so without this an upstream tool
/// whose schema wants a number/boolean/array/object would receive a quoted
/// string and its JSON-Schema validation would reject it. Coercion is by
/// declared `type_name`; a value that fails to parse falls back to a string
/// rather than erroring (the validator already gate-kept the value).
#[cfg(feature = "mcp-client")]
fn map_upstream_args(
    mcp: &super::manifest::McpProxyDef,
    arg_defs: &HashMap<String, super::manifest::ArgDef>,
    validated: &HashMap<String, String>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut upstream_args = serde_json::Map::new();
    for (local_name, value) in validated {
        let upstream_name = mcp
            .field_map
            .get(local_name)
            .cloned()
            .unwrap_or_else(|| local_name.clone());
        let type_name = arg_defs
            .get(local_name)
            .map(|d| d.type_name.as_str())
            .unwrap_or("string");
        upstream_args.insert(upstream_name, coerce_arg_value(type_name, value));
    }
    upstream_args
}

/// Coerce a validated argument's string form to the JSON type its declared
/// ToolClad `type` implies, for MCP upstream dispatch. Unknown/string-like types
/// (`string`, `enum`, `url`, `scope_target`, …) stay strings; a value that
/// doesn't parse falls back to a string so a mislabeled arg can't panic a call.
#[cfg(feature = "mcp-client")]
fn coerce_arg_value(type_name: &str, value: &str) -> serde_json::Value {
    use serde_json::Value;
    match type_name {
        "integer" => value
            .parse::<i64>()
            .map(Value::from)
            .unwrap_or_else(|_| Value::String(value.to_string())),
        "number" | "float" => value
            .parse::<f64>()
            .map(Value::from)
            .unwrap_or_else(|_| Value::String(value.to_string())),
        "boolean" => value
            .parse::<bool>()
            .map(Value::from)
            .unwrap_or_else(|_| Value::String(value.to_string())),
        "array" | "object" => serde_json::from_str::<Value>(value)
            .unwrap_or_else(|_| Value::String(value.to_string())),
        _ => Value::String(value.to_string()),
    }
}

#[async_trait]
impl ActionExecutor for ToolCladExecutor {
    fn validate_configuration(&self) -> Result<(), String> {
        self.scope.as_ref().map_err(Clone::clone)?;
        for manifest in self.manifests.values() {
            manifest.validate_source_backend()?;
        }
        self.boundary.as_ref().map_err(Clone::clone)?.validate()
    }

    fn prepare_action(
        &self,
        action: &ProposedAction,
        config: &LoopConfig,
    ) -> Result<PreparedAction, String> {
        let ProposedAction::ToolCall {
            call_id,
            name,
            arguments,
        } = action
        else {
            return PreparedAction::new(action.clone(), None);
        };
        crate::reasoning::phases::validate_tool_call_arguments(
            name,
            arguments,
            &config.tool_definitions,
        )?;
        let base = if self.manifests.contains_key(name) {
            name.as_str()
        } else {
            name.split_once('.')
                .map(|(base, _)| base)
                .ok_or("invalid ToolClad command name")?
        };
        let manifest = self
            .manifests
            .get(base)
            .ok_or_else(|| format!("No ToolClad manifest for '{name}'"))?;
        let ceiling = self.boundary.as_ref().map_err(Clone::clone)?;
        let boundary = if matches!(manifest.tool.mode.as_str(), "oneshot" | "session") {
            ceiling.without_host_mounts()
        } else {
            ceiling.clone()
        };
        let mut needs_approval = manifest.tool.human_approval;
        let validated = match manifest.tool.mode.as_str() {
            "session" => {
                let command_name = name
                    .strip_prefix(&format!("{base}."))
                    .ok_or("session requires a declared command")?;
                let command = manifest
                    .session
                    .as_ref()
                    .and_then(|s| s.commands.get(command_name))
                    .ok_or("unknown session command")?;
                needs_approval |= command.human_approval;
                let mut definitions = command.args.clone();
                definitions.insert(
                    "command".into(),
                    ArgDef {
                        required: true,
                        type_name: "string".into(),
                        pattern: Some(command.pattern.clone()),
                        ..Default::default()
                    },
                );
                let values = self.validate_arguments(name, &definitions, arguments)?;
                super::session_executor::validate_terminal_input(
                    &values["command"],
                    &command.pattern,
                )?;
                if command.extract_target {
                    let pattern = regex::Regex::new(&format!(r"\A(?:{})\z", command.pattern))
                        .map_err(|e| e.to_string())?;
                    let target = pattern.captures(&values["command"]).and_then(|captures| captures.name("target").map(|m| m.as_str().to_owned()))
                        .ok_or("scoped session patterns must capture the actual target as (?P<target>...)")?;
                    self.scope
                        .as_ref()
                        .map_err(Clone::clone)?
                        .as_ref()
                        .ok_or("session target requires project scope")?
                        .check(&target)?;
                }
                values
            }
            "browser" => {
                let command_name = name
                    .strip_prefix(&format!("{base}."))
                    .ok_or("browser requires a declared command")?;
                let browser = manifest
                    .browser
                    .as_ref()
                    .ok_or("missing browser definition")?;
                let command = browser
                    .commands
                    .get(command_name)
                    .ok_or("unknown browser command")?;
                needs_approval |= command.human_approval;
                let mut values = self.validate_arguments(name, &command.args, arguments)?;
                let scope = browser
                    .scope
                    .as_ref()
                    .ok_or("browser requires a configured scope")?;
                let policy = super::browser_network::BrowserNetworkPolicy::new(
                    scope,
                    browser.network.as_ref(),
                )?;
                if command_name == "navigate" {
                    let url = policy.check_destination(
                        values.get("url").map(String::as_str).unwrap_or(""),
                        "GET",
                    )?;
                    values.insert("url".into(), url.to_string());
                }
                values
            }
            "oneshot" => self.parse_and_validate(name, arguments)?.1,
            other => return Err(format!("Unknown ToolClad mode: {other}")),
        };
        let source = Self::prepare_source(
            manifest,
            ceiling,
            &validated,
            Duration::from_secs(manifest.tool.timeout_seconds).min(config.tool_timeout),
        )?;
        let boundary = source
            .as_ref()
            .map(|s| s.execution_boundary(&boundary))
            .unwrap_or(boundary);
        let files = Self::prepare_files(manifest, ceiling, &validated)?;
        let mut boundary_descriptor = boundary.descriptor()?;
        boundary_descriptor["filesystem"] = source
            .as_ref()
            .map(|s| s.descriptor().clone())
            .unwrap_or_else(|| files.descriptor());
        let argv = if manifest.tool.mode == "oneshot"
            && manifest.http.is_none()
            && manifest.mcp.is_none()
            && manifest.source.is_none()
        {
            Some(build_argv(manifest, &validated)?)
        } else {
            None
        };
        #[cfg(feature = "mcp-client")]
        let mcp_registry = if manifest.mcp.is_some() {
            Some(crate::integrations::mcp::registry::McpServerRegistry::load()?)
        } else {
            None
        };
        let mut contract_input =
            serde_json::json!({"manifest": manifest, "custom_types": self.custom_types});
        #[cfg(feature = "mcp-client")]
        if let (Some(mcp), Some(registry)) = (&manifest.mcp, &mcp_registry) {
            let spec = registry
                .get(&mcp.server)
                .ok_or("MCP server is missing from registry")?;
            contract_input["mcp_server"] = serde_json::json!({"command": spec.command, "args": spec.args, "env": spec.env, "public_key_url": spec.public_key_url, "public_key_pem": spec.public_key_pem});
        }
        // Keep this mutable under all feature combinations without suppressing
        // unused_mut: include the fixed executor's verification mode as well.
        contract_input["command_boundary"] = boundary_descriptor.clone();
        contract_input["mount_ceiling"] = ceiling.descriptor()?;
        if source.is_some() {
            contract_input["source_broker_implementation"] =
                serde_json::json!(SourcePlan::implementation_hash());
        }
        contract_input["mcp_verification"] = serde_json::json!(self.enforce_mcp_verification);
        let (action_type, action_id, resource_type, resource_id) = match &manifest.tool.cedar {
            Some(cedar) => (
                format!("{}::Action", cedar.resource),
                cedar.action.clone(),
                cedar.resource.clone(),
                name.clone(),
            ),
            None => (
                "Action".into(),
                format!("tool_call::{name}"),
                "Resource".into(),
                "default".into(),
            ),
        };
        let transport = if let Some(source) = &source {
            source.transport()
        } else if manifest.tool.mode == "session" {
            "selected_pty_boundary"
        } else if manifest.tool.mode == "browser" {
            "browser_backend_unavailable"
        } else if manifest.mcp.is_some() {
            "selected_mcp_stdio_boundary"
        } else if manifest.http.is_some() {
            "native_http_broker"
        } else {
            "selected_command_boundary"
        };
        let mut resolved = serde_json::json!({"command_boundary": boundary_descriptor, "execution_transport": transport});
        if manifest.tool.mode == "session" {
            let command = name
                .strip_prefix(&format!("{base}."))
                .and_then(|command| manifest.session.as_ref()?.commands.get(command))
                .ok_or("missing prepared terminal command")?;
            resolved["session_finalizes"] = serde_json::json!(command.finalize);
            resolved["filesystem_lifetime"] = serde_json::json!("terminal_session");
        }
        if let Some(browser) = &manifest.browser {
            resolved["browser_network"] =
                serde_json::json!(browser.network.clone().unwrap_or_default());
        }
        if let Some(argv) = &argv {
            resolved["argv"] = serde_json::json!(argv);
        }
        if let Some(http) = &manifest.http {
            let (method, url) = resolve_http_destination(http, &validated)?;
            resolved["http_method"] = serde_json::json!(method.as_str());
            resolved["http_url"] = serde_json::json!(url);
        }
        #[cfg(feature = "mcp-client")]
        if let Some(mcp) = &manifest.mcp {
            resolved["mcp_server"] = serde_json::json!(mcp.server);
            resolved["mcp_tool"] = serde_json::json!(mcp.tool);
            resolved["upstream_arguments"] =
                serde_json::json!(map_upstream_args(mcp, &manifest.args, &validated));
        }
        PreparedAction::new(
            ProposedAction::ToolCall {
                call_id: call_id.clone(),
                name: name.clone(),
                arguments: canonical_json(
                    &serde_json::to_value(&validated).map_err(|e| e.to_string())?,
                )?,
            },
            Some(ToolContract {
                name: name.clone(),
                version: manifest.tool.version.clone(),
                digest: digest_json(&contract_input)?,
                action_type,
                action_id,
                resource_type,
                resource_id,
                requires_approval: needs_approval,
            }),
        )
        .and_then(|prepared| prepared.with_resolved(resolved))
        .map(|prepared| {
            prepared.with_backend(ToolCladSnapshot {
                executor_id: self.executor_id,
                #[cfg(feature = "mcp-client")]
                enforce_mcp_verification: self.enforce_mcp_verification,
                manifest: manifest.clone(),
                validated,
                argv,
                boundary: boundary.clone(),
                files,
                source,
                #[cfg(feature = "mcp-client")]
                mcp_registry,
            })
        })
    }

    fn cancel_run(&self, run: &str, deadline: std::time::Instant) {
        self.command_runs.cancel(run);
        self.session_executor.cancel_run(run, deadline);
    }

    async fn close_run(&self, run: &str, deadline: std::time::Instant) -> Result<(), String> {
        self.cancel_run(run, deadline);
        let (commands, sessions) = tokio::join!(
            self.command_runs.close(run),
            self.session_executor.close_run(run, deadline)
        );
        commands.and(sessions)
    }

    async fn execute_authorized(
        &self,
        actions: Vec<AuthorizedAction>,
        config: &LoopConfig,
        circuit_breakers: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        let mut observations = Vec::new();
        for grant in actions {
            let ProposedAction::ToolCall {
                call_id,
                name,
                arguments,
            } = grant.action().clone()
            else {
                continue;
            };
            let result = crate::sandbox::worker_origin::scope(grant.worker_origin(), async {
                grant.check_live()?;
                circuit_breakers
                    .check(&name)
                    .await
                    .map_err(|e| e.to_string())?;
                let deadline = grant.deadline();
                let run = grant.run_key().to_owned();
                let binding = grant.session_binding().to_owned();
                let run_deadline = grant.run_deadline();
                let effect_journal = grant.effect_journal();
                let prepared = grant.into_prepared()?;
                let snapshot = prepared
                    .backend::<ToolCladSnapshot>()
                    .ok_or("missing prepared ToolClad backend")?;
                if snapshot.executor_id != self.executor_id {
                    return Err("authorization belongs to another executor instance".into());
                }
                let manifest = &snapshot.manifest;
                let files = snapshot.files.with_journal(effect_journal.clone());
                let timeout = Duration::from_secs(manifest.tool.timeout_seconds)
                    .min(config.tool_timeout)
                    .min(deadline.saturating_duration_since(std::time::Instant::now()));
                if timeout.is_zero() {
                    return Err("tool execution budget exhausted".into());
                }
                if manifest.tool.mode == "session" {
                    return self
                        .session_executor
                        .execute_prepared(super::session_executor::SessionCall {
                            tool: &name,
                            manifest,
                            command: snapshot
                                .validated
                                .get("command")
                                .ok_or("missing prepared terminal command")?,
                            boundary: &snapshot.boundary,
                            files: &files,
                            contract: &prepared
                                .contract()
                                .ok_or("missing terminal contract")?
                                .digest,
                            run: &run,
                            binding: &binding,
                            run_deadline,
                            deadline: std::time::Instant::now()
                                .checked_add(timeout)
                                .ok_or("terminal deadline overflow")?
                                .min(deadline),
                        })
                        .await;
                }
                if manifest.tool.mode == "browser" {
                    return self
                        .browser_executor
                        .execute_prepared_browser_command(&name, &arguments);
                }
                let timeout =
                    timeout.min(deadline.saturating_duration_since(std::time::Instant::now()));
                if timeout.is_zero() {
                    return Err("authorization expired before worker start".into());
                }
                let owner = self.command_runs.admit(&run, run_deadline)?;
                owner
                    .scope(async {
                        if let Some(source) = &snapshot.source {
                            return source
                                .execute_prepared(
                                    &name,
                                    std::time::Instant::now()
                                        .checked_add(timeout)
                                        .ok_or("source deadline overflow")?
                                        .min(deadline),
                                )
                                .await;
                        }
                        // Retain MCP cleanup in the run even when an outer
                        // deadline drops discovery or an active protocol call.
                        if manifest.mcp.is_some() {
                            #[cfg(feature = "mcp-client")]
                            return tokio::time::timeout(
                                timeout,
                                self.invoke_mcp_backend_with_registry(
                                    snapshot
                                        .mcp_registry
                                        .as_ref()
                                        .ok_or("missing prepared MCP registry")?,
                                    &name,
                                    manifest,
                                    &snapshot.validated,
                                    McpDispatchConfig {
                                        enforce_verification: snapshot.enforce_mcp_verification,
                                        boundary: &snapshot.boundary,
                                        files: &files,
                                        timeout,
                                    },
                                ),
                            )
                            .await
                            .map_err(|_| "prepared MCP invocation timed out")?;
                            #[cfg(not(feature = "mcp-client"))]
                            return Err("MCP execution requires the 'mcp-client' feature".into());
                        }
                        if manifest.http.is_some() {
                            Self::execute_http_backend(
                                &name,
                                manifest,
                                &snapshot.validated,
                                timeout,
                                &snapshot.boundary,
                                Some(effect_journal.as_ref().ok_or(
                                    "governed HTTP effects require an active audited dispatcher",
                                )?),
                            )
                            .await
                        } else {
                            Self::execute_shell_backend(
                                &name,
                                manifest,
                                snapshot.argv.as_deref().ok_or("missing prepared argv")?,
                                timeout,
                                &snapshot.boundary,
                                &files,
                            )
                            .await
                        }
                    })
                    .await
            })
            .await;
            let (content, is_error) = match result {
                Ok(value) => {
                    let failed =
                        value.get("status").and_then(|status| status.as_str()) == Some("error");
                    (serde_json::to_string(&value).unwrap_or_default(), failed)
                }
                Err(error) => (format!("ToolClad error: {error}"), true),
            };
            observations.push(Observation {
                source: format!("toolclad:{name}"),
                content,
                is_error,
                call_id: Some(call_id),
                metadata: HashMap::new(),
            });
        }
        observations
    }

    async fn execute_actions(
        &self,
        actions: &[ProposedAction],
        config: &LoopConfig,
        _circuit_breakers: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        let mut observations = Vec::new();

        for action in actions {
            if let ProposedAction::ToolCall {
                call_id,
                name,
                arguments,
            } = action
            {
                if !self.handles(name) {
                    continue; // Not a ToolClad tool — skip
                }

                let base = name.split('.').next().unwrap_or(name);
                let requires_approval = self.manifests.get(base).is_some_and(|manifest| {
                    manifest.tool.human_approval
                        || name.split_once('.').is_some_and(|(_, command)| {
                            manifest
                                .session
                                .as_ref()
                                .and_then(|s| s.commands.get(command))
                                .is_some_and(|c| c.human_approval)
                                || manifest
                                    .browser
                                    .as_ref()
                                    .and_then(|s| s.commands.get(command))
                                    .is_some_and(|c| c.human_approval)
                        })
                });
                if requires_approval {
                    observations.push(Observation::tool_error(name, "tool requires an authorized exact-call approval; use governed dispatch").with_call_id(call_id.clone()));
                    continue;
                }

                let is_mcp_tool = self
                    .manifests
                    .get(name.as_str())
                    .map(|m| m.mcp.is_some())
                    .unwrap_or(false);
                let is_http_tool = self.manifests.get(name).is_some_and(|m| m.http.is_some());

                // Dispatch to appropriate executor based on tool type. MCP
                // tools are awaited directly here rather than going through
                // the sync `execute_tool` -> `block_on` bridge, since this
                // loop already runs on an async runtime.
                let result = if self.session_executor.handles(name) {
                    match self.prepare_action(action, config) {
                        Ok(prepared) => {
                            let snapshot = prepared
                                .backend::<ToolCladSnapshot>()
                                .expect("prepared ToolClad snapshot");
                            let now = std::time::Instant::now();
                            let run = format!("sdk:{}", self.executor_id);
                            let budget =
                                Duration::from_secs(snapshot.manifest.tool.timeout_seconds)
                                    .min(config.tool_timeout);
                            match (
                                now.checked_add(budget),
                                self.session_executor.sdk_run_deadline(config.timeout),
                            ) {
                                (Some(deadline), Ok(run_deadline)) => {
                                    self.session_executor
                                        .execute_prepared(super::session_executor::SessionCall {
                                            tool: name,
                                            manifest: &snapshot.manifest,
                                            command: snapshot
                                                .validated
                                                .get("command")
                                                .expect("validated terminal command"),
                                            boundary: &snapshot.boundary,
                                            files: &snapshot.files,
                                            contract: &prepared
                                                .contract()
                                                .expect("terminal contract")
                                                .digest,
                                            run: &run,
                                            binding: &run,
                                            run_deadline,
                                            deadline,
                                        })
                                        .await
                                }
                                (_, Err(error)) => Err(error),
                                _ => Err("terminal deadline overflow".into()),
                            }
                        }
                        Err(error) => Err(error),
                    }
                } else if self.browser_executor.handles(name) {
                    self.browser_executor
                        .execute_browser_command(name, arguments)
                } else if is_mcp_tool {
                    match self.parse_and_validate(name, arguments) {
                        Ok((manifest, validated)) => {
                            // Bound the MCP call: it spawns a subprocess and does
                            // a stdio handshake with no inherent deadline, so a
                            // hung/slow server would otherwise hang the caller
                            // indefinitely (the DSL `tool_call()` path has no
                            // outer timeout). Use the tool's declared deadline,
                            // capped by config; an exhausted budget never spawns.
                            let call_timeout = Duration::from_secs(manifest.tool.timeout_seconds)
                                .min(config.tool_timeout);
                            match tokio::time::timeout(
                                call_timeout,
                                self.execute_mcp_backend_async(name, manifest, &validated),
                            )
                            .await
                            {
                                Ok(r) => r,
                                Err(_) => Err(format!(
                                    "MCP tool '{}' timed out after {:?}",
                                    name, call_timeout
                                )),
                            }
                        }
                        Err(e) => Err(e),
                    }
                } else {
                    async {
                        let (manifest, validated) = self.parse_and_validate(name, arguments)?;
                        let boundary = self.boundary.as_ref().map_err(Clone::clone)?;
                        let timeout = Duration::from_secs(manifest.tool.timeout_seconds)
                            .min(config.tool_timeout);
                        if let Some(source) =
                            Self::prepare_source(manifest, boundary, &validated, timeout)?
                        {
                            source
                                .execute_prepared(name, std::time::Instant::now() + timeout)
                                .await
                        } else if is_http_tool {
                            Self::execute_http_backend(
                                name, manifest, &validated, timeout, boundary, None,
                            )
                            .await
                        } else {
                            let argv = build_argv(manifest, &validated)?;
                            let files = Self::prepare_files(manifest, boundary, &validated)?;
                            Self::execute_shell_backend(
                                name, manifest, &argv, timeout, boundary, &files,
                            )
                            .await
                        }
                    }
                    .await
                };

                let (content, is_error) = match result {
                    Ok(envelope) => (
                        serde_json::to_string_pretty(&envelope).unwrap_or_default(),
                        envelope.get("status").and_then(|s| s.as_str()) == Some("error"),
                    ),
                    Err(e) => (format!("ToolClad error: {}", e), true),
                };

                observations.push(Observation {
                    source: format!("toolclad:{}", name),
                    content,
                    is_error,
                    call_id: Some(call_id.clone()),
                    metadata: HashMap::new(),
                });
            }
        }

        observations
    }

    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tool_defs.clone()
    }
}

// ---- Output Parsing ----

/// Parse raw tool output based on the manifest's `output.format` and `output.parser` fields.
///
/// Custom parsers must be explicitly enabled by the operator:
/// - `output.parser` must start with `custom:` to request a non-builtin parser.
/// - The substring after `custom:` must match an entry in the
///   `SYMBIONT_TOOLCLAD_ALLOWED_PARSERS` env var (colon-separated list of
///   absolute paths). Without this allowlist, custom parsers are refused.
///
/// This closes a trivial RCE: before the allowlist, any manifest could set
/// `output.parser = "/any/path/to/binary"` and cause the runtime to exec it.
fn parse_output(
    manifest: &Manifest,
    raw_output: &str,
    _timeout: Duration,
) -> Result<serde_json::Value, String> {
    let default_parser = match manifest.output.format.as_str() {
        "json" => "builtin:json",
        "xml" => "builtin:xml",
        "csv" => "builtin:csv",
        "jsonl" => "builtin:jsonl",
        _ => "builtin:text",
    };
    let parser = manifest.output.parser.as_deref().unwrap_or(default_parser);

    match parser {
        "builtin:json" => parse_json(raw_output),
        "builtin:xml" => parse_xml(raw_output),
        "builtin:csv" => parse_csv(raw_output),
        "builtin:jsonl" => parse_jsonl(raw_output),
        "builtin:text" => Ok(serde_json::json!({"raw_output": raw_output})),
        other => {
            // Everything that isn't a known builtin must explicitly opt into
            // the custom parser path and pass the allowlist check.
            let Some(path) = other.strip_prefix("custom:") else {
                return Err(format!(
                    "Unknown parser '{}' — expected one of builtin:json|xml|csv|jsonl|text, \
                     or 'custom:<path>' with <path> present in SYMBIONT_TOOLCLAD_ALLOWED_PARSERS",
                    other
                ));
            };
            let path = path.trim();
            if !is_custom_parser_allowed(path) {
                return Err(format!(
                    "Custom parser '{}' is not in SYMBIONT_TOOLCLAD_ALLOWED_PARSERS — refusing to exec",
                    path
                ));
            }
            Err("custom parser requires the selected command boundary".into())
        }
    }
}

/// Returns true iff `path` is present in the colon-separated
/// `SYMBIONT_TOOLCLAD_ALLOWED_PARSERS` env var AND is an absolute path.
///
/// Relative paths are rejected: they would be resolved against the runtime's
/// working directory, which can drift, and make path-confusion attacks easy.
fn is_custom_parser_allowed(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    if !std::path::Path::new(path).is_absolute() {
        tracing::warn!(
            "ToolClad custom parser path {:?} is not absolute; refusing",
            path
        );
        return false;
    }
    let Ok(list) = std::env::var("SYMBIONT_TOOLCLAD_ALLOWED_PARSERS") else {
        tracing::warn!(
            "ToolClad manifest requested custom parser {:?} but SYMBIONT_TOOLCLAD_ALLOWED_PARSERS is unset",
            path
        );
        return false;
    };
    list.split(':').any(|entry| entry.trim() == path)
}

/// Parse raw output as JSON.
fn parse_json(raw_output: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(raw_output).map_err(|e| format!("Failed to parse output as JSON: {}", e))
}

/// Parse raw output as XML (placeholder — wraps as string since full XML-to-JSON
/// conversion would require a crate like `quick-xml`).
fn parse_xml(raw_output: &str) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "xml_output": raw_output,
        "_note": "Basic XML wrapping; install quick-xml for full XML-to-JSON conversion"
    }))
}

/// Parse raw output as CSV: first line is headers, subsequent lines are data rows.
/// Returns an array of objects.
fn parse_csv(raw_output: &str) -> Result<serde_json::Value, String> {
    let mut lines = raw_output.lines();

    let header_line = lines.next().ok_or("CSV output is empty — no header row")?;
    let headers: Vec<&str> = header_line.split(',').map(|h| h.trim()).collect();

    let mut rows = Vec::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let values: Vec<&str> = line.split(',').map(|v| v.trim()).collect();
        let mut row = serde_json::Map::new();
        for (i, header) in headers.iter().enumerate() {
            let value = values.get(i).copied().unwrap_or("");
            row.insert(
                header.to_string(),
                serde_json::Value::String(value.to_string()),
            );
        }
        rows.push(serde_json::Value::Object(row));
    }

    Ok(serde_json::Value::Array(rows))
}

/// Parse raw output as JSON Lines: each line is a separate JSON value.
/// Returns an array of parsed values.
fn parse_jsonl(raw_output: &str) -> Result<serde_json::Value, String> {
    let mut items = Vec::new();
    for (i, line) in raw_output.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line)
            .map_err(|e| format!("Failed to parse JSONL line {}: {}", i + 1, e))?;
        items.push(value);
    }
    Ok(serde_json::Value::Array(items))
}

async fn parse_output_with_boundary(
    manifest: &Manifest,
    raw_output: &str,
    timeout: Duration,
    boundary: &CommandBoundary,
) -> Result<serde_json::Value, String> {
    if let Some(path) = manifest
        .output
        .parser
        .as_deref()
        .and_then(|p| p.strip_prefix("custom:"))
    {
        let path = path.trim();
        if !is_custom_parser_allowed(path) {
            return Err(format!(
                "Custom parser '{path}' is not in SYMBIONT_TOOLCLAD_ALLOWED_PARSERS"
            ));
        }
        let output = boundary.parse(path, raw_output, timeout).await?;
        if !output.success {
            return Err(format!(
                "Custom parser '{path}' exited with {}: {}",
                output.exit_code,
                output.stderr.trim()
            ));
        }
        return serde_json::from_str(output.stdout.trim())
            .map_err(|e| format!("Custom parser '{path}' produced invalid JSON: {e}"));
    }
    parse_output(manifest, raw_output, timeout)
}

// ---- Output Schema Validation ----

/// Validate parsed output against the manifest's output schema.
/// Returns a list of warnings (never fails — partial results are OK).
fn validate_output_schema(parsed: &serde_json::Value, schema: &serde_json::Value) -> Vec<String> {
    let mut warnings = Vec::new();

    // If schema has no properties defined, skip validation
    let properties = match schema.get("properties").and_then(|p| p.as_object()) {
        Some(props) => props,
        None => return warnings,
    };

    // If parsed output is wrapped as raw_output, skip property checks
    if parsed.get("raw_output").is_some() {
        return warnings;
    }

    // Check required properties
    let required: Vec<&str> = schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    for key in required {
        if parsed.get(key).is_none() {
            warnings.push(format!(
                "Required property '{}' missing from parsed output",
                key
            ));
        }
    }

    // Check declared properties exist and types match
    for (key, prop_schema) in properties {
        if let Some(value) = parsed.get(key) {
            if let Some(expected_type) = prop_schema.get("type").and_then(|t| t.as_str()) {
                let type_ok = match expected_type {
                    "string" => value.is_string(),
                    "number" => value.is_number(),
                    "integer" => value.is_i64() || value.is_u64(),
                    "boolean" => value.is_boolean(),
                    "array" => value.is_array(),
                    "object" => value.is_object(),
                    "null" => value.is_null(),
                    _ => true, // Unknown type — don't warn
                };
                if !type_ok {
                    warnings.push(format!(
                        "Property '{}' has type '{}' but expected '{}'",
                        key,
                        json_type_name(value),
                        expected_type
                    ));
                }
            }
        }
    }

    warnings
}

/// Return a human-readable type name for a JSON value.
fn json_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Parse trusted command syntax before inserting argument values. Each value
/// stays in its original argv element, including quotes, spaces, and backslashes.
fn build_argv(manifest: &Manifest, args: &HashMap<String, String>) -> Result<Vec<String>, String> {
    let template = manifest
        .command
        .template
        .as_ref()
        .ok_or("No command template defined (and no custom executor)")?;

    let mut result = template.clone();

    // Apply defaults
    for (key, val) in &manifest.command.defaults {
        let placeholder = format!("{{{}}}", key);
        if result.contains(&placeholder) && !args.contains_key(key) {
            result = result.replace(&placeholder, val.to_string().trim_matches('"'));
        }
    }

    // Apply mappings — e.g., scan_type -> _scan_flags
    for (arg_name, mapping) in &manifest.command.mappings {
        if let Some(arg_value) = args.get(arg_name) {
            if let Some(flags) = mapping.get(arg_value) {
                // Convention: _{arg_name}_flags or _scan_flags
                let mapped_var = format!("{{_{}_flags}}", arg_name);
                result = result.replace(&mapped_var, flags);
                // Also try the generic _scan_flags pattern
                result = result.replace("{_scan_flags}", flags);
            }
        }
    }

    // Apply conditionals
    for (cond_name, cond_def) in &manifest.command.conditionals {
        let placeholder = format!("{{_{}}}", cond_name);
        if evaluate_condition(&cond_def.when, args) {
            result = result.replace(&placeholder, &cond_def.template);
        } else {
            result = result.replace(&placeholder, "");
        }
    }

    // Only operator-authored fragments have been expanded so far. Model input
    // must never participate in quote parsing or create additional argv elements.
    let tokens = split_command_to_argv(&result)?;
    let mut values = args.clone();
    values.insert(
        "_scan_id".into(),
        chrono::Utc::now().timestamp().to_string(),
    );
    values.insert("_output_file".into(), "/dev/null".into());
    values.insert("_evidence_dir".into(), "/tmp/evidence".into());

    if values
        .keys()
        .any(|key| tokens[0].contains(&format!("{{{key}}}")))
    {
        return Err("Command executable must be fixed by the manifest".into());
    }
    // Empty values must retain their slots: dropping an option's empty value
    // could turn the next operand into that option's value. Optional syntax
    // must be omitted explicitly through a trusted conditional fragment.
    Ok(tokens
        .into_iter()
        .map(|token| interpolate_argument(&token, &values))
        .collect())
}

fn interpolate_argument(template: &str, values: &HashMap<String, String>) -> String {
    let mut output = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        output.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('}') else {
            output.push_str(&rest[start..]);
            return output;
        };
        let end = start + end;
        let key = &rest[start + 1..end];
        match values.get(key) {
            Some(value) => output.push_str(value),
            None => output.push_str(&rest[start..=end]),
        }
        rest = &rest[end + 1..];
    }
    output.push_str(rest);
    output
}

/// Simple condition evaluator for `when` expressions.
fn evaluate_condition(when: &str, args: &HashMap<String, String>) -> bool {
    // Support: "argname != ''" and "argname == 'value'" and "argname != 0"
    let when = when.trim();

    if when.contains(" and ") {
        return when
            .split(" and ")
            .all(|part| evaluate_condition(part, args));
    }

    if when.contains("!=") {
        let parts: Vec<&str> = when.splitn(2, "!=").collect();
        let key = parts[0].trim();
        let expected = parts[1].trim().trim_matches('\'').trim_matches('"');
        let actual = args.get(key).map(|s| s.as_str()).unwrap_or("");
        return actual != expected;
    }

    if when.contains("==") {
        let parts: Vec<&str> = when.splitn(2, "==").collect();
        let key = parts[0].trim();
        let expected = parts[1].trim().trim_matches('\'').trim_matches('"');
        let actual = args.get(key).map(|s| s.as_str()).unwrap_or("");
        return actual == expected;
    }

    false
}

/// Resolve the same destination before policy evaluation and before dispatch.
/// Credentials belong in explicit headers/body, not implicit URL userinfo.
fn resolve_http_destination(
    http: &super::manifest::HttpDef,
    args: &HashMap<String, String>,
) -> Result<(reqwest::Method, String), String> {
    if http.url.contains("{_secret:") {
        return Err("HTTP secret placeholders must use headers or the request body".into());
    }
    let method = match http.method.to_ascii_uppercase().as_str() {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "DELETE" => reqwest::Method::DELETE,
        "PATCH" => reqwest::Method::PATCH,
        "HEAD" => reqwest::Method::HEAD,
        other => return Err(format!("Unsupported HTTP method: {other}")),
    };
    let value = render_http_template(&http.url, args)?;
    if value.len() > 8192 || value.bytes().any(|byte| byte.is_ascii_control()) {
        return Err("HTTP URL contains control characters or exceeds 8 KiB".into());
    }
    let parsed = url::Url::parse(&value).map_err(|e| format!("Invalid HTTP URL: {e}"))?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("HTTP URL credentials require explicit headers instead".into());
    }
    let url = parsed.to_string();
    crate::net_guard::reject_ssrf_url(&url)?;
    Ok((method, url))
}

/// Expand only placeholders present in the operator's HTTP template.
/// Argument and secret values are data and are never scanned for placeholders.
fn render_http_template(template: &str, args: &HashMap<String, String>) -> Result<String, String> {
    let pattern =
        regex::Regex::new(r"\{(_secret:)?([a-zA-Z0-9_]+)\}").expect("valid template pattern");
    let mut output = String::new();
    let mut end = 0;
    for capture in pattern.captures_iter(template) {
        let span = capture.get(0).expect("matched placeholder");
        output.push_str(&template[end..span.start()]);
        if capture.get(1).is_some() {
            let name = &capture[2];
            let key = format!("TOOLCLAD_SECRET_{}", name.to_uppercase());
            let value = std::env::var(&key)
                .map_err(|_| format!("Secret '{name}' not found (set {key})"))?;
            output.push_str(&value);
        } else {
            output.push_str(
                args.get(&capture[2])
                    .map(String::as_str)
                    .unwrap_or(span.as_str()),
            );
        }
        end = span.end();
    }
    output.push_str(&template[end..]);
    Ok(output)
}

/// Split a command string into argv (program + arguments).
///
/// Handles single and double quoting so that arguments containing spaces
/// are preserved as a single element. Does NOT invoke a shell — this
/// prevents shell metacharacter injection.
fn split_command_to_argv(command: &str) -> Result<Vec<String>, String> {
    let mut argv = Vec::new();
    let mut current = String::new();
    let mut chars = command.chars().peekable();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut started = false;

    while let Some(c) = chars.next() {
        match c {
            '\'' if !in_double_quote => {
                in_single_quote = !in_single_quote;
                started = true;
            }
            '"' if !in_single_quote => {
                in_double_quote = !in_double_quote;
                started = true;
            }
            '\\' if !in_single_quote => {
                if let Some(next) = chars.next() {
                    current.push(next);
                    started = true;
                } else {
                    return Err("Trailing escape in command template".into());
                }
            }
            c if c.is_whitespace() && !in_single_quote && !in_double_quote => {
                if started {
                    argv.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            _ => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        argv.push(current);
    }
    if in_single_quote || in_double_quote {
        return Err("Unterminated quote in command template".to_string());
    }
    if argv.is_empty() || argv[0].is_empty() {
        return Err("Empty command after template interpolation".to_string());
    }
    Ok(argv)
}

/// Generate MCP-compatible ToolDefinitions from a manifest.
/// Oneshot tools produce one definition. Session/browser tools produce
/// one definition per declared command (e.g., "msfconsole_session.run").
fn generate_tool_definitions(manifest: &Manifest) -> Vec<ToolDefinition> {
    match manifest.tool.mode.as_str() {
        "session" => generate_session_tool_defs(manifest),
        "browser" => generate_browser_tool_defs(manifest),
        _ => vec![generate_oneshot_tool_def(manifest)],
    }
}

/// Generate tool definitions for session commands.
fn generate_session_tool_defs(manifest: &Manifest) -> Vec<ToolDefinition> {
    let session = match &manifest.session {
        Some(s) => s,
        None => return vec![generate_oneshot_tool_def(manifest)],
    };
    session
        .commands
        .iter()
        .map(|(cmd_name, cmd_def)| {
            let mut properties = serde_json::Map::new();
            properties.insert(
                "command".to_string(),
                serde_json::json!({
                    "type": "string",
                    "description": format!("Command matching pattern: {}", cmd_def.pattern)
                }),
            );
            for (arg_name, arg_def) in &cmd_def.args {
                let mut prop = serde_json::Map::new();
                prop.insert("type".to_string(), serde_json::json!("string"));
                prop.insert(
                    "description".to_string(),
                    serde_json::json!(arg_def.description),
                );
                properties.insert(arg_name.clone(), serde_json::Value::Object(prop));
            }
            ToolDefinition {
                name: format!("{}.{}", manifest.tool.name, cmd_name),
                description: cmd_def.description.clone(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": properties,
                    "required": ["command"]
                }),
            }
        })
        .collect()
}

/// Generate tool definitions for browser commands.
fn generate_browser_tool_defs(manifest: &Manifest) -> Vec<ToolDefinition> {
    let browser = match &manifest.browser {
        Some(b) => b,
        None => return vec![generate_oneshot_tool_def(manifest)],
    };
    browser
        .commands
        .iter()
        .map(|(cmd_name, cmd_def)| {
            let mut properties = serde_json::Map::new();
            for (arg_name, arg_def) in &cmd_def.args {
                let mut prop = serde_json::Map::new();
                prop.insert("type".to_string(), serde_json::json!("string"));
                prop.insert(
                    "description".to_string(),
                    serde_json::json!(arg_def.description),
                );
                if let Some(allowed) = &arg_def.allowed {
                    prop.insert("enum".to_string(), serde_json::json!(allowed));
                }
                properties.insert(arg_name.clone(), serde_json::Value::Object(prop));
            }
            let required: Vec<_> = cmd_def
                .args
                .iter()
                .filter(|(_, d)| d.required)
                .map(|(n, _)| serde_json::json!(n))
                .collect();
            ToolDefinition {
                name: format!("{}.{}", manifest.tool.name, cmd_name),
                description: cmd_def.description.clone(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": properties,
                    "required": required
                }),
            }
        })
        .collect()
}

/// Generate a single MCP tool definition for a oneshot manifest.
fn generate_oneshot_tool_def(manifest: &Manifest) -> ToolDefinition {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();

    let mut sorted_args: Vec<_> = manifest.args.iter().collect();
    sorted_args.sort_by_key(|(_, def)| def.position);

    for (name, def) in &sorted_args {
        let mut prop = serde_json::Map::new();
        prop.insert("type".to_string(), serde_json::json!("string"));
        prop.insert(
            "description".to_string(),
            serde_json::json!(def.description),
        );
        if let Some(allowed) = &def.allowed {
            prop.insert("enum".to_string(), serde_json::json!(allowed));
        }
        if let Some(default) = &def.default {
            prop.insert(
                "default".to_string(),
                serde_json::json!(default.to_string().trim_matches('"')),
            );
        }
        properties.insert(name.to_string(), serde_json::Value::Object(prop));
        if def.required {
            required.push(serde_json::json!(name));
        }
    }

    let parameters = serde_json::json!({
        "type": "object",
        "properties": properties,
        "required": required
    });

    ToolDefinition {
        name: manifest.tool.name.clone(),
        description: manifest.tool.description.clone(),
        parameters,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_preparation_preserves_policy_and_argv_values_through_custom_types() {
        let manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "path_fixture"
version = "1"
description = "Worker path fixture"
[args.path]
position = 1
type = "worker_path"
required = true
[command]
template = "cat '{path}'"
[output]
format = "text"
"#,
        )
        .unwrap();
        let mut executor = ToolCladExecutor::new(vec![("path_fixture".into(), manifest)]);
        executor.custom_types.insert(
            "worker_path".into(),
            ArgDef {
                type_name: "path".into(),
                ..Default::default()
            },
        );
        let config = LoopConfig {
            tool_definitions: executor.tool_definitions(),
            ..Default::default()
        };
        let prepare = |path: &str| {
            executor.prepare_action(
                &ProposedAction::ToolCall {
                    call_id: "path".into(),
                    name: "path_fixture".into(),
                    arguments: serde_json::json!({"path": path}).to_string(),
                },
                &config,
            )
        };
        // This relative file exists in the controller checkout. Its host
        // canonical location must never replace the worker's argument.
        assert!(std::path::Path::new("Cargo.toml").is_file());
        let prepared = prepare("Cargo.toml").unwrap();
        assert_eq!(prepared.policy_context()["arguments"]["path"], "Cargo.toml");
        assert_eq!(
            prepared.policy_context()["resolved"]["argv"],
            serde_json::json!(["cat", "Cargo.toml"])
        );
        assert!(prepare("/workspace/Cargo.toml").is_err());
        assert!(prepare("data/..").is_err());
    }

    #[cfg(feature = "cedar")]
    #[tokio::test]
    async fn http_preparation_exposes_client_destination_to_cedar() {
        use crate::reasoning::{
            conversation::Conversation,
            loop_types::{LoopDecision, LoopState},
            policy_bridge::ReasoningPolicyGate,
            CedarPolicy, CedarPolicyGate,
        };
        let manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "http_fixture"
version = "1"
description = "HTTP canonical destination fixture"
[args.url]
position = 1
type = "url"
required = true
[http]
method = "get"
url = "{url}"
[output]
format = "json"
"#,
        )
        .unwrap();
        let executor = ToolCladExecutor::new(vec![("http_fixture".into(), manifest)]);
        let config = LoopConfig {
            tool_definitions: executor.tool_definitions(),
            ..Default::default()
        };
        let prepare = |url: &str| {
            executor.prepare_action(
                &ProposedAction::ToolCall {
                    call_id: "destination".into(),
                    name: "http_fixture".into(),
                    arguments: serde_json::json!({"url":url}).to_string(),
                },
                &config,
            )
        };
        let gate = CedarPolicyGate::deny_by_default();
        gate.add_policy(CedarPolicy {
            name:"destination".into(),active:true,
            source:r#"permit(principal, action == Action::"tool_call::http_fixture", resource) when { context.invocation.resolved.http_method == "GET" && context.invocation.resolved.http_url == "https://example.com/public/" };"#.into(),
        }).await;
        let state = LoopState::new(crate::types::AgentId::new(), Conversation::new());
        let allowed = prepare("HTTPS://EXAMPLE.COM:443/public/").unwrap();
        assert!(matches!(
            gate.evaluate_prepared(&state.agent_id, &allowed, &state)
                .await,
            LoopDecision::Allow
        ));
        let changed = prepare("https://example.com/public/../admin").unwrap();
        assert_eq!(
            changed.policy_context()["resolved"]["http_url"],
            "https://example.com/admin"
        );
        assert!(matches!(
            gate.evaluate_prepared(&state.agent_id, &changed, &state)
                .await,
            LoopDecision::Deny { .. }
        ));
        assert!(prepare("https://user:password@example.com/public/").is_err());
    }

    #[test]
    fn browser_navigation_binds_the_canonical_url_before_authorization() {
        let manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "browser_fixture"
version = "1"
mode = "browser"
description = "Browser preparation fixture"
[browser.scope]
allowed_domains = ["example.com", "127.0.0.1"]
blocked_domains = ["admin.example.com"]
[browser.commands.navigate]
description = "Navigate"
human_approval = true
[browser.commands.navigate.args.url]
position = 0
required = true
type = "url"
[output]
format = "json"
"#,
        )
        .unwrap();
        let executor = ToolCladExecutor::new(vec![("browser_fixture".into(), manifest.clone())]);
        let config = LoopConfig {
            tool_definitions: executor.tool_definitions(),
            ..Default::default()
        };
        let prepare = |url: &str| {
            executor.prepare_action(
                &ProposedAction::ToolCall {
                    call_id: "navigation".into(),
                    name: "browser_fixture.navigate".into(),
                    arguments: serde_json::json!({"url": url}).to_string(),
                },
                &config,
            )
        };
        let canonical = prepare("https://example.com/page").unwrap();
        let alternate = prepare("HTTPS://EXAMPLE.COM:443/path/../page").unwrap();
        assert_eq!(canonical.fingerprint(), alternate.fingerprint());
        assert_eq!(
            alternate.policy_context()["arguments"]["url"],
            "https://example.com/page"
        );
        assert!(alternate.contract().unwrap().requires_approval);
        for denied in [
            "https://example.com@outside.invalid/",
            "https://user:password@example.com/",
            "https://ADMIN.EXAMPLE.COM./",
            "https://admin%2eexample.com/",
            "https://example.com\\@outside.invalid/",
            "file:///etc/passwd",
            "http://127.0.0.1:8181/",
        ] {
            assert!(prepare(denied).is_err(), "accepted {denied}");
        }
        let mut private_manifest = manifest;
        private_manifest.browser.as_mut().unwrap().network =
            Some(super::super::manifest::BrowserNetworkDef {
                private_origins: vec!["http://127.0.0.1:8181".into()],
                ..Default::default()
            });
        let private_executor =
            ToolCladExecutor::new(vec![("browser_fixture".into(), private_manifest.clone())]);
        let action = ProposedAction::ToolCall {
            call_id: "private-navigation".into(),
            name: "browser_fixture.navigate".into(),
            arguments: serde_json::json!({"url":"http://127.0.0.1:8181/"}).to_string(),
        };
        let authorized_target = private_executor.prepare_action(&action, &config).unwrap();
        assert_eq!(
            authorized_target.policy_context()["resolved"]["browser_network"]["private_origins"],
            serde_json::json!(["http://127.0.0.1:8181"])
        );
        assert!(authorized_target.contract().unwrap().requires_approval);
        private_manifest
            .browser
            .as_mut()
            .unwrap()
            .network
            .as_mut()
            .unwrap()
            .allowed_methods
            .push("POST".into());
        let changed_executor =
            ToolCladExecutor::new(vec![("browser_fixture".into(), private_manifest)]);
        let changed = changed_executor.prepare_action(&action, &config).unwrap();
        assert_ne!(
            authorized_target.contract().unwrap().digest,
            changed.contract().unwrap().digest
        );
    }

    #[cfg(feature = "mcp-client")]
    #[tokio::test]
    async fn registry_helper_uses_registered_contract_approval_and_argument_rules() {
        let mut manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "mcp_fixture"
version = "1"
description = "Registry helper contract fixture"
[args.count]
position = 1
required = true
type = "integer"
min = 1
max = 5
[mcp]
server = "fixture"
tool = "count"
[output]
format = "json"
"#,
        )
        .unwrap();
        let registry = crate::integrations::mcp::registry::McpServerRegistry::from_toml_str(
            "[servers.fixture]\ncommand = '/missing/forbidden-worker'",
        )
        .unwrap();
        let executor = ToolCladExecutor::new(vec![("mcp_fixture".into(), manifest.clone())]);
        let invalid = HashMap::from([("count".into(), "not-an-integer".into())]);
        let error = executor
            .execute_mcp_backend_async_with_registry(&registry, "mcp_fixture", &manifest, &invalid)
            .await
            .unwrap_err();
        assert!(error.contains("Validation failed"), "{error}");
        let mut replacement = manifest.clone();
        replacement.mcp.as_mut().unwrap().tool = "replacement".into();
        let error = executor
            .execute_mcp_backend_async_with_registry(
                &registry,
                "mcp_fixture",
                &replacement,
                &HashMap::from([("count".into(), "5".into())]),
            )
            .await
            .unwrap_err();
        assert!(error.contains("registered tool contract"), "{error}");
        manifest.tool.human_approval = true;
        let executor = ToolCladExecutor::new(vec![("mcp_fixture".into(), manifest)]);
        replacement.tool.human_approval = false;
        let error = executor
            .execute_mcp_backend_async_with_registry(
                &registry,
                "mcp_fixture",
                &replacement,
                &HashMap::from([("count".into(), "5".into())]),
            )
            .await
            .unwrap_err();
        assert!(
            error.contains("requires an authorized exact-call approval"),
            "{error}"
        );
    }

    #[test]
    fn scope_is_mandatory_before_effects_and_allowed_work_still_executes() {
        let manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "scope_fixture"
version = "1"
binary = "/usr/bin/printf"
description = "Scoped argument fixture"
[args.target]
position = 1
required = true
type = "scope_target"
[command]
template = "/usr/bin/printf '%s' '{target}'"
[output]
format = "text"
"#,
        )
        .unwrap();
        let executor = ToolCladExecutor::new(vec![("scope_fixture".into(), manifest)])
            .with_development_host_execution();
        assert!(executor
            .execute_tool("scope_fixture", r#"{"target":"example.com"}"#)
            .unwrap_err()
            .contains("requires a configured project scope"));
        let executor = executor.with_scope(super::super::scope::Scope {
            domains: vec!["example.com".into()],
            ..Default::default()
        });
        assert!(executor
            .execute_tool("scope_fixture", r#"{"target":"outside.example"}"#)
            .unwrap_err()
            .contains("Scope denied"));
        assert!(executor
            .execute_tool(
                "scope_fixture",
                r#"{"target":"example.com", "extra":"ignored"}"#
            )
            .unwrap_err()
            .contains("Unknown argument"));
        let result = executor
            .execute_tool("scope_fixture", r#"{"target":"example.com"}"#)
            .unwrap();
        assert_eq!(result["results"]["raw_output"], "example.com");
    }

    #[test]
    fn http_arguments_cannot_introduce_secret_or_argument_placeholders() {
        let args = HashMap::from([
            (
                "query".into(),
                "{_secret:unavailable_test_key}/{other}".into(),
            ),
            ("other".into(), "unexpected expansion".into()),
        ]);
        assert_eq!(
            render_http_template("https://example.com/{query}/{missing}", &args).unwrap(),
            "https://example.com/{_secret:unavailable_test_key}/{other}/{missing}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn http_manifest_secret_is_expanded_once() {
        let key = "TOOLCLAD_SECRET_HTTP_TEMPLATE_FIXTURE";
        let saved = std::env::var_os(key);
        std::env::set_var(key, "literal-{other}-{_secret:unavailable_test_key}");
        let result = render_http_template(
            "Bearer {_secret:http_template_fixture}",
            &HashMap::from([("other".into(), "unexpected expansion".into())]),
        );
        match saved {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
        assert_eq!(
            result.unwrap(),
            "Bearer literal-{other}-{_secret:unavailable_test_key}"
        );
    }

    #[test]
    fn http_dispatch_rejects_obfuscated_private_targets_before_connecting() {
        let manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "http_fixture"
version = "1"
description = "HTTP destination fixture"
[args.url]
position = 1
required = true
type = "url"
[http]
method = "GET"
url = "{url}"
[output]
format = "text"
"#,
        )
        .unwrap();
        for url in [
            "http://2130706433/",
            "http://[::ffff:127.0.0.1]/",
            "http://[::10.1.2.3]/",
        ] {
            let mut fixture = manifest.clone();
            fixture.args.clear();
            fixture.http.as_mut().unwrap().url = url.into();
            let executor = ToolCladExecutor::new(vec![("http_fixture".into(), fixture)])
                .with_development_host_execution();
            let error = executor.execute_tool("http_fixture", "{}").unwrap_err();
            assert!(error.contains("SSRF"), "{error}");
        }
        let mut secret_url = manifest;
        secret_url.args.clear();
        secret_url.http.as_mut().unwrap().url = "https://example.com/{_secret:fixture}".into();
        let executor = ToolCladExecutor::new(vec![("http_fixture".into(), secret_url)])
            .with_development_host_execution();
        assert!(executor
            .execute_tool("http_fixture", "{}")
            .unwrap_err()
            .contains("must use headers or the request body"));
    }

    #[tokio::test]
    async fn http_dispatch_reports_network_errors_without_a_nested_runtime() {
        let manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "http_fixture"
version = "1"
description = "Async HTTP worker fixture"
timeout_seconds = 30
[http]
method = "GET"
url = "http://example.invalid/"
[output]
format = "text"
"#,
        )
        .unwrap();
        let executor = ToolCladExecutor::new(vec![("http_fixture".into(), manifest)])
            .with_development_host_execution();
        // example.invalid is a reserved TLD, so resolution fails immediately and
        // the budget is never the thing under test. A 50ms budget expired before
        // dispatch under parallel load, producing a third error the assertion
        // below does not accept; give enough headroom that the network error is
        // what actually surfaces.
        let config = LoopConfig {
            tool_timeout: Duration::from_secs(5),
            ..Default::default()
        };
        let actions = [ProposedAction::ToolCall {
            call_id: "fixture-call".into(),
            name: "http_fixture".into(),
            arguments: "{}".into(),
        }];
        let observations = executor
            .execute_actions(&actions, &config, &CircuitBreakerRegistry::default())
            .await;
        assert_eq!(observations.len(), 1);
        assert!(observations[0].is_error);
        assert!(
            observations[0].content.contains("HTTP request failed")
                || observations[0]
                    .content
                    .contains("HTTP request deadline expired"),
            "{}",
            observations[0].content
        );
        assert!(!observations[0].content.contains("worker failed"));
        // A synchronous invocation from a runtime also fails explicitly,
        // instead of trying to start a nested synchronous runtime.
        assert!(executor
            .execute_tool("http_fixture", "{}")
            .unwrap_err()
            .contains("must use execute_actions"));
    }

    #[test]
    fn curl_headers_cannot_add_options_or_read_files() {
        let manifest: Manifest = toml::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tools/curl_fetch.clad.toml"
        )))
        .unwrap();
        let executor = ToolCladExecutor::new(vec![("curl_fetch".into(), manifest)])
            .with_development_host_execution()
            .with_scope(super::super::scope::Scope {
                domains: vec!["example.com".into()],
                ..Default::default()
            });
        for headers in [
            "@/tmp/fixture",
            "X-Test: good\r\nInjected: bad",
            "X-Test: good\0bad",
        ] {
            let json = serde_json::json!({"url":"https://example.com/", "headers":headers});
            assert!(executor
                .parse_and_validate("curl_fetch", &json.to_string())
                .is_err());
        }
        let header = "X-Test: ok' --output /tmp/unwanted -H 'X-End: ok";
        let json = serde_json::json!({"url":"https://example.com/", "headers":header});
        let (manifest, args) = executor
            .parse_and_validate("curl_fetch", &json.to_string())
            .unwrap();
        let argv = build_argv(manifest, &args).unwrap();
        let index = argv.iter().position(|s| s == "-H").unwrap();
        assert_eq!(argv[index + 1], header);
        assert!(!argv.iter().any(|s| s == "--output"));
        assert_eq!(&argv[argv.len() - 2..], ["--", "https://example.com/"]);
    }

    #[test]
    fn nmap_extra_flags_are_limited_to_fixed_options() {
        let manifest: Manifest = toml::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tools/nmap_scan.clad.toml"
        )))
        .unwrap();
        let executor = ToolCladExecutor::new(vec![("nmap_scan".into(), manifest)])
            .with_development_host_execution()
            .with_scope(super::super::scope::Scope {
                domains: vec!["example.com".into()],
                ..Default::default()
            });
        for extra in ["--script=/tmp/fixture.lua", "-n -Pn", "--datadir=/tmp"] {
            let json = serde_json::json!({
                "target": "example.com", "scan_type": "ping", "extra_flags": extra
            });
            assert!(executor
                .parse_and_validate("nmap_scan", &json.to_string())
                .is_err());
        }
        for extra in ["", "-n", "-Pn"] {
            let json = serde_json::json!({
                "target": "example.com", "scan_type": "ping", "extra_flags": extra
            });
            let (manifest, args) = executor
                .parse_and_validate("nmap_scan", &json.to_string())
                .unwrap();
            let argv = build_argv(manifest, &args).unwrap();
            assert_eq!(argv.last().unwrap(), "example.com");
            assert_eq!(argv.iter().any(|arg| arg == extra), !extra.is_empty());
        }
    }

    #[cfg(unix)]
    #[test]
    fn executable_receives_one_literal_argument() {
        let manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "argv_fixture"
version = "1"
binary = "/usr/bin/printf"
description = "Argument boundary fixture"
[args.message]
position = 1
required = true
type = "literal_text"
[command]
template = "/usr/bin/printf '%s' '{message}'"
[output]
format = "text"
"#,
        )
        .unwrap();
        let executor = ToolCladExecutor::new(vec![("argv_fixture".into(), manifest)])
            .with_development_host_execution();
        let message = "  \nalpha'  --output /tmp/unwanted  'omega\\tail\n  ";
        let result = executor
            .execute_tool(
                "argv_fixture",
                &serde_json::json!({"message":message}).to_string(),
            )
            .unwrap();
        assert_eq!(result["results"]["raw_output"], message);
    }

    #[cfg(unix)]
    #[test]
    fn empty_values_keep_argument_positions() {
        let manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "empty_fixture"
version = "1"
binary = "/usr/bin/printf"
description = "Empty argument fixture"
[args.left]
position = 1
required = false
type = "string"
[args.right]
position = 2
required = true
type = "string"
[command]
template = "/usr/bin/printf '%s|%s' '{left}' '{right}'"
[output]
format = "text"
"#,
        )
        .unwrap();
        let executor = ToolCladExecutor::new(vec![("empty_fixture".into(), manifest)])
            .with_development_host_execution();
        let result = executor
            .execute_tool("empty_fixture", r#"{"left":"","right":"-n"}"#)
            .unwrap();
        assert_eq!(result["results"]["raw_output"], "|-n");
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn custom_parser_uses_the_remaining_tool_deadline() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let parser = dir.path().join("parser");
        std::fs::write(
            &parser,
            "#!/bin/sh\nprintf started > \"$0.started\"\nsleep 5\n",
        )
        .unwrap();
        std::fs::set_permissions(&parser, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "parser_fixture"
version = "1"
binary = "/bin/sh"
description = "Parser deadline fixture"
timeout_seconds = 2
[command]
template = "/bin/sh -c 'sleep 1; printf raw'"
[output]
format = "json"
"#,
        )
        .unwrap();
        manifest.output.parser = Some(format!("custom:{}", parser.display()));
        let executor = ToolCladExecutor::new(vec![("parser_fixture".into(), manifest)])
            .with_development_host_execution();
        let key = "SYMBIONT_TOOLCLAD_ALLOWED_PARSERS";
        let previous = std::env::var_os(key);
        std::env::set_var(key, &parser);
        let started = std::time::Instant::now();
        let result = executor.execute_tool("parser_fixture", "{}");
        match previous {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
        let error = result.unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        assert!(dir.path().join("parser.started").exists());
        assert!(started.elapsed() < Duration::from_millis(2800));
    }

    #[test]
    fn trusted_template_quotes_and_empty_arguments_are_preserved() {
        assert_eq!(
            split_command_to_argv("echo '' \"\" 'two  spaces'").unwrap(),
            ["echo", "", "", "two  spaces"]
        );
        assert!(split_command_to_argv("echo trailing\\").is_err());
    }

    #[test]
    fn inserted_values_are_not_expanded_again() {
        let values = HashMap::from([
            ("first".into(), "{second}".into()),
            ("second".into(), "changed".into()),
        ]);
        assert_eq!(
            interpolate_argument("prefix={first}", &values),
            "prefix={second}"
        );
    }

    #[test]
    fn mappings_expand_trusted_flags_without_splitting_values() {
        let manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "scan"
version = "1"
binary = "scan"
description = "Mapping fixture"
[command]
template = "scan {_scan_flags} -- {target} {optional}"
[command.mappings.mode]
fast = "--fast --rate 100"
[output]
format = "text"
"#,
        )
        .unwrap();
        let args = HashMap::from([
            ("mode".into(), "fast".into()),
            ("target".into(), "target with spaces".into()),
            ("optional".into(), "".into()),
        ]);
        assert_eq!(
            build_argv(&manifest, &args).unwrap(),
            [
                "scan",
                "--fast",
                "--rate",
                "100",
                "--",
                "target with spaces",
                ""
            ]
        );
    }

    #[test]
    fn executable_cannot_be_selected_by_an_argument() {
        let manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "fixture"
version = "1"
binary = "fixture"
description = "Executable selection fixture"
[command]
template = "{program} value"
[output]
format = "text"
"#,
        )
        .unwrap();
        let args = HashMap::from([("program".into(), "/bin/sh".into())]);
        assert!(build_argv(&manifest, &args)
            .unwrap_err()
            .contains("executable must be fixed"));
    }

    #[test]
    fn test_build_simple_command() {
        let manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "echo_test"
version = "1.0.0"
binary = "echo"
description = "Test"

[args.message]
position = 1
required = true
type = "string"

[command]
template = "echo {message}"

[output]
format = "text"

[output.schema]
type = "object"
"#,
        )
        .unwrap();
        let mut args = HashMap::new();
        args.insert("message".to_string(), "hello".to_string());
        let cmd = build_argv(&manifest, &args).unwrap();
        assert_eq!(cmd, ["echo", "hello"]);
    }

    #[test]
    fn test_build_command_with_defaults() {
        let manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "test"
version = "1.0.0"
binary = "test"
description = "Test"

[args.target]
position = 1
required = true
type = "string"

[command]
template = "scan --rate {rate} {target}"

[command.defaults]
rate = 100

[output]
format = "text"

[output.schema]
type = "object"
"#,
        )
        .unwrap();
        let mut args = HashMap::new();
        args.insert("target".to_string(), "example.com".to_string());
        let cmd = build_argv(&manifest, &args).unwrap();
        assert_eq!(cmd, ["scan", "--rate", "100", "example.com"]);
    }

    #[test]
    fn test_execute_tool_shell_backend_succeeds_and_captures_stdout() {
        let manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "echo_test"
version = "1.0.0"
binary = "echo"
description = "Test"

[args.message]
position = 1
required = true
type = "string"

[command]
template = "echo {message}"

[output]
format = "text"

[output.schema]
type = "object"
"#,
        )
        .unwrap();

        let executor = ToolCladExecutor::new(vec![("echo_test".to_string(), manifest)])
            .with_development_host_execution();
        let result = executor
            .execute_tool("echo_test", r#"{"message": "hello-world"}"#)
            .expect("a fast, well-behaved command must still succeed");

        assert_eq!(result["status"], "success");
        assert_eq!(
            result["results"]["raw_output"],
            serde_json::Value::String("hello-world\n".to_string())
        );
    }

    #[test]
    fn test_execute_tool_shell_backend_times_out_instead_of_hanging() {
        // Regression test: the manifest's timeout_seconds used to be computed
        // into a discarded binding and never applied, so this would hang for
        // the full `sleep 5` instead of failing after ~1s.
        let manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "slow_test"
version = "1.0.0"
binary = "sleep"
description = "Test"
timeout_seconds = 1

[args.duration]
position = 1
required = true
type = "string"

[command]
template = "sleep {duration}"

[output]
format = "text"

[output.schema]
type = "object"
"#,
        )
        .unwrap();

        let executor = ToolCladExecutor::new(vec![("slow_test".to_string(), manifest)])
            .with_development_host_execution();
        let start = std::time::Instant::now();
        let result = executor.execute_tool("slow_test", r#"{"duration": "5"}"#);
        let elapsed = start.elapsed();

        let err = result.expect_err("a tool exceeding its timeout must return an error");
        assert!(
            err.contains("slow_test"),
            "timeout error should name the tool: {err}"
        );
        assert!(
            err.contains("timed out"),
            "timeout error should say it timed out: {err}"
        );
        assert!(
            elapsed < Duration::from_secs(4),
            "must return promptly after the 1s timeout instead of waiting out the full \
             5s sleep, took {:?}",
            elapsed
        );
    }

    #[test]
    fn test_generate_oneshot_tool_def() {
        let manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "whois"
version = "1.0.0"
binary = "whois"
description = "WHOIS lookup"

[args.target]
position = 1
required = true
type = "scope_target"
description = "Domain or IP"

[command]
template = "whois {target}"

[output]
format = "text"

[output.schema]
type = "object"
"#,
        )
        .unwrap();
        let td = generate_oneshot_tool_def(&manifest);
        assert_eq!(td.name, "whois");
        assert_eq!(td.description, "WHOIS lookup");
        let required = td.parameters["required"].as_array().unwrap();
        assert!(required.contains(&serde_json::json!("target")));
    }

    // ---- Parser Tests ----

    #[test]
    fn test_parse_json_valid() {
        let result = parse_json(r#"{"key": "value", "count": 42}"#).unwrap();
        assert_eq!(result["key"], "value");
        assert_eq!(result["count"], 42);
    }

    #[test]
    fn test_parse_json_invalid() {
        let result = parse_json("not json at all");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_csv_basic() {
        let csv = "name,age,city\nAlice,30,NYC\nBob,25,LA";
        let result = parse_csv(csv).unwrap();
        let rows = result.as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["name"], "Alice");
        assert_eq!(rows[0]["age"], "30");
        assert_eq!(rows[1]["city"], "LA");
    }

    #[test]
    fn test_parse_csv_empty_body() {
        let csv = "name,age";
        let result = parse_csv(csv).unwrap();
        let rows = result.as_array().unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn test_parse_csv_no_header() {
        let result = parse_csv("");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_jsonl_valid() {
        let jsonl = r#"{"a":1}
{"b":2}
{"c":3}"#;
        let result = parse_jsonl(jsonl).unwrap();
        let items = result.as_array().unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0]["a"], 1);
        assert_eq!(items[2]["c"], 3);
    }

    #[test]
    fn test_parse_jsonl_with_blanks() {
        let jsonl = r#"{"a":1}

{"b":2}
"#;
        let result = parse_jsonl(jsonl).unwrap();
        let items = result.as_array().unwrap();
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn test_parse_jsonl_invalid_line() {
        let jsonl = "{\"a\":1}\nnot json";
        let result = parse_jsonl(jsonl);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("line 2"));
    }

    #[test]
    fn test_parse_xml_wraps() {
        let xml = "<root><item>hello</item></root>";
        let result = parse_xml(xml).unwrap();
        assert_eq!(result["xml_output"], xml);
        assert!(result.get("_note").is_some());
    }

    #[test]
    fn test_parse_output_default_text() {
        let manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "test"
version = "1.0.0"
binary = "test"
description = "Test"

[command]
template = "test"

[output]
format = "text"

[output.schema]
type = "object"
"#,
        )
        .unwrap();
        let result = parse_output(&manifest, "hello world", Duration::from_secs(1)).unwrap();
        assert_eq!(result["raw_output"], "hello world");
    }

    #[test]
    fn test_parse_output_json_format() {
        let manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "test"
version = "1.0.0"
binary = "test"
description = "Test"

[command]
template = "test"

[output]
format = "json"

[output.schema]
type = "object"
"#,
        )
        .unwrap();
        let result = parse_output(&manifest, r#"{"status":"ok"}"#, Duration::from_secs(1)).unwrap();
        assert_eq!(result["status"], "ok");
    }

    #[test]
    fn test_parse_output_explicit_parser() {
        let manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "test"
version = "1.0.0"
binary = "test"
description = "Test"

[command]
template = "test"

[output]
format = "text"
parser = "builtin:csv"

[output.schema]
type = "object"
"#,
        )
        .unwrap();
        let result = parse_output(&manifest, "a,b\n1,2", Duration::from_secs(1)).unwrap();
        let rows = result.as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["a"], "1");
    }

    #[test]
    fn test_parse_output_unknown_parser_rejected() {
        // A manifest specifying an arbitrary path must NOT be exec'd.
        let manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "rce"
version = "1.0.0"
binary = "test"
description = "Attempts to hijack output parser"

[command]
template = "test"

[output]
format = "text"
parser = "/bin/sh"

[output.schema]
type = "object"
"#,
        )
        .unwrap();
        let err = parse_output(&manifest, "ignored", Duration::from_secs(1)).unwrap_err();
        assert!(
            err.contains("Unknown parser"),
            "expected rejection, got: {}",
            err
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_parse_output_custom_parser_requires_allowlist() {
        let manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "needs-allowlist"
version = "1.0.0"
binary = "test"
description = "Custom parser path without allowlist entry"

[command]
template = "test"

[output]
format = "text"
parser = "custom:/opt/parsers/json-fixer"

[output.schema]
type = "object"
"#,
        )
        .unwrap();
        // Make sure any previous test didn't leave the env var set.
        std::env::remove_var("SYMBIONT_TOOLCLAD_ALLOWED_PARSERS");
        let err = parse_output(&manifest, "ignored", Duration::from_secs(1)).unwrap_err();
        assert!(
            err.contains("not in SYMBIONT_TOOLCLAD_ALLOWED_PARSERS"),
            "expected allowlist rejection, got: {}",
            err
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_parse_output_custom_parser_relative_path_rejected() {
        // Even with an allowlist entry, relative paths must be refused.
        std::env::set_var("SYMBIONT_TOOLCLAD_ALLOWED_PARSERS", "./parsers/my-parser");
        assert!(!is_custom_parser_allowed("./parsers/my-parser"));
        std::env::remove_var("SYMBIONT_TOOLCLAD_ALLOWED_PARSERS");
    }

    // ---- Schema Validation Tests ----

    #[test]
    fn test_validate_schema_no_properties() {
        let parsed = serde_json::json!({"foo": "bar"});
        let schema = serde_json::json!({"type": "object"});
        let warnings = validate_output_schema(&parsed, &schema);
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_validate_schema_missing_required() {
        let parsed = serde_json::json!({"foo": "bar"});
        let schema = serde_json::json!({
            "type": "object",
            "required": ["missing_key"],
            "properties": {
                "missing_key": {"type": "string"}
            }
        });
        let warnings = validate_output_schema(&parsed, &schema);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("missing_key"));
    }

    #[test]
    fn test_validate_schema_type_mismatch() {
        let parsed = serde_json::json!({"count": "not_a_number"});
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "count": {"type": "number"}
            }
        });
        let warnings = validate_output_schema(&parsed, &schema);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("count"));
        assert!(warnings[0].contains("number"));
    }

    #[test]
    fn test_validate_schema_raw_output_skips() {
        let parsed = serde_json::json!({"raw_output": "some text"});
        let schema = serde_json::json!({
            "type": "object",
            "required": ["specific_field"],
            "properties": {
                "specific_field": {"type": "string"}
            }
        });
        let warnings = validate_output_schema(&parsed, &schema);
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_validate_schema_all_types() {
        let parsed = serde_json::json!({
            "s": "hello",
            "n": 42,
            "b": true,
            "a": [1, 2],
            "o": {"nested": true}
        });
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "s": {"type": "string"},
                "n": {"type": "number"},
                "b": {"type": "boolean"},
                "a": {"type": "array"},
                "o": {"type": "object"}
            }
        });
        let warnings = validate_output_schema(&parsed, &schema);
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_manifest_version_recorded() {
        let manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "versioned"
version = "2.5.0"
binary = "echo"
description = "Test"

[command]
template = "echo test"

[output]
format = "text"

[output.schema]
type = "object"
"#,
        )
        .unwrap();
        let executor = ToolCladExecutor::new(vec![("versioned".to_string(), manifest)])
            .with_development_host_execution();
        assert_eq!(
            executor.manifest_versions.get("versioned").unwrap(),
            "2.5.0"
        );
    }

    // ---- MCP Proxy Tests ----

    #[test]
    fn test_mcp_proxy_tool_def_generation() {
        let manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "governed_search"
version = "1.0.0"
description = "Search via governed MCP proxy"

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
"#,
        )
        .unwrap();
        let td = generate_oneshot_tool_def(&manifest);
        assert_eq!(td.name, "governed_search");
        assert_eq!(td.description, "Search via governed MCP proxy");
        let props = td.parameters["properties"].as_object().unwrap();
        assert!(props.contains_key("query"));
        assert!(props.contains_key("max_results"));
        let required = td.parameters["required"].as_array().unwrap();
        assert!(required.contains(&serde_json::json!("query")));
    }

    // `execute_mcp_backend` used to fabricate a "delegated" envelope without
    // ever contacting a server. It now performs a real (feature-gated) MCP
    // call, so the field-mapping logic it depends on is tested directly via
    // `map_upstream_args`, and the full dispatch is covered end-to-end
    // against a real fixture server in `tests/toolclad_mcp.rs`.

    #[cfg(feature = "mcp-client")]
    #[test]
    fn test_mcp_field_map_maps_local_to_upstream_names() {
        let toml_str = r#"
server = "brave-search"
tool = "brave_web_search"

[field_map]
query = "q"
"#;
        let mcp: crate::toolclad::manifest::McpProxyDef = toml::from_str(toml_str).unwrap();

        let mut args = HashMap::new();
        args.insert("query".to_string(), "rust async".to_string());

        let upstream = map_upstream_args(&mcp, &HashMap::new(), &args);
        assert_eq!(upstream["q"], "rust async");
    }

    #[cfg(feature = "mcp-client")]
    #[test]
    fn test_mcp_field_map_passthrough_when_unmapped() {
        let toml_str = r#"
server = "my-server"
tool = "upstream_tool"
"#;
        let mcp: crate::toolclad::manifest::McpProxyDef = toml::from_str(toml_str).unwrap();

        let mut args = HashMap::new();
        args.insert("input".to_string(), "hello".to_string());

        // No field_map entry for "input", so it passes through unchanged.
        let upstream = map_upstream_args(&mcp, &HashMap::new(), &args);
        assert_eq!(upstream["input"], "hello");
    }

    #[cfg(feature = "mcp-client")]
    #[test]
    fn test_mcp_args_coerced_to_declared_json_types() {
        use crate::toolclad::manifest::ArgDef;
        let mcp: crate::toolclad::manifest::McpProxyDef =
            toml::from_str("server = \"s\"\ntool = \"t\"\n").unwrap();

        let mut arg_defs = HashMap::new();
        for (n, t) in [
            ("count", "integer"),
            ("ratio", "number"),
            ("flag", "boolean"),
            ("items", "array"),
            ("opts", "object"),
            ("label", "string"),
            ("mode", "enum"),
        ] {
            arg_defs.insert(
                n.to_string(),
                ArgDef {
                    type_name: t.to_string(),
                    ..Default::default()
                },
            );
        }

        // Values as parse_and_validate would produce them (everything stringified).
        let mut validated = HashMap::new();
        validated.insert("count".to_string(), "5".to_string());
        validated.insert("ratio".to_string(), "1.5".to_string());
        validated.insert("flag".to_string(), "true".to_string());
        validated.insert("items".to_string(), "[1,2,3]".to_string());
        validated.insert("opts".to_string(), r#"{"k":1}"#.to_string());
        validated.insert("label".to_string(), "hello".to_string());
        validated.insert("mode".to_string(), "fast".to_string());

        let up = map_upstream_args(&mcp, &arg_defs, &validated);
        // Numbers/booleans become real JSON scalars, not quoted strings.
        assert_eq!(up["count"], serde_json::json!(5));
        assert_eq!(up["ratio"], serde_json::json!(1.5));
        assert_eq!(up["flag"], serde_json::json!(true));
        // Arrays/objects are parsed back into structured JSON.
        assert_eq!(up["items"], serde_json::json!([1, 2, 3]));
        assert_eq!(up["opts"], serde_json::json!({"k": 1}));
        // String-like types stay strings.
        assert_eq!(up["label"], serde_json::json!("hello"));
        assert_eq!(up["mode"], serde_json::json!("fast"));

        // A malformed value for a typed arg falls back to a string, never panics.
        let mut bad = HashMap::new();
        bad.insert("count".to_string(), "not-a-number".to_string());
        let up2 = map_upstream_args(&mcp, &arg_defs, &bad);
        assert_eq!(up2["count"], serde_json::json!("not-a-number"));
    }

    #[test]
    fn test_mcp_proxy_dispatch_fails_closed_without_async_runtime() {
        // `execute_tool` (the sync `symbi tools` CLI path) dispatches
        // MCP-backed manifests through the sync `execute_mcp_backend`
        // bridge. Called from a plain `#[test]` fn there is no tokio runtime
        // to bridge onto (with `mcp-client`) and the feature may not even be
        // enabled — either way this must fail closed with a clear error,
        // never panic, and never fabricate a result.
        let manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "mcp_tool"
version = "1.0.0"
description = "MCP proxy tool"

[args.query]
position = 1
required = true
type = "string"
description = "Query"

[mcp]
server = "test-server"
tool = "test_tool"

[output]
format = "json"

[output.schema]
type = "object"
"#,
        )
        .unwrap();

        let executor = ToolCladExecutor::new(vec![("mcp_tool".to_string(), manifest)])
            .with_development_host_execution();

        let result = executor.execute_tool("mcp_tool", r#"{"query": "test"}"#);
        assert!(
            result.is_err(),
            "expected fail-closed error, got: {result:?}"
        );
    }
}

#[cfg(all(test, target_os = "linux"))]
mod source_broker_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn source_prepared_contract_binds_result_and_refuses_mixed_backends() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a"), "original").unwrap();
        let manifest: Manifest =
            toml::from_str(include_str!("../../../../tools/read_file.clad.toml")).unwrap();
        let mut boundary = CommandBoundary::default();
        boundary.docker.volumes = vec![format!("{}:/workspace:ro", root.path().display())];
        let executor = ToolCladExecutor::new(vec![("read_file".into(), manifest.clone())])
            .with_command_boundary(boundary.clone());
        let config = LoopConfig {
            tool_definitions: executor.get_tool_definitions(),
            ..Default::default()
        };
        let action = ProposedAction::ToolCall {
            call_id: "read".into(),
            name: "read_file".into(),
            arguments: json!({"path":"a"}).to_string(),
        };
        let prepared = executor.prepare_action(&action, &config).unwrap();
        let snapshot = prepared.backend::<ToolCladSnapshot>().unwrap();
        assert!(snapshot.argv.is_none());
        assert!(snapshot.boundary.docker.volumes.is_empty());
        std::fs::write(root.path().join("a"), "changed").unwrap();
        assert_eq!(
            snapshot
                .source
                .as_ref()
                .unwrap()
                .execute(
                    "read_file",
                    std::time::Instant::now() + Duration::from_secs(1)
                )
                .unwrap()["results"]["text"],
            "original"
        );
        let later = executor.prepare_action(&action, &config).unwrap();
        assert_ne!(
            prepared.contract().unwrap().digest,
            later.contract().unwrap().digest
        );
        let mut mixed = manifest;
        mixed.command.template = Some("touch /tmp/forbidden".into());
        let executor = ToolCladExecutor::new(vec![("read_file".into(), mixed)])
            .with_command_boundary(boundary);
        assert!(executor.validate_configuration().is_err());
        assert!(executor.prepare_action(&action, &config).is_err());
    }
}
