//! Real PTY workers through the shipping session executor. Run explicitly with
//! Docker and the cached fixture image; missing infrastructure is a failure.
#![cfg(all(unix, feature = "toolclad-session"))]
use std::{os::unix::fs::PermissionsExt, sync::Arc, time::Duration};
use symbi_runtime::{
    sandbox::command::CommandBoundary,
    toolclad::{session_executor::SessionExecutor, Manifest},
};

const SERVER: &str = r#"
import json, os, pathlib, socket, sys, termios, time, uuid
root = pathlib.Path('/workspace')
cookie = str(uuid.uuid4())
proof = dict(pty=sys.stdin.isatty() and sys.stdout.isatty(), non_root=os.getuid() == 65534,
    echo_off=not bool(termios.tcgetattr(0)[3] & termios.ECHO),
    host_file_denied=not pathlib.Path(sys.argv[1]).exists(),
    ambient_absent='SYMBI_DOCKER_AMBIENT_CANARY' not in os.environ,
    adjacent_denied=not (root/'adjacent.txt').exists())
try:
    connection = socket.create_connection(('127.0.0.1', int(sys.argv[2])), timeout=0.2)
    connection.close()
    proof['network_denied'] = False
except OSError:
    proof['network_denied'] = True
assert all(proof.values()), proof
(root / ('proof-' + cookie)).write_text(json.dumps(proof))
def prompt(): print('READY> ', end='', flush=True)
prompt()
count = sum(map(int,(root/'numbers.txt').read_text().split())) if (root/'numbers.txt').exists() else 0
for line in sys.stdin:
    command = line.strip()
    if command.startswith('add '):
        count += int(command.split()[1])
        (root / ('value-' + cookie)).write_text(str(count))
    elif command == 'finish':
        (root/'result.json').write_text(json.dumps(dict(value=count,cookie=cookie,proof=proof)))
    elif command in ('hang', 'background'):
        if os.fork() == 0:
            os.setsid()
            (root / ('detached-' + cookie)).touch()
            time.sleep(30)
            (root / 'late-effect').touch()
            os._exit(0)
        if command == 'hang':
            (root / 'busy').touch()
            time.sleep(60)
    elif command in ('flood', 'stderr'):
        stream = sys.stderr if command == 'stderr' else sys.stdout
        stream.write('界' * 100000)
        stream.flush()
        time.sleep(60)
    print(json.dumps(dict(value=count, cookie=cookie, proof=proof)), flush=True)
    prompt()
"#;

