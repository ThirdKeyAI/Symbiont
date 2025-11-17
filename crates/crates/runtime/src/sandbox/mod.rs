//! Sandbox abstraction layer for multi-tier sandbox execution
//!
//! This module provides a unified interface for different sandbox technologies
//! including Docker, GVisor, Firecracker, and E2B.dev integration.

pub mod e2b;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use e2b::E2BSandbox;

/// Sandbox tier enumeration representing different isolation levels
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SandboxTier {
    /// Docker container sandbox
    Docker,
    /// gVisor sandbox for enhanced security
    GVisor,
    /// Firecracker microVM sandbox
    Firecracker,
    /// E2B.dev cloud sandbox
    E2B,
}

/// Result of sandbox code execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    /// Exit code of the execution
    pub exit_code: i32,
    /// Standard output
    pub stdout: String,
    /// Standard error output
    pub stderr: String,
    /// Execution duration in milliseconds
    pub execution_time_ms: u64,
    /// Whether execution was successful
    pub success: bool,
}

impl ExecutionResult {
    /// Create a successful execution result
    pub fn success(stdout: String, execution_time_ms: u64) -> Self {
        Self {
            exit_code: 0,
            stdout,
            stderr: String::new(),
            execution_time_ms,
            success: true,
        }
    }

    /// Create a failed execution result
    pub fn failure(exit_code: i32, stderr: String, execution_time_ms: u64) -> Self {
        Self {
            exit_code,
            stdout: String::new(),
            stderr,
            execution_time_ms,
            success: false,
        }
    }

    /// Create an error execution result
    pub fn error(error_message: String) -> Self {
        Self {
            exit_code: -1,
            stdout: String::new(),
            stderr: error_message,
            execution_time_ms: 0,
            success: false,
        }
    }
}

/// Trait for sandbox runners providing code execution capabilities
#[async_trait]
pub trait SandboxRunner: Send + Sync {
    /// Execute code in the sandbox with provided environment variables
    ///
    /// # Arguments
    /// * `code` - The code to execute in the sandbox
    /// * `env` - Environment variables to set in the sandbox
    ///
    /// # Returns
    /// Result containing execution output or error
    async fn execute(
        &self,
        code: &str,
        env: HashMap<String, String>,
    ) -> Result<ExecutionResult, anyhow::Error>;
}