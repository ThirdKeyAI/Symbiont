//! MCP Server implementation for Symbiont.
//!
//! Exposes Symbiont agents as MCP tools over stdio transport using the rmcp SDK.
//! MCP clients (Claude Code, Cursor, etc.) can invoke agents, list available agents,
//! parse DSL files, read agent definitions, and verify schemas via SchemaPin.

use std::future::Future;
use std::sync::Arc;

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    service::RequestContext,
    tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt,
};
use schemars::JsonSchema;
use serde::Deserialize;
use symbi_runtime::http_input::llm_client::LlmClient;
use symbi_runtime::integrations::schemapin::{
    native_client::{NativeSchemaPinClient, SchemaPinClient},
    types::VerifyArgs,
};

// ---------------------------------------------------------------------------
// Parameter structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct InvokeAgentParams {
    /// Agent name (matches .dsl filename without extension in agents/ directory)
    pub agent: String,
    /// The prompt or input to send to the agent
    pub prompt: String,
    /// Optional custom system prompt to prepend to the agent's DSL context
    pub system_prompt: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ParseDslParams {
    /// Path to a .dsl file to parse
    pub file: Option<String>,
    /// Inline DSL content to parse (used if file is not provided)
    pub content: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetAgentDslParams {
    /// Agent name (filename without .dsl extension)
    pub agent: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct VerifySchemaParams {
    /// JSON schema content to verify
    pub schema: String,
    /// URL of the public key to verify against
    pub public_key_url: String,
}

// ---------------------------------------------------------------------------
// Server struct
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct SymbiMcpServer {
    llm_client: Option<Arc<LlmClient>>,
    agent_dsl_sources: Arc<Vec<(String, String)>>,
    schema_pin: Arc<NativeSchemaPinClient>,
    tool_router: ToolRouter<Self>,
}

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

#[tool_router]
impl SymbiMcpServer {
    pub fn new() -> Self {
        let llm_client = LlmClient::from_env().map(Arc::new);
        let agent_dsl_sources = Arc::new(scan_agent_dsl_files());
        let schema_pin = Arc::new(NativeSchemaPinClient::new());
        Self {
            llm_client,
            agent_dsl_sources,
            schema_pin,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Invoke a Symbiont agent with a prompt. Sends the prompt to the named agent, which uses LLM-backed reasoning governed by its DSL definition."
    )]
    async fn invoke_agent(
        &self,
        Parameters(params): Parameters<InvokeAgentParams>,
    ) -> Result<CallToolResult, McpError> {
        let llm = match &self.llm_client {
            Some(c) => c.clone(),
            None => {
                return Ok(CallToolResult::success(vec![Content::text(
                    "No LLM provider configured. Set one of: OPENROUTER_API_KEY, OPENAI_API_KEY, or ANTHROPIC_API_KEY.",
                )]));
            }
        };

        // Find DSL sources matching the requested agent
        let agent_sources: Vec<&(String, String)> = self
            .agent_dsl_sources
            .iter()
            .filter(|(filename, _)| {
                let stem = filename.strip_suffix(".dsl").unwrap_or(filename);
                stem == params.agent
            })
            .collect();

        // Build system prompt from DSL context
        let mut system_parts: Vec<String> = Vec::new();

        if !agent_sources.is_empty() {
            system_parts.push(
                "You are an AI agent operating within the Symbiont runtime. \
                 Your behavior is governed by the following agent definitions:"
                    .to_string(),
            );
            for (filename, content) in &agent_sources {
                system_parts.push(format!("\n--- {} ---\n{}", filename, content));
            }
            system_parts.push(
                "\nFollow the capabilities and policies defined above. \
                 Provide thorough, professional analysis."
                    .to_string(),
            );
        } else {
            system_parts.push(
                "You are an AI agent operating within the Symbiont runtime. \
                 Provide thorough, professional analysis based on the input provided."
                    .to_string(),
            );
        }

        // Inject auto-generated AGENTS.md context (safe: only parser-derived content)
        if let Some(context) = load_agents_md_context() {
            system_parts.push(format!(
                "\n<project-context>\n{}\n</project-context>",
                context
            ));
        }

        if let Some(custom) = &params.system_prompt {
            system_parts.push(format!("\n{}", custom));
        }

        let system_prompt = system_parts.join("\n");

        match llm.chat_completion(&system_prompt, &params.prompt).await {
            Ok(response) => Ok(CallToolResult::success(vec![Content::text(response)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "LLM invocation failed: {}",
                e
            ))])),
        }
    }

    #[tool(description = "List available Symbiont agents found in the agents/ directory.")]
    async fn list_agents(&self) -> Result<CallToolResult, McpError> {
        let agents: Vec<serde_json::Value> = self
            .agent_dsl_sources
            .iter()
            .map(|(filename, content)| {
                let name = filename.strip_suffix(".dsl").unwrap_or(filename);

                // Quick check for schedule/channel blocks
                let has_schedules = content.contains("schedule ");
                let has_channels = content.contains("channel ");

                serde_json::json!({
                    "name": name,
                    "file": filename,
                    "has_schedules": has_schedules,
                    "has_channels": has_channels,
                })
            })
            .collect();

        let json = serde_json::to_string_pretty(&agents).unwrap_or_else(|_| "[]".to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Parse and validate Symbiont DSL content. Provide either a file path or inline DSL content. Returns metadata, with-blocks, schedules, channels, and any parse errors."
    )]
    async fn parse_dsl(
        &self,
        Parameters(params): Parameters<ParseDslParams>,
    ) -> Result<CallToolResult, McpError> {
        let (source, label) = if let Some(ref file) = params.file {
            // Validate path: must be relative, no traversal, and end in .dsl
            let path = std::path::Path::new(file);
            if path.is_absolute()
                || path
                    .components()
                    .any(|c| c == std::path::Component::ParentDir)
            {
                return Ok(CallToolResult::error(vec![Content::text(
                    "File path must be relative and cannot contain '..' components.",
                )]));
            }
            if path.extension().and_then(|e| e.to_str()) != Some("dsl") {
                return Ok(CallToolResult::error(vec![Content::text(
                    "Only .dsl files can be parsed.",
                )]));
            }
            match tokio::fs::read_to_string(file).await {
                Ok(content) => (content, file.clone()),
                Err(e) => {
                    return Ok(CallToolResult::error(vec![Content::text(format!(
                        "Failed to read file '{}': {}",
                        file, e
                    ))]));
                }
            }
        } else if let Some(ref content) = params.content {
            (content.clone(), "<inline>".to_string())
        } else {
            return Ok(CallToolResult::error(vec![Content::text(
                "Either 'file' or 'content' must be provided.",
            )]));
        };

        let tree = match dsl::parse_dsl(&source) {
            Ok(t) => t,
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "DSL parse error ({}): {}",
                    label, e
                ))]));
            }
        };

        let root = tree.root_node();
        let has_errors = root.has_error();

        let metadata = dsl::extract_metadata(&tree, &source);

        let with_blocks = dsl::extract_with_blocks(&tree, &source).unwrap_or_default();
        let with_blocks_json: Vec<serde_json::Value> = with_blocks
            .iter()
            .map(|wb| {
                serde_json::json!({
                    "sandbox_tier": wb.sandbox_tier.as_ref().map(|t| t.to_string()),
                    "timeout": wb.timeout,
                    "attributes": wb.attributes.iter().map(|a| {
                        serde_json::json!({ "name": a.name, "value": a.value })
                    }).collect::<Vec<_>>(),
                })
            })
            .collect();

        let schedules = dsl::extract_schedule_definitions(&tree, &source).unwrap_or_default();
        let schedules_json: Vec<serde_json::Value> = schedules
            .iter()
            .map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "cron": s.cron,
                    "at": s.at,
                    "timezone": s.timezone,
                    "agent": s.agent,
                    "policy": s.policy,
                    "one_shot": s.one_shot,
                    "deliver": s.deliver,
                })
            })
            .collect();

        let channels = dsl::extract_channel_definitions(&tree, &source).unwrap_or_default();
        let channels_json: Vec<serde_json::Value> = channels
            .iter()
            .map(|ch| {
                serde_json::json!({
                    "name": ch.name,
                    "platform": ch.platform,
                    "workspace": ch.workspace,
                    "channels": ch.channels,
                    "default_agent": ch.default_agent,
                })
            })
            .collect();

        let result = serde_json::json!({
            "source": label,
            "has_errors": has_errors,
            "metadata": metadata,
            "with_blocks": with_blocks_json,
            "schedules": schedules_json,
            "channels": channels_json,
        });

        let json = serde_json::to_string_pretty(&result).unwrap_or_default();
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Get the raw DSL source for a specific agent. Returns the full .dsl file content."
    )]
    async fn get_agent_dsl(
        &self,
        Parameters(params): Parameters<GetAgentDslParams>,
    ) -> Result<CallToolResult, McpError> {
        // First check pre-scanned sources
        for (filename, content) in self.agent_dsl_sources.iter() {
            let stem = filename.strip_suffix(".dsl").unwrap_or(filename);
            if stem == params.agent {
                return Ok(CallToolResult::success(vec![Content::text(
                    content.clone(),
                )]));
            }
        }

        // Validate agent name: alphanumeric, hyphens, underscores only
        if !params
            .agent
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            return Ok(CallToolResult::error(vec![Content::text(
                "Agent name must contain only alphanumeric characters, hyphens, and underscores.",
            )]));
        }
        // Fall back to reading from disk (in case agents were added after startup)
        let path = format!("agents/{}.dsl", params.agent);
        match tokio::fs::read_to_string(&path).await {
            Ok(content) => Ok(CallToolResult::success(vec![Content::text(content)])),
            Err(_) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Agent '{}' not found. Use list_agents to see available agents.",
                params.agent
            ))])),
        }
    }

    #[tool(
        description = "Get the project's AGENTS.md file content. Returns the full AGENTS.md from the working directory, which describes available agents, their capabilities, schedules, channels, and invocation methods."
    )]
    async fn get_agents_md(&self) -> Result<CallToolResult, McpError> {
        match tokio::fs::read_to_string("AGENTS.md").await {
            Ok(content) => Ok(CallToolResult::success(vec![Content::text(content)])),
            Err(_) => Ok(CallToolResult::error(vec![Content::text(
                "No AGENTS.md found in the working directory. Run 'symbi agents-md generate' to create one.",
            )])),
        }
    }

    #[tool(
        description = "Verify an MCP tool schema using SchemaPin (ECDSA P-256 signature verification). Checks schema integrity against a public key published at a well-known URL."
    )]
    async fn verify_schema(
        &self,
        Parameters(params): Parameters<VerifySchemaParams>,
    ) -> Result<CallToolResult, McpError> {
        // Write schema content to a temp file for the native client
        let tmp = match tempfile::NamedTempFile::new() {
            Ok(t) => t,
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Failed to create temp file: {}",
                    e
                ))]));
            }
        };
        if let Err(e) = tokio::fs::write(tmp.path(), &params.schema).await {
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "Failed to write schema to temp file: {}",
                e
            ))]));
        }

        let args = VerifyArgs::new(
            tmp.path().to_string_lossy().to_string(),
            params.public_key_url.clone(),
        );

        match self.schema_pin.verify_schema(args).await {
            Ok(result) => {
                let json = serde_json::json!({
                    "verified": result.success,
                    "message": result.message,
                    "schema_hash": result.schema_hash,
                    "public_key_url": result.public_key_url,
                    "signature": result.signature.map(|s| serde_json::json!({
                        "algorithm": s.algorithm,
                        "key_fingerprint": s.key_fingerprint,
                        "valid": s.valid,
                    })),
                    "timestamp": result.timestamp,
                });
                let text = serde_json::to_string_pretty(&json).unwrap_or_else(|_| json.to_string());
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Schema verification failed: {}",
                e
            ))])),
        }
    }
}

