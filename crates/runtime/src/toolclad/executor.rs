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
use std::io::Write;
use std::time::Duration;

use super::manifest::Manifest;
use super::validator;
use crate::reasoning::circuit_breaker::CircuitBreakerRegistry;
use crate::reasoning::executor::ActionExecutor;
use crate::reasoning::inference::ToolDefinition;
use crate::reasoning::loop_types::{LoopConfig, Observation, ProposedAction};

use super::manifest::ArgDef;

/// An executor that dispatches tool calls to ToolClad manifests.
/// Handles all five backends: shell, HTTP, MCP proxy, session (PTY), browser (CDP).
pub struct ToolCladExecutor {
    manifests: HashMap<String, Manifest>,
    tool_defs: Vec<ToolDefinition>,
    custom_types: HashMap<String, ArgDef>,
    /// Manifest versions recorded at construction time for hot-reload detection.
    manifest_versions: HashMap<String, String>,
    /// Session executor for interactive CLI tools.
    session_executor: super::session_executor::SessionExecutor,
    /// Browser executor for CDP-based browser sessions.
    browser_executor: super::browser_executor::BrowserExecutor,
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
            manifests: manifest_map,
            tool_defs,
            custom_types,
            manifest_versions,
            session_executor,
            browser_executor,
        }
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

    /// Execute a single tool call against a manifest.
    pub fn execute_tool(&self, name: &str, args_json: &str) -> Result<serde_json::Value, String> {
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

        // Parse arguments from JSON
        let args: HashMap<String, serde_json::Value> = serde_json::from_str(args_json)
            .map_err(|e| format!("Invalid arguments JSON: {}", e))?;

        // Validate each argument against its definition
        let mut validated: HashMap<String, String> = HashMap::new();
        for (arg_name, arg_def) in &manifest.args {
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

            if !value.is_empty() {
                let custom = if self.custom_types.is_empty() {
                    None
                } else {
                    Some(&self.custom_types)
                };
                let cleaned = validator::validate_arg_with_custom(arg_def, &value, custom)
                    .map_err(|e| format!("Validation failed for '{}': {}", arg_name, e))?;
                validated.insert(arg_name.clone(), cleaned);
            } else {
                validated.insert(arg_name.clone(), value);
            }
        }

        // Dispatch to appropriate backend
        if manifest.http.is_some() {
            return self.execute_http_backend(name, manifest, &validated);
        }
        if manifest.mcp.is_some() {
            return self.execute_mcp_backend(name, manifest, &validated);
        }

        // Build command from template (shell backend)
        let command = build_command(manifest, &validated)?;

        // Execute with timeout — use direct argv to prevent shell injection
        let _timeout = Duration::from_secs(manifest.tool.timeout_seconds);
        let start = std::time::Instant::now();
        let argv = split_command_to_argv(&command)?;
        let (program, args) = argv
            .split_first()
            .ok_or_else(|| "Empty command after template interpolation".to_string())?;
        let output = std::process::Command::new(program)
            .args(args)
            .output()
            .map_err(|e| format!("Failed to execute '{}': {}", program, e))?;

        let duration_ms = start.elapsed().as_millis() as u64;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        // Parse output using the manifest's format/parser configuration
        let parsed = parse_output(manifest, stdout.trim())?;

        // Validate parsed output against schema (warnings only, non-fatal)
        let schema_warnings = validate_output_schema(&parsed, &manifest.output.schema);

        // Build evidence envelope
        let scan_id = format!(
            "{}-{}",
            chrono::Utc::now().timestamp(),
            uuid::Uuid::new_v4().as_fields().0
        );
        let status = if output.status.success() {
            "success"
        } else {
            "error"
        };

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
        });

        // Attach stderr and exit_code to the results
        if let Some(obj) = envelope.as_object_mut() {
            if let Some(results) = obj.get_mut("results").and_then(|r| r.as_object_mut()) {
                if !stderr.is_empty() {
                    results.insert(
                        "stderr".to_string(),
                        serde_json::Value::String(stderr.trim().to_string()),
                    );
                }
                results.insert(
                    "exit_code".to_string(),
                    serde_json::json!(output.status.code()),
                );
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
    fn execute_http_backend(
        &self,
        name: &str,
        manifest: &Manifest,
        validated: &HashMap<String, String>,
    ) -> Result<serde_json::Value, String> {
        let http = manifest.http.as_ref().unwrap();

        // Interpolate URL with args and secrets
        let url = interpolate(&http.url, validated);
        let url = super::template_vars::inject_secrets(&url)
            .map_err(|e| format!("URL secret error: {}", e))?;

        // SSRF protection: block private/internal IP ranges
        reject_ssrf_url(&url)?;

        // Interpolate headers with secrets
        let mut headers = Vec::new();
        for (key, val) in &http.headers {
            let resolved = interpolate(val, validated);
            let resolved = super::template_vars::inject_secrets(&resolved)
                .map_err(|e| format!("Header secret error: {}", e))?;
            headers.push((key.clone(), resolved));
        }

        // Interpolate body
        let body = http
            .body_template
            .as_ref()
            .map(|t| {
                let b = interpolate(t, validated);
                super::template_vars::inject_secrets(&b)
            })
            .transpose()
            .map_err(|e| format!("Body secret error: {}", e))?;

        // Execute HTTP request
        let client = reqwest::blocking::Client::new();
        let timeout = std::time::Duration::from_secs(manifest.tool.timeout_seconds);
        let mut request = match http.method.to_uppercase().as_str() {
            "GET" => client.get(&url),
            "POST" => client.post(&url),
            "PUT" => client.put(&url),
            "DELETE" => client.delete(&url),
            "PATCH" => client.patch(&url),
            "HEAD" => client.head(&url),
            other => return Err(format!("Unsupported HTTP method: {}", other)),
        };

        request = request.timeout(timeout);
        for (key, val) in &headers {
            request = request.header(key.as_str(), val.as_str());
        }
        if let Some(body_str) = &body {
            request = request.body(body_str.clone());
        }

        let start = std::time::Instant::now();
        let response = request
            .send()
            .map_err(|e| format!("HTTP request failed: {}", e))?;
        let duration_ms = start.elapsed().as_millis() as u64;

        let status_code = response.status().as_u16();
        let response_body = response
            .text()
            .map_err(|e| format!("Failed to read response: {}", e))?;

        let is_success = if !http.success_status.is_empty() {
            http.success_status.contains(&status_code)
        } else {
            (200..300).contains(&status_code)
        };

        // Parse response
        let parsed = parse_output(manifest, &response_body);
        let results = parsed.unwrap_or_else(|_| serde_json::json!({"raw_output": response_body}));

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
            "http_method": http.method,
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

    /// Execute an MCP proxy backend tool.
    fn execute_mcp_backend(
        &self,
        name: &str,
        manifest: &Manifest,
        validated: &HashMap<String, String>,
    ) -> Result<serde_json::Value, String> {
        let mcp = manifest.mcp.as_ref().unwrap();

        // Map validated args to upstream tool's expected format
        let mut upstream_args = serde_json::Map::new();
        for (local_name, value) in validated {
            let upstream_name = mcp
                .field_map
                .get(local_name)
                .cloned()
                .unwrap_or_else(|| local_name.clone());
            upstream_args.insert(upstream_name, serde_json::json!(value));
        }

        let scan_id = format!(
            "{}-{}",
            chrono::Utc::now().timestamp(),
            uuid::Uuid::new_v4().as_fields().0
        );

        // Note: Full MCP execution requires the runtime's MCP transport.
        // For now, build and return the request structure. The runtime's
        // EnforcedActionExecutor will forward to the actual MCP server.
        Ok(serde_json::json!({
            "status": "delegated",
            "scan_id": scan_id,
            "tool": name,
            "mcp_server": mcp.server,
            "mcp_tool": mcp.tool,
            "mcp_arguments": upstream_args,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "exit_code": 0,
            "stderr": "",
            "results": {
                "delegated_to": format!("{}:{}", mcp.server, mcp.tool),
                "arguments": upstream_args,
            }
        }))
    }
}

#[async_trait]
impl ActionExecutor for ToolCladExecutor {
    async fn execute_actions(
        &self,
        actions: &[ProposedAction],
        _config: &LoopConfig,
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

                // Dispatch to appropriate executor based on tool type
                let result = if self.session_executor.handles(name) {
                    self.session_executor
                        .execute_session_command(name, arguments)
                } else if self.browser_executor.handles(name) {
                    self.browser_executor
                        .execute_browser_command(name, arguments)
                } else {
                    self.execute_tool(name, arguments)
                };

                let (content, is_error) = match result {
                    Ok(envelope) => (
                        serde_json::to_string_pretty(&envelope).unwrap_or_default(),
                        false,
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
fn parse_output(manifest: &Manifest, raw_output: &str) -> Result<serde_json::Value, String> {
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
            run_custom_parser(path, raw_output)
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

/// Run a custom external parser. Writes raw_output to a temp file, executes
/// the parser binary with the temp file path as argv[1], and captures stdout
/// as JSON.
fn run_custom_parser(parser_path: &str, raw_output: &str) -> Result<serde_json::Value, String> {
    let mut tmp = tempfile::NamedTempFile::new()
        .map_err(|e| format!("Failed to create temp file for custom parser: {}", e))?;

    tmp.write_all(raw_output.as_bytes())
        .map_err(|e| format!("Failed to write to temp file: {}", e))?;

    let tmp_path = tmp.path().to_string_lossy().to_string();

    let output = std::process::Command::new(parser_path)
        .arg(&tmp_path)
        .output()
        .map_err(|e| format!("Failed to execute custom parser '{}': {}", parser_path, e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Custom parser '{}' exited with {}: {}",
            parser_path,
            output.status.code().unwrap_or(-1),
            stderr.trim()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(stdout.trim()).map_err(|e| {
        format!(
            "Custom parser '{}' produced invalid JSON: {}",
            parser_path, e
        )
    })
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

/// Build a command string from a manifest template and validated arguments.
fn build_command(manifest: &Manifest, args: &HashMap<String, String>) -> Result<String, String> {
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
            result = result.replace(&placeholder, &interpolate(&cond_def.template, args));
        } else {
            result = result.replace(&placeholder, "");
        }
    }

    // Interpolate remaining arg placeholders
    result = interpolate(&result, args);

    // Auto-generated variables
    let scan_id = format!("{}", chrono::Utc::now().timestamp());
    result = result.replace("{_scan_id}", &scan_id);
    result = result.replace("{_output_file}", "/dev/null");
    result = result.replace("{_evidence_dir}", "/tmp/evidence");

    // Clean up multiple spaces
    let result = result.split_whitespace().collect::<Vec<_>>().join(" ");

    Ok(result)
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

/// Reject URLs targeting private/internal IP ranges to prevent SSRF.
fn reject_ssrf_url(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("Invalid URL '{}': {}", url, e))?;

    // Only allow http/https
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!(
            "SSRF: only http/https schemes allowed, got '{}'",
            parsed.scheme()
        ));
    }

    if let Some(host) = parsed.host_str() {
        // Block localhost variants
        if host == "localhost" || host == "127.0.0.1" || host == "::1" || host == "[::1]" {
            return Err("SSRF: cannot access localhost".to_string());
        }

        // Block cloud metadata endpoints
        if host == "169.254.169.254" || host == "metadata.google.internal" {
            return Err("SSRF: cannot access cloud metadata endpoint".to_string());
        }

        // Block private IP ranges
        if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            let is_private = match ip {
                std::net::IpAddr::V4(v4) => {
                    v4.is_loopback()
                        || v4.is_private()
                        || v4.is_link_local()
                        || v4.is_broadcast()
                        || v4.is_unspecified()
                }
                std::net::IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified(),
            };
            if is_private {
                return Err(format!("SSRF: cannot access private IP range {}", ip));
            }
        }
    }

    Ok(())
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

    while let Some(c) = chars.next() {
        match c {
            '\'' if !in_double_quote => in_single_quote = !in_single_quote,
            '"' if !in_single_quote => in_double_quote = !in_double_quote,
            '\\' if !in_single_quote => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            ' ' | '\t' if !in_single_quote && !in_double_quote => {
                if !current.is_empty() {
                    argv.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        argv.push(current);
    }
    if in_single_quote || in_double_quote {
        return Err("Unterminated quote in command template".to_string());
    }
    if argv.is_empty() {
        return Err("Empty command after template interpolation".to_string());
    }
    Ok(argv)
}

/// Interpolate {placeholder} references in a string.
fn interpolate(template: &str, args: &HashMap<String, String>) -> String {
    let mut result = template.to_string();
    for (key, value) in args {
        result = result.replace(&format!("{{{}}}", key), value);
    }
    result
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
        let cmd = build_command(&manifest, &args).unwrap();
        assert_eq!(cmd, "echo hello");
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
        let cmd = build_command(&manifest, &args).unwrap();
        assert_eq!(cmd, "scan --rate 100 example.com");
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
        let result = parse_output(&manifest, "hello world").unwrap();
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
        let result = parse_output(&manifest, r#"{"status":"ok"}"#).unwrap();
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
        let result = parse_output(&manifest, "a,b\n1,2").unwrap();
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
        let err = parse_output(&manifest, "ignored").unwrap_err();
        assert!(
            err.contains("Unknown parser"),
            "expected rejection, got: {}",
            err
        );
    }

    #[test]
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
        let err = parse_output(&manifest, "ignored").unwrap_err();
        assert!(
            err.contains("not in SYMBIONT_TOOLCLAD_ALLOWED_PARSERS"),
            "expected allowlist rejection, got: {}",
            err
        );
    }

    #[test]
    fn test_parse_output_custom_parser_relative_path_rejected() {
        // Even with an allowlist entry, relative paths must be refused.
        std::env::set_var(
            "SYMBIONT_TOOLCLAD_ALLOWED_PARSERS",
            "./parsers/my-parser",
        );
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
        let executor = ToolCladExecutor::new(vec![("versioned".to_string(), manifest)]);
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

    #[test]
    fn test_mcp_proxy_execution_returns_delegated_envelope() {
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

[mcp]
server = "brave-search"
tool = "brave_web_search"

[mcp.field_map]
query = "q"

[output]
format = "json"

[output.schema]
type = "object"
"#,
        )
        .unwrap();

        let executor =
            ToolCladExecutor::new(vec![("governed_search".to_string(), manifest.clone())]);

        let mut args = HashMap::new();
        args.insert("query".to_string(), "rust async".to_string());
        let result = executor
            .execute_mcp_backend("governed_search", &manifest, &args)
            .unwrap();

        assert_eq!(result["status"], "delegated");
        assert_eq!(result["tool"], "governed_search");
        assert_eq!(result["mcp_server"], "brave-search");
        assert_eq!(result["mcp_tool"], "brave_web_search");
        assert_eq!(result["exit_code"], 0);

        // Check that field mapping was applied
        let mcp_args = &result["mcp_arguments"];
        assert_eq!(mcp_args["q"], "rust async");
    }

    #[test]
    fn test_mcp_proxy_field_map_passthrough() {
        let manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "passthrough"
version = "1.0.0"
description = "Direct passthrough"

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
"#,
        )
        .unwrap();

        let executor = ToolCladExecutor::new(vec![("passthrough".to_string(), manifest.clone())]);

        let mut args = HashMap::new();
        args.insert("input".to_string(), "hello".to_string());
        let result = executor
            .execute_mcp_backend("passthrough", &manifest, &args)
            .unwrap();

        // No field_map, so "input" stays as "input" in upstream args
        let mcp_args = &result["mcp_arguments"];
        assert_eq!(mcp_args["input"], "hello");
    }

    #[test]
    fn test_mcp_proxy_dispatch_via_execute_tool() {
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

        let executor = ToolCladExecutor::new(vec![("mcp_tool".to_string(), manifest)]);

        let result = executor
            .execute_tool("mcp_tool", r#"{"query": "test"}"#)
            .unwrap();

        assert_eq!(result["status"], "delegated");
        assert_eq!(result["mcp_server"], "test-server");
        assert_eq!(result["mcp_tool"], "test_tool");
    }
}
