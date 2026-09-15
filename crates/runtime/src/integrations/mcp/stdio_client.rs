//! MCP stdio sessions through the selected isolation boundary.
//! Each invocation owns one worker across discovery, SchemaPin verification and
//! tool execution. Docker is the SDK default; host execution is explicit and
//! limited to development. Worker ownership includes bounded container cleanup.

use crate::integrations::mcp::registry::StdioServerSpec;
use crate::integrations::mcp::types::{McpTool, ToolProvider, VerificationStatus};
use crate::integrations::schemapin::key_store::LocalKeyStore;
use crate::integrations::schemapin::native_client::NativeSchemaPinClient;
use crate::integrations::schemapin::types::PinnedKey;
use crate::sandbox::files::FileAccessPlan;
use rmcp::{model::CallToolRequestParams, ServiceExt};
use sha2::Digest;
use std::time::Duration;
use tokio::io::{AsyncRead, ReadBuf};

pub struct RmcpStdioClient;

/// A completed MCP call and any files published after confirmed worker cleanup.
pub struct McpInvocationResult {
    pub content: serde_json::Value,
    pub created_files: serde_json::Value,
}

const MAX_STREAM_BYTES: usize = 10 * 1024 * 1024;

/// Cap bytes before the JSON codec allocates an entire untrusted frame.
struct LimitedReader<R> {
    inner: R,
    remaining: usize,
}

