use super::*;
use crate::reasoning::{
    executor::DefaultActionExecutor, loop_types::LoopState, policy_bridge::DefaultPolicyGate,
};

fn sources(input: &[(&str, &str)]) -> Vec<(String, String)> {
    input
        .iter()
        .map(|(file, source)| ((*file).into(), (*source).into()))
        .collect()
}

#[test]
fn aliases_share_source_principal_and_ambiguous_identities_are_unavailable() {
    let registry = RegisteredDelegationRegistry::from_sources(sources(&[(
        "alias.symbi",
        "agent declared {}",
    )]))
    .unwrap();
    assert!(Arc::ptr_eq(
        &registry.resolve("alias").unwrap(),
        &registry.resolve("declared").unwrap()
    ));
    for entries in [
        sources(&[
            ("one.symbi", "agent duplicate {}"),
            ("two.symbi", "agent duplicate {}"),
        ]),
        sources(&[
            ("first.symbi", "agent second {}"),
            ("second.symbi", "agent other {}"),
        ]),
        sources(&[("same.symbi", "agent valid {}"), ("same.dsl", "invalid")]),
    ] {
        let registry = RegisteredDelegationRegistry::from_sources(entries).unwrap();
        assert!(registry.names().is_empty());
    }
}

#[test]
fn selection_excludes_siblings_and_refuses_unsupported_requirements() {
    let registry = RegisteredDelegationRegistry::from_sources(sources(&[
        ("multiple.symbi", "agent reader {} agent writer {}"),
        (
            "managed.symbi",
            "metadata { executor = \"claude_code\" } agent managed {}",
        ),
        (
            "executable.symbi",
            "agent executable() { function work() { return 1 } }",
        ),
        (
            "rules.symbi",
            "policy requirements { require: true } agent rules {}",
        ),
    ]))
    .unwrap();
    assert_eq!(registry.names(), ["reader", "writer"]);
    assert!(!registry
        .resolve("reader")
        .unwrap()
        .prompt
        .contains("agent writer"));
    assert!(registry.resolve("multiple").is_err());
}

#[cfg(unix)]
#[test]
fn project_loading_refuses_links_and_retains_the_startup_source() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("agents")).unwrap();
    std::fs::write(root.path().join("agents/alias.symbi"), "agent declared {}").unwrap();
    std::fs::write(outside.path().join("secret.symbi"), "agent outside {}").unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("secret.symbi"),
        root.path().join("agents/link.symbi"),
    )
    .unwrap();
    std::fs::hard_link(
        outside.path().join("secret.symbi"),
        root.path().join("agents/hard.symbi"),
    )
    .unwrap();
    let registry = RegisteredDelegationRegistry::load(root.path()).unwrap();
    assert_eq!(registry.names(), ["alias", "declared"]);
    std::fs::write(root.path().join("agents/alias.symbi"), "agent replaced {}").unwrap();
    assert_eq!(
        registry
            .resolve("alias")
            .unwrap()
            .agent
            .settings()
            .agent_name,
        "declared"
    );
}

#[tokio::test]
async fn parent_authorization_binds_the_declared_target_and_source() {
    let registry = Arc::new(
        RegisteredDelegationRegistry::from_sources(sources(&[(
            "alias.symbi",
            "agent declared {}",
        )]))
        .unwrap(),
    );
    let executor = registry.wrap(Arc::new(DefaultActionExecutor::default()));
    let action = ProposedAction::Delegate {
        call_id: "delegate-call".into(),
        target: "alias".into(),
        message: "task".into(),
    };
    let state = LoopState::new(AgentId::new(), Conversation::new());
    let grant = crate::reasoning::dispatch::authorize_action(
        &action,
        &state,
        &LoopConfig::default(),
        executor.as_ref(),
        &DefaultPolicyGate::permissive_for_dev_only(),
    )
    .await
    .unwrap();
    assert!(
        matches!(grant.action(), ProposedAction::Delegate { target, .. } if target == "declared")
    );
    assert_eq!(
        grant.prepared().policy_context()["resolved"]["delegation_target"]["principal"],
        json!(delegated_agent_id("declared"))
    );
    let frozen = grant.prepared().backend::<Arc<RegisteredTarget>>().unwrap();
    assert!(Arc::ptr_eq(frozen, &registry.resolve("alias").unwrap()));
}
