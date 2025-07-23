//! Model Context Protocol (MCP) Integration
//!
//! Provides secure MCP client implementation with schema verification

pub mod client;
pub mod types;

// Re-export main types and traits
pub use types::{
    McpClientConfig, McpClientError, McpTool, ToolDiscoveryEvent, ToolProvider,
    ToolVerificationRequest, ToolVerificationResponse, VerificationStatus,
};

pub use client::{McpClient, MockMcpClient, SecureMcpClient};
