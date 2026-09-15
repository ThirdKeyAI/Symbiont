#![cfg(target_os = "linux")]

use std::{
    os::unix::fs::{symlink, MetadataExt},
    time::{Duration, Instant},
};
use symbi_runtime::sandbox::{command::CommandBoundary, workspace::WorkspacePlan};

fn fixture() -> (tempfile::TempDir, CommandBoundary) {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("input.txt"), "permitted needle").unwrap();
    let mut boundary = CommandBoundary::default();
    boundary.docker.volumes = vec![format!("{}:/workspace:rw", root.path().display())];
    (root, boundary)
}
fn finish(plan: &WorkspacePlan) -> serde_json::Value {
    plan.execute(Instant::now() + Duration::from_secs(2))
        .unwrap()
}

#[test]
fn reads_bind_snapshots_and_search_respects_overlapping_ceilings() {
    let (root, mut boundary) = fixture();
    let plan = WorkspacePlan::prepare(&boundary, "read_file", "input.txt", None).unwrap();
    std::fs::write(root.path().join("input.txt"), "changed after preparation").unwrap();
    assert!(finish(&plan)["results"]["raw_output"]
        .as_str()
        .unwrap()
        .contains("permitted needle"));
    assert!(plan
        .execute(Instant::now() + Duration::from_secs(1))
        .is_err());
    let overlay = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("nested")).unwrap();
    std::fs::write(root.path().join("nested/hidden.txt"), "hidden needle").unwrap();
    std::fs::write(overlay.path().join("visible.txt"), "visible needle").unwrap();
    boundary
        .docker
        .volumes
        .push(format!("{}:/workspace/nested:ro", overlay.path().display()));
    symlink("input.txt", root.path().join("linked.txt")).unwrap();
    let search = WorkspacePlan::prepare(&boundary, "search", ".", Some("needle")).unwrap();
    let result = finish(&search).to_string();
    assert!(result.contains("nested/visible.txt"), "{result}");
    assert!(!result.contains("hidden needle") && !result.contains("linked.txt"));
    let grant = search.descriptor().to_string();
    assert!(!grant.contains("hidden.txt"));
    boundary.docker.volumes.remove(0);
    let search = WorkspacePlan::prepare(&boundary, "search", ".", Some("needle")).unwrap();
    assert!(finish(&search).to_string().contains("nested/visible.txt"));
}

#[test]
fn edits_retain_one_inode_and_refuse_changed_content_or_path() {
    for change in ["none", "content", "path"] {
        let (root, boundary) = fixture();
        let path = root.path().join("input.txt");
        let inode = path.metadata().unwrap().ino();
        let plan = WorkspacePlan::prepare(
            &boundary,
            "edit_file",
            "input.txt",
            Some("new exact content"),
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "permitted needle");
        match change {
            "content" => std::fs::write(&path, "concurrent host change").unwrap(),
            "path" => {
                std::fs::rename(&path, root.path().join("moved.txt")).unwrap();
                std::fs::write(&path, "competing file").unwrap();
            }
            _ => {}
        }
        let result = plan.execute(Instant::now() + Duration::from_secs(1));
        if change == "none" {
            assert_eq!(result.unwrap()["written_file"]["mode"], "in_place");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "new exact content");
            assert_eq!(path.metadata().unwrap().ino(), inode);
        } else {
            assert!(result.unwrap_err().contains("changed since authorization"));
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                if change == "content" {
                    "concurrent host change"
                } else {
                    "competing file"
                }
            );
            if change == "path" {
                assert_eq!(
                    std::fs::read_to_string(root.path().join("moved.txt")).unwrap(),
                    "permitted needle"
                );
            }
        }
    }
}

