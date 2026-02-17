//! ComposioMcpSource — discovers Composio tools and prepares them for SecureMcpClient registration
//!
//! Connects to Composio MCP servers, lists available tools via JSON-RPC `tools/list`,
//! and converts them into `McpTool` instances with policy filtering applied.

use std::collections::HashMap;

use regex::Regex;

use super::config::{
    resolve_secret, ComposioGlobalConfig, McpConfigFile, McpServerEntry, ServerPolicy,
};
use super::error::ComposioError;
use super::transport::SseTransport;
use crate::integrations::mcp::types::{McpTool, ToolProvider, VerificationStatus};

/// Discovers and manages Composio-hosted MCP tools
pub struct ComposioMcpSource {
    config: ComposioGlobalConfig,
    servers: Vec<McpServerEntry>,
    transports: HashMap<String, SseTransport>,
}

impl ComposioMcpSource {
    /// Create a source from a parsed MCP config file.
    ///
    /// Filters out non-Composio server entries and resolves the API key.
    pub fn from_config(config: McpConfigFile) -> Result<Self, ComposioError> {
        let composio_config = config.composio.ok_or_else(|| ComposioError::ConfigError {
            reason: "missing [composio] section in config".to_string(),
        })?;

        Ok(Self {
            config: composio_config,
            servers: config.mcp_servers,
            transports: HashMap::new(),
        })
    }

    /// Discover tools from all configured Composio servers.
    pub async fn discover_all(&mut self) -> Result<Vec<McpTool>, ComposioError> {
        let composio_servers: Vec<_> = self
            .servers
            .iter()
            .filter_map(|s| match s {
                McpServerEntry::Composio {
                    name,
                    server_id,
                    user_id,
                    url,
                    policy,
                } => Some((
                    name.clone(),
                    server_id.clone(),
                    user_id.clone(),
                    url.clone(),
                    policy.clone(),
                )),
                _ => None,
            })
            .collect();

        let mut all_tools = Vec::new();
        for (name, server_id, user_id, url, policy) in composio_servers {
            let tools = self
                .discover_composio_server(
                    &name,
                    &server_id,
                    &user_id,
                    url.as_deref(),
                    policy.as_ref(),
                )
                .await?;
            all_tools.extend(tools);
        }

        Ok(all_tools)
    }

    /// Discover tools from a specific server by name.
    pub async fn discover_server(&mut self, name: &str) -> Result<Vec<McpTool>, ComposioError> {
        let server = self
            .servers
            .iter()
            .find(|s| s.name() == name)
            .ok_or_else(|| ComposioError::DiscoveryError {
                server: name.to_string(),
                reason: format!("no server named '{name}' in config"),
            })?
            .clone();

        match server {
            McpServerEntry::Composio {
                name,
                server_id,
                user_id,
                url,
                policy,
            } => {
                self.discover_composio_server(
                    &name,
                    &server_id,
                    &user_id,
                    url.as_deref(),
                    policy.as_ref(),
                )
                .await
            }
            McpServerEntry::Stdio { name, .. } => Err(ComposioError::DiscoveryError {
                server: name,
                reason: "stdio servers are not managed by ComposioMcpSource".to_string(),
            }),
        }
    }

    /// Internal: discover tools from a single Composio server
    async fn discover_composio_server(
        &mut self,
        name: &str,
        server_id: &str,
        user_id: &str,
        direct_url: Option<&str>,
        policy: Option<&ServerPolicy>,
    ) -> Result<Vec<McpTool>, ComposioError> {
        let api_key = resolve_secret(&self.config.api_key)?;
        let url = match direct_url {
            Some(u) => u.to_string(),
            None => self.build_endpoint_url(server_id, user_id),
        };

        // Reuse or create transport
        if !self.transports.contains_key(name) {
            let transport = SseTransport::new(url.clone(), api_key);
            self.transports.insert(name.to_string(), transport);
        }

        let transport = self.transports.get(name).unwrap();

        // Call tools/list via JSON-RPC
        let result = transport
            .request("tools/list", serde_json::json!({}))
            .await
            .map_err(|e| ComposioError::DiscoveryError {
                server: name.to_string(),
                reason: e.to_string(),
            })?;

        // Parse tool list from response
        let tools_array = result
            .get("tools")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ComposioError::DiscoveryError {
                server: name.to_string(),
                reason: "response missing 'tools' array".to_string(),
            })?;

        let mut mcp_tools = Vec::new();
        for raw_tool in tools_array {
            let tool = self.to_mcp_tool(raw_tool, name)?;

            // Apply policy filter
            if let Some(policy) = policy {
                if !policy.allowed_tools.is_empty() && !is_tool_allowed(&tool.name, policy) {
                    continue;
                }
            }

            mcp_tools.push(tool);
        }

