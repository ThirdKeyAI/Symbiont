//! Test-only stdio MCP server exposing a single `echo` tool.
//!
//! Built as a `[[bin]]` target of `symbi-runtime`, gated behind the
//! `mcp-client` feature (`required-features = ["mcp-client"]` in
//! `Cargo.toml`). Used exclusively by the `mcp_stdio` integration test as a
//! real subprocess to exercise `RmcpStdioClient` end-to-end over stdio.
//!
//! Mirrors the server macro shapes used in `src/mcp_server/mod.rs`
//! (`#[tool_router]` / `#[tool]` / `#[tool_handler]` / `ServerHandler`).

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    tool, tool_handler, tool_router,
    transport::stdio,
    ServerHandler, ServiceExt,
};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
struct EchoArgs {
    /// Text to echo back
    text: String,
}

#[derive(Clone)]
struct Echo {
    // Used by `#[tool_handler]`-generated code via `self.tool_router.call(...)`.
    // The dead-code pass cannot see the macro-expanded reference.
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl Echo {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Echo the input text")]
    async fn echo(&self, Parameters(args): Parameters<EchoArgs>) -> String {
        args.text
    }
}

#[tool_handler]
impl ServerHandler for Echo {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let service = Echo::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
