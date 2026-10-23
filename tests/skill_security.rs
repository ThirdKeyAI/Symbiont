//! Security decisions must survive the shipping CLI's exit-status boundary.
use std::{
    path::Path,
    process::{Command, Output, Stdio},
};

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_symbi"))
        .current_dir(root)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

#[test]
fn signing_and_verification_failures_have_truthful_exit_codes() {
    let root = tempfile::tempdir().unwrap();
    let skill = root.path().join("skill");
    std::fs::create_dir(&skill).unwrap();
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nname: fixture\n---\nSafe instructions.",
    )
    .unwrap();
    let unsigned = run(
        root.path(),
        &["skills", "verify", "skill", "--domain", "example.invalid"],
    );
    assert_eq!(unsigned.status.code(), Some(1));
    let key = schemapin::crypto::generate_key_pair().unwrap();
    std::fs::write(root.path().join("signing-key.pem"), key.private_key_pem).unwrap();
    let signed = run(
        root.path(),
        &[
            "skills",
            "sign",
            "skill",
            "--domain",
            "example.invalid",
            "--key",
            "signing-key.pem",
        ],
    );
    assert!(
        signed.status.success(),
        "{}",
        String::from_utf8_lossy(&signed.stderr)
    );
    assert!(skill.join(".schemapin.sig").is_file());
    let signature = symbi_runtime::skills::load_skill_signature(&skill).unwrap();
    assert_eq!(
        signature.schemapin_version,
        symbi_runtime::skills::signature::FORMAT
    );
    let discovery =
        schemapin::discovery::build_well_known_response(&key.public_key_pem, None, vec![], "1.3");
    let mut loader = symbi_runtime::skills::SkillLoader::new(Default::default());
    assert!(matches!(
        loader.verify_skill_with_discovery(&skill, &discovery, "example.invalid"),
        symbi_runtime::skills::SignatureStatus::Verified { .. }
            | symbi_runtime::skills::SignatureStatus::Pinned { .. }
    ));
    let wrong_authority = run(
        root.path(),
        &["skills", "verify", "skill", "--domain", "other.invalid"],
    );
    assert_eq!(wrong_authority.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&wrong_authority.stderr).contains("does not match"));
    // Reserved .invalid cannot provide a discovery document. Failure to fetch
    // verification material must not exit successfully merely because a signature exists.
    let unavailable = run(
        root.path(),
        &["skills", "verify", "skill", "--domain", "example.invalid"],
    );
    assert_eq!(unavailable.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&unavailable.stdout).contains("Online verification failed"));
}

#[test]
fn scanning_accepts_clean_content_and_rejects_incomplete_or_linked_trees() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("skill")).unwrap();
    std::fs::write(root.path().join("skill/SKILL.md"), "Safe instructions.").unwrap();
    assert!(run(root.path(), &["skills", "scan", "skill"])
        .status
        .success());
    std::fs::write(
        root.path().join("skill/oversized.txt"),
        vec![b'x'; 1024 * 1024 + 1],
    )
    .unwrap();
    assert_eq!(
        run(root.path(), &["skills", "scan", "skill"]).status.code(),
        Some(1)
    );
    std::fs::remove_file(root.path().join("skill/oversized.txt")).unwrap();
    #[cfg(unix)]
    {
        std::fs::write(root.path().join("outside.txt"), "Outside content").unwrap();
        std::os::unix::fs::symlink("../outside.txt", root.path().join("skill/link.txt")).unwrap();
        assert_eq!(
            run(root.path(), &["skills", "scan", "skill"]).status.code(),
            Some(1)
        );
    }
    assert_eq!(
        run(root.path(), &["skills", "scan", "missing"])
            .status
            .code(),
        Some(1)
    );
}
