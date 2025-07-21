//! Model Context Protocol (MCP) Integration
//! 
//! Provides secure MCP client implementation with schema verification

pub mod types;
pub mod client;

// Re-export main types and traits
pub use types::{
    McpTool, McpClientConfig, McpClientError, ToolProvider, VerificationStatus,
    ToolDiscoveryEvent, ToolVerificationRequest, ToolVerificationResponse,
};

pub use client::{
    McpClient, SecureMcpClient, MockMcpClient,
};