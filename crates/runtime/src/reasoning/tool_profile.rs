//! Tool Profile Filtering
//!
//! Filters tool definitions before the LLM sees them. Supports glob patterns
//! for include/exclude, max_tools cap, and require_verified flag.
//! Part of the orga-adaptive feature gate.

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::reasoning::inference::ToolDefinition;

/// A profile that controls which tools are visible to the LLM.
///
/// The filtering pipeline applies in order:
/// 1. Include filter (if non-empty, only matching tools pass)
/// 2. Exclude filter (matching tools are removed)
/// 3. Verified filter (if `require_verified`, only `[verified]` tools pass)
/// 4. Max tools cap (truncate to `max_tools` if set)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolProfile {
    /// Glob patterns for tools to include. Empty = include all.
    #[serde(default)]
    pub include: Vec<String>,
    /// Glob patterns for tools to exclude. Applied after include.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Maximum number of tools to expose to the LLM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tools: Option<usize>,
    /// Only pass tools whose description contains `[verified]`.
    #[serde(default)]
    pub require_verified: bool,
}

impl ToolProfile {
    /// A permissive profile that passes all tools through unchanged.
    pub fn permissive() -> Self {
        Self {
            include: Vec::new(),
            exclude: Vec::new(),
            max_tools: None,
            require_verified: false,
        }
    }

    /// Include only tools matching the given glob patterns.
    pub fn include_only(patterns: &[&str]) -> Self {
        Self {
            include: patterns.iter().map(|s| s.to_string()).collect(),
            exclude: Vec::new(),
            max_tools: None,
            require_verified: false,
        }
    }

    /// Exclude tools matching the given glob patterns.
    pub fn exclude_only(patterns: &[&str]) -> Self {
        Self {
            include: Vec::new(),
            exclude: patterns.iter().map(|s| s.to_string()).collect(),
            max_tools: None,
            require_verified: false,
        }
    }

    /// Apply the filtering pipeline to a set of tool definitions.
    pub fn filter_tools(&self, available: &[ToolDefinition]) -> Vec<ToolDefinition> {
        let mut result: Vec<ToolDefinition> = if self.include.is_empty() {
            available.to_vec()
        } else {
            available
                .iter()
                .filter(|t| self.include.iter().any(|p| glob_matches(p, &t.name)))
                .cloned()
                .collect()
        };

        if !self.exclude.is_empty() {
            result.retain(|t| !self.exclude.iter().any(|p| glob_matches(p, &t.name)));
        }

        if self.require_verified {
            result.retain(|t| t.description.contains("[verified]"));
        }

        if let Some(max) = self.max_tools {
            result.truncate(max);
        }

        result
    }
}

/// Test whether a tool name matches a glob pattern.
fn glob_matches(pattern: &str, name: &str) -> bool {
    let regex_pattern = format!("^{}$", glob_to_regex(pattern));
    match Regex::new(&regex_pattern) {
        Ok(re) => re.is_match(name),
        Err(_) => false,
    }
}

/// Convert a glob pattern to a regex string.
fn glob_to_regex(glob: &str) -> String {
    let mut result = String::new();
    for ch in glob.chars() {
        match ch {
            '*' => result.push_str(".*"),
            '?' => result.push('.'),
            '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' | '\\' => {
                result.push('\\');
                result.push(ch);
            }
            _ => result.push(ch),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: format!("Tool {}", name),
            parameters: serde_json::json!({}),
        }
    }

    fn make_verified_tool(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: format!("[verified] Tool {}", name),
            parameters: serde_json::json!({}),
        }
    }

    fn sample_tools() -> Vec<ToolDefinition> {
        vec![
            make_tool("web_search"),
            make_tool("file_read"),
            make_tool("file_write"),
            make_tool("code_execute"),
            make_verified_tool("verified_tool"),
        ]
    }

    #[test]
    fn test_permissive_passthrough() {
        let profile = ToolProfile::permissive();
        let tools = sample_tools();
        let filtered = profile.filter_tools(&tools);
        assert_eq!(filtered.len(), tools.len());
    }

    #[test]
    fn test_include_only() {
        let profile = ToolProfile::include_only(&["file_*"]);
        let filtered = profile.filter_tools(&sample_tools());
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|t| t.name.starts_with("file_")));
    }

    #[test]
    fn test_exclude() {
        let profile = ToolProfile::exclude_only(&["file_*"]);
        let filtered = profile.filter_tools(&sample_tools());
        assert_eq!(filtered.len(), 3);
        assert!(filtered.iter().all(|t| !t.name.starts_with("file_")));
    }

    #[test]
    fn test_combined_include_exclude() {
        let profile = ToolProfile {
            include: vec!["file_*".to_string(), "web_*".to_string()],
            exclude: vec!["file_write".to_string()],
            max_tools: None,
            require_verified: false,
        };
        let filtered = profile.filter_tools(&sample_tools());
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().any(|t| t.name == "web_search"));
        assert!(filtered.iter().any(|t| t.name == "file_read"));
    }

    #[test]
    fn test_max_tools_truncation() {
        let profile = ToolProfile {
            include: Vec::new(),
            exclude: Vec::new(),
            max_tools: Some(2),
            require_verified: false,
        };
        let filtered = profile.filter_tools(&sample_tools());
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_require_verified() {
        let profile = ToolProfile {
            include: Vec::new(),
            exclude: Vec::new(),
            max_tools: None,
            require_verified: true,
        };
        let filtered = profile.filter_tools(&sample_tools());
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "verified_tool");
    }

    #[test]
    fn test_glob_star_matching() {
        assert!(glob_matches("web_*", "web_search"));
        assert!(glob_matches("web_*", "web_fetch"));
        assert!(!glob_matches("web_*", "file_read"));
    }

    #[test]
    fn test_glob_question_mark() {
        assert!(glob_matches("tool_?", "tool_a"));
        assert!(glob_matches("tool_?", "tool_1"));
        assert!(!glob_matches("tool_?", "tool_ab"));
    }

    #[test]
    fn test_glob_exact_match() {
        assert!(glob_matches("exact_name", "exact_name"));
        assert!(!glob_matches("exact_name", "other_name"));
    }

    #[test]
    fn test_empty_input() {
        let profile = ToolProfile::permissive();
        let filtered = profile.filter_tools(&[]);
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_serde_roundtrip() {
        let profile = ToolProfile {
            include: vec!["web_*".to_string()],
            exclude: vec!["debug_*".to_string()],
            max_tools: Some(10),
            require_verified: true,
        };
        let json = serde_json::to_string(&profile).unwrap();
        let restored: ToolProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.include, profile.include);
        assert_eq!(restored.exclude, profile.exclude);
        assert_eq!(restored.max_tools, profile.max_tools);
        assert_eq!(restored.require_verified, profile.require_verified);
    }
}
