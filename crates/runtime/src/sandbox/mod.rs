//! Sandbox abstraction layer for multi-tier sandbox execution
//!
//! This module provides a unified interface for different sandbox technologies
//! including Docker, GVisor, Firecracker, E2B.dev, and native (non-isolated) execution.

pub mod docker;
pub mod e2b;
pub mod firecracker;
pub mod gvisor;
#[cfg(feature = "native-sandbox")]
pub mod native;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use docker::{DockerConfig, DockerRunner};
pub use e2b::E2BSandbox;
pub use firecracker::{FirecrackerConfig, FirecrackerRunner};
pub use gvisor::{GVisorConfig, GVisorRunner};
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
        // All tiers (Docker, GVisor, Firecracker, E2B) ship with OSS runner
        // implementations. The only tier the guard refuses unconditionally
        // in production is `None` — direct host execution — unless the
        // operator has explicitly opted in via `SYMBIONT_ALLOW_UNISOLATED=1`.
        if !matches!(self, SandboxTier::None) {
            return Ok(());
        }
        let is_prod = crate::env::is_production().map_err(|e| e.to_string())?;
        if is_prod {
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

/// Build a `SandboxRunner` for the given tier using the supplied profile.
///
/// `profile` carries the per-tier configuration knobs the operator set in
/// `symbiont.toml` (`[sandbox.docker]`, `[sandbox.gvisor]`,
/// `[sandbox.firecracker]`). Defaults are applied for any tier the operator
/// hasn't customised.
///
/// Returns `Err` if the tier is `SandboxTier::None` — host-only execution
/// must come from a deliberate code path, not a runner factory call.
pub fn build_runner(
    tier: SandboxTier,
    profile: &SandboxRunnerProfile,
) -> Result<Box<dyn SandboxRunner>, anyhow::Error> {
    match tier {
        SandboxTier::None => Err(anyhow::anyhow!(
            "SandboxTier::None has no runner; agents must use docker, gvisor, firecracker, or e2b"
        )),
        SandboxTier::Docker => {
            let cfg = profile.docker.clone().unwrap_or_default();
            Ok(Box::new(DockerRunner::new(cfg)?))
        }
        SandboxTier::GVisor => {
            let cfg = profile.gvisor.clone().unwrap_or_default();
            Ok(Box::new(GVisorRunner::new(cfg)?))
        }
        SandboxTier::Firecracker => {
            let cfg = profile.firecracker.clone().ok_or_else(|| {
                anyhow::anyhow!(
                    "Firecracker tier selected but [sandbox.firecracker] is not configured \
                     in symbiont.toml. Set kernel_image_path and rootfs_path."
                )
            })?;
            Ok(Box::new(FirecrackerRunner::new(cfg)?))
        }
        SandboxTier::E2B => {
            let api_key = std::env::var("E2B_API_KEY")
                .map_err(|_| anyhow::anyhow!("E2B tier selected but E2B_API_KEY is not set"))?;
            Ok(Box::new(E2BSandbox::new_with_default_endpoint(api_key)))
        }
    }
}

/// Per-tier configuration container surfaced to `build_runner`.
///
/// Each field is optional: if the operator hasn't supplied a value for a
/// given tier, `build_runner` falls back to that tier's `Default::default()`
/// (with the exception of Firecracker, which has no sensible defaults and
/// therefore returns an error if the field is `None`).
#[derive(Debug, Clone, Default)]
pub struct SandboxRunnerProfile {
    pub docker: Option<DockerConfig>,
    pub gvisor: Option<GVisorConfig>,
    pub firecracker: Option<FirecrackerConfig>,
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
        assert!(SandboxTier::Firecracker.enforce_production_guard().is_ok());
        assert!(SandboxTier::E2B.enforce_production_guard().is_ok());
    }

    #[test]
    fn build_runner_rejects_none_tier() {
        let profile = SandboxRunnerProfile::default();
        match build_runner(SandboxTier::None, &profile) {
            Ok(_) => panic!("None tier must not yield a runner"),
            Err(e) => assert!(e.to_string().contains("has no runner"), "got: {}", e),
        }
    }

    #[test]
    fn build_runner_rejects_firecracker_without_config() {
        let profile = SandboxRunnerProfile::default();
        match build_runner(SandboxTier::Firecracker, &profile) {
            Ok(_) => panic!("missing firecracker config must error"),
            Err(e) => assert!(
                e.to_string().contains("[sandbox.firecracker]"),
                "error should point at config: {}",
                e
            ),
        }
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
