//! BrowserExecutor -- CDP-based headless/live browser session manager.
//!
//! Manages browser sessions via Chrome DevTools Protocol. Validates navigation
//! against URL scope rules, executes typed browser commands, and captures
//! accessibility tree snapshots and screenshots as evidence.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::browser_state::*;
use super::manifest::Manifest;
use super::validator;

/// Manages browser sessions.
pub struct BrowserExecutor {
    sessions: Arc<Mutex<HashMap<String, BrowserSessionState>>>,
    manifests: HashMap<String, Manifest>,
}

/// Browser session state (without actual CDP connection for now).
struct BrowserSessionState {
    #[allow(dead_code)]
    page_state: PageState,
    #[allow(dead_code)]
    scope_checker: BrowserScopeChecker,
    interaction_count: u32,
    #[allow(dead_code)]
    manifest_name: String,
    #[allow(dead_code)]
    session_id: String,
    #[allow(dead_code)]
    status: BrowserStatus,
}

impl BrowserExecutor {
    pub fn new(manifests: Vec<(String, Manifest)>) -> Self {
        let browser_manifests: HashMap<String, Manifest> = manifests
            .into_iter()
            .filter(|(_, m)| m.tool.mode == "browser")
            .collect();
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            manifests: browser_manifests,
        }
    }

    pub fn handles(&self, tool_name: &str) -> bool {
        if let Some(base) = tool_name.split('.').next() {
            if let Some(m) = self.manifests.get(base) {
                if let Some(browser) = &m.browser {
                    let cmd = tool_name
                        .strip_prefix(base)
                        .unwrap_or("")
                        .trim_start_matches('.');
                    return !cmd.is_empty() && browser.commands.contains_key(cmd);
                }
            }
        }
        false
    }

    /// Execute a browser command.
    pub fn execute_browser_command(
        &self,
        tool_name: &str,
        args_json: &str,
    ) -> Result<serde_json::Value, String> {
        let (manifest_name, command_name) = parse_browser_tool_name(tool_name)?;

        let manifest = self
            .manifests
            .get(&manifest_name)
            .ok_or_else(|| format!("No browser manifest for '{}'", manifest_name))?;
        let browser_def = manifest
            .browser
            .as_ref()
            .ok_or("Manifest has no [browser] section")?;
        let cmd_def = browser_def
            .commands
            .get(&command_name)
            .ok_or_else(|| format!("Unknown browser command: {}", command_name))?;

        let args: HashMap<String, serde_json::Value> =
            serde_json::from_str(args_json).map_err(|e| format!("Invalid arguments: {}", e))?;

        // Validate command-specific args
        for (arg_name, arg_def) in &cmd_def.args {
            if let Some(value) = args.get(arg_name) {
                let val_str = match value {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                validator::validate_arg(arg_def, &val_str)
                    .map_err(|e| format!("Arg '{}' validation: {}", arg_name, e))?;
            } else if arg_def.required {
                return Err(format!("Missing required arg: {}", arg_name));
            }
        }

        // Scope check for navigation commands
        if command_name == "navigate" {
            if let Some(url_val) = args.get("url") {
                let url = url_val.as_str().unwrap_or("");
                if let Some(scope) = &browser_def.scope {
                    let checker = BrowserScopeChecker::new(scope);
                    checker.check_url(url)?;
                }
            }
        }

        // Check max interactions
        {
            let sessions = self.sessions.lock().map_err(|e| e.to_string())?;
            if let Some(session) = sessions.get(&manifest_name) {
                if session.interaction_count >= browser_def.max_interactions {
                    return Err(format!(
                        "Browser session exceeded max interactions ({})",
                        browser_def.max_interactions
                    ));
                }
            }
        }

        // Build result based on command type
        let result = match command_name.as_str() {
            "navigate" => {
                let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
                serde_json::json!({
                    "url": url,
                    "title": "",
                    "domain": extract_domain(url).unwrap_or_default(),
                    "page_state": { "page_loaded": true },
                    "note": "CDP execution requires 'toolclad-browser' feature"
                })
            }
            "snapshot" => {
                let selector = args.get("selector").and_then(|v| v.as_str());
                serde_json::json!({
                    "content": format!("Accessibility tree snapshot{}",
                        selector.map(|s| format!(" (scoped to '{}')", s)).unwrap_or_default()),
                    "extract_mode": "accessibility_tree",
                    "note": "CDP execution requires 'toolclad-browser' feature"
                })
            }
            "click" | "type_text" | "submit_form" | "extract" | "extract_html" | "screenshot"
            | "execute_js" | "wait_for" | "go_back" | "list_tabs" | "network_timing" => {
                serde_json::json!({
                    "command": command_name,
                    "args": args,
                    "note": "CDP execution requires 'toolclad-browser' feature"
                })
            }
            _ => {
                return Err(format!("Unknown browser command: {}", command_name));
            }
        };

        // Update interaction count
        {
            let mut sessions = self.sessions.lock().map_err(|e| e.to_string())?;
            let session = sessions.entry(manifest_name.clone()).or_insert_with(|| {
                let scope_checker = browser_def
                    .scope
                    .as_ref()
                    .map(BrowserScopeChecker::new)
                    .unwrap_or(BrowserScopeChecker {
                        allowed_domains: vec![],
                        blocked_domains: vec![],
                        allow_external: true,
                    });
                BrowserSessionState {
                    page_state: PageState::default(),
                    scope_checker,
                    interaction_count: 0,
                    manifest_name: manifest_name.clone(),
                    session_id: format!(
                        "browser-{}-{}",
                        manifest_name,
                        uuid::Uuid::new_v4().as_fields().0
                    ),
                    status: BrowserStatus::Ready,
                }
            });
            session.interaction_count += 1;
        }

        let scan_id = format!(
            "{}-{}",
            chrono::Utc::now().timestamp(),
            uuid::Uuid::new_v4().as_fields().0
        );

        Ok(serde_json::json!({
            "status": "success",
            "scan_id": scan_id,
            "tool": tool_name,
            "command": command_name,
            "duration_ms": 0,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "exit_code": 0,
            "stderr": "",
            "results": result
        }))
    }

    pub fn cleanup(&self) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.clear();
        }
    }
}

