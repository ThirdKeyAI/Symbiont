use super::*;
use symbi_runtime::reasoning::policy_bridge::DefaultPolicyGate;

fn fixture(sources: &[(&str, &str)]) -> (tempfile::TempDir, LlmAgentInvoker) {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("agents")).unwrap();
    for (file, source) in sources {
        std::fs::write(root.path().join("agents").join(file), source).unwrap();
    }
    let invoker =
        LlmAgentInvoker::new(root.path().into(), None, Arc::new(DefaultPolicyGate::new())).unwrap();
    (root, invoker)
}

#[test]
fn aliases_bind_the_declared_source_and_cannot_load_later_replacements() {
    let (root, invoker) = fixture(&[("alias.symbi", "agent declared {}")]);
    for name in ["alias", "declared"] {
        assert_eq!(
            invoker.selected_agent(name).unwrap().settings().agent_name,
            "declared"
        );
    }
    std::fs::write(root.path().join("agents/alias.symbi"), "agent replaced {}").unwrap();
    assert_eq!(
        invoker
            .selected_agent("alias")
            .unwrap()
            .settings()
            .agent_name,
        "declared"
    );
    assert!(invoker.selected_agent("replaced").is_err());
}

#[test]
fn unknown_ambiguous_and_executable_sources_are_refused() {
    let (_root, invoker) = fixture(&[
        ("one.symbi", "agent declared {}"),
        ("two.dsl", "agent declared {}"),
        (
            "managed.symbi",
            "metadata { executor = \"claude_code\" }\nagent managed {}",
        ),
    ]);
    for name in ["unknown", "declared", "managed"] {
        assert!(invoker.selected_agent(name).is_err(), "{name}");
    }
}
