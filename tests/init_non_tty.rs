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
