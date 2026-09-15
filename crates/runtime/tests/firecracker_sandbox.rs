//! Actual KVM execution through the public runner, selected command boundary and
//! ToolClad. Explicit runs require locally provisioned artifacts; nothing is downloaded.
#![cfg(target_os = "linux")]
use std::{collections::HashMap, path::PathBuf, time::Duration};
use symbi_runtime::{
    sandbox::{
        command::{CommandBoundary, CommandTier},
        supervisor::SupervisorConfig,
        FirecrackerConfig, FirecrackerRunner, SandboxRunner,
    },
    toolclad::{Manifest, ToolCladExecutor},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Fixture {
    root: tempfile::TempDir,
    config: FirecrackerConfig,
}

#[cfg(feature = "cli-executor")]
#[path = "firecracker_sandbox/cli.rs"]
mod cli;

#[cfg(feature = "toolclad-session")]
mod terminal {
    use super::*;
    use symbi_runtime::toolclad::session_executor::SessionExecutor;

    fn manifest(fixture: &Fixture) -> Manifest {
        let canary = fixture.root.path().join("host-canary");
        std::fs::write(&canary, "synthetic-host-only").unwrap();
        let mut manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "terminal_fixture"
mode = "session"
version = "1"
description = "Actual guest terminal"
timeout_seconds = 3
[session]
startup_command = "placeholder"
ready_pattern = "READY>"
startup_timeout_seconds = 3
idle_timeout_seconds = 10
session_timeout_seconds = 10
max_interactions = 10
[session.interaction]
output_wait_ms = 500
output_max_bytes = 65536
[session.commands.send]
pattern = "(?:add [1-9]|echo .+|hang|flood|stderr|background|exit)"
description = "Bounded fixture operation"
[output]
format = "text"
"#,
        )
        .unwrap();
        manifest.session.as_mut().unwrap().startup_command =
            format!("/bin/pty_fixture '{}'", canary.display());
        manifest
    }
    fn make_executor(fixture: &Fixture, manifest: Manifest) -> SessionExecutor {
        SessionExecutor::new(vec![("terminal_fixture".into(), manifest)])
            .with_command_boundary(fixture.boundary())
    }
    async fn call(executor: &SessionExecutor, command: &str) -> Result<serde_json::Value, String> {
        executor
            .execute_session_command_async(
                "terminal_fixture.send",
                &serde_json::json!({"command":command}).to_string(),
            )
            .await
    }
    fn payload(result: &serde_json::Value) -> serde_json::Value {
        let text = result["results"]["output"].as_str().unwrap();
        assert!(
            text.contains("terminal-stderr"),
            "PTY must merge stderr into its output"
        );
        text.lines()
            .find_map(|line| serde_json::from_str(line).ok())
            .expect("guest JSON response")
    }
    fn canary_intact(fixture: &Fixture) {
        assert_eq!(
            std::fs::read_to_string(fixture.root.path().join("host-canary")).unwrap(),
            "synthetic-host-only"
        );
    }

    #[tokio::test]
    #[ignore = "requires KVM and a matching rootfs containing pty_fixture"]
    async fn controlling_pty_preserves_state_effects_and_long_lines() {
        let fixture = Fixture::new();
        let executor = make_executor(&fixture, manifest(&fixture));
        let first = call(&executor, "add 1").await.unwrap();
        let second = call(&executor, "add 2").await.unwrap();
        assert_eq!(first["session_id"], second["session_id"]);
        assert_eq!(payload(&first)["value"], 1);
        let result = payload(&second);
        assert_eq!(result["value"], 3);
        assert!(result["proof"]
            .as_object()
            .unwrap()
            .values()
            .all(|v| v == true));
        let exact = "字x".repeat(4096);
        let result = call(&executor, &format!("echo {exact}")).await.unwrap();
        assert_eq!(payload(&result)["echo"], exact);
        assert_eq!(result["execution_status"], "prompt_observed");
        assert!(result["exit_code"].is_null());
        call(&executor, "background").await.unwrap();
        executor.cleanup_async().await.unwrap();
        assert!(
            fixture.leases().is_empty(),
            "cleanup must acknowledge removal before returning"
        );
        canary_intact(&fixture);
    }

    #[tokio::test]
    #[ignore = "requires KVM and a matching rootfs containing pty_fixture"]
    async fn deadlines_stream_failures_and_exit_close_the_guest_session() {
        for operation in ["hang", "flood", "stderr", "exit"] {
            let mut fixture = Fixture::new();
            fixture.config.max_output_bytes = 8192;
            let executor = make_executor(&fixture, manifest(&fixture));
            call(&executor, "add 1").await.unwrap();
            let error = call(&executor, operation).await.unwrap_err();
            // The failed interaction remains an error, but acknowledged VM
            // removal is successful cleanup. A closed session never restarts.
            executor.cleanup_async().await.unwrap();
            assert!(fixture.leases().is_empty(), "{operation}: {error}");
            assert!(call(&executor, "add 1").await.is_err());
            canary_intact(&fixture);
        }
    }

    #[tokio::test]
    #[ignore = "requires KVM and a matching rootfs containing pty_fixture"]
    async fn idle_lifetime_and_interaction_limits_prevent_session_restart() {
        for mode in ["idle", "vm_lifetime", "session_lifetime", "interactions"] {
            let mut fixture = Fixture::new();
            let mut definition = manifest(&fixture);
            let session = definition.session.as_mut().unwrap();
            match mode {
                "idle" => session.idle_timeout_seconds = 1,
                "vm_lifetime" => fixture.config.max_execution_time = Duration::from_secs(2),
                "session_lifetime" => session.session_timeout_seconds = 2,
                _ => session.max_interactions = 1,
            }
            let executor = make_executor(&fixture, definition);
            call(&executor, "add 1").await.unwrap();
            fixture.settled().await;
            assert!(call(&executor, "add 1").await.is_err(), "{mode}");
            let _ = executor.cleanup_async().await;
            canary_intact(&fixture);
        }
    }

    #[tokio::test]
    #[ignore = "requires KVM and a matching rootfs containing pty_fixture"]
    async fn startup_expiry_and_owner_drop_remove_terminal_workers() {
        let fixture = Fixture::new();
        let mut definition = manifest(&fixture);
        definition.session.as_mut().unwrap().startup_command = "/bin/sleep 30".into();
        definition.session.as_mut().unwrap().startup_timeout_seconds = 1;
        let executor = make_executor(&fixture, definition);
        assert!(call(&executor, "add 1").await.is_err());
        let _ = executor.cleanup_async().await;
        assert!(fixture.leases().is_empty());
        canary_intact(&fixture);

        let fixture = Fixture::new();
        let worker = std::sync::Arc::new(make_executor(&fixture, manifest(&fixture)));
        call(&worker, "background").await.unwrap();
        let owner = worker.clone();
        let task = tokio::spawn(async move { call(&owner, "hang").await });
        tokio::time::sleep(Duration::from_millis(100)).await;
        task.abort();
        let _ = task.await;
        drop(worker);
        fixture.settled().await;
        canary_intact(&fixture);
    }
}

