//! Real contained MCP transport tests; protocol-only host fixtures live separately.
#![cfg(all(unix, feature = "mcp-client"))]
use std::{collections::HashMap, os::unix::fs::PermissionsExt, time::Duration};
use symbi_runtime::{
    integrations::mcp::{registry::StdioServerSpec, stdio_client::RmcpStdioClient},
    sandbox::{
        command::CommandBoundary,
        files::{FileAccess, FileAccessPlan},
    },
};

const SERVER: &str = r#"
import json, os, pathlib, sys
root = pathlib.Path('/workspace')
def event(kind):
    with (root / 'events').open('a') as output:
        output.write(kind + ':' + str(os.getpid()) + '\n')
event('start')
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request: continue
    method = request['method']
    if method == 'initialize':
        result = {'protocolVersion':request['params']['protocolVersion'], 'capabilities':{'tools':{}}, 'serverInfo':{'name':'fixture','version':'1'}}
    elif method == 'tools/list':
        event('list')
        result = {'tools':[{'name':'echo','description':json.dumps({'input_visible':(root/'input').exists(), 'adjacent_visible':(root/'adjacent').exists()}),'inputSchema':{'type':'object','properties':{'text':{'type':'string'}}}}]}
    elif method == 'tools/call':
        event('call')
        result = {'content':[{'type':'text','text':json.dumps({'text':request['params']['arguments']['text'], 'uid':os.getuid(), 'explicit':os.getenv('EXPLICIT_FIXTURE'), 'host_visible':pathlib.Path(os.environ['HOST_CANARY']).exists(), 'input':(root/'input').read_text() if (root/'input').exists() else None, 'adjacent_visible':(root/'adjacent').exists(), 'events':(root/'events').read_text()})}]}
    else: result = {}
    print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':result}), flush=True)
"#;

fn fixture() -> (tempfile::TempDir, StdioServerSpec, CommandBoundary, String) {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("output");
    std::fs::create_dir(&output).unwrap();
    std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o777)).unwrap();
    std::fs::write(output.join("input"), "authorized snapshot").unwrap();
    std::fs::write(output.join("adjacent"), "ungranted neighbor").unwrap();
    let canary = directory.path().join("canary");
    std::fs::write(&canary, "synthetic host value").unwrap();
    let spec = StdioServerSpec {
        command: "/usr/local/bin/python3".into(),
        args: vec!["-u".into(), "-c".into(), SERVER.into()],
        env: HashMap::from([
            ("EXPLICIT_FIXTURE".into(), "provided".into()),
            ("HOST_CANARY".into(), canary.display().to_string()),
        ]),
        ..Default::default()
    };
    let mut profile = CommandBoundary::default();
    profile
        .docker
        .volumes
        .push(format!("{}:/workspace:rw", output.display()));
    let label = format!("symbi.mcp-test={}", uuid::Uuid::new_v4());
    profile.docker.extra_flags.push(format!("--label={label}"));
    (directory, spec, profile, label)
}

async fn assert_removed(label: &str) {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let output = tokio::process::Command::new("docker")
                .args(["ps", "-aq", "--filter", &format!("label={label}")])
                .output()
                .await
                .unwrap();
            assert!(output.status.success());
            if output.stdout.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("MCP container must be removed");
}

