//! Real containers through the public CLI executor. No model or real credentials.
#![cfg(all(unix, feature = "cli-executor"))]

use async_trait::async_trait;
use std::{collections::HashMap, os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};
use symbi_runtime::{
    cli_executor::{
        AiCliAdapter, CliExecutor, CliExecutorConfig, CodeGenRequest, CodeGenResult, StdinStrategy,
    },
    sandbox::{command::CommandBoundary, ExecutionResult},
};

const SCRIPT: &str = r#"
import json, os, pathlib, socket, sys, time
root = pathlib.Path('/workspace')
mode, canary, port = sys.argv[1:]
assert os.getuid() == 65534
assert not pathlib.Path(canary).exists()
assert 'SYMBI_DOCKER_AMBIENT_CANARY' not in os.environ
assert 'ANTHROPIC_API_KEY' not in os.environ
assert os.environ['HOME'] == '/tmp'
assert os.environ['FIXTURE_HANDSHAKE'] == 'explicit'
try:
    socket.create_connection(('127.0.0.1', int(port)), 0.2).close()
    raise AssertionError('reached host observer')
except OSError: pass
assert pathlib.Path('/sys/fs/cgroup/pids.max').read_text().strip() == '24'
assert int(pathlib.Path('/sys/fs/cgroup/memory.max').read_text()) == 134217728
assert pathlib.Path('/sys/fs/cgroup/cpu.max').read_text().split()[0] != 'max'
(root / 'started').touch()
if mode in ('cancel', 'deadline', 'background'):
    if os.fork() == 0:
        os.setsid()
        while True:
            (root / 'ticks').write_text(str(time.monotonic()))
            time.sleep(0.02)
    while not (root / 'ticks').exists(): time.sleep(0.01)
    if mode != 'background': time.sleep(60)
if mode in ('stdout_flood', 'stderr_flood'):
    stream = sys.stdout if mode == 'stdout_flood' else sys.stderr
    stream.write('x' * 65536); stream.flush(); time.sleep(60)
if mode == 'idle': time.sleep(60)
if mode == 'active_stdout':
    for _ in range(90): print('active', flush=True); time.sleep(0.05)
if mode == 'pid_limit':
    children = []
    try:
        for _ in range(64):
            child = os.fork()
            if child == 0: time.sleep(60); os._exit(0)
            children.append(child)
    except OSError: pass
    assert 0 < len(children) < 24
    for child in children: os.kill(child, 9); os.waitpid(child, 0)
if mode == 'memory_limit':
    data = bytearray(512 * 1024 * 1024)
    raise AssertionError('memory bound failed')
if mode == 'stdin':
    assert sys.stdin.read() == 'approved input\nsecond line\n'
(root / 'answer').write_text('useful contained edit')
print(json.dumps(dict(result='allowed', mode=mode)), flush=True)
if mode == 'nonzero': sys.exit(7)
"#;