#[tokio::test]
#[ignore = "requires KVM, Firecracker, matching guest rootfs and kernel"]
async fn stdio_streams_before_eof_and_preserves_large_binary_data() {
    let fixture = Fixture::new();
    let exact = "  explicit\n🌍 ";
    let mut worker = fixture
        .runner()
        .spawn_stdio(
            &[
                "/bin/sh".into(),
                "-c".into(),
                "printf '%s' \"$EXACT\" >&2; exec /bin/cat".into(),
            ],
            HashMap::from([("EXACT".into(), exact.into())]),
            Duration::from_secs(10),
        )
        .await
        .unwrap();
    let first = b"live before EOF\0\xff\n";
    worker.stdin.write_all(first).await.unwrap();
    let mut echoed = vec![0; first.len()];
    tokio::time::timeout(
        Duration::from_secs(3),
        worker.stdout.read_exact(&mut echoed),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(echoed, first);
    let input = [0, 255, b' ', b'\n', b'\'', b'$'].repeat(350_000);
    let mut output = Vec::new();
    let mut errors = Vec::new();
    let (written, read, error_read) = tokio::join!(
        async {
            worker.stdin.write_all(&input).await?;
            worker.stdin.shutdown().await
        },
        worker.stdout.read_to_end(&mut output),
        worker.stderr.read_to_end(&mut errors),
    );
    written.unwrap();
    read.unwrap();
    error_read.unwrap();
    worker.guard.finish().await.unwrap();
    assert_eq!(output, input);
    assert_eq!(errors, exact.as_bytes());
    fixture.settled().await;
}

#[tokio::test]
#[ignore = "requires KVM, Firecracker, matching guest rootfs and kernel"]
async fn stdio_input_output_failures_and_exit_status_are_retained() {
    let mut fixture = Fixture::new();
    fixture.config.max_output_bytes = 4096;
    for code in [
        "printf failure >&2; exit 17",
        "yes output",
        "yes error >&2",
        "cat >/dev/null",
    ] {
        let mut worker = fixture
            .runner()
            .spawn_stdio(
                &["/bin/sh".into(), "-c".into(), code.into()],
                HashMap::new(),
                Duration::from_secs(10),
            )
            .await
            .unwrap();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let (write, _, _) = tokio::join!(
            async {
                if code == "cat >/dev/null" {
                    worker
                        .stdin
                        .write_all(&vec![b'x'; 11 * 1024 * 1024])
                        .await?;
                }
                worker.stdin.shutdown().await
            },
            worker.stdout.read_to_end(&mut stdout),
            worker.stderr.read_to_end(&mut stderr),
        );
        let error = worker.guard.finish().await.unwrap_err().to_string();
        assert_eq!(worker.guard.finish().await.unwrap_err().to_string(), error);
        worker.guard.finish_cleanup().await.unwrap();
        if code == "cat >/dev/null" {
            assert!(write.is_err());
            assert!(error.contains("input limit"), "{error}");
        } else if code.contains("exit 17") {
            assert!(error.contains("exit 17"), "{error}");
            assert_eq!(stderr, b"failure");
        } else {
            assert!(
                error.contains("truncated=true") || error.contains("output limit"),
                "{error}"
            );
        }
        assert!(stdout.len() <= 4096 && stderr.len() <= 4096);
        fixture.settled().await;
    }
}

#[tokio::test]
#[ignore = "requires KVM, Firecracker, matching guest rootfs and kernel"]
async fn stdio_drop_and_backpressure_deadline_remove_descendants() {
    let mut fixture = Fixture::new();
    let worker = fixture
        .runner()
        .spawn_stdio(
            &[
                "/bin/sh".into(),
                "-c".into(),
                "setsid sleep 30 & wait".into(),
            ],
            HashMap::new(),
            Duration::from_secs(10),
        )
        .await
        .unwrap();
    assert!(!fixture.leases().is_empty());
    drop(worker);
    fixture.settled().await;
    fixture.config.max_execution_time = Duration::from_secs(2);
    let started = std::time::Instant::now();
    let mut worker = fixture
        .runner()
        .spawn_stdio(
            &[
                "/bin/sh".into(),
                "-c".into(),
                "setsid sleep 30 & yes backpressure".into(),
            ],
            HashMap::new(),
            Duration::from_secs(10),
        )
        .await
        .unwrap();
    // Never consume stdout: bounded queues must still permit owner expiry.
    fixture.settled().await;
    assert!(worker.guard.finish().await.is_err());
    worker.guard.finish_cleanup().await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(8));
}

