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
use symbi_runtime::integrations::mcp::project::ProjectReader;
use symbi_runtime::integrations::schemapin::{
    native_client::{NativeSchemaPinClient, SchemaPinClient},
    types::VerifyArgs,
};
use symbi_runtime::reasoning::{
    inference::InferenceProvider,
    policy_bridge::ReasoningPolicyGate,
    providers::cloud::CloudInferenceProvider,
    response_run::{run_response, ResponseRequest},
};

// ---------------------------------------------------------------------------
// Parameter structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct InvokeAgentParams {
    /// Agent name (matches `.symbi` filename without extension in agents/ directory; `.dsl` also accepted)
    pub agent: String,
    /// The prompt or input to send to the agent
    pub prompt: String,
    /// Optional caller instructions; these cannot override runtime policy or source identity
    pub system_prompt: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ParseDslParams {
    /// Path to a `.symbi` (or legacy `.dsl`) file to parse
    pub file: Option<String>,
    /// Inline DSL content to parse (used if file is not provided)
    pub content: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetAgentDslParams {
    /// Agent name (filename without `.symbi` / `.dsl` extension)
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
    provider: Option<Arc<dyn InferenceProvider>>,
    project: Result<Arc<ProjectReader>, String>,
    response_gate: Arc<tokio::sync::OnceCell<Arc<dyn ReasoningPolicyGate>>>,
    agent_dsl_sources: Arc<Vec<(String, String)>>,
    schema_pin: Arc<NativeSchemaPinClient>,
    // Used by `#[tool_handler]`-generated code via `self.tool_router.call(...)`.
    // The dead-code pass cannot see the macro-expanded reference.
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

#[tool_router]
impl SymbiMcpServer {
    pub fn new() -> Self {
        let provider = CloudInferenceProvider::from_env()
            .map(|provider| Arc::new(provider) as Arc<dyn InferenceProvider>);
        let project = std::env::current_dir()
            .map_err(|error| error.to_string())
            .and_then(|path| ProjectReader::open(&path))
            .map(Arc::new);
        let agent_dsl_sources = Arc::new(
            project
                .as_ref()
                .map(|reader| scan_agent_dsl_files(reader))
                .unwrap_or_default(),
        );
        let schema_pin = Arc::new(NativeSchemaPinClient::new());
        Self {
            provider,
            project,
            response_gate: Arc::new(tokio::sync::OnceCell::new()),
            agent_dsl_sources,
            schema_pin,
            tool_router: Self::tool_router(),
        }
    }

    fn read_project_file(&self, path: &str, limit: usize) -> Result<String, String> {
        self.project
            .as_ref()
            .map_err(Clone::clone)?
            .read_text(std::path::Path::new(path), limit)
    }

    #[tool(
        description = "Request one policy-checked text response from a registered conversational agent. Requires protected audit storage; returns its audit reference. Tools and executable DSL statements are unavailable on this route."
    )]
    async fn invoke_agent(
        &self,
        Parameters(params): Parameters<InvokeAgentParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let invoke = async {
            let project = self.project.as_ref().map_err(Clone::clone)?;
            let candidates: Vec<_> = self
                .agent_dsl_sources
                .iter()
                .filter(|(filename, _)| {
                    dsl::strip_symbi_extension(filename).unwrap_or(filename) == params.agent
                })
                .collect();
            if candidates.len() != 1 {
                return Err("agent name must identify one registered source file".to_string());
            }
            let (_, source) = candidates[0];
            let settings = dsl::resolve_execution_settings(source, &params.agent)?;
            let agent = dsl::ConversationalAgent::parse(source, &settings.agent_name)?;
            let provider = self.provider.clone().ok_or("No LLM provider configured")?;
            let gate = self
                .response_gate
                .get_or_init(|| async {
                    symbi_runtime::reasoning::governed_gate(symbi_runtime::reasoning::GateOptions {
                        policies_dir: project.path().join("policies"),
                        surface: Some("mcp-server".into()),
                        insecure_allow_all: false,
                        escalation: None,
                    })
                    .await
                })
                .await
                .clone();
            let mut instructions = Vec::new();
            if let Some(context) = load_agents_md_context(project) {
                instructions.push(format!("Project guidance (caller input):\n{context}"));
            }
            if let Some(custom) = params.system_prompt {
                instructions.push(custom);
            }
            let outcome = run_response(ResponseRequest {
                project: project.path().to_owned(),
                agent,
                surface: "mcp-server".into(),
                input: params.prompt,
                caller_instructions: (!instructions.is_empty()).then(|| instructions.join("\n\n")),
                provider,
                gate,
                cancellation: context.ct.child_token(),
            })
            .await;
            let mut result = match outcome.result {
                Ok(response) => CallToolResult::success(vec![Content::text(response)]),
                Err(error) => CallToolResult::error(vec![Content::text(error)]),
            };
            result.structured_content =
                Some(serde_json::json!({"agent_id":outcome.agent_id,"audit":outcome.audit}));
            Ok::<_, String>(result)
        }
        .await;
        Ok(invoke.unwrap_or_else(|error| CallToolResult::error(vec![Content::text(error)])))
    }

    #[tool(description = "List available Symbiont agents found in the agents/ directory.")]
    async fn list_agents(&self) -> Result<CallToolResult, McpError> {
        let agents: Vec<serde_json::Value> = self
            .agent_dsl_sources
            .iter()
            .map(|(filename, content)| {
                let name = dsl::strip_symbi_extension(filename).unwrap_or(filename);

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
            if !dsl::is_symbi_file(path) {
                return Ok(CallToolResult::error(vec![Content::text(
                    "Only .symbi (or legacy .dsl) files can be parsed.",
                )]));
            }
            match self.read_project_file(file, 1024 * 1024) {
                Ok(content) => (content, file.clone()),
                Err(e) => {
                    return Ok(CallToolResult::error(vec![Content::text(format!(
                        "Failed to read file '{}': {}",
                        file, e
                    ))]));
                }
            }
        } else if let Some(ref content) = params.content {
            if content.len() > 1024 * 1024 {
                return Ok(CallToolResult::error(vec![Content::text(
                    "DSL content exceeds 1 MiB",
                )]));
            }
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
            let stem = dsl::strip_symbi_extension(filename).unwrap_or(filename);
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
        // Fall back to reading from disk (in case agents were added after startup).
        // Try `.symbi` first (canonical), then `.dsl` (legacy).
        for ext in [dsl::SYMBI_EXTENSION, dsl::LEGACY_DSL_EXTENSION] {
            let path = format!("agents/{}.{}", params.agent, ext);
            if let Ok(content) = self.read_project_file(&path, 1024 * 1024) {
                return Ok(CallToolResult::success(vec![Content::text(content)]));
            }
        }
        Ok(CallToolResult::error(vec![Content::text(format!(
            "Agent '{}' not found. Use list_agents to see available agents.",
            params.agent
        ))]))
    }

    #[tool(
        description = "Get the project's AGENTS.md file content. Returns the full AGENTS.md from the working directory, which describes available agents, their capabilities, schedules, channels, and invocation methods."
    )]
    async fn get_agents_md(&self) -> Result<CallToolResult, McpError> {
        match self.read_project_file("AGENTS.md", 64*1024) {
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
        if params.schema.len() > 1024 * 1024 || params.public_key_url.len() > 8192 {
            return Ok(CallToolResult::error(vec![Content::text(
                "Schema or key URL exceeds its byte limit",
            )]));
        }
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
        // ServerInfo (alias for InitializeResult) became #[non_exhaustive]
        // in rmcp 1.x; out-of-crate code can no longer use struct literal
        // syntax even with `..Default::default()`. Build via mutation.
        let mut info = ServerInfo::default();
        info.instructions = Some(
            "Symbiont AI Agent Runtime — invoke agents, parse DSL, \
             manage agent definitions, verify schemas via SchemaPin"
                .to_string(),
        );
        info.capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_resources()
            .build();
        info
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, McpError>> + Send + '_ {
        let resources = if self.read_project_file("AGENTS.md", 64 * 1024).is_ok() {
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
            match self.read_project_file("AGENTS.md", 64 * 1024) {
                Ok(content) => Ok(ReadResourceResult::new(vec![ResourceContents::text(
                    content,
                    "file:///AGENTS.md",
                )])),
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

/// Scan the agents/ directory for `.symbi` (or legacy `.dsl`) files and
/// return (filename, content) pairs.
fn scan_agent_dsl_files(project: &ProjectReader) -> Vec<(String, String)> {
    let mut sources = Vec::new();
    let mut bytes = 0;
    if let Ok(entries) = std::fs::read_dir(project.path().join("agents")) {
        for entry in entries.take(1024).flatten() {
            let path = std::path::Path::new("agents").join(entry.file_name());
            if dsl::is_symbi_file(&path) {
                if let Ok(content) = project.read_text(&path, 1024 * 1024) {
                    bytes += content.len();
                    if bytes > 16 * 1024 * 1024 {
                        break;
                    }
                    sources.push((entry.file_name().to_string_lossy().into_owned(), content));
                }
            }
        }
    }
    sources
}

/// Load bounded project guidance as caller input, never runtime authority.
///
/// Only returns content between `<!-- agents-md:auto-start -->` and
/// `<!-- agents-md:auto-end -->` markers — this is DSL-parser-derived content,
/// but it remains untrusted project text and cannot grant runtime permissions.
/// Truncates to 2000 chars to avoid blowing context windows.
fn load_agents_md_context(project: &ProjectReader) -> Option<String> {
    let content = project
        .read_text(std::path::Path::new("AGENTS.md"), 64 * 1024)
        .ok()?;
    let section = crate::commands::agents_md::extract_auto_section(&content)?;
    if section.is_empty() {
        return None;
    }
    let truncated = if section.len() > 2000 {
        format!("{}...", section.chars().take(2000).collect::<String>())
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
