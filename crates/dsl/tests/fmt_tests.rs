//! Fixture-driven tests for `dsl::format::format_source`.
//!
//! Each fixture under `fixtures/fmt/<name>/` contains an `input.symbi` and an
//! `expected.symbi`. The formatter should produce `expected.symbi` when given
//! `input.symbi`.

use std::fs;
use std::path::PathBuf;

fn fixture_dir(name: &str) -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest)
        .join("tests")
        .join("fixtures")
        .join("fmt")
        .join(name)
}

fn run_fixture(name: &str) {
    let dir = fixture_dir(name);
    let input = fs::read_to_string(dir.join("input.symbi")).expect("read input.symbi");
    let expected = fs::read_to_string(dir.join("expected.symbi")).expect("read expected.symbi");

    let actual = dsl::format::format_source(&input).expect("format_source");

    assert_eq!(
        actual, expected,
        "fixture {name} mismatch\n--- expected ---\n{expected}\n--- actual ---\n{actual}\n"
    );
}

#[test]
fn fixture_01_metadata() {
    run_fixture("01_metadata");
}

#[test]
fn fixture_02_agent_simple() {
    run_fixture("02_agent_simple");
}

#[test]
fn fixture_03_comments() {
    run_fixture("03_comments");
}

/// Every example agent under symbiont/agents/ must:
/// 1. parse without errors through the v2 grammar (no fallback to verbatim);
/// 2. round-trip through the formatter (pass1 == pass2).
#[test]
fn all_example_agents_parse_and_are_idempotent() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    // crates/dsl/ → ../../agents
    let agents_dir = std::path::PathBuf::from(manifest)
        .join("..")
        .join("..")
        .join("agents");
    let mut files: Vec<_> = std::fs::read_dir(&agents_dir)
        .expect("read agents dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("symbi"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "expected at least one example agent");

    for path in files {
        let input = std::fs::read_to_string(&path).expect("read agent");
        let pass1 = dsl::format::format_source(&input).expect("pass1");
        let pass2 = dsl::format::format_source(&pass1).expect("pass2");
        assert_eq!(
            pass1,
            pass2,
            "formatter not idempotent on {}",
            path.display()
        );

        // Confirm the file did not fall back to the tolerant verbatim path:
        // re-parse the formatted output and assert no parse errors.
        let tree = dsl::parse_dsl(&pass1).expect("re-parse formatted");
        assert!(
            !tree.root_node().has_error(),
            "formatted output of {} contains parse errors",
            path.display()
        );
    }
}

#[test]
fn idempotent_canonical_form() {
    let dir = fixture_dir("05_idempotent");
    let input = std::fs::read_to_string(dir.join("input.symbi")).expect("read input");
    let pass1 = dsl::format::format_source(&input).expect("pass1");
    assert_eq!(
        input, pass1,
        "canonical input should be unchanged after one pass"
    );
    let pass2 = dsl::format::format_source(&pass1).expect("pass2");
    assert_eq!(pass1, pass2, "formatter is not idempotent");
}