#[cfg(feature = "mcp-client")]
#[tokio::test]
#[ignore = "requires KVM and rootfs containing the static mcp_fixture example"]
async fn mcp_verifies_signed_schema_and_invokes_in_the_same_guest() {
    use schemapin::crypto::{generate_key_pair, sign_data};
    use symbi_runtime::integrations::mcp::{
        registry::StdioServerSpec, stdio_client::RmcpStdioClient,
    };
    let fixture = Fixture::new();
    let key = generate_key_pair().unwrap();
    let mut schema = serde_json::json!({"type":"object","properties":{"text":{"type":"string"}},"required":["text"],"additionalProperties":false});
    schema["signature"] = serde_json::json!(sign_data(
        &key.private_key_pem,
        schemapin::canonicalize::canonicalize_schema(&schema).as_bytes()
    )
    .unwrap());
    let canary = fixture.root.path().join("host-secret");
    std::fs::write(&canary, "synthetic-host-only").unwrap();
    let mut spec = StdioServerSpec {
        command: "/bin/mcp_fixture".into(),
        env: HashMap::from([
            ("FIXTURE_SCHEMA".into(), schema.to_string()),
            ("HOST_CANARY".into(), canary.display().to_string()),
            ("EXPLICIT_FIXTURE".into(), "retained".into()),
        ]),
        public_key_pem: Some(key.public_key_pem),
        ..Default::default()
    };
    let exact = " \n'$(text)' 🌍 ";
    let args = serde_json::Map::from_iter([("text".into(), serde_json::json!(exact))]);
    let result = RmcpStdioClient::verified_invoke_with_boundary(
        &spec,
        "echo",
        args.clone(),
        true,
        Duration::from_secs(10),
        &fixture.boundary(),
    )
    .await
    .unwrap();
    let payload: serde_json::Value =
        serde_json::from_str(result[0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(payload["text"], exact);
    assert_eq!(payload["same_session"], true);
    assert_eq!(payload["uid"], 65534);
    assert_eq!(payload["gid"], 65534);
    assert_eq!(payload["explicit"], "retained");
    assert_eq!(payload["host_visible"], false);
    assert_eq!(payload["ambient_visible"], false);
    assert_eq!(payload["key_visible"], false);
    fixture.settled().await;
    for mode in ["unsigned", "tampered", "swapped_key"] {
        let mut bad = schema.clone();
        match mode {
            "unsigned" => {
                bad.as_object_mut().unwrap().remove("signature");
            }
            "tampered" => bad["additionalProperties"] = serde_json::json!(true),
            _ => {
                let other = generate_key_pair().unwrap();
                bad.as_object_mut().unwrap().remove("signature");
                bad["signature"] = serde_json::json!(sign_data(
                    &other.private_key_pem,
                    schemapin::canonicalize::canonicalize_schema(&bad).as_bytes()
                )
                .unwrap());
                spec.public_key_pem = Some(other.public_key_pem);
            }
        }
        spec.env.insert("FIXTURE_SCHEMA".into(), bad.to_string());
        assert!(
            RmcpStdioClient::verified_invoke_with_boundary(
                &spec,
                "echo",
                args.clone(),
                true,
                Duration::from_secs(10),
                &fixture.boundary()
            )
            .await
            .is_err(),
            "{mode}"
        );
        fixture.settled().await;
    }
    assert_eq!(
        std::fs::read_to_string(canary).unwrap(),
        "synthetic-host-only"
    );
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let required = |name: &str| PathBuf::from(std::env::var_os(name).expect(name));
        let config = FirecrackerConfig {
            kernel_image_path: required("SYMBI_FIRECRACKER_KERNEL"),
            rootfs_path: required("SYMBI_FIRECRACKER_ROOTFS"),
            firecracker_binary: required("SYMBI_FIRECRACKER_BINARY")
                .to_str()
                .unwrap()
                .into(),
            mem_mib: 256,
            max_execution_time: Duration::from_secs(12),
            startup_timeout: Duration::from_secs(5),
            supervisor: SupervisorConfig {
                binary: required("SYMBIONT_SANDBOX_SUPERVISOR"),
                state_dir: root.path().join("leases"),
                service_uid: None,
            },
            ..Default::default()
        };
        Self { root, config }
    }
    fn runner(&self) -> FirecrackerRunner {
        FirecrackerRunner::new(self.config.clone()).unwrap()
    }
    fn boundary(&self) -> CommandBoundary {
        let mut boundary = CommandBoundary::default();
        boundary.tier = CommandTier::Firecracker;
        boundary.firecracker = Some(self.config.clone());
        boundary
    }
    fn leases(&self) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(&self.config.supervisor.state_dir) else {
            return vec![];
        };
        entries
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.extension().is_some_and(|e| e == "json")
                    || p.file_name().unwrap().to_string_lossy().starts_with("vm-")
            })
            .collect()
    }
    async fn settled(&self) {
        tokio::time::timeout(Duration::from_secs(12), async {
            while !self.leases().is_empty() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("VM leases or work directories remained after cleanup");
    }
}

#[tokio::test]
#[ignore = "requires KVM, Firecracker, matching guest rootfs and kernel"]
async fn exact_argv_environment_binary_streams_and_selected_parser() {
    let fixture = Fixture::new();
    let value = "  data\n'\"; $(touch /tmp/unexpected) {value} 🌍  ";
    let result = fixture
        .runner()
        .execute_command(
            &["/bin/printf".into(), "%s".into(), value.into()],
            HashMap::new(),
            None,
            false,
            Duration::from_secs(10),
        )
        .await
        .unwrap();
    assert!(result.success, "{result:?}");
    assert_eq!(result.stdout, value);
    let result = fixture
        .runner()
        .execute(
            "printf '%s' \"$EXACT\"; printf '\\000stderr' >&2",
            HashMap::from([("EXACT".into(), value.into())]),
        )
        .await
        .unwrap();
    assert!(result.success, "{result:?}");
    assert_eq!(result.stdout, value);
    assert_eq!(result.stderr, "\0stderr");
    let input = "binary\0with unicode 🌍\n".repeat(65536);
    let result = fixture
        .runner()
        .execute_command(
            &["/bin/cat".into()],
            HashMap::new(),
            Some(input.as_bytes().to_vec()),
            false,
            Duration::from_secs(10),
        )
        .await
        .unwrap();
    assert!(result.success, "{result:?}");
    assert_eq!(result.stdout, input);
    let result = fixture
        .boundary()
        .parse("/bin/cat", &input, Duration::from_secs(10))
        .await
        .unwrap();
    assert!(result.success, "{result:?}");
    assert_eq!(result.stdout, input);
    fixture.settled().await;
}

#[tokio::test]
#[ignore = "requires KVM, Firecracker, matching guest rootfs and kernel"]
async fn nonroot_readonly_root_scratch_no_host_mounts_or_network() {
    let fixture = Fixture::new();
    let canary = fixture.root.path().join("host-canary");
    std::fs::write(&canary, "synthetic-host-only").unwrap();
    let script = format!(
        r#"set -eu
[ "$(id -u)" = 65534 ]
[ "$(id -g)" = 65534 ]
[ ! -e '{}' ]
[ ! -r /root/protected-canary ]
[ -z "${{SYMBI_FIRECRACKER_AMBIENT_CANARY+x}}" ]
[ -z "${{OPENAI_API_KEY+x}}" ]
[ "$(ls /sys/class/net)" = lo ]
! touch /bin/forbidden 2>/dev/null
printf scratch > /tmp/allowed
[ "$(cat /tmp/allowed)" = scratch ]
printf 'nonroot readonly scratch no-host no-network minimal-env'
"#,
        canary.display()
    );
    let result = fixture
        .runner()
        .execute(&script, HashMap::new())
        .await
        .unwrap();
    assert!(result.success, "{result:?}");
    assert_eq!(
        result.stdout,
        "nonroot readonly scratch no-host no-network minimal-env"
    );
    assert_eq!(
        std::fs::read_to_string(canary).unwrap(),
        "synthetic-host-only"
    );
    fixture.settled().await;
}

#[tokio::test]
#[ignore = "requires KVM, Firecracker, matching guest rootfs and kernel"]
async fn nonzero_missing_program_and_output_overflow_are_not_success() {
    let mut fixture = Fixture::new();
    let result = fixture
        .runner()
        .execute("printf failure >&2; exit 17", HashMap::new())
        .await
        .unwrap();
    assert!(!result.success);
    assert_eq!(result.exit_code, 17);
    assert_eq!(result.stderr, "failure");
    let result = fixture
        .boundary()
        .execute(&["/missing-program".into()], Duration::from_secs(10))
        .await
        .unwrap();
    assert!(!result.success);
    assert_eq!(result.exit_code, 125);
    assert!(result.stderr.contains("cannot start guest command"));
    fixture.config.max_output_bytes = 4096;
    for command in ["yes output", "yes error >&2"] {
        let result = fixture
            .runner()
            .execute(command, HashMap::new())
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.stdout_truncated || result.stderr_truncated);
        assert!(result.stdout.len() <= 4096 && result.stderr.len() <= 4096);
    }
    fixture.settled().await;
}