fn parse_browser_tool_name(name: &str) -> Result<(String, String), String> {
    let parts: Vec<&str> = name.splitn(2, '.').collect();
    if parts.len() != 2 {
        return Err(format!(
            "Invalid browser tool name: '{}' (expected 'browser.command')",
            name
        ));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

/// Extract domain from a URL (reused from browser_state).
fn extract_domain(url: &str) -> Option<String> {
    let after_scheme = url.split("://").nth(1)?;
    let domain = after_scheme.split('/').next()?;
    let domain = domain.split(':').next()?;
    Some(domain.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_browser_manifest() -> Manifest {
        let toml_str = r#"
[tool]
name = "test_browser"
mode = "browser"
version = "1.0.0"
description = "Test browser"

[browser]
engine = "cdp"
connect = "launch"
extract_mode = "accessibility_tree"

[browser.scope]
allowed_domains = ["*.example.com"]

[browser.commands.navigate]
description = "Navigate to URL"
risk_tier = "medium"

[browser.commands.navigate.args.url]
position = 0
type = "url"
required = true
schemes = ["https"]
description = "URL to navigate to"

[browser.commands.snapshot]
description = "Get accessibility tree"
risk_tier = "low"

[browser.commands.click]
description = "Click element"
risk_tier = "low"

[browser.commands.click.args.selector]
position = 0
type = "string"
required = true
description = "CSS selector"

[output]
format = "json"

[output.schema]
type = "object"
"#;
        toml::from_str(toml_str).unwrap()
    }

    #[test]
    fn test_browser_executor_handles() {
        let manifest = make_browser_manifest();
        let executor = BrowserExecutor::new(vec![("test_browser".to_string(), manifest)]);
        assert!(executor.handles("test_browser.navigate"));
        assert!(executor.handles("test_browser.snapshot"));
        assert!(executor.handles("test_browser.click"));
        assert!(!executor.handles("test_browser.unknown"));
        assert!(!executor.handles("other.navigate"));
    }

    #[test]
    fn test_navigate_scope_check() {
        let manifest = make_browser_manifest();
        let executor = BrowserExecutor::new(vec![("test_browser".to_string(), manifest)]);

        // Allowed domain
        let result = executor.execute_browser_command(
            "test_browser.navigate",
            r#"{"url": "https://app.example.com/page"}"#,
        );
        assert!(result.is_ok());

        // Blocked domain
        let result = executor.execute_browser_command(
            "test_browser.navigate",
            r#"{"url": "https://evil.com/page"}"#,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not in allowed domains"));
    }

    #[test]
    fn test_snapshot_command() {
        let manifest = make_browser_manifest();
        let executor = BrowserExecutor::new(vec![("test_browser".to_string(), manifest)]);

        let result = executor.execute_browser_command("test_browser.snapshot", "{}");
        assert!(result.is_ok());
        let envelope = result.unwrap();
        assert_eq!(envelope["status"], "success");
        assert!(envelope["results"]["content"]
            .as_str()
            .unwrap()
            .contains("Accessibility tree"));
    }

    #[test]
    fn test_click_requires_selector() {
        let manifest = make_browser_manifest();
        let executor = BrowserExecutor::new(vec![("test_browser".to_string(), manifest)]);

        let result = executor.execute_browser_command("test_browser.click", "{}");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required arg"));
    }

    #[test]
    fn test_interaction_count() {
        let manifest = make_browser_manifest();
        let executor = BrowserExecutor::new(vec![("test_browser".to_string(), manifest)]);

        // Multiple commands should increment count
        for _ in 0..5 {
            executor
                .execute_browser_command("test_browser.snapshot", "{}")
                .unwrap();
        }

        let sessions = executor.sessions.lock().unwrap();
        assert_eq!(sessions["test_browser"].interaction_count, 5);
    }

    #[test]
    fn test_parse_browser_tool_name() {
        let (base, cmd) = parse_browser_tool_name("my_browser.navigate").unwrap();
        assert_eq!(base, "my_browser");
        assert_eq!(cmd, "navigate");
        assert!(parse_browser_tool_name("no_dot").is_err());
    }
}