struct Fixture {
    root: tempfile::TempDir,
    manifest: Manifest,
    profile: CommandBoundary,
    label: String,
    sink: std::net::TcpListener,
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
        let canary = root.path().join("host-canary");
        std::fs::write(&canary, "synthetic host value").unwrap();
        let sink = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        sink.set_nonblocking(true).unwrap();
        let mut manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "terminal_fixture"
mode = "session"
version = "1"
description = "Bounded interactive fixture"
timeout_seconds = 10
[session]
startup_command = "placeholder"
ready_pattern = "^READY>$"
startup_timeout_seconds = 5
idle_timeout_seconds = 30
session_timeout_seconds = 30
max_interactions = 10
[session.interaction]
output_wait_ms = 1000
output_max_bytes = 16384
[session.commands.send]
pattern = "(?:add [1-9]|hang|flood|stderr|background)"
description = "Interactive fixture command"
[output]
format = "text"
"#,
        )
        .unwrap();
        manifest.session.as_mut().unwrap().startup_command = format!(
            "/usr/local/bin/python3 -u -c {} '{}' {}",
            shlex::try_quote(SERVER).unwrap(),
            canary.display(),
            sink.local_addr().unwrap().port()
        );
        let label = format!("symbi.pty-e2e={}", uuid::Uuid::new_v4());
        let mut profile = CommandBoundary::default();
        profile.docker.image =
            std::env::var("SYMBI_PTY_TEST_IMAGE").unwrap_or_else(|_| "python:3.12-slim".into());
        profile.docker.max_output_bytes = 65536;
        profile.docker.volumes = vec![format!("{}:/workspace:rw", output.display())];
        profile.docker.extra_flags.push(format!("--label={label}"));
        Self {
            root,
            manifest,
            profile,
            label,
            sink,
        }
    }
    fn executor(&self) -> SessionExecutor {
        SessionExecutor::new(vec![("terminal_fixture".into(), self.manifest.clone())])
            .with_command_boundary(self.profile.clone())
    }
    fn check_host(&self) {
        assert_eq!(
            std::fs::read_to_string(self.root.path().join("host-canary")).unwrap(),
            "synthetic host value"
        );
        assert!(matches!(self.sink.accept(), Err(e) if e.kind()==std::io::ErrorKind::WouldBlock));
        assert!(!self.root.path().join("output/late-effect").exists());
    }
    async fn workers(&self) -> Vec<String> {
        let output = tokio::process::Command::new("docker")
            .args(["ps", "-aq", "--filter", &format!("label={}", self.label)])
            .output()
            .await
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout)
            .unwrap()
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    }
    async fn removed(&self) {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let output = tokio::process::Command::new("docker")
                    .args(["ps", "-aq", "--filter", &format!("label={}", self.label)])
                    .output()
                    .await
                    .unwrap();
                assert!(output.status.success());
                if output.stdout.is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("owned PTY container must be removed");
    }
}
async fn command(executor: &SessionExecutor, command: &str) -> Result<serde_json::Value, String> {
    executor
        .execute_session_command_async(
            "terminal_fixture.send",
            &serde_json::json!({"command":command}).to_string(),
        )
        .await
}
fn payload(result: &serde_json::Value) -> serde_json::Value {
    result["results"]["output"]
        .as_str()
        .unwrap()
        .lines()
        .find_map(|line| serde_json::from_str(line).ok())
        .expect("real terminal response JSON")
}

#[tokio::test]
#[ignore = "requires Docker and cached fixture image; synthetic ambient canary must be set"]
async fn real_pty_preserves_state_and_denies_host_files_credentials_and_network() {
    let fixture = Fixture::new();
    let executor = fixture.executor();
    let first = command(&executor, "add 1").await;
    let second = command(&executor, "add 2").await;
    let cleanup = executor.cleanup_async().await;
    fixture.removed().await;
    let first = first.unwrap();
    let second = second.unwrap();
    cleanup.unwrap();
    assert_eq!(first["session_id"], second["session_id"]);
    let one = payload(&first);
    let two = payload(&second);
    assert_eq!(one["cookie"], two["cookie"]);
    assert_eq!(one["value"], 1);
    assert_eq!(two["value"], 3);
    assert!(two["proof"]
        .as_object()
        .unwrap()
        .values()
        .all(|v| v == true));
    assert!(
        std::fs::read_dir(fixture.root.path().join("output"))
            .unwrap()
            .next()
            .is_none(),
        "undeclared scratch effects must remain private"
    );
    assert_eq!(second["execution_status"], "prompt_observed");
    assert!(second["exit_code"].is_null());
    fixture.check_host();
}

#[tokio::test]
#[ignore = "requires Docker and cached fixture image; synthetic ambient canary must be set"]
async fn active_deadline_and_stream_limits_remove_detached_workers() {
    for operation in ["hang", "flood", "stderr"] {
        let mut fixture = Fixture::new();
        fixture.manifest.tool.timeout_seconds = 2;
        let executor = fixture.executor();
        command(&executor, "add 1").await.unwrap();
        let started = std::time::Instant::now();
        let error = command(&executor, operation).await.unwrap_err();
        fixture.removed().await;
        executor.cleanup_async().await.unwrap();
        assert!(started.elapsed() < Duration::from_secs(10), "{error}");
        assert!(
            error.contains("timed out") || error.contains("output limit"),
            "{error}"
        );
        fixture.check_host();
    }
}

