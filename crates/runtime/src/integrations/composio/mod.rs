//! Composio MCP Integration
//!
//! Provides a thin proxy layer for discovering and using Composio-hosted MCP tools
//! (GitHub, Slack, Jira, etc.) through the existing `SecureMcpClient` enforcement
//! pipeline. OAuth for third-party services is handled entirely by Composio —
//! Symbiont only needs an API key and server IDs.
//!
//! # Configuration
//!
//! Tools are configured via `~/.symbiont/mcp-config.toml`:
//!
//! ```toml
//! [composio]
//! api_key = "env:COMPOSIO_API_KEY"
//!
//! [[mcp_servers]]
//! type = "composio"
//! name = "github"
//! server_id = "srv_github_123"
//! user_id = "user_456"
//! ```

pub mod config;
pub mod error;
pub mod source;
pub mod transport;

pub use config::{
    load_mcp_config, ComposioGlobalConfig, McpConfigFile, McpServerEntry, ServerPolicy,
};
pub use error::ComposioError;
pub use source::ComposioMcpSource;
pub use transport::{SseEvent, SseTransport};
