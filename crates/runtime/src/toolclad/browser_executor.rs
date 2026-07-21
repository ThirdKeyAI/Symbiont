//! BrowserExecutor -- CDP-based browser tool manager (honest-failure stub).
//!
//! Browser (`mode = "browser"`) ToolClad tools are validated and scope-checked
//! here, but CDP execution is not yet implemented. Until the `toolclad-browser`
//! feature carries a real Chrome DevTools Protocol backend, every reachable
//! command returns an honest error rather than a fabricated success — an agent
//! must never be told a page was navigated/clicked/scraped when nothing ran.

use std::collections::HashMap;

use super::browser_state::BrowserScopeChecker;
use super::manifest::Manifest;
use super::validator;

/// Manages browser tool manifests. Validates and scope-checks browser commands;
/// actual CDP execution awaits the `toolclad-browser` backend.
pub struct BrowserExecutor {
    manifests: HashMap<String, Manifest>,
}

impl BrowserExecutor {
    pub fn new(manifests: Vec<(String, Manifest)>) -> Self {
        let browser_manifests: HashMap<String, Manifest> = manifests
            .into_iter()
            .filter(|(_, m)| m.tool.mode == "browser")
            .collect();
        Self {
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

    /// Validate and scope-check a browser command, then return an honest error:
    /// CDP execution is not implemented. NEVER returns a fabricated success.
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

        // Validate command-specific args.
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

        // Scope check for navigation.
        //
        // Deny-by-default: `navigate` requires an explicit `[browser.scope]`. A
        // manifest with no scope is treated as "deny all" rather than fail-open
        // (codered F-pattern-scout-0005) — otherwise a missing scope would let
        // the agent's browser reach arbitrary URLs (file:///, 169.254.169.254,
        // internal services), enabling SSRF and credential theft. This guard
        // stays live and tested even before a real CDP backend exists.
        if command_name == "navigate" {
            let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
            match &browser_def.scope {
                Some(scope) => {
                    let checker = BrowserScopeChecker::new(scope);
                    checker.check_url(url)?;
                }
                None => {
                    return Err(
                        "Browser navigate denied: manifest defines no [browser.scope]; \
                         declare an explicit scope (allowed_domains / allow_external) \
                         to permit navigation"
                            .to_string(),
                    );
                }
            }
        }

        // Guards passed — but CDP execution is not implemented. Return an honest
        // error; NEVER a fabricated success envelope.
        Err(browser_not_implemented_error())
    }
}

/// Honest "not implemented" message, feature-gated. Without `toolclad-browser`
/// the message points at the feature to enable; with it, the CDP backend is
/// simply not built yet (follow-up). Either way, no command fabricates success.
#[cfg(not(feature = "toolclad-browser"))]
fn browser_not_implemented_error() -> String {
    "Browser mode requires the 'toolclad-browser' feature (CDP backend not yet available)"
        .to_string()
}

#[cfg(feature = "toolclad-browser")]
fn browser_not_implemented_error() -> String {
    "Browser CDP execution is not yet implemented".to_string()
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

        // Allowed domain: the scope check passes, then the honest
        // not-implemented error is returned — NOT a scope error, NOT success.
        let result = executor.execute_browser_command(
            "test_browser.navigate",
            r#"{"url": "https://app.example.com/page"}"#,
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            !err.contains("not in allowed domains"),
            "scope should have passed, got: {err}"
        );
        assert!(
            err.contains("toolclad-browser") || err.contains("not yet implemented"),
            "expected the honest not-implemented error, got: {err}"
        );

        // Blocked domain: the scope check denies before the honest error.
        let result = executor.execute_browser_command(
            "test_browser.navigate",
            r#"{"url": "https://evil.com/page"}"#,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not in allowed domains"));
    }

    #[test]
    fn test_navigate_denied_when_scope_missing() {
        // A manifest with no [browser.scope] must deny navigation (deny-by-default),
        // not fail open. Regression for codered F-pattern-scout-0005.
        let toml_str = r#"
[tool]
name = "noscope_browser"
mode = "browser"
version = "1.0.0"
description = "Browser without scope"

[browser]
engine = "cdp"
connect = "launch"
extract_mode = "accessibility_tree"

[browser.commands.navigate]
description = "Navigate to URL"
risk_tier = "medium"

[browser.commands.navigate.args.url]
position = 0
type = "url"
required = true
schemes = ["https"]
description = "URL to navigate to"

[output]
format = "json"

[output.schema]
type = "object"
"#;
        let manifest: Manifest = toml::from_str(toml_str).unwrap();
        assert!(manifest.browser.as_ref().unwrap().scope.is_none());
        let executor = BrowserExecutor::new(vec![("noscope_browser".to_string(), manifest)]);

        let result = executor.execute_browser_command(
            "noscope_browser.navigate",
            r#"{"url": "https://app.example.com/page"}"#,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no [browser.scope]"));
    }

    #[test]
    fn test_snapshot_command() {
        let manifest = make_browser_manifest();
        let executor = BrowserExecutor::new(vec![("test_browser".to_string(), manifest)]);

        let result = executor.execute_browser_command("test_browser.snapshot", "{}");
        assert!(
            result.is_err(),
            "browser commands must not fabricate success"
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("toolclad-browser") || err.contains("not yet implemented"),
            "expected the honest not-implemented error, got: {err}"
        );
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
    fn test_parse_browser_tool_name() {
        let (base, cmd) = parse_browser_tool_name("my_browser.navigate").unwrap();
        assert_eq!(base, "my_browser");
        assert_eq!(cmd, "navigate");
        assert!(parse_browser_tool_name("no_dot").is_err());
    }

    #[test]
    fn test_no_fabricated_success() {
        // Core regression lock: a valid, in-scope browser command must NEVER
        // return Ok with a fabricated success envelope — it must return an
        // honest error until a real CDP backend exists.
        let manifest = make_browser_manifest();
        let executor = BrowserExecutor::new(vec![("test_browser".to_string(), manifest)]);
        for (tool, args) in [
            (
                "test_browser.navigate",
                r#"{"url": "https://app.example.com/x"}"#,
            ),
            ("test_browser.snapshot", "{}"),
        ] {
            let result = executor.execute_browser_command(tool, args);
            assert!(
                result.is_err(),
                "{tool} must not fabricate success, got: {result:?}"
            );
        }
    }
}