struct Adapter {
    args: Vec<String>,
    stdin: StdinStrategy,
}
#[async_trait]
impl AiCliAdapter for Adapter {
    fn name(&self) -> &str {
        "fixture"
    }
    fn executable(&self) -> &str {
        "python3"
    }
    fn build_args(&self, _: &CodeGenRequest) -> Vec<String> {
        self.args.clone()
    }
    fn non_interactive_env(&self) -> HashMap<String, String> {
        HashMap::from([("FIXTURE_HANDSHAKE".into(), "explicit".into())])
    }
    fn stdin_strategy(&self) -> StdinStrategy {
        self.stdin.clone()
    }
    fn parse_output(&self, _: &CodeGenRequest, execution: ExecutionResult) -> CodeGenResult {
        // Deliberately optimistic parser: the executor must preserve OS failure.
        CodeGenResult {
            success: true,
            execution,
            parsed_output: None,
            files_modified: vec![],
            adapter_name: self.name().into(),
        }
    }
    async fn health_check(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

struct Fixture {
    root: tempfile::TempDir,
    label: String,
    boundary: CommandBoundary,
    observer: std::net::TcpListener,
}
impl Fixture {
    fn new() -> Self {
        assert_eq!(
            std::env::var("SYMBI_DOCKER_AMBIENT_CANARY").as_deref(),
            Ok("synthetic-ambient-value")
        );
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("output");
        std::fs::create_dir(&output).unwrap();
        std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o777)).unwrap();
        std::fs::write(root.path().join("canary"), "synthetic host value").unwrap();
        let label = format!("symbi.cli-e2e={}", uuid::Uuid::new_v4());
        let mut boundary = CommandBoundary::default();
        boundary.docker.volumes = vec![format!("{}:/workspace:rw", output.display())];
        boundary.docker.extra_flags = vec![format!("--label={label}")];
        boundary.docker.memory_limit = Some("128m".into());
        boundary.docker.pids_limit = 24;
        boundary.docker.max_execution_time = Duration::from_secs(30);
        let observer = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        observer.set_nonblocking(true).unwrap();
        Self {
            root,
            label,
            boundary,
            observer,
        }
    }
    fn adapter(&self, mode: &str) -> Adapter {
        Adapter {
            args: vec![
                "-u".into(),
                "-c".into(),
                SCRIPT.into(),
                mode.into(),
                self.root.path().join("canary").display().to_string(),
                self.observer.local_addr().unwrap().port().to_string(),
            ],
            stdin: if mode == "stdin" {
                StdinStrategy::Scripted(vec!["approved input".into(), "second line".into()])
            } else {
                StdinStrategy::CloseImmediately
            },
        }
    }
    fn request(&self) -> CodeGenRequest {
        CodeGenRequest {
            prompt: "perform approved edit".into(),
            working_dir: PathBuf::from("/workspace"),
            target_files: vec![],
            system_context: None,
            model: None,
            options: HashMap::new(),
        }
    }
    fn executor(&self, config: CliExecutorConfig) -> CliExecutor {
        CliExecutor::new(config).with_command_boundary(self.boundary.clone())
    }
    fn workers(&self) -> Vec<String> {
        let output = std::process::Command::new("docker")
            .args(["ps", "-aq", "--filter", &format!("label={}", self.label)])
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout)
            .unwrap()
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    }
    async fn clean(&self) {
        for _ in 0..100 {
            if self.workers().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(self.workers().is_empty(), "contained CLI worker leaked");
        assert_eq!(
            std::fs::read_to_string(self.root.path().join("canary")).unwrap(),
            "synthetic host value"
        );
        assert_eq!(
            self.observer.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        let ticks = self.root.path().join("output/ticks");
        if ticks.exists() {
            let before = std::fs::read(&ticks).unwrap();
            tokio::time::sleep(Duration::from_millis(150)).await;
            assert_eq!(
                std::fs::read(&ticks).unwrap(),
                before,
                "detached descendant still active"
            );
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for worker in self.workers() {
            let _ = std::process::Command::new("docker")
                .args(["rm", "-f", &worker])
                .output();
        }
    }
}

#[tokio::test]
#[ignore = "requires Docker and matching sandbox supervisor"]
async fn cli_useful_work_credentials_network_and_resource_bounds() {
    for mode in [
        "normal",
        "stdin",
        "pid_limit",
        "nonzero",
        "memory_limit",
        "background",
    ] {
        let fixture = Fixture::new();
        let result = fixture
            .executor(CliExecutorConfig::default())
            .execute(&fixture.adapter(mode), &fixture.request())
            .await
            .unwrap();
        fixture.clean().await;
        assert!(
            fixture.root.path().join("output/started").exists(),
            "fixture did not start: {}",
            result.execution.stderr
        );
        assert_eq!(
            result.success,
            !matches!(mode, "nonzero" | "memory_limit"),
            "{mode}: {:?}",
            result.execution
        );
        if mode != "memory_limit" {
            assert_eq!(
                std::fs::read_to_string(fixture.root.path().join("output/answer")).unwrap(),
                "useful contained edit"
            );
        }
        if mode == "nonzero" {
            assert_eq!(result.execution.exit_code, 7);
        }
    }
}

#[tokio::test]
#[ignore = "requires Docker and matching sandbox supervisor"]
async fn cli_output_overflow_removes_worker_without_success() {
    for mode in ["stdout_flood", "stderr_flood"] {
        let fixture = Fixture::new();
        let result = fixture
            .executor(CliExecutorConfig {
                max_output_bytes: 256,
                ..Default::default()
            })
            .execute(&fixture.adapter(mode), &fixture.request())
            .await;
        assert!(result.unwrap_err().to_string().contains("output limit"));
        assert!(fixture.root.path().join("output/started").exists());
        fixture.clean().await;
    }
}

#[tokio::test]
#[ignore = "requires Docker and matching sandbox supervisor"]
async fn cli_idle_uses_combined_stream_activity() {
    for mode in ["active_stdout", "idle"] {
        let fixture = Fixture::new();
        let result = fixture
            .executor(CliExecutorConfig {
                // Include realistic Docker startup scheduling margin. The
                // stdout-only fixture runs for 4.5s, longer than this budget,
                // so independently timing out silent stderr would still fail.
                idle_timeout: Duration::from_secs(3),
                ..Default::default()
            })
            .execute(&fixture.adapter(mode), &fixture.request())
            .await;
        if mode == "active_stdout" {
            assert!(result.unwrap().success);
        } else {
            assert!(result.unwrap_err().to_string().contains("idle timeout"));
        }
        assert!(fixture.root.path().join("output/started").exists());
        fixture.clean().await;
    }
}

#[tokio::test]
#[ignore = "requires Docker and matching sandbox supervisor"]
async fn cli_deadline_removes_detached_descendants() {
    let fixture = Fixture::new();
    let result = fixture
        .executor(CliExecutorConfig {
            max_runtime: Duration::from_secs(3),
            ..Default::default()
        })
        .execute(&fixture.adapter("deadline"), &fixture.request())
        .await;
    assert!(result.is_err() || !result.unwrap().success);
    assert!(fixture.root.path().join("output/ticks").exists());
    fixture.clean().await;
}

#[tokio::test]
#[ignore = "requires Docker and matching sandbox supervisor"]
async fn cli_caller_cancellation_removes_detached_descendants() {
    let fixture = Fixture::new();
    let executor = fixture.executor(CliExecutorConfig::default());
    let adapter = fixture.adapter("cancel");
    let request = fixture.request();
    let task = tokio::spawn(async move { executor.execute(&adapter, &request).await });
    for _ in 0..100 {
        if fixture.root.path().join("output/ticks").exists() {
            break;
        }
        assert!(!task.is_finished(), "fixture exited before cancellation");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(fixture.root.path().join("output/ticks").exists());
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    fixture.clean().await;
}