#[tokio::test]
#[ignore = "requires KVM, Firecracker, matching guest rootfs and kernel"]
async fn detached_descendant_output_closes_and_whole_vm_deadline_is_bounded() {
    let fixture = Fixture::new();
    let result = fixture.runner().execute(
        "setsid /bin/sh -c 'printf started > /tmp/started; sleep 30' & while [ ! -f /tmp/started ]; do sleep 0.01; done; cat /tmp/started",
        HashMap::new()).await.unwrap();
    assert!(result.success, "{result:?}");
    assert_eq!(result.stdout, "started");
    let start = std::time::Instant::now();
    let result = fixture
        .runner()
        .execute_command(
            &[
                "/bin/sh".into(),
                "-c".into(),
                "setsid sleep 30 & wait".into(),
            ],
            HashMap::new(),
            None,
            false,
            Duration::from_secs(2),
        )
        .await;
    assert!(result.as_ref().map_or(true, |r| !r.success), "{result:?}");
    fixture.settled().await;
    assert!(start.elapsed() < Duration::from_secs(10));
}

#[tokio::test]
#[ignore = "requires KVM, Firecracker, matching guest rootfs and kernel"]
async fn dropping_execution_future_removes_owned_vm() {
    let fixture = Fixture::new();
    let runner = fixture.runner();
    let task = tokio::spawn(async move {
        runner
            .execute("setsid sleep 30 & wait", HashMap::new())
            .await
    });
    let pid = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            for path in fixture.leases() {
                if path.extension().is_some_and(|e| e == "json") {
                    if let Ok(data) = std::fs::read(&path) {
                        let value: serde_json::Value = serde_json::from_slice(&data).unwrap();
                        if let Some(pid) = value["state"]["pid"].as_u64() {
                            return pid;
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    task.abort();
    let _ = task.await;
    fixture.settled().await;
    let pid = pid.expect("VMM never reached durable ownership");
    assert!(
        !PathBuf::from(format!("/proc/{pid}")).exists(),
        "VMM survived cancellation"
    );
}

#[tokio::test]
#[ignore = "requires KVM, Firecracker, matching guest rootfs and kernel"]
async fn missing_guest_init_and_stale_guest_cannot_report_success() {
    let mut fixture = Fixture::new();
    fixture.config.boot_args =
        "console=ttyS0 reboot=k panic=1 pci=off ro init=/missing-init".into();
    let result = fixture.runner().execute("true", HashMap::new()).await;
    assert!(result.is_err(), "{result:?}");
    fixture.settled().await;
    fixture.config.boot_args = FirecrackerConfig::default().boot_args;
    fixture.config.rootfs_path = std::env::var_os("SYMBI_FIRECRACKER_STALE_ROOTFS")
        .expect("stale test rootfs required")
        .into();
    let result = fixture.runner().execute("true", HashMap::new()).await;
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("protocol mismatch"));
    fixture.settled().await;
}

#[test]
#[ignore = "requires KVM, Firecracker, matching guest rootfs and kernel"]
fn toolclad_command_uses_selected_vm() {
    let fixture = Fixture::new();
    let manifest: Manifest = toml::from_str(
        r#"
[tool]
name = "fixture"
version = "1"
binary = "/bin/printf"
description = "Guest output fixture"
timeout_seconds = 10
[command]
template = "/bin/printf '%s' '{message}'"
[args.message]
position = 1
type = "literal_text"
required = true
[output]
format = "text"
"#,
    )
    .unwrap();
    let executor = ToolCladExecutor::new(vec![("fixture".into(), manifest)])
        .with_command_boundary(fixture.boundary());
    let value = "  exact '$data'\n{template} 🌍  ";
    let result = executor
        .execute_tool("fixture", &serde_json::json!({"message":value}).to_string())
        .unwrap();
    assert_eq!(result["status"], "success", "{result}");
    assert_eq!(result["results"]["raw_output"], value, "{result}");
    assert!(fixture.leases().is_empty());
}
