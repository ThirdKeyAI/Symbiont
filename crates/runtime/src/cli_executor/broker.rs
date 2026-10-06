//! Private MCP capability channel from a contained CLI to governed tool calls.
//!
//! Containers receive only the public channel mount; VMs receive fixed vsock
//! capabilities. Policy, identity, approval and journal state stay in the runtime.

use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::watch,
    task::{JoinHandle, JoinSet},
};

use super::inference_broker::{self, ProtectedInference};
use crate::{
    reasoning::{
        governed_session::{BrokerToolCall, GovernedToolSession},
        prepared::digest_json,
    },
    sandbox::command::{CommandBoundary, CommandTier},
};

const FRAME_LIMIT: usize = 1024 * 1024;
const SESSION_IO_LIMIT: usize = 64 * 1024 * 1024;
const CHILD_CHANNEL: &str = "/opt/symbi-broker";

pub fn managed_executable(
    project: &std::path::Path,
    boundary: &CommandBoundary,
) -> Result<String, String> {
    resolve_managed_executable(&managed_configuration(project)?, boundary)
}

/// Doctor checks only an explicitly configured managed CLI installation.
/// A project without this section can still use ordinary native tools.
pub fn configured_managed_executable(
    project: &Path,
    boundary: &CommandBoundary,
) -> Result<Option<String>, String> {
    let root = managed_configuration(project)?;
    if root.get("managed_cli").is_none() {
        return Ok(None);
    }
    resolve_managed_executable(&root, boundary).map(Some)
}

