//! Smoke test for Composio MCP integration
//!
//! Usage:
//!   COMPOSIO_API_KEY=<key> COMPOSIO_MCP_URL=<url> \
//!     cargo run --features composio -p symbi-runtime --example composio_smoke_test

#[cfg(feature = "composio")]
use symbi_runtime::integrations::composio::{
    config::{ComposioGlobalConfig, McpConfigFile, McpServerEntry},
    ComposioMcpSource,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "composio")]
    {
        let api_key = std::env::var("COMPOSIO_API_KEY").expect("COMPOSIO_API_KEY env var required");

        // Use the tool_router URL format — must be provided via env var
        let mcp_url = std::env::var("COMPOSIO_MCP_URL").expect("COMPOSIO_MCP_URL env var required");

        println!("Composio MCP Smoke Test");
        println!("=======================");
        println!("URL: {}", mcp_url);
        println!();

        let config = McpConfigFile {
            composio: Some(ComposioGlobalConfig {
                api_key,
                base_url: "https://backend.composio.dev".to_string(),
            }),
            mcp_servers: vec![McpServerEntry::Composio {
                name: "composio-test".to_string(),
                server_id: "from-url".to_string(),
                user_id: "default".to_string(),
                url: Some(mcp_url),
                policy: None,
            }],
        };

        let mut source = ComposioMcpSource::from_config(config)?;
        println!("Discovering tools from Composio...");

        match source.discover_all().await {
            Ok(tools) => {
                println!("Discovered {} tools:", tools.len());
                for tool in &tools {
                    println!(
                        "  - {} : {}",
                        tool.name,
                        &tool.description[..tool.description.len().min(80)]
                    );
                }
            }
            Err(e) => {
                eprintln!("Discovery failed: {}", e);
            }
        }
    }

    #[cfg(not(feature = "composio"))]
    {
        println!("composio feature not enabled. Run with --features composio");
    }

    Ok(())
}
