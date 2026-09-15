//! Enforcement tests spawn children. A landlock domain cannot be lifted once
//! applied, so restricting the test process itself would poison every test
//! that ran afterwards.
#![cfg(target_os = "linux")]

use std::process::Command;
use symbi_runtime::sandbox::command::BoundaryRoots;
use symbi_runtime::sandbox::landlock::{detect_abi, prepare, LandlockProfile};

fn allowed_only(dir: &std::path::Path) -> BoundaryRoots {
    BoundaryRoots {
        source_roots: vec![format!("{}:/workspace:ro", dir.display())],
        output_roots: vec![],
    }
}

#[test]
fn a_restricted_child_reads_its_grant_and_is_refused_everything_else() {
    if detect_abi() < 4 {
        eprintln!("skipped: kernel Landlock ABI below 4");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("allowed"), b"ok").expect("write");
    let denied = tempfile::tempdir().expect("tempdir");
    std::fs::write(denied.path().join("secret"), b"no").expect("write");

    let domain =
        prepare(&LandlockProfile::default(), &allowed_only(dir.path())).expect("prepare domain");

    let mut allowed_read = Command::new("/bin/cat");
    allowed_read.arg(dir.path().join("allowed"));
    domain.apply_to_std(&mut allowed_read);
    assert!(
        allowed_read.status().expect("spawn").success(),
        "the granted path must stay readable"
    );

    let mut denied_read = Command::new("/bin/cat");
    denied_read.arg(denied.path().join("secret"));
    domain.apply_to_std(&mut denied_read);
    assert!(
        !denied_read.status().expect("spawn").success(),
        "a path outside every grant must be refused"
    );
}
