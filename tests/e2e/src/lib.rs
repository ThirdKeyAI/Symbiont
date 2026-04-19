//! Shared harness for Symbiont end-to-end tests.
//!
//! The main entry point is [`spawn_runtime_server`], which:
//! - builds a default `RuntimeConfig` (plus a caller-supplied mutator),
//! - constructs an `AgentRuntime`,
//! - binds an `HttpApiServer` to `127.0.0.1:0`,
//! - spawns the serve loop on a tokio task,
//! - returns the live base URL and a shutdown handle.
//!
//! The harness is only useful when the crate's `e2e` feature is enabled.
//! Default `cargo test --workspace` skips the E2E suite entirely so the
//! main CI stays fast; E2E is opt-in via
//! `cargo test -p symbi-e2e --features e2e`.

#![cfg(feature = "e2e")]

use std::path::PathBuf;
use std::sync::Arc;

use symbi_runtime::api::server::{HttpApiConfig, HttpApiServer};
use symbi_runtime::{AgentRuntime, RuntimeConfig};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// Handle returned by the test harness.
///
/// Dropping the handle (or explicitly calling [`TestServer::shutdown`])
/// aborts the serve task; the tokio runtime will clean up the port.
pub struct TestServer {
    pub base_url: String,
    pub runtime: Arc<AgentRuntime>,
    /// Kept so the temp dir outlives the server; referenced by the
    /// RuntimeConfig (api keys file, etc.).
    pub _tempdir: TempDir,
    handle: Option<JoinHandle<()>>,
}

impl TestServer {
    pub async fn shutdown(mut self) {
        if let Some(h) = self.handle.take() {
            h.abort();
            let _ = h.await;
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(h) = self.handle.take() {
            h.abort();
        }
    }
}

/// Options handed to [`spawn_runtime_server`].
pub struct SpawnOptions {
    /// Additional mutation applied to the default `RuntimeConfig`.
    pub configure: Box<dyn FnOnce(&mut RuntimeConfig) + Send>,
    /// Optional API keys file path (JSON). When None, the server falls
    /// back to `SYMBIONT_API_TOKEN`.
    pub api_keys_file: Option<PathBuf>,
    /// Temp dir whose lifetime is bound to the server. Pass in the one
    /// you created so it persists — the harness won't outlive it.
    pub tempdir: TempDir,
    /// Whether to enable per-IP rate limiting (default off — isolates
    /// tests that aren't specifically about the rate limiter).
    pub enable_rate_limiting: bool,
}

impl SpawnOptions {
    pub fn new(tempdir: TempDir) -> Self {
        Self {
            configure: Box::new(|_| {}),
            api_keys_file: None,
            tempdir,
            enable_rate_limiting: false,
        }
    }
}

/// Find a free TCP port on loopback. There is a theoretical race between
/// closing the listener and the server's own `bind()`, but on localhost
/// it's a non-issue in practice.
pub async fn pick_free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);
    port
}

/// Spawn a full AgentRuntime + HttpApiServer on a random loopback port.
pub async fn spawn_runtime_server(opts: SpawnOptions) -> TestServer {
    let mut config = RuntimeConfig::default();
    (opts.configure)(&mut config);

    let runtime = Arc::new(
        AgentRuntime::new(config)
            .await
            .expect("AgentRuntime construction"),
    );

    let port = pick_free_port().await;
    let http_config = HttpApiConfig {
        bind_address: "127.0.0.1".to_string(),
        port,
        enable_cors: false,
        enable_tracing: false,
        enable_rate_limiting: opts.enable_rate_limiting,
        api_keys_file: opts.api_keys_file,
        serve_agents_md: false,
    };

    let mut server = HttpApiServer::new(http_config).with_runtime_provider(runtime.clone());
    let handle = tokio::spawn(async move {
        // `start` takes `&mut self` and loops forever; abort on drop is fine.
        let _ = server.start().await;
    });

    // Busy-wait until the port is open. Bound to a couple of seconds so a
    // real bind failure surfaces as a test failure rather than a hang.
    let base_url = format!("http://127.0.0.1:{}", port);
    wait_for_port(port).await;

    TestServer {
        base_url,
        runtime,
        _tempdir: opts.tempdir,
        handle: Some(handle),
    }
}

async fn wait_for_port(port: u16) {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return;
        }
        if Instant::now() > deadline {
            panic!("HTTP server on port {} never became reachable", port);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Convenience reqwest client with short timeouts — we are always hitting
/// loopback in these tests, so a slow response means the runtime hung.
pub fn test_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("reqwest client")
}
