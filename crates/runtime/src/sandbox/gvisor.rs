//! gVisor sandbox runner
//!
//! Executes code inside a gVisor-isolated container. gVisor (`runsc`) is an
//! OCI-compatible runtime, so we drive it through the existing Docker
//! integration with `--runtime=runsc` injected. This requires `runsc` to be
//! installed on the host and registered with the Docker daemon
//! (`/etc/docker/daemon.json`):
//!
//! ```json
//! { "runtimes": { "runsc": { "path": "/usr/local/bin/runsc" } } }
//! ```
//!
//! The wrapper validates that the `runsc` binary exists at construction
//! time so misconfigurations surface before the first execution.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Stdio;

use super::docker::{DockerConfig, DockerRunner};
use super::{ExecutionResult, SandboxRunner};

/// Configuration for gVisor sandbox execution. Layers on top of
/// `DockerConfig` since gVisor runs as a Docker OCI runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GVisorConfig {
    /// Underlying Docker configuration (image, limits, network, etc).
    pub docker: DockerConfig,
    /// Name of the registered Docker runtime for gVisor (default: `runsc`).
    pub runtime_name: String,
    /// Path to the `runsc` binary used for the preflight check
    /// (default: `runsc` resolved via $PATH).
    pub runsc_binary: String,
}

impl Default for GVisorConfig {
    fn default() -> Self {
        Self {
            docker: DockerConfig::default(),
            runtime_name: "runsc".to_string(),
            runsc_binary: "runsc".to_string(),
        }
    }
}

impl GVisorConfig {
    /// Create a config for a specific image with sensible defaults.
    pub fn for_image(image: &str) -> Self {
        Self {
            docker: DockerConfig::for_image(image),
            ..Default::default()
        }
    }
}

/// gVisor sandbox runner — delegates to `DockerRunner` with `--runtime=runsc`
/// injected into the Docker invocation.
pub struct GVisorRunner {
    inner: DockerRunner,
}

impl GVisorRunner {
    /// Create a new gVisor runner. Validates that the `runsc` binary is
    /// reachable before constructing the underlying Docker runner.
    pub fn new(mut config: GVisorConfig) -> Result<Self, anyhow::Error> {
        // Preflight: confirm runsc is installed. We only check that the
        // binary is reachable; whether the Docker daemon has it registered
        // surfaces at first `docker run` either way.
        let runsc_check = std::process::Command::new(&config.runsc_binary)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        match runsc_check {
            Ok(status) if status.success() => {}
            _ => {
                anyhow::bail!(
                    "gVisor (`{}`) is not available. Install runsc \
                     (https://gvisor.dev/docs/user_guide/install/) and register it \
                     with the Docker daemon at /etc/docker/daemon.json.",
                    config.runsc_binary
                );
            }
        }

        // Inject --runtime=<runtime_name> into the Docker invocation.
        config
            .docker
            .extra_flags
            .insert(0, format!("--runtime={}", config.runtime_name));

        let inner = DockerRunner::new(config.docker)?;
        tracing::info!(
            "gVisor sandbox initialized (runtime={})",
            config.runtime_name
        );
        Ok(Self { inner })
    }
}

#[async_trait]
impl SandboxRunner for GVisorRunner {
    async fn execute(
        &self,
        code: &str,
        env: HashMap<String, String>,
    ) -> Result<ExecutionResult, anyhow::Error> {
        self.inner.execute(code, env).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_default_uses_runsc() {
        let cfg = GVisorConfig::default();
        assert_eq!(cfg.runtime_name, "runsc");
        assert_eq!(cfg.runsc_binary, "runsc");
    }

    #[test]
    fn config_for_image_carries_image() {
        let cfg = GVisorConfig::for_image("alpine:latest");
        assert_eq!(cfg.docker.image, "alpine:latest");
        assert_eq!(cfg.runtime_name, "runsc");
    }

    #[test]
    fn runtime_flag_is_prepended_to_extra_flags() {
        // We can't actually construct a GVisorRunner without runsc on the
        // host, but we can verify the injection logic by replicating it.
        let mut cfg = GVisorConfig::default();
        cfg.docker.extra_flags = vec!["--label".into(), "test=1".into()];
        cfg.docker
            .extra_flags
            .insert(0, format!("--runtime={}", cfg.runtime_name));
        assert_eq!(cfg.docker.extra_flags[0], "--runtime=runsc");
        assert_eq!(cfg.docker.extra_flags[1], "--label");
    }
}
