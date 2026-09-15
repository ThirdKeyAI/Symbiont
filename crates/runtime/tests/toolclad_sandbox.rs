//! Real Docker integration through ToolClad's public execution surface.
//! Requires the locally cached python:3.12-slim image; never pulls an image.
#![cfg(unix)]
use std::{path::Path, time::Duration};
use symbi_runtime::{
    sandbox::command::CommandBoundary,
    toolclad::{Manifest, ToolCladExecutor},
};

fn manifest(command: &str) -> Manifest {
    let mut manifest: Manifest = toml::from_str(
        r#"
[tool]
name = "fixture"
version = "1"
binary = "/usr/local/bin/python3"
description = "Container execution fixture"
timeout_seconds = 15
[command]
template = "placeholder"
[output]
format = "json"
"#,
    )
    .unwrap();
    manifest.command.template = Some(command.into());
    manifest
}

fn mount(profile: &mut CommandBoundary, host: &Path, destination: &str, writable: bool) {
    profile.docker.volumes.push(format!(
        "{}:{destination}:{}",
        host.display(),
        if writable { "rw" } else { "ro" }
    ));
}

#[test]
#[ignore = "requires Docker and cached python:3.12-slim"]
fn default_toolclad_executes_in_container_and_preserves_literal_arguments() {
    let root = tempfile::tempdir().unwrap();
    let canary = root.path().join("host-canary");
    std::fs::write(&canary, "synthetic canary").unwrap();
    let command = format!(
        r#"/usr/local/bin/python3 -c 'import json,os,pathlib,sys; print(json.dumps(dict(uid=os.getuid(),host_visible=pathlib.Path(sys.argv[1]).exists(),value=sys.argv[2])))' '{}' '{{message}}'"#,
        canary.display()
    );
    let mut fixture = manifest(&command);
    fixture.args.insert(
        "message".into(),
        symbi_runtime::toolclad::manifest::ArgDef {
            type_name: "string".into(),
            required: true,
            ..Default::default()
        },
    );
    let executor = ToolCladExecutor::new(vec![("fixture".into(), fixture)]);
    let message = "quote'  --output /tmp/not-authorized  'tail\\value";
    let result = executor
        .execute_tool(
            "fixture",
            &serde_json::json!({"message":message}).to_string(),
        )
        .unwrap();
    assert_eq!(result["status"], "success", "{result}");
    assert_eq!(result["results"]["uid"], 65534);
    assert_eq!(result["results"]["host_visible"], false);
    assert_eq!(result["results"]["value"], message);
    assert_eq!(std::fs::read_to_string(canary).unwrap(), "synthetic canary");
}

#[test]
#[serial_test::serial]
#[ignore = "requires Docker and cached python:3.12-slim"]
fn custom_parser_receives_bounded_output_without_host_mounts() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("output");
    std::fs::create_dir(&output).unwrap();
    std::fs::write(output.join("adjacent"), "host only").unwrap();
    let parser_code = r#"import json, os, pathlib
assert not pathlib.Path("/workspace/adjacent").exists()
assert os.getuid() == 65534
pathlib.Path("/workspace/scratch").write_text("private scratch")
print(json.dumps(dict(length=len(pathlib.Path(__file__).read_bytes()), uid=os.getuid())))
"#;
    let command = format!(
        "/usr/local/bin/python3 -c 'import sys; sys.stdout.write(\"#\" + \"x\" * (2 * 1024 * 1024) + chr(10) + {})'",
        serde_json::to_string(parser_code).unwrap()
    );
    let mut profile = CommandBoundary::default();
    mount(&mut profile, &output, "/workspace", true);
    let mut fixture = manifest(&command);
    fixture.output.parser = Some("custom:/usr/local/bin/python3".into());
    let executor =
        ToolCladExecutor::new(vec![("fixture".into(), fixture)]).with_command_boundary(profile);
    let previous = std::env::var_os("SYMBIONT_TOOLCLAD_ALLOWED_PARSERS");
    std::env::set_var(
        "SYMBIONT_TOOLCLAD_ALLOWED_PARSERS",
        "/usr/local/bin/python3",
    );
    let result = executor.execute_tool("fixture", "{}");
    match previous {
        Some(value) => std::env::set_var("SYMBIONT_TOOLCLAD_ALLOWED_PARSERS", value),
        None => std::env::remove_var("SYMBIONT_TOOLCLAD_ALLOWED_PARSERS"),
    }
    let result = result.unwrap();
    assert_eq!(result["status"], "success", "{result}");
    assert_eq!(
        result["results"]["length"],
        2 * 1024 * 1024 + 2 + parser_code.len()
    );
    assert!(!output.join("scratch").exists());
    assert_eq!(
        std::fs::read_to_string(output.join("adjacent")).unwrap(),
        "host only"
    );
}

#[tokio::test]
#[ignore = "requires Docker and cached python:3.12-slim"]
async fn parser_cancellation_removes_worker_with_detached_descendant() {
    let mut profile = CommandBoundary::default();
    let label = format!("symbi.parser-test={}", uuid::Uuid::new_v4());
    profile.docker.extra_flags.push(format!("--label={label}"));
    let task = tokio::spawn(async move {
        profile
            .parse(
                "/usr/local/bin/python3",
                r#"import os, pathlib, time
if os.fork() == 0:
    os.setsid()
    pathlib.Path("/workspace/started").touch()
    time.sleep(30)
else:
    time.sleep(60)
"#,
                Duration::from_secs(60),
            )
            .await
    });
    let started = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let result = tokio::process::Command::new("docker")
                .args(["ps", "-q", "--filter", &format!("label={label}")])
                .output()
                .await
                .unwrap();
            assert!(result.status.success());
            let ids = String::from_utf8_lossy(&result.stdout);
            if let Some(id) = ids.lines().next() {
                let probe = tokio::process::Command::new("docker")
                    .args(["exec", id, "test", "-f", "/workspace/started"])
                    .output()
                    .await
                    .unwrap();
                if probe.status.success() {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    task.abort();
    let _ = task.await;
    let removed = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let result = tokio::process::Command::new("docker")
                .args(["ps", "-aq", "--filter", &format!("label={label}")])
                .output()
                .await
                .unwrap();
            assert!(result.status.success());
            if result.stdout.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    assert!(
        started.is_ok(),
        "parser never reached its descendant checkpoint"
    );
    assert!(removed.is_ok(), "cancelled parser container remains");
}
