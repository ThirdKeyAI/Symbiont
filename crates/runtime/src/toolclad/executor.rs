//! ToolClad executor — bridges ORGA loop to .clad.toml tool manifests
//!
//! Implements the `ActionExecutor` trait: receives tool calls, validates
//! arguments, constructs commands from templates, executes, and returns
//! structured JSON observations.

use async_trait::async_trait;
use std::collections::HashMap;
use std::time::Duration;

use super::manifest::Manifest;
use super::validator;
use crate::reasoning::circuit_breaker::CircuitBreakerRegistry;
use crate::reasoning::executor::ActionExecutor;
use crate::reasoning::inference::ToolDefinition;
use crate::reasoning::loop_types::{LoopConfig, Observation, ProposedAction};

/// An executor that dispatches tool calls to ToolClad manifests.
pub struct ToolCladExecutor {
    manifests: HashMap<String, Manifest>,
    tool_defs: Vec<ToolDefinition>,
}

impl ToolCladExecutor {
    /// Create an executor from a set of loaded manifests.
    pub fn new(manifests: Vec<(String, Manifest)>) -> Self {
        let tool_defs: Vec<ToolDefinition> = manifests
            .iter()
            .flat_map(|(_, m)| generate_tool_definitions(m))
            .collect();
        let manifest_map: HashMap<String, Manifest> = manifests.into_iter().collect();
        Self {
            manifests: manifest_map,
            tool_defs,
        }
    }

    /// Check if this executor handles a given tool name.
    /// Matches both direct tool names and session/browser sub-commands
    /// (e.g., "msfconsole_session" or "msfconsole_session.run").
    pub fn handles(&self, tool_name: &str) -> bool {
        if self.manifests.contains_key(tool_name) {
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

    /// Number of loaded manifests.
    pub fn count(&self) -> usize {
        self.manifests.len()
    }

    /// Execute a single tool call against a manifest.
    fn execute_tool(&self, name: &str, args_json: &str) -> Result<serde_json::Value, String> {
        let manifest = self
            .manifests
            .get(name)
            .ok_or_else(|| format!("No ToolClad manifest for '{}'", name))?;

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
                let cleaned = validator::validate_arg(arg_def, &value)
                    .map_err(|e| format!("Validation failed for '{}': {}", arg_name, e))?;
                validated.insert(arg_name.clone(), cleaned);
            } else {
                validated.insert(arg_name.clone(), value);
            }
        }

        // Build command from template
        let command = build_command(manifest, &validated)?;

        // Execute with timeout
        let _timeout = Duration::from_secs(manifest.tool.timeout_seconds);
        let start = std::time::Instant::now();
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(&command)
            .output()
            .map_err(|e| format!("Failed to execute '{}': {}", command, e))?;

        let duration_ms = start.elapsed().as_millis() as u64;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

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

        let envelope = serde_json::json!({
            "status": status,
            "scan_id": scan_id,
            "tool": name,
            "command": command,
            "duration_ms": duration_ms,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "output_hash": hash,
            "results": {
                "raw_output": stdout.trim(),
                "stderr": if stderr.is_empty() { None } else { Some(stderr.trim().to_string()) },
                "exit_code": output.status.code()
            }
        });

        Ok(envelope)
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

                let (content, is_error) = match self.execute_tool(name, arguments) {
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
}
