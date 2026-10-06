//! `symbi init` must not require a terminal.
//!
//! The unit test beside `should_skip_prompts` only exercises the pure helper,
//! so it stays green if the wiring in `run` is reverted or the prompts go back
//! to `.expect(...)`. This drives the real binary with stdin closed — the
//! exact shape of a Dockerfile, a CI step, or a piped shell — which is the
//! only way the panic can actually be caught.

use std::process::{Command, Stdio};

#[test]
fn init_succeeds_without_a_terminal() {
    let dir = tempfile::tempdir().expect("tempdir");

    let out = Command::new(env!("CARGO_BIN_EXE_symbi"))
        .current_dir(dir.path())
        .arg("init")
        .arg("--dir")
        .arg(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn symbi init");

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("panicked"),
        "init panicked without a TTY:\n{stderr}"
    );
    assert!(
        out.status.success(),
        "init exited {:?} without a TTY\nstderr:\n{stderr}",
        out.status.code()
    );

    // It must actually scaffold, not just exit quietly.
    assert!(
        dir.path().join("symbiont.toml").is_file(),
        "init exited 0 but wrote no symbiont.toml"
    );
    assert!(
        dir.path().join("policies/default.cedar").is_file(),
        "init exited 0 but wrote no default policy"
    );
}

#[test]
fn landlock_init_inherits_project_boundary_without_container_scaffolding() {
    for (profile, agent) in [("assistant", "assistant"), ("dev-agent", "dev")] {
        // The dev-agent profile provisions a real Landlock boundary during
        // init, so it needs both its five settings and a kernel that can
        // enforce the tier. Skip rather than fail where the kernel cannot.
        let developer = profile == "dev-agent";
        if developer && symbi_runtime::sandbox::landlock::detect_abi() < 6 {
            eprintln!("skipped dev-agent: kernel Landlock ABI below 6");
            continue;
        }
        let dir = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_symbi"));
        command
            .current_dir(dir.path())
            .args([
                "init",
                "--sandbox",
                "landlock",
                "--profile",
                profile,
                "--dir",
            ])
            .arg(dir.path());
        if developer {
            command
                .arg("--source")
                .arg(source.path())
                .arg("--managed-executable")
                .arg("/bin/cat")
                .args([
                    "--inference-url",
                    "http://127.0.0.1:1/",
                    "--inference-model",
                    "fixture-model",
                    "--inference-key-env",
                    "FIXTURE_PROVIDER_KEY",
                ]);
        }
        let out = command.stdin(Stdio::null()).output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let config = std::fs::read_to_string(dir.path().join("symbiont.toml")).unwrap();
        assert!(config.contains("tier = \"landlock\""));
        assert!(!dir.path().join("docker-compose.yml").exists());
        let source =
            std::fs::read_to_string(dir.path().join(format!("agents/{agent}.symbi"))).unwrap();
        assert!(
            !source.contains("sandbox ="),
            "agent must inherit the project boundary"
        );
        if profile == "assistant" {
            dsl::ConversationalAgent::parse(&source, agent)
                .expect("generated agent must be accepted by conversational execution");
        } else {
            dsl::ExecutionPolicy::parse(&source, agent)
                .expect("generated managed agent must have a supported execution policy");
        }
        let check = Command::new(env!("CARGO_BIN_EXE_symbi"))
            .current_dir(dir.path())
            .args(["dsl", "--check", "-f"])
            .arg(dir.path().join(format!("agents/{agent}.symbi")))
            .output()
            .unwrap();
        assert!(
            check.status.success(),
            "{}",
            String::from_utf8_lossy(&check.stderr)
        );
        assert!(String::from_utf8_lossy(&out.stdout).contains("symbi doctor"));
    }
}