impl<R: AsyncRead + Unpin> AsyncRead for LimitedReader<R> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if output.remaining() == 0 {
            return std::task::Poll::Ready(Ok(()));
        }
        let mut bytes = [0u8; 8192];
        let size = bytes
            .len()
            .min(output.remaining())
            .min(self.remaining.max(1));
        let mut buffer = ReadBuf::new(&mut bytes[..size]);
        match std::pin::Pin::new(&mut self.inner).poll_read(cx, &mut buffer) {
            std::task::Poll::Ready(Ok(())) => {
                let bytes = buffer.filled();
                if bytes.len() > self.remaining {
                    return std::task::Poll::Ready(Err(std::io::Error::other(
                        "MCP output limit exceeded",
                    )));
                }
                self.remaining -= bytes.len();
                output.put_slice(bytes);
                std::task::Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

type WorkerTransport = (
    LimitedReader<crate::sandbox::streams::Reader>,
    crate::sandbox::streams::Writer,
);

async fn run_stdio<T, F, Fut>(
    spec: &StdioServerSpec,
    boundary: &crate::sandbox::command::CommandBoundary,
    deadline: Duration,
    operation: F,
) -> Result<T, String>
where
    F: FnOnce(WorkerTransport) -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    if deadline.is_zero() {
        return Err("MCP deadline exhausted before spawn".into());
    }
    boundary.validate()?;
    let started = std::time::Instant::now();
    let worker = if boundary.tier == crate::sandbox::command::CommandTier::DevelopmentHost {
        let mut command = tokio::process::Command::new(&spec.command);
        command.args(&spec.args).env_clear().kill_on_drop(true);
        for key in [
            "PATH",
            "LANG",
            "LC_ALL",
            "LC_CTYPE",
            "TZ",
            "SystemRoot",
            "WINDIR",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command.envs(&spec.env);
        command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        #[cfg(unix)]
        command.process_group(0);
        let child = command
            .spawn()
            .map_err(|e| format!("Failed to spawn MCP server '{}': {e}", spec.command))?;
        crate::sandbox::streams::StdioStreams::development(child, MAX_STREAM_BYTES)
    } else {
        let mut argv = vec![spec.command.clone()];
        argv.extend(spec.args.clone());
        boundary
            .spawn_streams(&argv, spec.env.clone(), deadline)
            .await?
    };
    let mut guard = worker.guard;
    let output_limit = guard.output_limit;
    let stdout = LimitedReader {
        inner: worker.stdout,
        remaining: output_limit,
    };
    let stdin = worker.stdin;
    let mut stderr = LimitedReader {
        inner: worker.stderr,
        remaining: output_limit,
    };
    let stderr_watch = async {
        tokio::io::copy(&mut stderr, &mut tokio::io::sink())
            .await
            .map_err(|e| format!("MCP stderr read failed: {e}"))?;
        // Closing stderr is valid and does not complete the protocol operation.
        std::future::pending::<Result<T, String>>().await
    };
    let result = tokio::select! {
        result = tokio::time::timeout(deadline.saturating_sub(started.elapsed()), operation((stdout, stdin))) => result.unwrap_or_else(|_| Err(format!("MCP operation timed out after {deadline:?}"))),
        result = stderr_watch => result,
    };
    guard.finish().await?;
    result
}

impl RmcpStdioClient {
    /// Connect to the stdio MCP server described by `spec`, list its tools,
    /// then disconnect.
    pub async fn list_tools(spec: &StdioServerSpec) -> Result<Vec<rmcp::model::Tool>, String> {
        Self::list_tools_with_boundary(spec, &crate::sandbox::command::CommandBoundary::default())
            .await
    }

    pub async fn list_tools_with_boundary(
        spec: &StdioServerSpec,
        boundary: &crate::sandbox::command::CommandBoundary,
    ) -> Result<Vec<rmcp::model::Tool>, String> {
        boundary.validate()?;
        let boundary = boundary.without_host_mounts();
        run_stdio(
            spec,
            &boundary,
            Duration::from_secs(30),
            |transport| async move {
                let client = ().serve(transport).await.map_err(|e| e.to_string())?;
                let tools = client.list_all_tools().await.map_err(|e| e.to_string());
                let _ = client.cancel().await;
                tools
            },
        )
        .await
    }

    /// Connect to the stdio MCP server described by `spec`, invoke `tool`
    /// with `args`, then disconnect. Returns the tool's content normalized to
    /// JSON. Requires SchemaPin verification by default. A `CallToolResult` with `is_error == true` is
    /// surfaced as `Err`, not `Ok`.
    pub async fn call_tool(
        spec: &StdioServerSpec,
        tool: &str,
        args: serde_json::Map<String, serde_json::Value>,
    ) -> Result<serde_json::Value, String> {
        Self::verified_invoke(spec, tool, args, true).await
    }

    /// Discover `tool` on the stdio MCP server described by `spec`, gate its
    /// invocation behind SchemaPin/TOFU verification, then invoke it.
    ///
    /// Fail-closed: when `enforce` is `true`, the tool's schema must carry a
    /// verifiable SchemaPin signature (checked via [`NativeSchemaPinClient`] +
    /// [`LocalKeyStore`] TOFU pinning, mirroring `SecureMcpClient::verify_schema`
    /// in `integrations/mcp/client.rs`) or the call is blocked before it ever
    /// reaches the server — no side effects on the target tool occur. When
    /// `enforce` is `false` (local dev opt-out), the tool runs unconditionally.
    pub async fn verified_invoke(
        spec: &StdioServerSpec,
        tool: &str,
        args: serde_json::Map<String, serde_json::Value>,
        enforce: bool,
    ) -> Result<serde_json::Value, String> {
        Self::verified_invoke_with_timeout(spec, tool, args, enforce, Duration::from_secs(30)).await
    }

    pub async fn verified_invoke_with_timeout(
        spec: &StdioServerSpec,
        tool: &str,
        args: serde_json::Map<String, serde_json::Value>,
        enforce: bool,
        timeout: Duration,
    ) -> Result<serde_json::Value, String> {
        Self::verified_invoke_with_boundary(
            spec,
            tool,
            args,
            enforce,
            timeout,
            &crate::sandbox::command::CommandBoundary::default(),
        )
        .await
    }

    /// Invoke without host file grants. Configured mounts are only ceilings;
    /// use `verified_invoke_with_files` for explicit per-operation file access.
    pub async fn verified_invoke_with_boundary(
        spec: &StdioServerSpec,
        tool: &str,
        args: serde_json::Map<String, serde_json::Value>,
        enforce: bool,
        timeout: Duration,
        boundary: &crate::sandbox::command::CommandBoundary,
    ) -> Result<serde_json::Value, String> {
        Self::verified_invoke_with_files(
            spec,
            tool,
            args,
            enforce,
            timeout,
            boundary,
            &FileAccessPlan::default(),
        )
        .await
        .map(|result| result.content)
    }

    /// Hold one worker and immutable input snapshots across discovery,
    /// verification and invocation. Publish new output only after success and
    /// confirmed cleanup; errors and cancellation never publish staged files.
    /// Output plans require live authority from governed dispatch. Standalone
    /// SDK callers can provide declared inputs, but cannot publish outputs.
    pub async fn verified_invoke_with_files(
        spec: &StdioServerSpec,
        tool: &str,
        args: serde_json::Map<String, serde_json::Value>,
        enforce: bool,
        timeout: Duration,
        boundary: &crate::sandbox::command::CommandBoundary,
        files: &FileAccessPlan,
    ) -> Result<McpInvocationResult, String> {
        if !enforce && crate::env::is_production().map_err(|e| e.to_string())? {
            return Err("MCP SchemaPin verification cannot be disabled in production".into());
        }
        boundary.validate()?;
        files.check_publication_authority()?;
        let started = std::time::Instant::now();
        let staged = files.stage(boundary)?;
        let content = Self::invoke_checked(
            spec,
            tool,
            args,
            timeout.saturating_sub(started.elapsed()),
            &staged.boundary,
            |rmcp_tool| async move {
                if !enforce {
                    return Ok(());
                }
                let mcp_tool = McpTool {
                    name: rmcp_tool.name.to_string(),
                    description: rmcp_tool
                        .description
                        .map(|c| c.to_string())
                        .unwrap_or_default(),
                    schema: serde_json::to_value(&*rmcp_tool.input_schema)
                        .map_err(|e| e.to_string())?,
                    provider: ToolProvider {
                        identifier: spec.command.clone(),
                        name: spec.command.clone(),
                        public_key_url: spec.public_key_url.clone().unwrap_or_default(),
                        version: None,
                    },
                    verification_status: VerificationStatus::Pending,
                    metadata: None,
                    sensitive_params: Vec::new(),
                };
                if mcp_tool
                    .schema
                    .get("signature")
                    .and_then(|value| value.as_str())
                    .is_none()
                {
                    return Err(format!(
                        "tool '{tool}' is not SchemaPin-verified (fail-closed)"
                    ));
                }
                let key_store = LocalKeyStore::new().map_err(|e| e.to_string())?;
                let verified = verify_via_schemapin(
                    &NativeSchemaPinClient::new(),
                    &key_store,
                    &mcp_tool,
                    spec.public_key_pem.as_deref(),
                )
                .await?;
                if !verified {
                    return Err(format!(
                        "tool '{tool}' is not SchemaPin-verified (fail-closed)"
                    ));
                }
                Ok(())
            },
        )
        .await?;
        if started.elapsed() >= timeout {
            return Err("MCP deadline expired before file publication".into());
        }
        Ok(McpInvocationResult {
            content,
            created_files: staged.publish_recorded().await?,
        })
    }

    /// Hold one connection across discovery, verification, and invocation.
    /// The verifier is private so production callers cannot replace it.
    async fn invoke_checked<F, Fut>(
        spec: &StdioServerSpec,
        tool: &str,
        args: serde_json::Map<String, serde_json::Value>,
        timeout: Duration,
        boundary: &crate::sandbox::command::CommandBoundary,
        verify: F,
    ) -> Result<serde_json::Value, String>
    where
        F: FnOnce(rmcp::model::Tool) -> Fut,
        Fut: std::future::Future<Output = Result<(), String>>,
    {
        let deadline = std::time::Instant::now()
            .checked_add(timeout)
            .ok_or("MCP deadline exceeds supported range")?;
        let encoded = serde_json::to_vec(&args).map_err(|e| e.to_string())?;
        if encoded.len() > MAX_STREAM_BYTES {
            return Err("MCP arguments exceed the input limit".into());
        }
        run_stdio(spec, boundary, timeout, |transport| async move {
            let connection = ().serve(transport).await.map_err(|e| e.to_string())?;
            let outcome = async {
                let discovered = connection
                    .list_all_tools()
                    .await
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .find(|candidate| candidate.name.as_ref() == tool)
                    .ok_or_else(|| format!("tool '{tool}' not found on server"))?;
                verify(discovered).await?;
                if std::time::Instant::now() >= deadline {
                    return Err("MCP authorization deadline expired before invocation".into());
                }
                let params = CallToolRequestParams::new(tool.to_string()).with_arguments(args);
                let result = connection
                    .call_tool(params)
                    .await
                    .map_err(|e| e.to_string())?;
                if result.is_error.unwrap_or(false) {
                    return Err(format!(
                        "tool '{tool}' reported error: {}",
                        serde_json::to_string(&result.content).unwrap_or_default()
                    ));
                }
                serde_json::to_value(&result.content).map_err(|e| e.to_string())
            }
            .await;
            let _ = connection.cancel().await;
            outcome
        })
        .await
    }
}

/// Verify `tool`'s schema via SchemaPin, TOFU-pinning the provider's public
/// key on first use. Signature verification uses the same fetched PEM that
/// passed the pin check; no second key fetch or temporary schema file is used.
///
/// Returns `Ok(false)` — not an error — when the schema carries no embedded
/// SchemaPin signature (SchemaPin convention: a top-level `signature` field in
/// the schema JSON, exactly what `NativeSchemaPinClient::verify_schema`
/// looks for) or has neither a provisioned key nor a public-key URL.
/// The caller (`verified_invoke`)
/// blocks under enforcement in that case, since an unsigned/unverifiable tool
/// must never be treated as verified. Genuine verification errors (key-fetch
/// failure, I/O, malformed schema, etc.) propagate as `Err`, which also blocks
/// the call under enforcement — fail closed either way.
async fn verify_via_schemapin(
    client: &NativeSchemaPinClient,
    key_store: &LocalKeyStore,
    tool: &McpTool,
    configured_key: Option<&str>,
) -> Result<bool, String> {
    // SchemaPin convention: a signed schema embeds a top-level `signature`
    // field. No signature, or no public key to check it against, means there
    // is nothing to verify — unverified, not an error.
    let has_signature = tool
        .schema
        .get("signature")
        .and_then(|s| s.as_str())
        .is_some();
    if !has_signature || (configured_key.is_none() && tool.provider.public_key_url.is_empty()) {
        return Ok(false);
    }

    // An explicitly provisioned trust anchor supports offline verification.
    // Otherwise fetch over guarded HTTPS. Both modes verify the signature and
    // enforce the same persistent provider pin before the call can be sent.
    let key_data = match configured_key {
        Some(key) => key.trim().to_owned(),
        None => {
            let key_url =
                url::Url::parse(&tool.provider.public_key_url).map_err(|e| e.to_string())?;
            if key_url.scheme() != "https" {
                return Err("MCP provider public key must use HTTPS".into());
            }
            client
                .fetch_public_key(&tool.provider.public_key_url)
                .await
                .map_err(|e| e.to_string())?
        }
    };
    let schema = serde_json::to_vec(&tool.schema).map_err(|e| e.to_string())?;
    let result = client
        .verify_schema_with_key(
            &schema,
            &tool.name,
            &tool.provider.public_key_url,
            &key_data,
        )
        .map_err(|e| e.to_string())?;
    if !result.success {
        return Ok(false);
    }
    let mut hasher = sha2::Sha256::new();
    hasher.update(key_data.as_bytes());
    let fingerprint = hex::encode(hasher.finalize());
    // `pin_key` is TOFU: Ok on first pin or a matching re-affirm, `KeyMismatch`
    // on a swap — which propagates as Err and blocks the call fail-closed.
    key_store
        .pin_key(PinnedKey::new(
            tool.provider.identifier.clone(),
            key_data.clone(),
            "ES256".to_string(),
            fingerprint,
        ))
        .map_err(|e| e.to_string())?;

    Ok(true)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::Write;
    use tokio::io::AsyncReadExt;

    const SERVER: &str = r#"
import json, os, sys
session = str(os.getpid())
def event(kind):
    with open(sys.argv[1], 'a') as output:
        output.write(kind + ':' + session + '\n')
event('start')
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request:
        continue
    method = request['method']
    if method == 'initialize':
        result = {'protocolVersion': request['params']['protocolVersion'], 'capabilities': {'tools': {}}, 'serverInfo': {'name': 'fixture', 'version': '1'}}
    elif method == 'tools/list':
        event('list')
        result = {'tools': [{'name': 'echo', 'inputSchema': {'type': 'object', 'properties': {'session': {'const': session}}}}]}
    elif method == 'tools/call':
        event('call')
        result = {'content': [{'type': 'text', 'text': json.dumps({'session': session, 'environment': os.getenv('SYMBIONT_MCP_ENV_FIXTURE')})}]}
    else:
        result = {}
    print(json.dumps({'jsonrpc': '2.0', 'id': request['id'], 'result': result}), flush=True)
"#;

    fn fixture(path: &std::path::Path) -> StdioServerSpec {
        StdioServerSpec {
            command: "python3".into(),
            args: vec![
                "-u".into(),
                "-c".into(),
                SERVER.into(),
                path.to_str().unwrap().into(),
            ],
            env: Default::default(),
            public_key_url: None,
            public_key_pem: None,
        }
    }

    #[tokio::test]
    async fn production_rejects_verification_and_host_opt_outs() {
        const CHILD: &str = "SYMBI_TEST_MCP_PRODUCTION";
        if std::env::var_os(CHILD).is_none() {
            let home = tempfile::tempdir().unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "integrations::mcp::stdio_client::tests::production_rejects_verification_and_host_opt_outs"])
                .env_clear().env("PATH", "/usr/bin:/bin").env("HOME", home.path())
                .env("SYMBIONT_ENV", "production").env(CHILD, "1")
                .output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
            return;
        }
        let spec = StdioServerSpec {
            command: "/missing/forbidden-worker".into(),
            ..Default::default()
        };
        let error = RmcpStdioClient::verified_invoke_with_boundary(
            &spec,
            "echo",
            Default::default(),
            false,
            Duration::from_secs(1),
            &crate::sandbox::command::CommandBoundary::default(),
        )
        .await
        .unwrap_err();
        assert!(
            error.contains("cannot be disabled in production"),
            "{error}"
        );
        let error = RmcpStdioClient::verified_invoke_with_boundary(
            &spec,
            "echo",
            Default::default(),
            true,
            Duration::from_secs(1),
            &crate::sandbox::command::CommandBoundary::development_host(),
        )
        .await
        .unwrap_err();
        assert!(error.contains("forbidden in production"), "{error}");
    }

    #[tokio::test]
    async fn provisioned_keys_verify_exact_schema_and_preserve_provider_pin() {
        use crate::integrations::schemapin::types::KeyStoreConfig;
        use schemapin::crypto::{generate_key_pair, sign_data};
        let directory = tempfile::tempdir().unwrap();
        let store = LocalKeyStore::with_config(KeyStoreConfig {
            store_path: directory.path().join("pins.json"),
            ..Default::default()
        })
        .unwrap();
        let key = generate_key_pair().unwrap();
        let other = generate_key_pair().unwrap();
        let signed = |private: &str| {
            let mut schema =
                serde_json::json!({"type":"object", "properties":{"text":{"type":"string"}}});
            schema["signature"] = serde_json::json!(sign_data(
                private,
                schemapin::canonicalize::canonicalize_schema(&schema).as_bytes()
            )
            .unwrap());
            schema
        };
        let mut tool = McpTool {
            name: "fixture".into(),
            description: String::new(),
            schema: signed(&key.private_key_pem),
            provider: ToolProvider {
                identifier: "provisioned-fixture".into(),
                name: "fixture".into(),
                public_key_url: String::new(),
                version: None,
            },
            verification_status: VerificationStatus::Pending,
            metadata: None,
            sensitive_params: Vec::new(),
        };
        let client = NativeSchemaPinClient::new();
        assert!(
            verify_via_schemapin(&client, &store, &tool, Some(&key.public_key_pem))
                .await
                .unwrap()
        );
        let pinned = std::fs::read(store.store_path()).unwrap();
        tool.schema["properties"]["text"]["type"] = serde_json::json!("integer");
        assert!(
            !verify_via_schemapin(&client, &store, &tool, Some(&key.public_key_pem))
                .await
                .unwrap()
        );
        tool.schema = signed(&other.private_key_pem);
        assert!(
            verify_via_schemapin(&client, &store, &tool, Some(&other.public_key_pem))
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(store.store_path()).unwrap(), pinned);
        assert!(!verify_via_schemapin(&client, &store, &tool, None)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn stream_limit_checks_bytes_before_json_decoding() {
        let mut reader = LimitedReader {
            inner: &b"12345"[..],
            remaining: 4,
        };
        let mut captured = Vec::new();
        assert!(reader
            .read_to_end(&mut captured)
            .await
            .unwrap_err()
            .to_string()
            .contains("output limit"));
        assert_eq!(captured, b"1234");
        let mut reader = LimitedReader {
            inner: &b"1234"[..],
            remaining: 4,
        };
        captured.clear();
        reader.read_to_end(&mut captured).await.unwrap();
        assert_eq!(captured, b"1234");
    }

    #[tokio::test]
    async fn stdout_and_stderr_floods_fail_within_the_operation_deadline() {
        let events = tempfile::NamedTempFile::new().unwrap();
        for stream in ["stdout", "stderr"] {
            let mut spec = fixture(events.path());
            spec.args[2] = format!("import sys, time; sys.{stream}.write('x' * {}); sys.{stream}.flush(); time.sleep(5)", MAX_STREAM_BYTES + 1);
            let error = tokio::time::timeout(
                Duration::from_secs(4),
                RmcpStdioClient::verified_invoke_with_boundary(
                    &spec,
                    "echo",
                    Default::default(),
                    false,
                    Duration::from_secs(3),
                    &crate::sandbox::command::CommandBoundary::development_host(),
                ),
            )
            .await
            .expect("output budget must finish before outer timeout")
            .unwrap_err();
            assert!(!error.contains("timed out"), "{stream}: {error}");
            if stream == "stderr" {
                assert!(error.contains("output limit"), "{error}");
            }
        }
    }

    #[tokio::test]
    async fn stalled_handshake_and_zero_budget_fail_closed() {
        let events = tempfile::NamedTempFile::new().unwrap();
        let mut spec = fixture(events.path());
        spec.args[2] = "import time; time.sleep(5)".into();
        let error = RmcpStdioClient::verified_invoke_with_boundary(
            &spec,
            "echo",
            Default::default(),
            false,
            Duration::from_millis(50),
            &crate::sandbox::command::CommandBoundary::development_host(),
        )
        .await
        .unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        spec.command = "/missing/mcp-fixture".into();
        let error = RmcpStdioClient::verified_invoke_with_boundary(
            &spec,
            "echo",
            Default::default(),
            false,
            Duration::ZERO,
            &crate::sandbox::command::CommandBoundary::development_host(),
        )
        .await
        .unwrap_err();
        assert!(error.contains("before spawn"), "{error}");
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn cancelled_call_kills_worker_and_ordinary_descendants() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("pids.json");
        let mut spec = fixture(&marker);
        spec.args[2] = r#"
import json, os, sys, time
child = os.fork()
if child:
    with open(sys.argv[1], 'w') as output:
        json.dump([os.getpid(), child], output)
time.sleep(5)
"#
        .into();
        let job = tokio::spawn(async move {
            RmcpStdioClient::verified_invoke_with_boundary(
                &spec,
                "echo",
                Default::default(),
                false,
                Duration::from_secs(30),
                &crate::sandbox::command::CommandBoundary::development_host(),
            )
            .await
        });
        let ready = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(bytes) = tokio::fs::read(&marker).await {
                    if let Ok(pids) = serde_json::from_slice::<Vec<u32>>(&bytes) {
                        break pids;
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        job.abort();
        assert!(job.await.unwrap_err().is_cancelled());
        let pids = ready.expect("fixture must reach started state");
        let alive = |pid| {
            std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .ok()
                .is_some_and(|stat| {
                    stat.rsplit_once(") ").is_some_and(|(_, fields)| {
                        !fields.starts_with('Z') && !fields.starts_with('X')
                    })
                })
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            while pids.iter().any(|pid| alive(*pid)) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("cancellation must stop the worker group before its own sleep expires");
    }

    #[tokio::test]
    async fn discovery_verification_and_effect_share_one_session() {
        let events = tempfile::NamedTempFile::new().unwrap();
        let spec = fixture(events.path());
        let log_path = events.path().to_path_buf();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            RmcpStdioClient::invoke_checked(
                &spec,
                "echo",
                Default::default(),
                Duration::from_secs(3),
                &crate::sandbox::command::CommandBoundary::development_host(),
                |tool| async move {
                    let session = tool.input_schema["properties"]["session"]["const"]
                        .as_str()
                        .unwrap();
                    writeln!(
                        std::fs::OpenOptions::new()
                            .append(true)
                            .open(log_path)
                            .unwrap(),
                        "verify:{session}"
                    )
                    .unwrap();
                    Ok(())
                },
            ),
        )
        .await
        .unwrap()
        .unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(result[0]["text"].as_str().unwrap()).unwrap();
        let session = payload["session"].as_str().unwrap();
        assert_eq!(
            std::fs::read_to_string(events.path()).unwrap(),
            format!("start:{session}\nlist:{session}\nverify:{session}\ncall:{session}\n")
        );
    }

    #[tokio::test]
    async fn failed_verification_prevents_effect_on_live_session() {
        let events = tempfile::NamedTempFile::new().unwrap();
        let spec = fixture(events.path());
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            RmcpStdioClient::invoke_checked(
                &spec,
                "echo",
                Default::default(),
                Duration::from_secs(3),
                &crate::sandbox::command::CommandBoundary::development_host(),
                |_| async { Err("signature refused".into()) },
            ),
        )
        .await
        .unwrap();
        assert_eq!(result.unwrap_err(), "signature refused");
        let evidence = std::fs::read_to_string(events.path()).unwrap();
        assert!(evidence.contains("list:"));
        assert!(!evidence.contains("call:"));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn child_environment_requires_explicit_configuration() {
        const KEY: &str = "SYMBIONT_MCP_ENV_FIXTURE";
        struct Restore(Option<std::ffi::OsString>);
        impl Drop for Restore {
            fn drop(&mut self) {
                match &self.0 {
                    Some(value) => std::env::set_var(KEY, value),
                    None => std::env::remove_var(KEY),
                }
            }
        }
        let _restore = Restore(std::env::var_os(KEY));
        std::env::set_var(KEY, "ambient-canary");
        let events = tempfile::NamedTempFile::new().unwrap();
        let mut spec = fixture(events.path());
        for configured in [false, true] {
            if configured {
                spec.env.insert(KEY.into(), "configured-fixture".into());
            }
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                RmcpStdioClient::verified_invoke_with_boundary(
                    &spec,
                    "echo",
                    Default::default(),
                    false,
                    Duration::from_secs(30),
                    &crate::sandbox::command::CommandBoundary::development_host(),
                ),
            )
            .await
            .unwrap()
            .unwrap();
            let payload: serde_json::Value =
                serde_json::from_str(result[0]["text"].as_str().unwrap()).unwrap();
            assert_eq!(
                payload["environment"],
                if configured {
                    serde_json::json!("configured-fixture")
                } else {
                    serde_json::Value::Null
                }
            );
        }
    }
}