        Ok(mcp_tools)
    }

    /// Build the Composio MCP endpoint URL for a given server.
    fn build_endpoint_url(&self, server_id: &str, user_id: &str) -> String {
        format!(
            "{}/v3/mcp/{}?user_id={}",
            self.config.base_url, server_id, user_id
        )
    }

    /// Convert a raw Composio tool JSON response to an `McpTool`.
    fn to_mcp_tool(
        &self,
        raw_tool: &serde_json::Value,
        server_name: &str,
    ) -> Result<McpTool, ComposioError> {
        let name = raw_tool
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ComposioError::DiscoveryError {
                server: server_name.to_string(),
                reason: "tool missing 'name' field".to_string(),
            })?
            .to_string();

        let description = raw_tool
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let schema = raw_tool
            .get("inputSchema")
            .or_else(|| raw_tool.get("input_schema"))
            .cloned()
            .unwrap_or(serde_json::json!({}));

        Ok(McpTool {
            name,
            description,
            schema,
            provider: ToolProvider {
                identifier: format!("composio.dev/{}", server_name),
                name: server_name.to_string(),
                public_key_url: String::new(),
                version: None,
            },
            verification_status: VerificationStatus::Skipped {
                reason: "Composio-hosted tool (external MCP)".to_string(),
            },
            metadata: None,
            sensitive_params: Vec::new(),
        })
    }
}

/// Check whether a tool name is allowed by the policy's `allowed_tools` glob patterns.
///
/// Converts simple glob patterns (`*` → `.*`) to regex for matching.
pub fn is_tool_allowed(tool_name: &str, policy: &ServerPolicy) -> bool {
    if policy.allowed_tools.is_empty() {
        return true;
    }

    for pattern in &policy.allowed_tools {
        let regex_pattern = format!("^{}$", glob_to_regex(pattern));
        if let Ok(re) = Regex::new(&regex_pattern) {
            if re.is_match(tool_name) {
                return true;
            }
        }
    }

    false
}

/// Check whether a tool name matches any of the `require_approval` glob patterns.
pub fn requires_approval(tool_name: &str, policy: &ServerPolicy) -> bool {
    for pattern in &policy.require_approval {
        let regex_pattern = format!("^{}$", glob_to_regex(pattern));
        if let Ok(re) = Regex::new(&regex_pattern) {
            if re.is_match(tool_name) {
                return true;
            }
        }
    }

    false
}

