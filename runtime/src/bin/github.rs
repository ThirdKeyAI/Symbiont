//! GitHub integration for MCP server discovery

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose, Engine as _};
use octocrab::Octocrab;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use url::Url;

#[derive(Debug, Clone)]
pub struct GitHubClient {
    client: Octocrab,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerInfo {
    pub name: String,
    pub description: Option<String>,
    pub entry_point: String,
    pub tools: Vec<McpToolInfo>,
    pub resources: Vec<McpResourceInfo>,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolInfo {
    pub name: String,
    pub description: Option<String>,
    pub schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResourceInfo {
    pub name: String,
    pub description: Option<String>,
    pub uri_template: String,
}

#[derive(Debug, Clone)]
pub struct GitHubRepoInfo {
    pub owner: String,
    pub repo: String,
    pub branch: Option<String>,
    pub path: Option<String>,
}

impl GitHubClient {
    /// Create new GitHub client
    pub fn new(token: Option<String>) -> Result<Self> {
        let mut builder = Octocrab::builder();
        
        if let Some(token) = token {
            builder = builder.personal_token(token);
        }
        
        let client = builder.build()?;
        
        Ok(Self { client })
    }
    
    /// Parse GitHub URL to extract repository information
    pub fn parse_github_url(url_str: &str) -> Result<GitHubRepoInfo> {
        let url = Url::parse(url_str)?;
        
        if url.host_str() != Some("github.com") {
            return Err(anyhow!("Not a GitHub URL: {}", url_str));
        }
        
        let path_segments: Vec<&str> = url.path_segments()
            .ok_or_else(|| anyhow!("Invalid GitHub URL path"))?
            .collect();
        
        if path_segments.len() < 2 {
            return Err(anyhow!("GitHub URL must contain owner and repository"));
        }
        
        let owner = path_segments[0].to_string();
        let repo = path_segments[1].to_string();
        
        // Handle different GitHub URL formats
        let (branch, path) = if path_segments.len() > 2 {
            match path_segments[2] {
                "tree" | "blob" => {
                    if path_segments.len() > 3 {
                        let branch = Some(path_segments[3].to_string());
                        let path = if path_segments.len() > 4 {
                            Some(path_segments[4..].join("/"))
                        } else {
                            None
                        };
                        (branch, path)
                    } else {
                        (None, None)
                    }
                }
                _ => (None, Some(path_segments[2..].join("/"))),
            }
        } else {
            (None, None)
        };
        
        Ok(GitHubRepoInfo {
            owner,
            repo,
            branch,
            path,
        })
    }
    
    /// Discover MCP servers in a GitHub repository
    pub async fn discover_mcp_servers(&self, repo_info: &GitHubRepoInfo) -> Result<Vec<McpServerInfo>> {
        let mut servers = Vec::new();
        
        // Check for common MCP server locations
        let search_paths = vec![
            "package.json",
            "mcp.json",
            "mcp-server.json", 
            ".mcp/config.json",
            "src/mcp.json",
            "server.js",
            "main.py",
            "README.md",
        ];
        
        let branch = repo_info.branch.as_deref().unwrap_or("main");
        
        for path in search_paths {
            if let Ok(content) = self.get_file_content(repo_info, branch, path).await {
                if let Some(server_info) = self.parse_mcp_config(&content, path).await? {
                    servers.push(server_info);
                }
            }
        }
        
        // If no explicit MCP config found, try to infer from repository structure
        if servers.is_empty() {
            if let Some(inferred_server) = self.infer_mcp_server(repo_info, branch).await? {
                servers.push(inferred_server);
            }
        }
        
        Ok(servers)
    }
    
    /// Get file content from GitHub repository
    async fn get_file_content(
        &self,
        repo_info: &GitHubRepoInfo,
        branch: &str,
        file_path: &str,
    ) -> Result<String> {
        let content = self
            .client
            .repos(&repo_info.owner, &repo_info.repo)
            .get_content()
            .path(file_path)
            .r#ref(branch)
            .send()
            .await?;
        
        match content.items.first() {
            Some(file) => {
                if let Some(content) = &file.content {
                    let decoded = general_purpose::STANDARD
                        .decode(content.replace('\n', ""))?;
                    Ok(String::from_utf8(decoded)?)
                } else {
                    Err(anyhow!("File content is empty"))
                }
            }
            None => Err(anyhow!("File not found")),
        }
    }
    
    /// Parse MCP configuration from file content
    async fn parse_mcp_config(
        &self,
        content: &str,
        file_path: &str,
    ) -> Result<Option<McpServerInfo>> {
        match file_path {
            "package.json" => self.parse_package_json(content).await,
            path if path.ends_with(".json") => self.parse_mcp_json(content).await,
            "README.md" => self.parse_readme_mcp_info(content).await,
            _ => Ok(None),
        }
    }
    
    /// Parse package.json for MCP server information
    async fn parse_package_json(&self, content: &str) -> Result<Option<McpServerInfo>> {
        let package_json: serde_json::Value = serde_json::from_str(content)?;
        
        // Check if this is an MCP server package
        if let Some(keywords) = package_json.get("keywords").and_then(|k| k.as_array()) {
            let has_mcp = keywords.iter().any(|k| {
                k.as_str().is_some_and(|s| s.contains("mcp") || s.contains("model-context-protocol"))
            });
            
            if has_mcp {
                let name = package_json.get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                
                let description = package_json.get("description")
                    .and_then(|d| d.as_str())
                    .map(|s| s.to_string());
                
                let entry_point = package_json.get("main")
                    .and_then(|m| m.as_str())
                    .unwrap_or("index.js")
                    .to_string();
                
                return Ok(Some(McpServerInfo {
                    name,
                    description,
                    entry_point,
                    tools: Vec::new(), // Will be discovered later
                    resources: Vec::new(),
                    metadata: HashMap::new(),
                }));
            }
        }
        
        Ok(None)
    }
    
    /// Parse MCP-specific JSON configuration
    async fn parse_mcp_json(&self, content: &str) -> Result<Option<McpServerInfo>> {
        let mcp_config: serde_json::Value = serde_json::from_str(content)?;
        
        let name = mcp_config.get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("unknown")
            .to_string();
        
        let description = mcp_config.get("description")
            .and_then(|d| d.as_str())
            .map(|s| s.to_string());
        
        let entry_point = mcp_config.get("entry_point")
            .and_then(|e| e.as_str())
            .unwrap_or("server.js")
            .to_string();
        
        // Parse tools if present
        let tools = if let Some(tools_array) = mcp_config.get("tools").and_then(|t| t.as_array()) {
            tools_array
                .iter()
                .filter_map(|tool| {
                    let name = tool.get("name")?.as_str()?.to_string();
                    let description = tool.get("description").and_then(|d| d.as_str()).map(|s| s.to_string());
                    let schema = tool.get("schema").cloned().unwrap_or(serde_json::json!({}));
                    
                    Some(McpToolInfo {
                        name,
                        description,
                        schema,
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        
        Ok(Some(McpServerInfo {
            name,
            description,
            entry_point,
            tools,
            resources: Vec::new(),
            metadata: HashMap::new(),
        }))
    }
    
    /// Parse README for MCP server information
    async fn parse_readme_mcp_info(&self, content: &str) -> Result<Option<McpServerInfo>> {
        // Simple heuristic: check if README mentions MCP
        let content_lower = content.to_lowercase();
        if content_lower.contains("mcp") || content_lower.contains("model context protocol") {
            // Extract title as server name
            let name = content.lines()
                .find(|line| line.starts_with('#'))
                .map(|line| line.trim_start_matches('#').trim().to_string())
                .unwrap_or_else(|| "MCP Server".to_string());
            
            return Ok(Some(McpServerInfo {
                name,
                description: Some("MCP Server discovered from README".to_string()),
                entry_point: "server.js".to_string(),
                tools: Vec::new(),
                resources: Vec::new(),
                metadata: HashMap::new(),
            }));
        }
        
        Ok(None)
    }
    
    /// Infer MCP server from repository structure
    async fn infer_mcp_server(
        &self,
        repo_info: &GitHubRepoInfo,
        branch: &str,
    ) -> Result<Option<McpServerInfo>> {
        // Check for common server files
        let server_files = vec!["server.js", "main.py", "index.js", "app.py"];
        
        for file in server_files {
            if self.get_file_content(repo_info, branch, file).await.is_ok() {
                return Ok(Some(McpServerInfo {
                    name: format!("{}/{}", repo_info.owner, repo_info.repo),
                    description: Some("Inferred MCP server".to_string()),
                    entry_point: file.to_string(),
                    tools: Vec::new(),
                    resources: Vec::new(),
                    metadata: HashMap::new(),
                }));
            }
        }
        
        Ok(None)
    }
    
    /// Get repository information
    pub async fn get_repo_info(&self, owner: &str, repo: &str) -> Result<serde_json::Value> {
        let repo = self.client.repos(owner, repo).get().await?;
        Ok(serde_json::to_value(repo)?)
    }
}
