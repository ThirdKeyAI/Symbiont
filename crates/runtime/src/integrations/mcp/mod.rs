//! Model Context Protocol (MCP) Integration
//!
//! Provides secure MCP client implementation with schema verification

pub mod client;
#[cfg(feature = "mcp-client")]
pub mod registry;
#[cfg(feature = "mcp-client")]
pub mod stdio_client;
pub mod types;

// Re-export main types and traits
pub use types::{
    McpClientConfig, McpClientError, McpTool, ToolDiscoveryEvent, ToolProvider,
    ToolVerificationRequest, ToolVerificationResponse, VerificationStatus,
};

pub use client::{McpClient, MockMcpClient, SecureMcpClient};