#[test]
fn artifact_creation_is_deferred_and_never_follows_competing_entries() {
    let (root, boundary) = fixture();
    let plan = WorkspacePlan::prepare(
        &boundary,
        "save_artifact",
        "new/sub/artifact.txt",
        Some("exact artifact"),
    )
    .unwrap();
    assert!(!root.path().join("new").exists());
    assert_eq!(finish(&plan)["written_file"]["mode"], "create_no_replace");
    assert_eq!(
        std::fs::read_to_string(root.path().join("new/sub/artifact.txt")).unwrap(),
        "exact artifact"
    );
    for path in ["competing.txt", "linked.txt"] {
        let plan =
            WorkspacePlan::prepare(&boundary, "edit_file", path, Some("must not publish")).unwrap();
        if path == "linked.txt" {
            symlink("input.txt", root.path().join(path)).unwrap();
        } else {
            std::fs::write(root.path().join(path), "protected competitor").unwrap();
        }
        assert!(plan
            .execute(Instant::now() + Duration::from_secs(1))
            .is_err());
    }
    let plan = WorkspacePlan::prepare(
        &boundary,
        "save_artifact",
        "redirect/child.txt",
        Some("must not publish"),
    )
    .unwrap();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), root.path().join("redirect")).unwrap();
    assert!(plan
        .execute(Instant::now() + Duration::from_secs(1))
        .is_err());
    assert!(!outside.path().join("child.txt").exists());
    assert_eq!(
        std::fs::read_to_string(root.path().join("input.txt")).unwrap(),
        "permitted needle"
    );
}

#[test]
fn links_devices_control_paths_readonly_and_expired_writes_are_refused() {
    let (root, mut boundary) = fixture();
    symlink("input.txt", root.path().join("linked.txt")).unwrap();
    std::fs::hard_link(root.path().join("input.txt"), root.path().join("hard.txt")).unwrap();
    let fifo = std::ffi::CString::new(root.path().join("pipe").to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    for path in [
        "linked.txt",
        "hard.txt",
        "pipe",
        "../outside",
        ".symbiont/key",
        ".git/config",
    ] {
        assert!(
            WorkspacePlan::prepare(&boundary, "read_file", path, None).is_err(),
            "{path}"
        );
        assert!(
            WorkspacePlan::prepare(&boundary, "edit_file", path, Some("x")).is_err(),
            "{path}"
        );
    }
    let plan =
        WorkspacePlan::prepare(&boundary, "save_artifact", "expired/file", Some("x")).unwrap();
    assert!(plan.execute(Instant::now()).is_err());
    assert!(!root.path().join("expired").exists());
    boundary.docker.volumes = vec![format!("{}:/workspace:ro", root.path().display())];
    assert!(WorkspacePlan::prepare(&boundary, "edit_file", "new.txt", Some("x")).is_err());
}

#[test]
fn searches_keep_strict_input_and_output_byte_limits() {
    let (root, boundary) = fixture();
    for i in 0..40 {
        std::fs::write(root.path().join(format!("{i:02}.txt")), "x".repeat(32768)).unwrap();
    }
    let plan = WorkspacePlan::prepare(&boundary, "search", ".", Some("absent")).unwrap();
    let bytes: u64 = plan.descriptor()["read"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["bytes"].as_u64().unwrap())
        .sum();
    assert!(bytes <= 1024 * 1024);
    assert_eq!(plan.descriptor()["limited"], true);
    assert!(finish(&plan).to_string().contains("limited=true"));
}

#[test]
fn concurrent_edits_cannot_both_consume_the_same_prior_content() {
    let (root, boundary) = fixture();
    let plans = ["first edit", "second edit"].map(|value| {
        WorkspacePlan::prepare(&boundary, "edit_file", "input.txt", Some(value)).unwrap()
    });
    let barrier = std::sync::Barrier::new(2);
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = plans
            .iter()
            .map(|plan| {
                scope.spawn(|| {
                    barrier.wait();
                    plan.execute(Instant::now() + Duration::from_secs(2))
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let bytes = std::fs::read(root.path().join("input.txt")).unwrap();
    let expected = results.into_iter().find_map(Result::ok).unwrap();
    use sha2::Digest;
    assert_eq!(
        expected["written_file"]["sha256"],
        format!("{:x}", sha2::Sha256::digest(bytes))
    );
}