fn managed_configuration(project: &Path) -> Result<toml::Value, String> {
    use std::io::Read;
    let file = std::fs::File::open(project.join("symbiont.toml")).map_err(|e| e.to_string())?;
    let mut text = String::new();
    file.take(1024 * 1024 + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > 1024 * 1024 {
        return Err("managed CLI configuration exceeds its size bound".into());
    }
    toml::from_str(&text).map_err(|e| e.to_string())
}

fn resolve_managed_executable(
    root: &toml::Value,
    boundary: &CommandBoundary,
) -> Result<String, String> {
    let setting = root
        .get("managed_cli")
        .and_then(|value| value.get("executable"));
    if boundary.tier != CommandTier::Landlock {
        if setting.is_some() {
            return Err("managed_cli.executable currently applies only to Landlock".into());
        }
        return Ok("claude".into());
    }
    #[cfg(target_os = "linux")]
    {
        let path = match setting {
            Some(value) => PathBuf::from(
                value
                    .as_str()
                    .ok_or("managed_cli.executable must be a path string")?,
            ),
            None => ["/usr/local/bin/claude", "/usr/bin/claude", "/bin/claude"]
                .iter()
                .map(PathBuf::from)
                .find(|path| path.is_file())
                .ok_or("configure [managed_cli] executable with the absolute CLI path")?,
        };
        let mut profile = boundary.landlock.clone();
        profile.allow_executable(&path)?;
        Ok(profile
            .executable()
            .ok_or("missing managed executable")?
            .to_str()
            .ok_or("invalid executable encoding")?
            .into())
    }
    #[cfg(not(target_os = "linux"))]
    Err("managed Landlock requires Linux".into())
}

/// Own the listener, connection tasks and the governed session until close.
/// Dropping this handle signals an independently owned cleanup task.
pub struct McpToolBroker {
    channel: PathBuf,
    has_inference: bool,
    stop: watch::Sender<bool>,
    task: Option<JoinHandle<Result<(), String>>>,
}

impl McpToolBroker {
    pub async fn start(
        session: Arc<GovernedToolSession>,
        private_parent: &Path,
    ) -> Result<Self, String> {
        Self::start_with_inference(session, private_parent, None).await
    }

    pub async fn start_with_inference(
        session: Arc<GovernedToolSession>,
        private_parent: &Path,
        inference: Option<ProtectedInference>,
    ) -> Result<Self, String> {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(private_parent)
            .map_err(|e| e.to_string())?;
        let metadata = std::fs::symlink_metadata(private_parent).map_err(|e| e.to_string())?;
        // SAFETY: geteuid has no memory-safety preconditions.
        if !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
        {
            return Err("broker parent must be a private runtime-owned directory".into());
        }
        let root = tempfile::Builder::new()
            .prefix("broker-")
            .tempdir_in(private_parent)
            .map_err(|e| e.to_string())?;
        let channel = root.path().join("channel");
        std::fs::create_dir(&channel).map_err(|e| e.to_string())?;
        let owner = ChannelOwner {
            _root: root,
            channel: channel.clone(),
        };
        let script = channel.join("bridge.py");
        std::fs::write(&script, include_str!("broker_bridge.py")).map_err(|e| e.to_string())?;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o444))
            .map_err(|e| e.to_string())?;
        let socket = channel.join("tools.sock");
        let listener = bind_socket(&channel, "tools.sock")?;
        // The 0700 parent excludes other host users. Containers see a read-only
        // 0555 mount; VMs connect through private vsock links. Each socket conveys
        // this run's capability.
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o666))
            .map_err(|e| e.to_string())?;
        let inference = match inference {
            Some(inference) => {
                let script = channel.join("inference_bridge.py");
                std::fs::write(&script, include_str!("inference_bridge.py"))
                    .map_err(|e| e.to_string())?;
                std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o444))
                    .map_err(|e| e.to_string())?;
                let socket = channel.join("inference.sock");
                let listener = bind_socket(&channel, "inference.sock")?;
                std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o666))
                    .map_err(|e| e.to_string())?;
                Some((listener, Arc::new(inference)))
            }
            None => None,
        };
        std::fs::set_permissions(&channel, std::fs::Permissions::from_mode(0o555))
            .map_err(|e| e.to_string())?;
        let (stop, stopped) = watch::channel(false);
        let has_inference = inference.is_some();
        let task = tokio::spawn(serve(listener, session, stopped, owner, inference));
        Ok(Self {
            channel,
            has_inference,
            stop,
            task: Some(task),
        })
    }

    /// Keep selected isolation/resource settings, but give this child only a
    /// scratch workspace and the runtime channel. Backend tools use their own
    /// separately authorized mounts. No direct network or source mounts remain.
    pub fn child_boundary(&self, selected: &CommandBoundary) -> Result<CommandBoundary, String> {
        selected.validate()?;
        let mut child = CommandBoundary::default();
        child.tier = selected.tier.clone();
        child.docker = selected.docker.clone();
        child.gvisor = selected.gvisor.clone();
        #[cfg(target_os = "linux")]
        if child.tier == CommandTier::Landlock {
            if !self.has_inference {
                return Err("managed inference broker is not configured".into());
            }
            child.landlock = selected.landlock.clone();
            child.landlock.require_network = true;
            child.landlock.workspace = Some(
                crate::sandbox::landlock::workspace::Workspace::managed(
                    &child.landlock,
                    self.socket_path(),
                    self.channel.join("inference.sock"),
                )
                .map_err(|e| e.to_string())?,
            );
            child.validate()?;
            return Ok(child);
        }
        if child.tier == CommandTier::Firecracker {
            let mut config = selected
                .firecracker
                .clone()
                .ok_or("missing Firecracker configuration")?;
            config.working_dir = "/tmp".into();
            config.services = Some(crate::sandbox::firecracker::services::GuestServices {
                tools: self.socket_path(),
                inference: self
                    .has_inference
                    .then(|| self.channel.join("inference.sock")),
            });
            child.firecracker = Some(config);
            child.validate()?;
            return Ok(child);
        }
        let config = match child.tier {
            CommandTier::Docker => &mut child.docker,
            CommandTier::GVisor => &mut child.gvisor.docker,
            _ => return Err(
                "managed broker requires Landlock, Docker, gVisor or Firecracker; no host fallback"
                    .into(),
            ),
        };
        config.volumes = vec![format!("{}:{CHILD_CHANNEL}:ro", self.channel.display())];
        config.network_mode = "none".into();
        config.working_dir = "/workspace".into();
        child.validate()?;
        Ok(child)
    }

    pub fn mcp_config(&self) -> Value {
        json!({"mcpServers": {"symbi": {"type": "stdio", "command": "python3",
            "args": [format!("{CHILD_CHANNEL}/bridge.py"), format!("{CHILD_CHANNEL}/tools.sock")]}}})
    }

    /// Build the bridge invocation for the selected worker. The code and exact
    /// endpoint become part of the immutable managed launch contract.
    pub fn mcp_config_for(&self, boundary: &CommandBoundary) -> Value {
        if boundary.tier == CommandTier::Landlock {
            return json!({"mcpServers": {"symbi": {"type": "stdio", "command": "python3",
                "args": ["-c", include_str!("broker_bridge.py"), "tcp:127.0.0.1:8766"]}}});
        }
        if boundary.tier != CommandTier::Firecracker {
            return self.mcp_config();
        }
        json!({"mcpServers": {"symbi": {"type": "stdio", "command": "python3",
            "args": ["-c", include_str!("broker_bridge.py"), "vsock:2:4051"]}}})
    }

    /// Arguments before the actual CLI executable for the private inference
    /// adapter. No provider credential is included in either transport.
    pub fn inference_bridge_args(&self, boundary: &CommandBoundary) -> Result<Vec<String>, String> {
        if !self.has_inference {
            return Err("managed inference broker is not configured".into());
        }
        Ok(if boundary.tier == CommandTier::Firecracker {
            vec![
                "-c".into(),
                include_str!("inference_bridge.py").into(),
                "--vsock".into(),
            ]
        } else if boundary.tier == CommandTier::Landlock {
            vec![
                "-c".into(),
                include_str!("inference_bridge.py").into(),
                "--inherited".into(),
            ]
        } else {
            vec![format!("{CHILD_CHANNEL}/inference_bridge.py")]
        })
    }

    /// Host-side endpoint for trusted adapters and transport tests. It exposes
    /// the same untrusted request surface as the child, never a policy bypass.
    pub fn socket_path(&self) -> PathBuf {
        self.channel.join("tools.sock")
    }

    pub async fn close(mut self) -> Result<(), String> {
        self.stop.send_replace(true);
        self.task
            .take()
            .ok_or("broker already closed")?
            .await
            .map_err(|e| format!("broker owner failed: {e}"))?
    }
}