/// Convert a simple glob pattern to a regex string.
///
/// Only handles `*` (match any characters) and `?` (match single character).
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
    use crate::integrations::composio::config::ServerPolicy;

    #[test]
    fn test_build_endpoint_url() {
        let config = McpConfigFile {
            composio: Some(ComposioGlobalConfig {
                api_key: "test-key".to_string(),
                base_url: "https://backend.composio.dev".to_string(),
            }),
            mcp_servers: vec![],
        };
        let source = ComposioMcpSource::from_config(config).unwrap();
        let url = source.build_endpoint_url("srv_123", "usr_456");
        assert_eq!(
            url,
            "https://backend.composio.dev/v3/mcp/srv_123?user_id=usr_456"
        );
    }

    #[test]
    fn test_build_endpoint_url_custom_base() {
        let config = McpConfigFile {
            composio: Some(ComposioGlobalConfig {
                api_key: "test-key".to_string(),
                base_url: "https://custom.composio.dev".to_string(),
            }),
            mcp_servers: vec![],
        };
        let source = ComposioMcpSource::from_config(config).unwrap();
        let url = source.build_endpoint_url("srv_abc", "usr_def");
        assert_eq!(
            url,
            "https://custom.composio.dev/v3/mcp/srv_abc?user_id=usr_def"
        );
    }

    #[test]
    fn test_to_mcp_tool() {
        let config = McpConfigFile {
            composio: Some(ComposioGlobalConfig {
                api_key: "key".to_string(),
                base_url: "https://backend.composio.dev".to_string(),
            }),
            mcp_servers: vec![],
        };
        let source = ComposioMcpSource::from_config(config).unwrap();

        let raw = serde_json::json!({
            "name": "GITHUB_CREATE_ISSUE",
            "description": "Create a GitHub issue",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": {"type": "string"},
                    "body": {"type": "string"}
                }
            }
        });

        let tool = source.to_mcp_tool(&raw, "github").unwrap();
        assert_eq!(tool.name, "GITHUB_CREATE_ISSUE");
        assert_eq!(tool.description, "Create a GitHub issue");
        assert_eq!(tool.provider.identifier, "composio.dev/github");
        assert_eq!(tool.provider.name, "github");
        assert!(tool.provider.public_key_url.is_empty());
        assert!(matches!(
            tool.verification_status,
            VerificationStatus::Skipped { .. }
        ));
    }

    #[test]
    fn test_to_mcp_tool_minimal() {
        let config = McpConfigFile {
            composio: Some(ComposioGlobalConfig {
                api_key: "key".to_string(),
                base_url: "https://backend.composio.dev".to_string(),
            }),
            mcp_servers: vec![],
        };
        let source = ComposioMcpSource::from_config(config).unwrap();

        let raw = serde_json::json!({
            "name": "SLACK_SEND_MESSAGE"
        });

        let tool = source.to_mcp_tool(&raw, "slack").unwrap();
        assert_eq!(tool.name, "SLACK_SEND_MESSAGE");
        assert_eq!(tool.description, "");
        assert_eq!(tool.schema, serde_json::json!({}));
    }

    #[test]
    fn test_to_mcp_tool_missing_name() {
        let config = McpConfigFile {
            composio: Some(ComposioGlobalConfig {
                api_key: "key".to_string(),
                base_url: "https://backend.composio.dev".to_string(),
            }),
            mcp_servers: vec![],
        };
        let source = ComposioMcpSource::from_config(config).unwrap();

        let raw = serde_json::json!({
            "description": "no name here"
        });

        let result = source.to_mcp_tool(&raw, "test");
        assert!(result.is_err());
    }

    #[test]
    fn test_policy_glob_matching_star() {
        let policy = ServerPolicy {
            allowed_tools: vec!["GITHUB_*".to_string()],
            ..Default::default()
        };
        assert!(is_tool_allowed("GITHUB_CREATE_ISSUE", &policy));
        assert!(is_tool_allowed("GITHUB_LIST_REPOS", &policy));
        assert!(!is_tool_allowed("SLACK_SEND_MESSAGE", &policy));
    }

    #[test]
    fn test_policy_glob_matching_question_mark() {
        let policy = ServerPolicy {
            allowed_tools: vec!["TOOL_?".to_string()],
            ..Default::default()
        };
        assert!(is_tool_allowed("TOOL_A", &policy));
        assert!(is_tool_allowed("TOOL_1", &policy));
        assert!(!is_tool_allowed("TOOL_AB", &policy));
    }

    #[test]
    fn test_policy_glob_empty_allows_all() {
        let policy = ServerPolicy {
            allowed_tools: vec![],
            ..Default::default()
        };
        assert!(is_tool_allowed("ANYTHING", &policy));
    }

    #[test]
    fn test_policy_glob_multiple_patterns() {
        let policy = ServerPolicy {
            allowed_tools: vec!["GITHUB_*".to_string(), "SLACK_*".to_string()],
            ..Default::default()
        };
        assert!(is_tool_allowed("GITHUB_CREATE_ISSUE", &policy));
        assert!(is_tool_allowed("SLACK_SEND_MESSAGE", &policy));
        assert!(!is_tool_allowed("JIRA_CREATE_TICKET", &policy));
    }

    #[test]
    fn test_requires_approval() {
        let policy = ServerPolicy {
            require_approval: vec!["*_DELETE_*".to_string()],
            ..Default::default()
        };
        assert!(requires_approval("GITHUB_DELETE_REPO", &policy));
        assert!(!requires_approval("GITHUB_CREATE_ISSUE", &policy));
    }

    #[test]
    fn test_from_config_missing_composio_section() {
        let config = McpConfigFile {
            composio: None,
            mcp_servers: vec![],
        };
        let result = ComposioMcpSource::from_config(config);
        assert!(result.is_err());
    }

    #[test]
    fn test_from_config_success() {
        let config = McpConfigFile {
            composio: Some(ComposioGlobalConfig {
                api_key: "test-key".to_string(),
                base_url: "https://backend.composio.dev".to_string(),
            }),
            mcp_servers: vec![McpServerEntry::Composio {
                name: "github".to_string(),
                server_id: "srv_1".to_string(),
                user_id: "usr_1".to_string(),
                url: None,
                policy: None,
            }],
        };
        let source = ComposioMcpSource::from_config(config).unwrap();
        assert_eq!(source.servers.len(), 1);
    }

    #[test]
    fn test_glob_to_regex() {
        assert_eq!(glob_to_regex("GITHUB_*"), "GITHUB_.*");
        assert_eq!(glob_to_regex("TOOL_?"), "TOOL_.");
        assert_eq!(glob_to_regex("exact_match"), "exact_match");
        assert_eq!(glob_to_regex("has.dot"), "has\\.dot");
    }
}