// ---------------------------------------------------------------------------
// ServerHandler — #[tool_handler] auto-generates list_tools + call_tool
// ---------------------------------------------------------------------------

#[tool_handler]
impl ServerHandler for SymbiMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "Symbiont AI Agent Runtime — invoke agents, parse DSL, \
                 manage agent definitions, verify schemas via SchemaPin"
                    .to_string(),
            ),
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
            ..Default::default()
        }
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, McpError>> + Send + '_ {
        let resources = if std::path::Path::new("AGENTS.md").exists() {
            vec![Resource {
                raw: RawResource {
                    uri: "file:///AGENTS.md".to_string(),
                    name: "AGENTS.md".to_string(),
                    title: None,
                    description: Some("Project agent instructions and topology".to_string()),
                    mime_type: Some("text/markdown".to_string()),
                    size: None,
                    icons: None,
                    meta: None,
                },
                annotations: None,
            }]
        } else {
            vec![]
        };
        std::future::ready(Ok(ListResourcesResult {
            resources,
            ..Default::default()
        }))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        if request.uri == "file:///AGENTS.md" {
            match tokio::fs::read_to_string("AGENTS.md").await {
                Ok(content) => Ok(ReadResourceResult {
                    contents: vec![ResourceContents::text(content, "file:///AGENTS.md")],
                }),
                Err(_) => Err(McpError::new(
                    ErrorCode::INVALID_PARAMS,
                    "AGENTS.md not found",
                    None::<serde_json::Value>,
                )),
            }
        } else {
            Err(McpError::new(
                ErrorCode::INVALID_PARAMS,
                format!("Unknown resource: {}", request.uri),
                None::<serde_json::Value>,
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Scan the agents/ directory for .dsl files and return (filename, content) pairs.
fn scan_agent_dsl_files() -> Vec<(String, String)> {
    let agents_dir = std::path::Path::new("agents");
    let mut sources = Vec::new();

    if !agents_dir.exists() || !agents_dir.is_dir() {
        return sources;
    }

    if let Ok(entries) = std::fs::read_dir(agents_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "dsl") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let filename = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    sources.push((filename, content));
                }
            }
        }
    }

    sources
}

/// Load the auto-generated section from AGENTS.md for safe context injection.
///
/// Only returns content between `<!-- agents-md:auto-start -->` and
/// `<!-- agents-md:auto-end -->` markers — this is DSL-parser-derived content,
/// not arbitrary user markdown, which eliminates prompt injection risk.
/// Truncates to 2000 chars to avoid blowing context windows.
fn load_agents_md_context() -> Option<String> {
    let content = std::fs::read_to_string("AGENTS.md").ok()?;
    let section = crate::commands::agents_md::extract_auto_section(&content)?;
    if section.is_empty() {
        return None;
    }
    let truncated = if section.len() > 2000 {
        format!("{}...", &section[..2000])
    } else {
        section.to_string()
    };
    Some(truncated)
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Start the MCP server over stdio transport.
pub async fn start_mcp_server() -> Result<(), Box<dyn std::error::Error>> {
    // Direct tracing to stderr — stdout is the MCP transport channel
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::WARN.into()),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let service = SymbiMcpServer::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