impl Drop for McpToolBroker {
    fn drop(&mut self) {
        self.stop.send_replace(true);
    }
}

struct ChannelOwner {
    _root: tempfile::TempDir,
    channel: PathBuf,
}

fn bind_socket(channel: &Path, name: &str) -> Result<UnixListener, String> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        let directory = std::fs::File::open(channel).map_err(|e| e.to_string())?;
        // Bind through an owned directory descriptor so a long project path
        // cannot exceed sockaddr_un. The child uses the short mounted path.
        UnixListener::bind(format!("/proc/self/fd/{}/{name}", directory.as_raw_fd()))
            .map_err(|e| format!("cannot bind private broker channel: {e}"))
    }
    #[cfg(not(target_os = "linux"))]
    UnixListener::bind(channel.join(name))
        .map_err(|e| format!("cannot bind private broker channel: {e}"))
}
impl Drop for ChannelOwner {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(&self.channel, std::fs::Permissions::from_mode(0o700));
        // TempDir retains ownership through cleanup of listener/connection tasks.
    }
}

async fn serve(
    listener: UnixListener,
    session: Arc<GovernedToolSession>,
    mut stopped: watch::Receiver<bool>,
    owner: ChannelOwner,
    inference: Option<(UnixListener, Arc<ProtectedInference>)>,
) -> Result<(), String> {
    let mut connections = JoinSet::new();
    let bytes = Arc::new(AtomicUsize::new(0));
    let mut cancelled = session.cancellation();
    let mut accepted = 0usize;
    let mut failure = None;
    loop {
        tokio::select! {
            biased;
            _ = stopped.wait_for(|stop| *stop) => break,
            _ = cancelled.wait_for(|stop| *stop) => { failure = Some("governed broker session stopped".to_string()); break; },
            _ = tokio::time::sleep_until(session.deadline().into()) => {
                session.cancel_with_reason("broker session deadline expired");
                failure = Some("broker lifetime expired".to_string()); break;
            },
            finished = connections.join_next(), if !connections.is_empty() => {
                if let Some(Err(error)) = finished { failure = Some(format!("broker connection task failed: {error}")); session.cancel(); break; }
            },
            incoming = listener.accept(), if connections.len() < 8 && accepted < 64 => {
                match incoming {
                    Ok((stream, _)) => { accepted += 1; connections.spawn(connection(stream, session.clone(), bytes.clone())); },
                    Err(error) => { failure = Some(format!("broker accept failed: {error}")); session.cancel(); break; }
                }
            },
            incoming = async { match &inference {
                Some((listener, _)) => listener.accept().await,
                None => std::future::pending().await,
            } }, if connections.len() < 8 && accepted < 64 => {
                match incoming {
                    Ok((stream, _)) => {
                        accepted += 1;
                        let inference = inference.as_ref().unwrap().1.clone();
                        connections.spawn(inference_broker::connection(stream, inference));
                    },
                    Err(error) => { failure = Some(format!("inference accept failed: {error}")); session.cancel(); break; }
                }
            },
        }
    }
    drop(listener);
    drop(inference);
    connections.abort_all();
    while connections.join_next().await.is_some() {}
    let cleanup = session.close().await;
    drop(owner);
    cleanup?;
    failure.map_or(Ok(()), Err)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RpcRequest {
    jsonrpc: String,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CallParams {
    name: String,
    #[serde(default = "empty_object")]
    arguments: Value,
    #[serde(default, rename = "_meta")]
    _meta: Option<Value>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InitializeParams {
    #[serde(rename = "protocolVersion")]
    protocol_version: String,
    capabilities: Value,
    #[serde(rename = "clientInfo")]
    client_info: Value,
    #[serde(default, rename = "_meta")]
    _meta: Option<Value>,
}
fn empty_object() -> Value {
    json!({})
}

fn rpc_error(id: Value, code: i32, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

async fn connection(
    stream: UnixStream,
    session: Arc<GovernedToolSession>,
    bytes: Arc<AtomicUsize>,
) -> Result<(), String> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut negotiated = false;
    let mut initialized = false;
    let connection_id = uuid::Uuid::new_v4();
    for _ in 0..1024 {
        let mut frame = Vec::new();
        let count = tokio::time::timeout(
            Duration::from_secs(30),
            (&mut reader)
                .take((FRAME_LIMIT + 1) as u64)
                .read_until(b'\n', &mut frame),
        )
        .await
        .map_err(|_| "broker frame read timed out")?
        .map_err(|e| e.to_string())?;
        if count == 0 {
            return Ok(());
        }
        if count > FRAME_LIMIT || frame.last() != Some(&b'\n') {
            return Err("broker frame exceeds its limit or is incomplete".into());
        }
        if bytes
            .fetch_add(count, Ordering::Relaxed)
            .saturating_add(count)
            > SESSION_IO_LIMIT
        {
            session.cancel();
            return Err("broker session I/O budget exhausted".into());
        }
        let request: Result<RpcRequest, _> = serde_json::from_slice(&frame);
        let response = match request {
            Err(_) => rpc_error(Value::Null, -32600, "invalid broker request"),
            Ok(request) => {
                let id = request.id.unwrap_or(Value::Null);
                let valid_id = id
                    .as_str()
                    .is_some_and(|id| !id.is_empty() && id.len() <= 128)
                    || id.as_i64().is_some();
                if request.jsonrpc != "2.0" {
                    rpc_error(Value::Null, -32600, "invalid JSON-RPC version")
                } else if request.method == "notifications/symbi/close" && id.is_null() {
                    // The private bridge sends this after stdin EOF. Earlier
                    // requests and responses have finished on this connection;
                    // no session authority or other connection is affected.
                    return Ok(());
                } else if request.method == "notifications/initialized"
                    && id.is_null()
                    && negotiated
                {
                    initialized = true;
                    continue;
                } else if !valid_id {
                    rpc_error(
                        Value::Null,
                        -32600,
                        "request requires a bounded string or integer identity",
                    )
                } else if request.method == "initialize" && !negotiated {
                    match serde_json::from_value::<InitializeParams>(request.params) {
                        Ok(params)
                            if params.capabilities.is_object()
                                && params.client_info.is_object() =>
                        {
                            negotiated = true;
                            let version = match params.protocol_version.as_str() {
                                version @ ("2024-11-05" | "2025-03-26" | "2025-06-18") => version,
                                _ => "2025-06-18",
                            };
                            json!({"jsonrpc": "2.0", "id": id, "result": {"protocolVersion": version,
                                "capabilities": {"tools": {"listChanged": false}},
                                "serverInfo": {"name": "symbi-governed-tools", "version": env!("CARGO_PKG_VERSION")}}})
                        }
                        _ => rpc_error(id, -32602, "invalid broker initialization"),
                    }
                } else if request.method == "ping" {
                    json!({"jsonrpc": "2.0", "id": id, "result": {}})
                } else if !initialized {
                    rpc_error(id, -32000, "broker initialization required")
                } else if request.method == "tools/list" {
                    json!({"jsonrpc": "2.0", "id": id, "result": {"tools": session.tool_definitions().iter().map(|tool|
                        json!({"name": tool.name, "description": tool.description, "inputSchema": tool.parameters})).collect::<Vec<_>>()}})
                } else if request.method == "tools/call" {
                    match serde_json::from_value::<CallParams>(request.params) {
                        Err(_) => rpc_error(id, -32602, "invalid governed tool parameters"),
                        Ok(call) => {
                            let call_id = format!("mcp-{connection_id}-{}", digest_json(&id)?);
                            match session
                                .call(BrokerToolCall {
                                    call_id,
                                    name: call.name,
                                    arguments: call.arguments,
                                })
                                .await
                            {
                                Ok(observation) => json!({"jsonrpc": "2.0", "id": id, "result": {
                                    "content": [{"type": "text", "text": observation.content}], "isError": observation.is_error}}),
                                Err(error) => rpc_error(id, -32000, error),
                            }
                        }
                    }
                } else {
                    rpc_error(id, -32601, "broker method is unavailable")
                }
            }
        };
        let mut encoded = serde_json::to_vec(&response).map_err(|e| e.to_string())?;
        encoded.push(b'\n');
        if encoded.len() > FRAME_LIMIT
            || bytes
                .fetch_add(encoded.len(), Ordering::Relaxed)
                .saturating_add(encoded.len())
                > SESSION_IO_LIMIT
        {
            session.cancel();
            return Err("broker response exceeds its limit".into());
        }
        tokio::time::timeout(Duration::from_secs(5), writer.write_all(&encoded))
            .await
            .map_err(|_| "broker response write timed out")?
            .map_err(|e| e.to_string())?;
    }
    Err("broker connection request budget exhausted".into())
}