#[tokio::test]
#[ignore = "requires Docker and cached fixture image; synthetic ambient canary must be set"]
async fn cancellation_and_drop_cleanup_persistent_workers() {
    for abort in [false, true] {
        let fixture = Fixture::new();
        let executor = Arc::new(fixture.executor());
        command(&executor, "add 1").await.unwrap();
        if abort {
            let worker = executor.clone();
            let task = tokio::spawn(async move { command(&worker, "hang").await });
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let ids = fixture.workers().await;
                    for id in ids {
                        let top = tokio::process::Command::new("docker")
                            .args(["top", &id, "-eo", "pid,ppid,comm"])
                            .output()
                            .await
                            .unwrap();
                        if top.status.success()
                            && String::from_utf8_lossy(&top.stdout)
                                .lines()
                                .filter(|line| line.contains("python"))
                                .count()
                                >= 2
                        {
                            return;
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap();
            task.abort();
            let _ = task.await;
        }
        drop(executor);
        fixture.removed().await;
        fixture.check_host();
    }
}

#[tokio::test]
#[ignore = "requires Docker and cached fixture image; synthetic ambient canary must be set"]
async fn idle_and_total_lifetime_limits_do_not_silently_restart_sessions() {
    for mode in ["idle", "container_lifetime", "sdk_lifetime", "interactions"] {
        let mut fixture = Fixture::new();
        let session = fixture.manifest.session.as_mut().unwrap();
        match mode {
            "idle" => session.idle_timeout_seconds = 1,
            "container_lifetime" => {
                fixture.profile.docker.max_execution_time = Duration::from_secs(2)
            }
            "sdk_lifetime" => session.session_timeout_seconds = 2,
            "interactions" => session.max_interactions = 1,
            _ => unreachable!(),
        }
        let executor = fixture.executor();
        command(&executor, "add 1").await.unwrap();
        fixture.removed().await;
        let error = command(&executor, "add 1").await.unwrap_err();
        assert!(error.contains("closed"), "{error}");
        executor.cleanup_async().await.unwrap();
        assert!(std::fs::read_dir(fixture.root.path().join("output"))
            .unwrap()
            .next()
            .is_none());
        fixture.check_host();
    }
}

#[cfg(feature = "cedar")]
mod governed {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use symbi_runtime::{
        reasoning::{
            cedar_gate::{CedarPolicy, CedarPolicyGate},
            conversation::{Conversation, MessageRole},
            inference::{
                FinishReason, InferenceError, InferenceOptions, InferenceProvider,
                InferenceResponse, ToolCallRequest, Usage,
            },
            loop_types::{BufferedJournal, LoopConfig, LoopEvent, TerminationReason},
            reasoning_loop::ReasoningLoopRunner,
        },
        toolclad::ToolCladExecutor,
        types::AgentId,
    };

    struct Scripted {
        calls: Vec<serde_json::Value>,
        step: AtomicUsize,
        barrier: Option<Arc<tokio::sync::Barrier>>,
        pause: Option<Arc<tokio::sync::Notify>>,
    }
    #[async_trait::async_trait]
    impl InferenceProvider for Scripted {
        async fn complete(
            &self,
            _: &Conversation,
            _: &InferenceOptions,
        ) -> Result<InferenceResponse, InferenceError> {
            let step = self.step.fetch_add(1, Ordering::SeqCst);
            if step == 1 {
                if let Some(barrier) = &self.barrier {
                    barrier.wait().await;
                }
            }
            if step == self.calls.len() {
                if let Some(pause) = &self.pause {
                    pause.notify_one();
                    std::future::pending::<()>().await;
                }
            }
            let tool_calls = self
                .calls
                .get(step)
                .map(|args| {
                    vec![ToolCallRequest {
                        id: format!("terminal-call-{step}"),
                        name: "terminal_fixture.send".into(),
                        arguments: args.to_string(),
                    }]
                })
                .unwrap_or_default();
            Ok(InferenceResponse {
                content: if tool_calls.is_empty() {
                    "fixture complete".into()
                } else {
                    String::new()
                },
                finish_reason: if tool_calls.is_empty() {
                    FinishReason::Stop
                } else {
                    FinishReason::ToolCalls
                },
                tool_calls,
                usage: Usage::default(),
                model: "scripted-fixture".into(),
            })
        }
        fn provider_name(&self) -> &str {
            "scripted-fixture"
        }
        fn default_model(&self) -> &str {
            "scripted-fixture"
        }
        fn supports_native_tools(&self) -> bool {
            true
        }
        fn supports_structured_output(&self) -> bool {
            true
        }
    }
    fn provider(commands: &[&str]) -> Scripted {
        Scripted {
            calls: commands
                .iter()
                .map(|command| serde_json::json!({"command":command}))
                .collect(),
            step: AtomicUsize::new(0),
            barrier: None,
            pause: None,
        }
    }
    async fn runner(
        executor: Arc<ToolCladExecutor>,
        provider: Scripted,
    ) -> (ReasoningLoopRunner, Arc<BufferedJournal>) {
        let gate = CedarPolicyGate::deny_by_default();
        gate.add_policy(CedarPolicy {name:"terminal_fixture".into(),active:true,source:r#"
permit(principal, action == Action::"respond", resource);
permit(principal, action == Action::"tool_call::terminal_fixture.send", resource)
when { context.invocation.arguments.command == "add 1" || context.invocation.arguments.command == "background" };
"#.into()}).await;
        let journal = Arc::new(BufferedJournal::new(100));
        (
            ReasoningLoopRunner::builder()
                .provider(Arc::new(provider))
                .executor(executor)
                .policy_gate(Arc::new(gate))
                .journal(journal.clone())
                .build(),
            journal,
        )
    }
    fn tool_executor(fixture: &Fixture) -> Arc<ToolCladExecutor> {
        Arc::new(
            ToolCladExecutor::new(vec![("terminal_fixture".into(), fixture.manifest.clone())])
                .with_command_boundary(fixture.profile.clone()),
        )
    }
    fn config() -> LoopConfig {
        LoopConfig {
            max_iterations: 5,
            timeout: Duration::from_secs(15),
            tool_timeout: Duration::from_secs(8),
            ..Default::default()
        }
    }

    #[tokio::test]
    #[ignore = "requires Docker and cached fixture image; synthetic ambient canary must be set"]
    async fn concurrent_principals_keep_separate_sessions_and_reuse_state_across_iterations() {
        let fixture = Fixture::new();
        let executor = tool_executor(&fixture);
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let mut left = provider(&["add 1", "add 1"]);
        left.barrier = Some(barrier.clone());
        let mut right = provider(&["add 1", "add 1"]);
        right.barrier = Some(barrier);
        let (left, left_journal) = runner(executor.clone(), left).await;
        let (right, right_journal) = runner(executor.clone(), right).await;
        let (left, right) = tokio::join!(
            left.run(AgentId::new(), Conversation::new(), config()),
            right.run(AgentId::new(), Conversation::new(), config())
        );
        fixture.removed().await;
        let mut cookies = Vec::new();
        for (result, journal) in [(left, left_journal), (right, right_journal)] {
            assert!(
                matches!(result.termination_reason, TerminationReason::Completed),
                "{result:?}"
            );
            let results: Vec<serde_json::Value> = result
                .conversation
                .messages()
                .iter()
                .filter(|m| m.role == MessageRole::Tool)
                .map(|m| serde_json::from_str(&m.content).expect("actual correlated tool result"))
                .collect();
            assert_eq!(results.len(), 2);
            assert_eq!(results[0]["session_id"], results[1]["session_id"]);
            let first = payload(&results[0]);
            let second = payload(&results[1]);
            assert_eq!(first["value"], 1);
            assert_eq!(second["value"], 2);
            assert_eq!(first["cookie"], second["cookie"]);
            cookies.push(second["cookie"].clone());
            let entries = journal.entries().await;
            assert!(matches!(
                entries.last().unwrap().event,
                LoopEvent::Terminated {
                    reason: TerminationReason::Completed,
                    ..
                }
            ));
        }
        assert_ne!(cookies[0], cookies[1]);
        drop(executor);
        fixture.check_host();
    }

    #[tokio::test]
    #[ignore = "requires Docker and cached fixture image; synthetic ambient canary must be set"]
    async fn loop_completion_and_cancellation_between_tool_calls_remove_background_workers() {
        for cancel in [false, true] {
            let fixture = Fixture::new();
            let executor = tool_executor(&fixture);
            let mut scripted = provider(&["background"]);
            let paused = Arc::new(tokio::sync::Notify::new());
            if cancel {
                scripted.pause = Some(paused.clone());
            }
            let (runner, journal) = runner(executor.clone(), scripted).await;
            if cancel {
                let task = tokio::spawn(async move {
                    runner
                        .run(AgentId::new(), Conversation::new(), config())
                        .await
                });
                tokio::time::timeout(Duration::from_secs(8), paused.notified())
                    .await
                    .expect("loop reached next inference after real execution");
                task.abort();
                let _ = task.await;
            } else {
                let result = runner
                    .run(AgentId::new(), Conversation::new(), config())
                    .await;
                assert!(
                    matches!(result.termination_reason, TerminationReason::Completed),
                    "{result:?}"
                );
            }
            fixture.removed().await;
            assert!(journal.entries().await.iter().any(|entry| matches!(&entry.event,
                LoopEvent::ToolBatchCompleted { observations, .. } if observations.iter().any(|o| !o.is_error))));
            fixture.check_host();
        }
    }

    #[tokio::test]
    #[ignore = "requires Docker and cached fixture image; synthetic ambient canary must be set"]
    async fn outer_loop_timeout_awaits_worker_cleanup_and_records_timeout() {
        let fixture = Fixture::new();
        let executor = tool_executor(&fixture);
        let mut scripted = provider(&["background"]);
        scripted.pause = Some(Arc::new(tokio::sync::Notify::new()));
        let (runner, journal) = runner(executor.clone(), scripted).await;
        let result = runner
            .run(
                AgentId::new(),
                Conversation::new(),
                LoopConfig {
                    timeout: Duration::from_secs(3),
                    ..config()
                },
            )
            .await;
        assert!(
            matches!(result.termination_reason, TerminationReason::Timeout),
            "{result:?}"
        );
        // Inspect immediately: successful run termination must await removal.
        let workers = tokio::process::Command::new("docker")
            .args(["ps", "-aq", "--filter", &format!("label={}", fixture.label)])
            .output()
            .await
            .unwrap();
        assert!(workers.status.success());
        assert!(workers.stdout.is_empty(), "cleanup must precede return");
        let entries = journal.entries().await;
        assert!(matches!(
            entries.last().unwrap().event,
            LoopEvent::Terminated {
                reason: TerminationReason::Timeout,
                ..
            }
        ));
        drop(executor);
        fixture.check_host();
    }

    #[tokio::test]
    #[ignore = "requires Docker and cached fixture image; synthetic ambient canary must be set"]
    async fn cedar_approval_and_terminal_frame_denials_precede_worker_creation() {
        for (input, approval, extra) in [
            ("add 2", false, false),
            ("add 1", true, false),
            ("add 1\nadd 1", false, false),
            ("prefix add 1", false, false),
            ("add 1", false, true),
        ] {
            let mut fixture = Fixture::new();
            fixture
                .manifest
                .session
                .as_mut()
                .unwrap()
                .commands
                .get_mut("send")
                .unwrap()
                .human_approval = approval;
            let executor = tool_executor(&fixture);
            let mut scripted = provider(&[input]);
            if extra {
                scripted.calls[0]["extra"] = serde_json::json!("unexpected");
            }
            let (runner, _) = runner(executor, scripted).await;
            let result = runner
                .run(AgentId::new(), Conversation::new(), config())
                .await;
            fixture.removed().await;
            assert!(
                matches!(result.termination_reason, TerminationReason::Completed),
                "{result:?}"
            );
            let messages: Vec<_> = result
                .conversation
                .messages()
                .iter()
                .filter(|m| m.role == MessageRole::Tool)
                .collect();
            assert_eq!(messages.len(), 1);
            assert!(
                messages[0].content.starts_with("[Policy denied]"),
                "{}",
                messages[0].content
            );
            assert!(std::fs::read_dir(fixture.root.path().join("output"))
                .unwrap()
                .next()
                .is_none());
            fixture.check_host();
        }
    }

    #[tokio::test]
    #[ignore = "requires Docker and cached fixture image; synthetic ambient canary must be set"]
    async fn bound_operator_approval_allows_real_terminal_work_and_rejects_replay() {
        use symbi_runtime::escalation::{
            Approver, Decision, EscalationGate, EscalationGateConfig, EscalationQueue, Surface,
        };
        for allow in [true, false] {
            let mut fixture = Fixture::new();
            fixture
                .manifest
                .session
                .as_mut()
                .unwrap()
                .commands
                .get_mut("send")
                .unwrap()
                .human_approval = true;
            let gate = CedarPolicyGate::deny_by_default();
            gate.add_policy(CedarPolicy { name: "fixture".into(), active: true,
                source: "permit(principal, action == Action::\"respond\", resource); permit(principal, action == Action::\"tool_call::terminal_fixture.send\", resource) when { context.invocation.arguments.command == \"add 1\" };".into()
            }).await;
            let queue = Arc::new(EscalationQueue::new());
            let agent = AgentId::new();
            let (journal, audit) =
                symbi_runtime::reasoning::run_audit::open_run_journal(fixture.root.path(), agent)
                    .await
                    .unwrap();
            let runner = ReasoningLoopRunner::builder()
                .executor(tool_executor(&fixture))
                .provider(Arc::new(provider(&["add 1"])))
                .policy_gate(Arc::new(EscalationGate::new(
                    Arc::new(gate),
                    queue.clone(),
                    EscalationGateConfig {
                        require_approval_tools: Vec::new(),
                        timeout: Duration::from_secs(5),
                    },
                )))
                .journal(journal)
                .build();
            let (result, ()) = tokio::time::timeout(Duration::from_secs(15), async {
                tokio::join!(runner.run(agent, Conversation::new(), config()), async {
                    let held = loop {
                        if let Some(held) = queue.list_pending_async().await.into_iter().next() {
                            break held;
                        }
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    };
                    assert!(
                        fixture.workers().await.is_empty(),
                        "worker must not start before approval"
                    );
                    let operator = || Approver {
                        surface: Surface::Tui,
                        id: "fixture-operator".into(),
                        display: "fixture operator".into(),
                    };
                    queue
                        .resolve_async(
                            &held.id,
                            if allow {
                                Decision::Approve { reason: None }
                            } else {
                                Decision::Deny { reason: None }
                            },
                            operator(),
                        )
                        .await
                        .unwrap();
                    assert!(queue
                        .resolve_async(&held.id, Decision::Approve { reason: None }, operator())
                        .await
                        .is_err());
                })
            })
            .await
            .expect("approval and execution must finish within the run budget");
            fixture.removed().await;
            let key: [u8; 32] = hex::decode(audit.public_key).unwrap().try_into().unwrap();
            let entries =
                symbi_runtime::reasoning::protected_journal::ProtectedJournal::verify_run(
                    &audit.path,
                    &key,
                    audit.run_id,
                )
                .unwrap();
            assert!(entries.iter().all(|entry| entry.agent_id == agent));
            assert!(matches!(
                entries.last().unwrap().event,
                symbi_runtime::reasoning::loop_types::LoopEvent::Terminated {
                    reason: TerminationReason::Completed,
                    ..
                }
            ));
            assert!(
                matches!(result.termination_reason, TerminationReason::Completed),
                "{result:?}"
            );
            let responses: Vec<_> = result
                .conversation
                .messages()
                .iter()
                .filter(|m| m.role == MessageRole::Tool)
                .collect();
            assert_eq!(responses.len(), 1);
            if allow {
                let envelope = serde_json::from_str(&responses[0].content).unwrap();
                assert_eq!(payload(&envelope)["value"], 1);
            } else {
                assert!(responses[0].content.starts_with("[Policy denied]"));
                assert!(std::fs::read_dir(fixture.root.path().join("output"))
                    .unwrap()
                    .next()
                    .is_none());
            }
            fixture.check_host();
        }
    }
}

#[tokio::test]
#[ignore = "requires Docker and cached fixture image; synthetic ambient canary must be set"]
async fn cancellation_during_creation_awaits_removal_without_starting_terminal() {
    let mut fixture = Fixture::new();
    let marker = fixture.root.path().join("created");
    let wrapper = fixture.root.path().join("docker-wrapper");
    std::fs::write(&wrapper, format!(
        "#!/bin/sh\n/usr/bin/docker \"$@\"\nresult=$?\nif [ \"$1\" = create ]; then touch '{}'; sleep 1; fi\nexit \"$result\"\n",
        marker.display())).unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    fixture.profile.docker.docker_binary = wrapper.display().to_string();
    let executor = Arc::new(fixture.executor());
    let worker = executor.clone();
    let call = tokio::spawn(async move { command(&worker, "add 1").await });
    tokio::time::timeout(Duration::from_secs(8), async {
        while !marker.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("container created before cancellation");
    call.abort();
    let _ = call.await;
    let error = executor.cleanup_async().await.unwrap_err();
    assert!(error.contains("cancelled"), "{error}");
    let workers = tokio::process::Command::new("docker")
        .args(["ps", "-aq", "--filter", &format!("label={}", fixture.label)])
        .output()
        .await
        .unwrap();
    assert!(workers.status.success());
    assert!(
        workers.stdout.is_empty(),
        "cleanup acknowledgement must await removal"
    );
    assert!(std::fs::read_dir(fixture.root.path().join("output"))
        .unwrap()
        .next()
        .is_none());
    fixture.check_host();
}

fn file_fixture() -> Fixture {
    let mut fixture = Fixture::new();
    let output = fixture.root.path().join("output");
    std::fs::write(output.join("numbers.txt"), "2 3 5").unwrap();
    std::fs::write(output.join("adjacent.txt"), "ungranted neighbor").unwrap();
    fixture.manifest.filesystem = Some(symbi_runtime::sandbox::files::FileAccess {
        read: vec!["numbers.txt".into()],
        create: vec!["result.json".into()],
        max_file_bytes: 1024,
    });
    let commands = &mut fixture.manifest.session.as_mut().unwrap().commands;
    let mut finish = commands["send"].clone();
    finish.pattern = "finish".into();
    finish.description = "Finalize and publish the session output".into();
    finish.finalize = true;
    commands.insert("finish".into(), finish);
    fixture
}
async fn finalize(executor: &SessionExecutor) -> Result<serde_json::Value, String> {
    executor
        .execute_session_command_async("terminal_fixture.finish", r#"{"command":"finish"}"#)
        .await
}

#[tokio::test]
#[ignore = "requires Docker and cached fixture image; synthetic ambient canary must be set"]
async fn explicit_files_preserve_state_and_publish_only_after_finalization() {
    use sha2::Digest;
    let fixture = file_fixture();
    let executor = fixture.executor();
    let first = command(&executor, "add 1").await.unwrap();
    let second = command(&executor, "add 2").await.unwrap();
    let target = fixture.root.path().join("output/result.json");
    assert!(!target.exists());
    assert_eq!(first["session_id"], second["session_id"]);
    assert_eq!(payload(&first)["value"], 11);
    assert_eq!(payload(&second)["value"], 13);
    assert_eq!(second["file_publication"], "pending");
    assert_eq!(second["created_files"], serde_json::json!([]));
    assert_eq!(fixture.workers().await.len(), 1);
    let finished = finalize(&executor).await.unwrap();
    assert!(
        fixture.workers().await.is_empty(),
        "publication requires completed cleanup"
    );
    executor.cleanup_async().await.unwrap();
    assert_eq!(finished["session_id"], first["session_id"]);
    assert_eq!(finished["execution_status"], "session_finalized");
    assert_eq!(finished["file_publication"], "published");
    assert_eq!(finished["session_closed"], true);
    let bytes = std::fs::read(&target).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
        payload(&finished)
    );
    assert_eq!(
        finished["created_files"][0]["sha256"],
        format!("{:x}", sha2::Sha256::digest(&bytes))
    );
    assert_eq!(
        std::fs::read_to_string(fixture.root.path().join("output/numbers.txt")).unwrap(),
        "2 3 5"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.root.path().join("output/adjacent.txt")).unwrap(),
        "ungranted neighbor"
    );
    assert!(command(&executor, "add 1").await.is_err());
    fixture.check_host();
}

#[tokio::test]
#[ignore = "requires Docker and cached fixture image; synthetic ambient canary must be set"]
async fn unfinished_cancelled_and_exhausted_sessions_never_publish() {
    for mode in ["cleanup", "timeout", "interactions"] {
        let mut fixture = file_fixture();
        if mode == "timeout" {
            fixture.manifest.tool.timeout_seconds = 2;
        }
        if mode == "interactions" {
            fixture.manifest.session.as_mut().unwrap().max_interactions = 1;
        }
        let executor = fixture.executor();
        let first = command(&executor, "add 1").await;
        if mode == "interactions" {
            assert!(first.unwrap_err().contains("unpublished output"));
        } else {
            assert_eq!(payload(&first.unwrap())["value"], 11);
        }
        if mode == "timeout" {
            assert!(command(&executor, "hang")
                .await
                .unwrap_err()
                .contains("timed out"));
        }
        assert!(executor
            .cleanup_async()
            .await
            .unwrap_err()
            .contains("unpublished"));
        fixture.removed().await;
        assert!(!fixture.root.path().join("output/result.json").exists());
        fixture.check_host();
    }
}

#[tokio::test]
#[ignore = "requires Docker and cached fixture image; synthetic ambient canary must be set"]
async fn live_session_cannot_change_inputs_output_parent_or_overwrite_competitors() {
    for change in ["input", "parent", "competing_output"] {
        let fixture = file_fixture();
        let executor = fixture.executor();
        command(&executor, "add 1").await.unwrap();
        let output = fixture.root.path().join("output");
        match change {
            "input" => std::fs::write(output.join("numbers.txt"), "99").unwrap(),
            "parent" => {
                std::fs::rename(&output, fixture.root.path().join("old-output")).unwrap();
                std::fs::create_dir(&output).unwrap();
                std::fs::write(output.join("numbers.txt"), "2 3 5").unwrap();
            }
            "competing_output" => {
                std::fs::write(output.join("result.json"), "protected competing output").unwrap()
            }
            _ => unreachable!(),
        }
        let error = finalize(&executor).await.unwrap_err();
        assert!(
            error.contains(if change == "competing_output" {
                "already exists"
            } else {
                "file grants changed"
            }),
            "{error}"
        );
        assert_eq!(
            fixture.workers().await.len(),
            1,
            "a new file grant must not create another session"
        );
        assert!(executor
            .cleanup_async()
            .await
            .unwrap_err()
            .contains("unpublished"));
        fixture.removed().await;
        if change == "competing_output" {
            assert_eq!(
                std::fs::read_to_string(output.join("result.json")).unwrap(),
                "protected competing output"
            );
        } else {
            assert!(!output.join("result.json").exists());
        }
        assert!(!fixture.root.path().join("old-output/result.json").exists());
        fixture.check_host();
    }
}

#[tokio::test]
#[ignore = "requires Docker and cached fixture image; synthetic ambient canary must be set"]
async fn output_contract_requires_a_finalizer_before_worker_start() {
    let mut fixture = file_fixture();
    fixture
        .manifest
        .session
        .as_mut()
        .unwrap()
        .commands
        .remove("finish");
    let executor = fixture.executor();
    assert!(command(&executor, "add 1")
        .await
        .unwrap_err()
        .contains("finalizing command"));
    assert!(fixture.workers().await.is_empty());
    executor.cleanup_async().await.unwrap();
}

#[tokio::test]
async fn direct_sdk_requires_explicit_required_file_arguments() {
    let manifest: Manifest = toml::from_str(
        r#"
[tool]
name = "terminal"
version = "1"
mode = "session"
description = "Required file argument"
[session]
startup_command = "/missing/terminal"
ready_pattern = "READY>"
[session.commands.send]
pattern = "read"
description = "Read a required input"
[session.commands.send.args.input]
type = "path"
position = 1
required = true
default = "input.txt"
[filesystem]
read = ["{input}"]
[output]
format = "text"
"#,
    )
    .unwrap();
    let executor = SessionExecutor::new(vec![("terminal".into(), manifest)]);
    let error = executor
        .execute_session_command_async("terminal.send", r#"{"command":"read"}"#)
        .await
        .unwrap_err();
    assert_eq!(error, "missing required argument: input");
}
