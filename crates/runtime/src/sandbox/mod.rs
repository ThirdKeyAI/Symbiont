//! Sandbox abstraction layer for multi-tier sandbox execution
//!
//! This module provides a unified interface for different sandbox technologies
//! including Docker, GVisor, Firecracker, E2B.dev, and native (non-isolated) execution.

pub mod docker;
pub mod e2b;
#[cfg(feature = "native-sandbox")]
pub mod native;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use docker::{DockerConfig, DockerRunner};
pub use e2b::E2BSandbox;
#[cfg(feature = "native-sandbox")]
pub use native::{NativeConfig, NativeRunner};

/// Sandbox tier enumeration representing different isolation levels
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SandboxTier {
    /// No isolation - direct host execution (⚠️ DEVELOPMENT ONLY)
    None,
    /// Docker container sandbox
    Docker,
    /// gVisor sandbox for enhanced security
    GVisor,
    /// Firecracker microVM sandbox
    Firecracker,
    /// E2B.dev cloud sandbox
    E2B,
}

impl SandboxTier {
    /// Fail-fast guard for production deployments.
    ///
    /// Call this at runtime startup before any agent is allowed to execute.
    /// When `SYMBIONT_ENV=production` and the tier is `None`, this returns
    /// `Err` so the operator must explicitly opt into an unisolated
    /// configuration via `SYMBIONT_ALLOW_UNISOLATED=1`. Without the guard,
    /// misconfigured deployments have previously left the unisolated tier
    /// running on real traffic.
    pub fn enforce_production_guard(&self) -> Result<(), String> {
        if !matches!(self, SandboxTier::None) {
            return Ok(());
        }
        let env = std::env::var("SYMBIONT_ENV").unwrap_or_default();
        if env.eq_ignore_ascii_case("production") {
            let allow = std::env::var("SYMBIONT_ALLOW_UNISOLATED")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            if !allow {
                return Err(
                    "SECURITY: SandboxTier::None is not permitted in production; \
                     set SYMBIONT_ALLOW_UNISOLATED=1 to override (not recommended)"
                        .to_string(),
                );
            }
            tracing::error!(
                "SandboxTier::None enabled in production via SYMBIONT_ALLOW_UNISOLATED=1 — \
                 agents run without host isolation"
            );
        }
        Ok(())
    }
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
    /// Whether stdout was truncated due to size limits
    #[serde(default)]
    pub stdout_truncated: bool,
    /// Whether stderr was truncated due to size limits
    #[serde(default)]
    pub stderr_truncated: bool,
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
            stdout_truncated: false,
            stderr_truncated: false,
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
            stdout_truncated: false,
            stderr_truncated: false,
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
            stdout_truncated: false,
            stderr_truncated: false,
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

#[cfg(test)]
mod tier_guard_tests {
    use super::*;

    fn scoped_env<F: FnOnce()>(vars: &[(&str, Option<&str>)], f: F) {
        // Save + clear, run f, then restore. Not truly thread-safe with
        // other env-mutating tests, but the guard helpers read the vars
        // once per call so serial execution within this module is enough.
        let saved: Vec<_> = vars
            .iter()
            .map(|(k, _)| (k.to_string(), std::env::var(k).ok()))
            .collect();
        for (k, v) in vars {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
        f();
        for (k, prior) in saved {
            match prior {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }

    #[test]
    #[serial_test::serial(sandbox_tier_env)]
    fn non_none_tier_passes_guard() {
        assert!(SandboxTier::Docker.enforce_production_guard().is_ok());
        assert!(SandboxTier::GVisor.enforce_production_guard().is_ok());
    }

    #[test]
    #[serial_test::serial(sandbox_tier_env)]
    fn none_tier_outside_production_passes() {
        scoped_env(&[("SYMBIONT_ENV", Some("development"))], || {
            assert!(SandboxTier::None.enforce_production_guard().is_ok());
        });
    }

    #[test]
    #[serial_test::serial(sandbox_tier_env)]
    fn none_tier_in_production_fails_without_override() {
        scoped_env(
            &[
                ("SYMBIONT_ENV", Some("production")),
                ("SYMBIONT_ALLOW_UNISOLATED", None),
            ],
            || {
                let res = SandboxTier::None.enforce_production_guard();
                assert!(res.is_err(), "production None must refuse, got {:?}", res);
            },
        );
    }

    #[test]
    #[serial_test::serial(sandbox_tier_env)]
    fn none_tier_in_production_with_override_passes() {
        scoped_env(
            &[
                ("SYMBIONT_ENV", Some("production")),
                ("SYMBIONT_ALLOW_UNISOLATED", Some("1")),
            ],
            || {
                assert!(SandboxTier::None.enforce_production_guard().is_ok());
            },
        );
    }
}
