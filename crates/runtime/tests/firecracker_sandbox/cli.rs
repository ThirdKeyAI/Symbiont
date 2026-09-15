use super::*;
use async_trait::async_trait;
use std::sync::Arc;
use symbi_runtime::cli_executor::{
    AiCliAdapter, CliExecutor, CliExecutorConfig, CodeGenRequest, CodeGenResult, StdinStrategy,
};
use symbi_runtime::sandbox::ExecutionResult;

struct Adapter {
    code: String,
    stdin: StdinStrategy,
}
#[async_trait]
impl AiCliAdapter for Adapter {
    fn name(&self) -> &str {
        "guest-cli-fixture"
    }
    fn executable(&self) -> &str {
        "python3"
    }
    fn build_args(&self, _: &CodeGenRequest) -> Vec<String> {
        vec!["-c".into(), self.code.clone()]
    }
    fn non_interactive_env(&self) -> HashMap<String, String> {
        HashMap::from([("FIXTURE_HANDSHAKE".into(), "explicit".into())])
    }
    fn stdin_strategy(&self) -> StdinStrategy {
        self.stdin.clone()
    }
    fn parse_output(&self, _: &CodeGenRequest, execution: ExecutionResult) -> CodeGenResult {
        // Deliberately optimistic: the executor must retain the observed outcome.
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
fn request() -> CodeGenRequest {
    CodeGenRequest {
        prompt: "guest fixture".into(),
        working_dir: "/tmp".into(),
        target_files: vec![],
        system_context: None,
        model: None,
        options: HashMap::new(),
    }
}

#[tokio::test]
#[ignore = "requires KVM and a matching guest rootfs with Python 3"]
async fn cli_observes_guest_exit_input_effects_and_loopback_isolation() {
    let fixture = Fixture::new();
    let canary = fixture.root.path().join("host-canary");
    std::fs::write(&canary, "host-only").unwrap();
    let adapter = Adapter {
        stdin: StdinStrategy::Scripted(vec!["approved input".into(), "second line".into()]),
        code: format!(
            r#"
import os, pathlib, socket, sys
assert os.getuid() == os.getgid() == 65534
assert not pathlib.Path({canary:?}).exists()
assert 'SYMBI_FIRECRACKER_AMBIENT_CANARY' not in os.environ
assert 'ANTHROPIC_API_KEY' not in os.environ
assert os.environ['FIXTURE_HANDSHAKE'] == 'explicit'
assert sys.stdin.read() == 'approved input\nsecond line\n'
assert {{p.name for p in pathlib.Path('/sys/class/net').iterdir()}} == {{'lo'}}
with socket.socket() as listener:
    listener.bind(('127.0.0.1', 0)); listener.listen()
    with socket.create_connection(listener.getsockname(), 1) as client:
        accepted, _ = listener.accept()
        with accepted:
            client.sendall(b'local'); assert accepted.recv(5) == b'local'
for port in (4050, 4051, 4052, 4053):
    with socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM) as stream:
        stream.settimeout(.3)
        try: stream.connect((2, port))
        except OSError: pass
        else: raise AssertionError('unissued guest capability')
pathlib.Path('/tmp/effect').write_text('actual guest effect')
print(pathlib.Path('/tmp/effect').read_text(), flush=True)
"#,
            canary = canary.to_str().unwrap()
        ),
    };
    let executor =
        CliExecutor::new(CliExecutorConfig::default()).with_command_boundary(fixture.boundary());
    executor.health_check(&adapter).await.unwrap();
    let result = executor.execute(&adapter, &request()).await.unwrap();
    assert!(result.success && result.execution.success);
    assert_eq!(result.execution.exit_code, 0);
    assert!(result.execution.stdout.contains("actual guest effect"));
    fixture.settled().await;
    assert_eq!(std::fs::read_to_string(canary).unwrap(), "host-only");
}

#[tokio::test]
#[ignore = "requires KVM and a matching guest rootfs with Python 3"]
async fn cli_rejects_nonzero_exit_overflow_idle_and_deadline() {
    for (code, expected, lifetime) in [
        (
            "print('claimed success', flush=True); raise SystemExit(7)",
            "exit 7",
            10,
        ),
        (
            "import sys; sys.stdout.write('x'*65536); sys.stdout.flush()",
            "stdout_truncated=true",
            10,
        ),
        ("import time; time.sleep(30)", "idle timeout", 10),
        (
            "import time\nwhile True: print('active', flush=True); time.sleep(.05)",
            "timed out",
            2,
        ),
    ] {
        let fixture = Fixture::new();
        let adapter = Adapter {
            code: code.into(),
            stdin: StdinStrategy::CloseImmediately,
        };
        let executor = CliExecutor::new(CliExecutorConfig {
            max_runtime: Duration::from_secs(lifetime),
            idle_timeout: Duration::from_secs(1),
            max_output_bytes: 4096,
            ..Default::default()
        })
        .with_command_boundary(fixture.boundary());
        let result = executor.execute(&adapter, &request()).await;
        if expected == "exit 7" {
            let result = result.unwrap();
            assert!(!result.success && !result.execution.success);
            assert_eq!(result.execution.exit_code, 7);
            assert!(result.execution.stdout.contains("claimed success"));
        } else {
            let error = result.unwrap_err().to_string();
            assert!(error.contains(expected), "{expected}: {error}");
        }
        fixture.settled().await;
    }
}

#[cfg(feature = "cedar")]
#[tokio::test]
#[ignore = "requires KVM and a matching guest rootfs with Python 3"]
async fn managed_guest_uses_only_issued_broker_capabilities() {
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;
    use symbi_runtime::{
        cli_executor::{broker::McpToolBroker, governed::ManagedCliActionExecutor},
        reasoning::{
            conversation::Conversation,
            governed_session::GovernedToolSession,
            loop_types::{BufferedJournal, LoopConfig, LoopState},
            CedarPolicy, CedarPolicyGate,
        },
        types::AgentId,
    };
    let fixture = Fixture::new();
    std::fs::set_permissions(fixture.root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let manifest: Manifest = toml::from_str(r#"
[tool]
name = "count_fixture"
version = "1"
description = "Actual normalized guest file effect"
binary = "python3"
[args.count]
position = 1
type = "integer"
required = true
min = 1
max = 5
clamp = true
[command]
template = "python3 -c 'from pathlib import Path; p=Path(\"/tmp/tool-effect\"); p.write_text(\"{count}\"); print(p.read_text())'"
[output]
format = "text"
"#).unwrap();
    let backend = Arc::new(
        ToolCladExecutor::new(vec![("count_fixture".into(), manifest)])
            .with_command_boundary(fixture.boundary()),
    );
    let gate = Arc::new(CedarPolicyGate::deny_by_default());
    gate.add_policy(CedarPolicy {
        name: "fixture".into(),
        active: true,
        source: r#"
permit(principal, action == Action::"tool_call::claude_code", resource);
permit(principal, action == Action::"tool_call::count_fixture", resource)
when { context.invocation.arguments.count == "5" };
"#
        .into(),
    })
    .await;
    let journal = Arc::new(BufferedJournal::new(100));
    let session = Arc::new(
        GovernedToolSession::start(
            backend,
            gate,
            journal.clone(),
            LoopState::new(
                AgentId::new(),
                Conversation::with_system("VM broker fixture"),
            ),
            LoopConfig {
                timeout: Duration::from_secs(20),
                ..Default::default()
            },
        )
        .await
        .unwrap(),
    );
    let broker = McpToolBroker::start(session.clone(), fixture.root.path())
        .await
        .unwrap();
    let child = broker.child_boundary(&fixture.boundary()).unwrap();
    assert_eq!(
        child.descriptor().unwrap()["vm"]["broker_ports"],
        json!([4051])
    );
    let config = child.firecracker.as_ref().unwrap();
    let restored: Result<FirecrackerConfig, _> =
        serde_json::from_value(serde_json::to_value(config).unwrap());
    assert!(restored.is_err() || restored.unwrap().services.is_none());
    let mcp = broker.mcp_config_for(&child);
    let code = format!(
        r#"
import json, os, pathlib, socket, subprocess
assert os.getuid() == 65534
for port in (4050, 4052, 4053):
    with socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM) as stream:
        stream.settimeout(.3)
        try: stream.connect((2, port))
        except OSError: pass
        else: raise AssertionError('unissued service available')
config = json.loads({config:?})['mcpServers']['symbi']
bridge = subprocess.Popen([config['command']] + config['args'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
def send(value):
    bridge.stdin.write(json.dumps(value)+'\n'); bridge.stdin.flush()
    if 'id' in value:
        response = json.loads(bridge.stdout.readline())
        assert response['id'] == value['id'], response
        return response
send(dict(jsonrpc='2.0', id=1, method='initialize', params=dict(protocolVersion='2025-06-18', capabilities={{}}, clientInfo=dict(name='fixture', version='1'))))
send(dict(jsonrpc='2.0', method='notifications/initialized'))
listed = send(dict(jsonrpc='2.0', id=2, method='tools/list'))
assert [tool['name'] for tool in listed['result']['tools']] == ['count_fixture']
message = dict(jsonrpc='2.0', id=3, method='tools/call', params=dict(name='count_fixture', arguments=dict(count='999')))
allowed = send(message)
assert not allowed['result']['isError'], allowed
effect = json.loads(allowed['result']['content'][0]['text'])
assert effect['status'] == 'success' and effect['results']['raw_output'] == '5\n', effect
assert 'error' in send(message), 'replay accepted'
denied = send(dict(jsonrpc='2.0', id=4, method='tools/call', params=dict(name='count_fixture', arguments=dict(count='1'))))
assert denied['result']['isError'], denied
forged = send(dict(jsonrpc='2.0', id=5, method='tools/call', params=dict(name='count_fixture', arguments=dict(count='5'), principal='forged')))
assert 'error' in forged, forged
assert not pathlib.Path('/tmp/tool-effect').exists(), 'tool scratch leaked into CLI guest'
bridge.stdin.close(); assert bridge.wait(timeout=5) == 0
print('governed VM broker effect and denials verified', flush=True)
"#,
        config = mcp.to_string()
    );
    let launch = Arc::new(
        ManagedCliActionExecutor::new(
            Arc::new(Adapter {
                code,
                stdin: StdinStrategy::CloseImmediately,
            }),
            request(),
            child,
            CliExecutorConfig::default(),
            json!({"tool_sandbox":fixture.boundary().descriptor().unwrap()}),
            false,
        )
        .unwrap(),
    );
    let outcome = session
        .dispatch_host_action(launch.clone(), launch.proposal("vm-admission"))
        .await
        .unwrap();
    assert_eq!(outcome.len(), 1);
    let execution = launch.take_result();
    assert!(
        !outcome[0].is_error,
        "{}: {execution:?}",
        outcome[0].content
    );
    assert!(execution
        .unwrap()
        .execution
        .stdout
        .contains("denials verified"));
    broker.close().await.unwrap();
    fixture.settled().await;
    let entries = journal.entries().await;
    assert!(serde_json::to_string(&entries)
        .unwrap()
        .contains("count_fixture"));
}

#[tokio::test]
#[ignore = "requires KVM and a matching guest rootfs with Python 3"]
async fn cli_drop_removes_live_descendants() {
    let fixture = Fixture::new();
    let ready = Arc::new(tokio::sync::Notify::new());
    let observed = ready.clone();
    let executor = CliExecutor::new(CliExecutorConfig::default())
        .with_command_boundary(fixture.boundary())
        .with_stdout_line_sink(Arc::new(move |line| {
            if line == "ready" {
                observed.notify_one();
            }
        }));
    let adapter = Adapter {
        stdin: StdinStrategy::CloseImmediately,
        code: r#"
import subprocess
child = subprocess.Popen(['/bin/sh', '-c', 'sleep 30'], start_new_session=True)
print('ready', flush=True)
child.wait()
"#
        .into(),
    };
    let task = tokio::spawn(async move { executor.execute(&adapter, &request()).await });
    tokio::time::timeout(Duration::from_secs(8), ready.notified())
        .await
        .unwrap();
    assert!(!fixture.leases().is_empty());
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    fixture.settled().await;
}