#[tokio::test]
#[ignore = "requires Docker and cached python:3.12-slim"]
async fn container_preserves_live_protocol_allowed_environment_and_denies_host_file() {
    let (directory, spec, profile, label) = fixture();
    let files = FileAccessPlan::prepare(
        &profile,
        Some(&FileAccess {
            read: vec!["input".into()],
            ..Default::default()
        }),
        &HashMap::new(),
    )
    .unwrap();
    std::fs::write(
        directory.path().join("output/input"),
        "changed after preparation",
    )
    .unwrap();
    let result = RmcpStdioClient::verified_invoke_with_files(
        &spec,
        "echo",
        serde_json::Map::from_iter([("text".into(), serde_json::json!("round trip"))]),
        false,
        Duration::from_secs(10),
        &profile,
        &files,
    )
    .await;
    assert_removed(&label).await;
    let result = result.unwrap();
    let payload: serde_json::Value =
        serde_json::from_str(result.content[0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(payload["text"], "round trip");
    assert_eq!(payload["uid"], 65534);
    assert_eq!(payload["explicit"], "provided");
    assert_eq!(payload["host_visible"], false);
    assert_eq!(payload["input"], "authorized snapshot");
    assert_eq!(payload["adjacent_visible"], false);
    assert_eq!(result.created_files, serde_json::json!([]));
    assert!(!directory.path().join("output/events").exists());
    let events = payload["events"].as_str().unwrap();
    let entries: Vec<_> = events
        .lines()
        .map(|line| line.split_once(':').unwrap())
        .collect();
    assert_eq!(
        entries.iter().map(|v| v.0).collect::<Vec<_>>(),
        ["start", "list", "call"]
    );
    assert!(entries.iter().all(|v| v.1 == entries[0].1), "{events}");
    assert_eq!(
        std::fs::read_to_string(directory.path().join("canary")).unwrap(),
        "synthetic host value"
    );
}

#[tokio::test]
#[ignore = "requires Docker and cached python:3.12-slim"]
async fn container_limits_protocol_streams_before_decoding() {
    for stream in ["stdout", "stderr"] {
        let (_directory, mut spec, mut profile, label) = fixture();
        profile.docker.max_output_bytes = 16 * 1024;
        spec.args[2] = format!("import sys,time; sys.{stream}.write('x' * 65536); sys.{stream}.flush(); time.sleep(60)");
        let started = std::time::Instant::now();
        let result = RmcpStdioClient::verified_invoke_with_boundary(
            &spec,
            "echo",
            Default::default(),
            false,
            Duration::from_secs(15),
            &profile,
        )
        .await;
        assert_removed(&label).await;
        let error = result.unwrap_err();
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{stream}: {error}"
        );
        if stream == "stderr" {
            assert!(error.contains("output limit"), "{error}");
        }
    }
}

#[tokio::test]
#[ignore = "requires Docker and cached python:3.12-slim"]
async fn cancellation_and_container_deadline_stop_detached_mcp_descendants() {
    for cancel in [true, false] {
        let (directory, mut spec, mut profile, label) = fixture();
        if !cancel {
            profile.docker.max_execution_time = Duration::from_secs(2);
        }
        spec.args[2] = "import os,pathlib,time; child=os.fork(); os.setsid() if child == 0 else None; pathlib.Path('/workspace/started').touch() if child else None; time.sleep(60); pathlib.Path('/workspace/late-effect').touch()".into();
        let job = tokio::spawn(async move {
            RmcpStdioClient::verified_invoke_with_boundary(
                &spec,
                "echo",
                Default::default(),
                false,
                Duration::from_secs(60),
                &profile,
            )
            .await
        });
        // Observe the actual parent and detached Python child through Docker.
        // No host directory is granted to the worker for readiness reporting.
        let mut last_observation = String::new();
        let ready = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let output = tokio::process::Command::new("docker")
                    .args(["ps", "-q", "--filter", &format!("label={label}")])
                    .output()
                    .await
                    .unwrap();
                assert!(output.status.success());
                last_observation = format!("ps: {}", String::from_utf8_lossy(&output.stdout));
                for id in String::from_utf8_lossy(&output.stdout).split_whitespace() {
                    let top = tokio::process::Command::new("docker")
                        .args(["top", id, "-eo", "pid,ppid,comm"])
                        .output()
                        .await
                        .unwrap();
                    last_observation.push_str(&format!(
                        " top: {} {}",
                        String::from_utf8_lossy(&top.stdout),
                        String::from_utf8_lossy(&top.stderr)
                    ));
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
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await;
        if cancel {
            job.abort();
        }
        let outcome = tokio::time::timeout(Duration::from_secs(10), job).await;
        assert_removed(&label).await;
        assert!(
            ready.is_ok(),
            "worker did not start (cancel={cancel}): {outcome:?}; {last_observation}"
        );
        let outcome = outcome.expect("container lifetime is independent of the longer MCP budget");
        if cancel {
            assert!(outcome.unwrap_err().is_cancelled());
        } else {
            assert!(outcome.unwrap().is_err());
        }
        assert!(!directory.path().join("output/late-effect").exists());
    }
}

#[tokio::test]
#[ignore = "requires Docker and cached python:3.12-slim"]
async fn cancellation_while_creating_never_starts_the_mcp_payload() {
    let (directory, spec, mut profile, label) = fixture();
    let marker = directory.path().join("creating");
    let wrapper = directory.path().join("docker-wrapper");
    std::fs::write(&wrapper, format!("#!/bin/sh\nif [ \"$1\" = create ]; then touch '{}'; sleep 1; fi\nexec /usr/bin/docker \"$@\"\n",marker.display())).unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    profile.docker.docker_binary = wrapper.display().to_string();
    let job = tokio::spawn(async move {
        RmcpStdioClient::verified_invoke_with_boundary(
            &spec,
            "echo",
            Default::default(),
            false,
            Duration::from_secs(15),
            &profile,
        )
        .await
    });
    let ready = tokio::time::timeout(Duration::from_secs(10), async {
        while !marker.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    job.abort();
    let _ = job.await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_removed(&label).await;
    assert!(ready.is_ok());
    assert!(
        !directory.path().join("output/events").exists(),
        "cancelled creation must not run the payload"
    );
}

#[tokio::test]
#[ignore = "requires Docker and cached python:3.12-slim"]
async fn default_sdk_call_requires_a_signed_schema() {
    let (directory, spec, _profile, _label) = fixture();
    let result = RmcpStdioClient::call_tool(
        &spec,
        "echo",
        serde_json::Map::from_iter([("text".into(), serde_json::json!("round trip"))]),
    )
    .await;
    assert!(result.unwrap_err().contains("SchemaPin-verified"));
    assert!(
        !directory.path().join("output/events").exists(),
        "default SDK must not mount the host fixture directory"
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("canary")).unwrap(),
        "synthetic host value"
    );
}

#[tokio::test]
#[ignore = "requires Docker and cached python:3.12-slim"]
async fn sdk_discovery_and_calls_receive_no_implicit_host_mounts() {
    let (directory, spec, profile, label) = fixture();
    let tools = RmcpStdioClient::list_tools_with_boundary(&spec, &profile)
        .await
        .unwrap();
    let visibility: serde_json::Value =
        serde_json::from_str(tools[0].description.as_deref().unwrap()).unwrap();
    assert_eq!(
        visibility,
        serde_json::json!({"input_visible": false, "adjacent_visible": false})
    );
    let result = RmcpStdioClient::verified_invoke_with_boundary(
        &spec,
        "echo",
        serde_json::Map::from_iter([("text".into(), serde_json::json!("useful scratch work"))]),
        false,
        Duration::from_secs(10),
        &profile,
    )
    .await
    .unwrap();
    let payload: serde_json::Value =
        serde_json::from_str(result[0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(payload["text"], "useful scratch work");
    assert_eq!(payload["input"], serde_json::Value::Null);
    assert_eq!(payload["adjacent_visible"], false);
    assert!(!directory.path().join("output/events").exists());
    assert_removed(&label).await;
}

#[tokio::test]
#[ignore = "requires Docker and cached python:3.12-slim"]
async fn sdk_verification_failure_does_not_publish_startup_writes() {
    let (directory, spec, profile, label) = fixture();
    let files = FileAccessPlan::prepare(
        &profile,
        Some(&FileAccess {
            read: vec!["input".into()],
            ..Default::default()
        }),
        &HashMap::new(),
    )
    .unwrap();
    let result = RmcpStdioClient::verified_invoke_with_files(
        &spec,
        "echo",
        Default::default(),
        true,
        Duration::from_secs(10),
        &profile,
        &files,
    )
    .await;
    assert!(result.err().unwrap().contains("SchemaPin-verified"));
    assert_removed(&label).await;
    assert!(!directory.path().join("output/events").exists());
}

#[tokio::test]
#[ignore = "requires Docker and cached python:3.12-slim"]
async fn toolclad_sdk_helper_enforces_registered_file_grants() {
    use symbi_runtime::{
        integrations::mcp::registry::McpServerRegistry,
        toolclad::{manifest::Manifest, ToolCladExecutor},
    };
    let (directory, spec, profile, label) = fixture();
    let manifest: Manifest = toml::from_str(
        r#"
[tool]
name = "echo"
version = "1"
description = "Explicit file grants"
[args.text]
position = 1
required = true
type = "string"
[output]
format = "json"
[mcp]
server = "fixture"
tool = "echo"
[filesystem]
read = ["input"]
"#,
    )
    .unwrap();
    let registry = McpServerRegistry::from_toml_str(&format!(
        "[servers.fixture]\ncommand={}\nargs={}\n[servers.fixture.env]\nEXPLICIT_FIXTURE=\"provided\"\nHOST_CANARY={}\n",
        serde_json::to_string(&spec.command).unwrap(),
        serde_json::to_string(&spec.args).unwrap(),
        serde_json::to_string(&spec.env["HOST_CANARY"]).unwrap(),
    )).unwrap();
    let executor = ToolCladExecutor::new(vec![("echo".into(), manifest.clone())])
        .with_command_boundary(profile)
        .with_mcp_verification(false);
    let envelope = executor
        .execute_mcp_backend_async_with_registry(
            &registry,
            "echo",
            &manifest,
            &HashMap::from([("text".into(), "helper round trip".into())]),
        )
        .await
        .unwrap();
    assert_removed(&label).await;
    let value: serde_json::Value =
        serde_json::from_str(envelope["results"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(value["text"], "helper round trip");
    assert_eq!(value["input"], "authorized snapshot");
    assert_eq!(value["adjacent_visible"], false);
    assert_eq!(envelope["created_files"], serde_json::json!([]));
    assert!(!directory.path().join("output/events").exists());
}

#[tokio::test]
async fn sdk_output_without_dispatch_authority_is_refused_before_transport() {
    let (directory, mut spec, profile, _) = fixture();
    spec.command = "/nonexistent-publication-fixture".into();
    let files = FileAccessPlan::prepare(
        &profile,
        Some(&FileAccess {
            create: vec!["events".into()],
            ..Default::default()
        }),
        &HashMap::new(),
    )
    .unwrap();
    let error = RmcpStdioClient::verified_invoke_with_files(
        &spec,
        "echo",
        Default::default(),
        true,
        Duration::from_secs(1),
        &profile,
        &files,
    )
    .await
    .err()
    .expect("standalone SDK output must be refused before transport");
    assert_eq!(
        error,
        "file publication requires an active audited dispatcher"
    );
    assert!(!directory.path().join("output/events").exists());
}
