//! Bounded readiness checks through the shipping native worker path.
use super::{workspace, LandlockProfile};
use anyhow::Context;
use std::{os::unix::fs::PermissionsExt, path::Path, time::Duration};
use tokio::{io::AsyncReadExt, net::UnixListener};

const DEADLINE: Duration = Duration::from_secs(10);
const PROBE: &str = r#"
import os, pathlib, socket, sys
assert os.getcwd() == '/tmp/symbi-workspace', 'unexpected workspace'
assert os.environ['HOME'] == '/tmp/symbi-home', 'unexpected private home'
assert os.readlink('/proc/self/ns/mnt') != sys.argv[2], 'mount namespace is shared'
assert os.stat('/tmp').st_dev != int(sys.argv[3]), 'scratch filesystem is shared'
pathlib.Path('doctor-check').write_text('private scratch')
assert pathlib.Path('doctor-check').read_text() == 'private scratch'
if sys.argv[1] == 'transport':
    assert os.readlink('/proc/self/ns/net') != sys.argv[4], 'network namespace is shared'
    with socket.socket() as listener:
        listener.settimeout(2)
        listener.bind(('127.0.0.1', 0)); listener.listen(1)
        with socket.create_connection(listener.getsockname(), 2) as client:
            client.sendall(b'D')
            connection, _ = listener.accept()
            with connection:
                connection.settimeout(2)
                assert connection.recv(1) == b'D', 'loopback transport failed'
    for name in ('SYMBI_TOOLS_FD', 'SYMBI_INFERENCE_FD'):
        with socket.socket(fileno=int(os.environ.pop(name))) as channel:
            channel.settimeout(2)
            channel.sendall(b'D')
print('native diagnostic passed')
"#;

fn selected(profile: &LandlockProfile) -> LandlockProfile {
    let mut selected = profile.clone();
    selected.workspace = None;
    selected.clear_executable();
    selected.require_network = true;
    selected.max_execution_time = selected.max_execution_time.min(DEADLINE);
    selected.max_output_bytes = selected.max_output_bytes.min(4096);
    selected
}

fn probe_argv(transport: bool) -> anyhow::Result<Vec<String>> {
    use std::os::unix::fs::MetadataExt;
    Ok(vec![
        "/usr/bin/python3".into(),
        "-I".into(),
        "-c".into(),
        PROBE.into(),
        if transport { "transport" } else { "workspace" }.into(),
        std::fs::read_link("/proc/self/ns/mnt")?
            .to_string_lossy()
            .into(),
        std::fs::metadata("/tmp")?.dev().to_string(),
        std::fs::read_link("/proc/self/ns/net")?
            .to_string_lossy()
            .into(),
    ])
}

async fn checked(profile: LandlockProfile, argv: Vec<String>) -> anyhow::Result<String> {
    let result = workspace::execute(profile, argv, DEADLINE)
        .await
        .map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        result.exit_code >= 0,
        "restricted diagnostic was terminated by a signal or worker resource/deadline limit: {:?}",
        result.stderr.trim()
    );
    anyhow::ensure!(
        result.success,
        "restricted diagnostic exited {}: {:?}",
        result.exit_code,
        result.stderr.trim()
    );
    Ok(result.stdout.trim().into())
}

/// Exercise private user/mount namespaces, writable scratch and confirmed cleanup.
pub async fn native_workspace(profile: &LandlockProfile) -> anyhow::Result<()> {
    let output = checked(selected(profile), probe_argv(false)?).await?;
    anyhow::ensure!(
        output == "native diagnostic passed",
        "unexpected workspace diagnostic output"
    );
    Ok(())
}

/// Dummy connected capabilities carry diagnostic bytes only. No tool dispatcher,
/// inference provider, source roots or credentials are attached to these sockets.
struct Channels {
    _directory: tempfile::TempDir,
    tools: UnixListener,
    inference: UnixListener,
}

impl Channels {
    fn attach(profile: &mut LandlockProfile) -> anyhow::Result<Self> {
        // Short paths also work when the operator's TMPDIR exceeds sun_path.
        let directory = tempfile::Builder::new()
            .prefix("symbi-doctor-")
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir_in("/tmp")?;
        let tools = directory.path().join("tools.sock");
        let inference = directory.path().join("inference.sock");
        let listeners = (UnixListener::bind(&tools)?, UnixListener::bind(&inference)?);
        profile.workspace = Some(workspace::Workspace::managed(profile, tools, inference)?);
        Ok(Self {
            _directory: directory,
            tools: listeners.0,
            inference: listeners.1,
        })
    }
}

/// Exercise private loopback and both inherited connections, then confirm cleanup.
pub async fn managed_transport(profile: &LandlockProfile) -> anyhow::Result<()> {
    let mut profile = selected(profile);
    let channels = Channels::attach(&mut profile)?;
    let output = checked(profile, probe_argv(true)?).await?;
    anyhow::ensure!(
        output == "native diagnostic passed",
        "unexpected transport diagnostic output"
    );
    for listener in [&channels.tools, &channels.inference] {
        tokio::time::timeout(Duration::from_secs(1), async {
            let (mut stream, _) = listener.accept().await?;
            let mut byte = [0];
            stream.read_exact(&mut byte).await?;
            anyhow::ensure!(byte == *b"D", "inherited diagnostic connection failed");
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("inherited diagnostic connection timed out")??;
    }
    Ok(())
}

/// Run only --version through the real adapter with disconnected diagnostic
/// endpoints. The CLI has the same namespace and executable/metadata grants as
/// a managed session, but cannot request tools or inference from the runtime.
pub async fn managed_cli(profile: &LandlockProfile, executable: &Path) -> anyhow::Result<String> {
    let mut profile = selected(profile);
    profile
        .allow_executable(executable)
        .map_err(anyhow::Error::msg)?;
    let _channels = Channels::attach(&mut profile)?;
    let output = checked(
        profile,
        vec![
            "/usr/bin/python3".into(),
            "-I".into(),
            "-c".into(),
            include_str!("../../cli_executor/inference_bridge.py").into(),
            "--inherited".into(),
            executable
                .to_str()
                .context("invalid executable encoding")?
                .into(),
            "--version".into(),
        ],
    )
    .await?;
    anyhow::ensure!(
        !output.is_empty(),
        "configured CLI returned no version output"
    );
    Ok(output)
}
