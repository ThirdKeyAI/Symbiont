//! Shipping executor and turn-owner regression coverage using synthetic inference.

use crate::{
    orchestrator::Orchestrator,
    orchestrator_executor::OrchestratorExecutor,
    sandbox_tools::normalize_path,
    turn_audit::{TurnAudit, TurnRuntime},
    validation::constraints::ProjectConstraints,
};
use std::{
    collections::VecDeque,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use symbi_runtime::{
    escalation::{
        Approver, Decision, EscalationGate, EscalationGateConfig, EscalationQueue, Surface,
    },
    reasoning::{
        conversation::{Conversation, ConversationMessage},
        executor::ActionExecutor,
        inference::{
            FinishReason, InferenceError, InferenceOptions, InferenceProvider, InferenceResponse,
            ToolCallRequest, Usage,
        },
        loop_types::{
            JournalEntry, LoopConfig, LoopEvent, LoopResult, Observation, ProposedAction,
            TerminationReason,
        },
        policy_bridge::{DefaultPolicyGate, ReasoningPolicyGate},
        protected_journal::ProtectedJournal,
        run_audit::RunAuditReference,
        CedarPolicy, CedarPolicyGate,
    },
    types::AgentId,
};

enum Step {
    Tools(Vec<ToolCallRequest>),
    Reply,
    Fail,
    Wait,
}
struct Provider {
    steps: Mutex<VecDeque<Step>>,
    calls: AtomicUsize,
    active: Arc<AtomicUsize>,
}
impl Provider {
    fn new(steps: Vec<Step>) -> Arc<Self> {
        Arc::new(Self {
            steps: Mutex::new(steps.into()),
            calls: AtomicUsize::new(0),
            active: Arc::new(AtomicUsize::new(0)),
        })
    }
}
struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
#[async_trait::async_trait]
impl InferenceProvider for Provider {
    async fn complete(
        &self,
        _: &Conversation,
        _: &InferenceOptions,
    ) -> Result<InferenceResponse, InferenceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.active.fetch_add(1, Ordering::SeqCst);
        let _active = Active(self.active.clone());
        let step = self
            .steps
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected inference retry");
        let tools = match step {
            Step::Tools(tools) => tools,
            Step::Reply => vec![],
            Step::Fail => {
                return Err(InferenceError::Provider(
                    "synthetic HTTP 503 after a tool call".into(),
                ))
            }
            Step::Wait => std::future::pending().await,
        };
        Ok(InferenceResponse {
            content: if tools.is_empty() {
                "fixture complete".into()
            } else {
                String::new()
            },
            finish_reason: if tools.is_empty() {
                FinishReason::Stop
            } else {
                FinishReason::ToolCalls
            },
            tool_calls: tools,
            usage: Usage::default(),
            model: "fixture".into(),
        })
    }
    fn provider_name(&self) -> &str {
        "fixture"
    }
    fn default_model(&self) -> &str {
        "fixture"
    }
    fn supports_native_tools(&self) -> bool {
        true
    }
    fn supports_structured_output(&self) -> bool {
        false
    }
}

fn call(name: &str, args: serde_json::Value) -> ToolCallRequest {
    ToolCallRequest {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.into(),
        arguments: args.to_string(),
    }
}
fn project(read_only: bool) -> tempfile::TempDir {
    let project = tempfile::tempdir().unwrap();
    let data = project.path().join("data");
    std::fs::create_dir(&data).unwrap();
    let info = project.path().metadata().unwrap();
    assert_ne!(
        info.uid(),
        0,
        "fixture must run as an unprivileged host user"
    );
    let mount = format!(
        "{}:/workspace:{}",
        data.display(),
        if read_only { "ro" } else { "rw" }
    );
    std::fs::write(
        project.path().join("symbiont.toml"),
        format!(
            r#"
[sandbox]
tier = "docker"
[sandbox.docker]
image = "sha256:7bf61a5ec2a4631b240bc8cf83404e2dd37fb06a4b4bbd34b2c36906a5fb4aee"
user = "{}:{}"
volumes = [{}]
extra_flags = ["--label=symbi.shell-governance={}"]
"#,
            info.uid(),
            info.gid(),
            serde_json::to_string(&mount).unwrap(),
            uuid::Uuid::new_v4()
        ),
    )
    .unwrap();
    project
}
fn executor(project: &Path, allow_shell: bool) -> Arc<OrchestratorExecutor> {
    let bridge = Arc::new(
        repl_core::RuntimeBridge::new()
            .with_project_root(project)
            .unwrap(),
    );
    Arc::new(OrchestratorExecutor::new(
        Arc::new(ProjectConstraints::default()),
        Arc::new(repl_core::ReplEngine::new(bridge.clone())),
        bridge,
        Arc::new(tokio::sync::RwLock::new(vec![])),
        allow_shell,
    ))
}

async fn canonical_fleet(
    project: &Path,
    source: &str,
    provider: Arc<Provider>,
    gate: Arc<dyn ReasoningPolicyGate>,
) -> (
    Arc<repl_core::RuntimeBridge>,
    crate::fleet_runner::FleetRunnerFactory,
    Arc<TurnAudit>,
) {
    let agents = project.join("agents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(agents.join("fixture.symbi"), source).unwrap();
    let bridge = Arc::new(
        repl_core::RuntimeBridge::new()
            .with_project_root(project)
            .unwrap(),
    );
    bridge.set_inference_provider(provider.clone());
    let cards = Arc::new(tokio::sync::RwLock::new(vec![]));
    let report = crate::agents::load_agents_into(&agents, &bridge, &cards).await;
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.sandbox_refused.is_empty());
    let audit = Arc::new(TurnAudit::default());
    let factory = crate::fleet_runner::FleetRunnerFactory::new(
        provider,
        Arc::new(ProjectConstraints::default()),
        bridge.clone(),
        cards,
        false,
        gate,
    )
    .with_audit(audit.clone());
    (bridge, factory, audit)
}

#[tokio::test]
async fn canonical_fleet_unavailable_selection_prevents_inference() {
    let project = project(false);
    let source =
        r#"agent unavailable() { capabilities = ["read"] with sandbox = "firecracker" {} }"#;
    let provider = Provider::new(vec![Step::Reply]);
    let (bridge, factory, audit) =
        canonical_fleet(project.path(), source, provider.clone(), permissive()).await;
    let principal = bridge
        .agent_registry()
        .get_agent("unavailable")
        .await
        .unwrap()
        .agent_id;
    let mut runner = factory
        .build("unavailable", &["read_file".into()])
        .await
        .unwrap();
    let error = runner.send("fixture").await.err().unwrap().to_string();
    assert!(error.to_lowercase().contains("firecracker"), "{error}");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    let entries = records(&audit.references().unwrap().entries[0], principal);
    assert!(
        matches!(&entries[0].event, LoopEvent::Started { execution_context, .. }
        if execution_context["agent_definition"]["name"] == "unavailable")
    );
    assert!(matches!(
        entries.last().unwrap().event,
        LoopEvent::Terminated {
            reason: TerminationReason::Error { .. },
            ..
        }
    ));
}

#[tokio::test]
async fn canonical_fleet_lifetime_and_prompt_only_routes_preserve_requirements() {
    let project = project(false);
    let source = r#"agent bounded() { with sandbox = "docker", timeout = 1.seconds {} }"#;
    let provider = Provider::new(vec![Step::Wait]);
    let (bridge, factory, audit) =
        canonical_fleet(project.path(), source, provider.clone(), permissive()).await;
    let registry = bridge.agent_registry();
    let original = registry.get_agent("bounded").await.unwrap();
    let error = registry
        .spawn_prompt_agent(
            "bounded".into(),
            "replacement".into(),
            vec!["shell".into()],
            None,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("cannot replace"));
    assert_eq!(
        registry.get_agent("bounded").await.unwrap().agent_id,
        original.agent_id
    );
    assert!(registry
        .ask_agent("bounded", "fixture", provider.as_ref())
        .await
        .unwrap_err()
        .to_string()
        .contains("governed executor"));
    assert!(bridge
        .delegate("bounded", "fixture")
        .await
        .unwrap_err()
        .to_string()
        .contains("governed source-bound executor"));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    let mut runner = factory.build("bounded", &[]).await.unwrap();
    let started = std::time::Instant::now();
    let error = runner.send("fixture").await.err().unwrap().to_string();
    assert!(error.contains("Timeout"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(4));
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    let entries = records(&audit.references().unwrap().entries[0], original.agent_id);
    assert!(
        matches!(&entries[0].event, LoopEvent::Started { config, .. } if config.timeout == Duration::from_secs(1))
    );
    assert!(matches!(
        entries.last().unwrap().event,
        LoopEvent::Terminated {
            reason: TerminationReason::Timeout,
            ..
        }
    ));
}

#[tokio::test]
async fn broker_canonical_fleet_binds_selected_source_through_approval_and_effect() {
    let project = project(false);
    let config_path = project.path().join("symbiont.toml");
    let project_config = std::fs::read_to_string(&config_path).unwrap();
    std::fs::write(
        &config_path,
        project_config.replace("tier = \"docker\"", "tier = \"firecracker\""),
    )
    .unwrap();
    let source = r#"metadata { description = "retained fixture", executor = "orga" }
agent writer() { capabilities = ["write"] with sandbox = "docker", timeout = 12.seconds {} policy files { allow: "edit_file" if invocation.arguments.path == "result" deny: "edit_file" if invocation.arguments.path != "result" } }
agent reader() { capabilities = ["read"] with sandbox = "firecracker" {} }"#;
    let provider = Provider::new(vec![
        Step::Tools(vec![
            call(
                "edit_file",
                serde_json::json!({"path":"blocked", "content":"must not exist"}),
            ),
            call(
                "edit_file",
                serde_json::json!({"path":"./result", "content":"exact source-bound effect"}),
            ),
        ]),
        Step::Reply,
    ]);
    let queue = Arc::new(EscalationQueue::new());
    let cedar = Arc::new(CedarPolicyGate::deny_by_default());
    let gate = Arc::new(EscalationGate::new(
        cedar.clone(),
        queue.clone(),
        EscalationGateConfig {
            require_approval_tools: vec![],
            timeout: Duration::from_secs(5),
        },
    ));
    let (bridge, factory, audit) =
        canonical_fleet(project.path(), source, provider.clone(), gate).await;
    let writer = bridge.agent_registry().get_agent("writer").await.unwrap();
    let hash = symbi_runtime::reasoning::prepared::digest_json(&serde_json::json!(source)).unwrap();
    cedar.add_policy(CedarPolicy { name:"fixture".into(), active:true, source:format!(
        "permit(principal == Agent::\"{}\", action, resource) when {{ context.agent_definition.source_hash == \"{}\" }}; forbid(principal, action == Action::\"tool_call::edit_file\", resource) unless {{ context.invocation.arguments.path == \"result\" }};", writer.agent_id, hash) }).await;
    let mut runner = factory
        .build(
            "writer",
            &["edit_file".into(), "read_file".into(), "delegate".into()],
        )
        .await
        .unwrap();
    let task = tokio::spawn(async move { runner.send("fixture").await });
    tokio::time::timeout(Duration::from_secs(4), async {
        while queue.list_pending_async().await.is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    std::fs::write(
        project.path().join("agents/fixture.symbi"),
        "agent writer() { with sandbox = \"firecracker\" {} }",
    )
    .unwrap();
    std::fs::write(&config_path, "[sandbox]\ntier=\"firecracker\"\n").unwrap();
    let held = resolve(&queue, true).await;
    let context = held.context_snapshot.as_ref().unwrap();
    assert_eq!(
        context["invocation"]["resolved"]["agent_definition"]["source_hash"],
        hash
    );
    assert_eq!(context["invocation"]["arguments"]["path"], "result");
    let response = task.await.unwrap().unwrap();
    assert_eq!(
        std::fs::read_to_string(project.path().join("data/result")).unwrap(),
        "exact source-bound effect"
    );
    assert!(!project.path().join("result").exists());
    let entries = records(response.audit.as_ref().unwrap(), writer.agent_id);
    assert!(
        matches!(&entries[0].event, LoopEvent::Started { execution_context, config, .. }
        if execution_context["agent_definition"]["source_hash"] == hash && config.timeout == Duration::from_secs(12))
    );
    assert!(!project.path().join("data/blocked").exists());
    let results = observations(&entries);
    assert_eq!(results.len(), 1);
    assert!(!results[0].is_error);
    assert!(entries.iter().any(|entry| matches!(&entry.event,
        LoopEvent::PolicyEvaluated { denied_count: 1, denied_calls, .. }
        if denied_calls.len() == 1 && denied_calls[0]["reason"].as_str().is_some_and(|reason| reason.contains("inline policy files")))));
    assert_eq!(context["invocation"]["source_policy"]["source_hash"], hash);
    assert_eq!(audit.references().unwrap().entries.len(), 1);
    assert_eq!(
        bridge
            .agent_registry()
            .get_agent("writer")
            .await
            .unwrap()
            .definition
            .unwrap()
            .source(),
        source
    );
    let calls = provider.calls.load(Ordering::SeqCst);
    let mut sibling = factory
        .build("reader", &["edit_file".into(), "read_file".into()])
        .await
        .unwrap();
    assert!(sibling.send("fixture").await.is_err());
    assert_eq!(provider.calls.load(Ordering::SeqCst), calls);
}
fn runtime(
    project: &Path,
    provider: Arc<Provider>,
    executor: Arc<OrchestratorExecutor>,
    gate: Arc<dyn ReasoningPolicyGate>,
) -> TurnRuntime {
    TurnRuntime {
        provider,
        executor,
        gate,
        project: Ok(project.to_owned()),
    }
}
fn conversation() -> Conversation {
    let mut conversation = Conversation::new();
    conversation.push(ConversationMessage::user("synthetic task"));
    conversation
}
fn records(reference: &RunAuditReference, principal: AgentId) -> Vec<JournalEntry> {
    let key: [u8; 32] = (0..32)
        .map(|i| u8::from_str_radix(&reference.public_key[2 * i..2 * i + 2], 16).unwrap())
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    let entries = ProtectedJournal::verify_run(&reference.path, &key, reference.run_id).unwrap();
    assert!(entries.iter().all(|entry| entry.agent_id == principal));
    assert_eq!(entries[0].sequence, 0);
    assert_eq!(
        reference.path.metadata().unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(ProtectedJournal::verify_run(&reference.path, &key, uuid::Uuid::new_v4()).is_err());
    entries
}
fn observations(entries: &[JournalEntry]) -> Vec<Observation> {
    entries
        .iter()
        .flat_map(|entry| match &entry.event {
            LoopEvent::ToolBatchCompleted { observations, .. } => observations.clone(),
            _ => vec![],
        })
        .collect()
}
async fn run(runtime: &TurnRuntime, principal: AgentId) -> (LoopResult, Vec<JournalEntry>) {
    let (result, reference) = runtime
        .run(
            principal,
            conversation(),
            LoopConfig::default(),
            Arc::new(TurnAudit::default()),
        )
        .await
        .unwrap();
    let entries = records(&reference, principal);
    assert!(matches!(
        entries.last().unwrap().event,
        LoopEvent::Terminated { .. }
    ));
    (result, entries)
}
fn permissive() -> Arc<dyn ReasoningPolicyGate> {
    Arc::new(DefaultPolicyGate::permissive_for_dev_only())
}
// Deliberately supplies Allow without a receipt; the executor contract must
// still require the receipt even when an embedding supplies a permissive gate.
struct AllowWithoutReceipt;
#[async_trait::async_trait]
impl ReasoningPolicyGate for AllowWithoutReceipt {
    async fn approve_prepared(
        &self,
        _: &symbi_runtime::reasoning::prepared::PreparedAction,
        _: &symbi_runtime::reasoning::loop_types::LoopState,
        _: &LoopConfig,
    ) -> Result<Option<symbi_runtime::reasoning::prepared::ApprovalReceipt>, String> {
        Ok(None)
    }
    async fn evaluate_action(
        &self,
        _: &AgentId,
        _: &ProposedAction,
        _: &symbi_runtime::reasoning::loop_types::LoopState,
    ) -> symbi_runtime::reasoning::loop_types::LoopDecision {
        symbi_runtime::reasoning::loop_types::LoopDecision::Allow
    }
}

fn approval_gate() -> (Arc<dyn ReasoningPolicyGate>, Arc<EscalationQueue>) {
    let queue = Arc::new(EscalationQueue::new());
    let gate = Arc::new(EscalationGate::new(
        permissive(),
        queue.clone(),
        EscalationGateConfig {
            require_approval_tools: vec![],
            timeout: Duration::from_secs(10),
        },
    ));
    (gate, queue)
}
async fn resolve(queue: &EscalationQueue, approve: bool) -> symbi_runtime::escalation::HeldAction {
    let action = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let pending = queue.list_pending_async().await;
            if let Some(action) = pending.first() {
                assert_eq!(pending.len(), 1);
                break action.clone();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let review = symbi_runtime::escalation::render_approval_request(&action).unwrap();
    assert!(review.contains(&action.id));
    assert!(
        !review.contains("def components(path)"),
        "fixed worker implementation must not flood the review"
    );
    queue
        .resolve_async(
            &action.id,
            if approve {
                Decision::Approve {
                    reason: Some("reviewed fixture".into()),
                }
            } else {
                Decision::Deny {
                    reason: Some("denied fixture".into()),
                }
            },
            Approver {
                surface: Surface::Tui,
                id: "fixture-operator".into(),
                display: "Fixture operator".into(),
            },
        )
        .await
        .unwrap();
    action
}

#[test]
fn path_normalization_and_mapped_capabilities_precede_authorization() {
    assert_eq!(
        normalize_path("./folder//item.txt", false).unwrap(),
        "folder/item.txt"
    );
    for path in [
        "",
        "../outside",
        "folder/../outside",
        "/etc/passwd",
        "a\\b",
        ".git/config",
        ".symbiont/governed/key",
        "a\0b",
        ".",
    ] {
        assert!(normalize_path(path, false).is_err(), "{path:?}");
    }
    let project = project(true);
    std::fs::write(project.path().join("data/file"), "approved snapshot").unwrap();
    let executor = executor(project.path(), false);
    let config = LoopConfig {
        tool_definitions: executor.tool_definitions(),
        ..Default::default()
    };
    let action = |name: &str, args: serde_json::Value| ProposedAction::ToolCall {
        call_id: "fixture".into(),
        name: name.into(),
        arguments: args.to_string(),
    };
    let prepared = executor
        .prepare_action(
            &action("read_file", serde_json::json!({"path":"./file"})),
            &config,
        )
        .unwrap();
    assert_eq!(prepared.policy_context()["arguments"]["path"], "file");
    assert!(prepared.policy_context()["resolved"]["argv"].is_null());
    assert!(prepared.policy_context()["resolved"]["worker_program_hash"].is_string());
    let context = prepared.policy_context();
    assert_eq!(
        context["resolved"]["file_access"]["read"][0]["path"],
        "file"
    );
    assert_eq!(
        context["resolved"]["file_access"],
        context["resolved"]["command_boundary"]["filesystem"]
    );
    assert!(executor
        .prepare_action(
            &action(
                "edit_file",
                serde_json::json!({"path":"file","content":"x"})
            ),
            &config
        )
        .is_err());
    assert!(executor
        .prepare_action(
            &action("read_file", serde_json::json!({"path":"file","extra":"x"})),
            &config
        )
        .is_err());
    assert!(executor
        .prepare_action(
            &action("shell", serde_json::json!({"command":"true"})),
            &config
        )
        .is_err());
    executor
        .prepare_action(&action("search", serde_json::json!({"query":"x"})), &config)
        .unwrap();
}

#[tokio::test]
async fn missing_approval_cannot_be_bypassed_by_a_permissive_gate() {
    let project = project(false);
    let provider = Provider::new(vec![
        Step::Tools(vec![call(
            "edit_file",
            serde_json::json!({"path":"denied","content":"value"}),
        )]),
        Step::Reply,
    ]);
    let runtime = runtime(
        project.path(),
        provider,
        executor(project.path(), true),
        Arc::new(AllowWithoutReceipt),
    );
    let (_, entries) = run(&runtime, AgentId::new()).await;
    let observations = observations(&entries);
    assert!(observations.is_empty());
    assert!(entries.iter().any(|entry| matches!(&entry.event, LoopEvent::PolicyEvaluated { denied_count: 1, denied_calls, .. } if serde_json::to_string(denied_calls).unwrap().contains("exact-call approval receipt"))));
    assert!(!project.path().join("data/denied").exists());
}

#[tokio::test]
async fn unsafe_audit_storage_and_unavailable_boundary_stop_inference() {
    for unsafe_storage in [true, false] {
        let project = project(false);
        if unsafe_storage {
            let directory = project.path().join(".symbiont/governed");
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o777)).unwrap();
        } else {
            std::fs::write(
                project.path().join("symbiont.toml"),
                "[sandbox]\ntier=\"firecracker\"\n",
            )
            .unwrap();
        }
        let provider = Provider::new(vec![Step::Reply]);
        let runtime = runtime(
            project.path(),
            provider.clone(),
            executor(project.path(), true),
            permissive(),
        );
        let outcome = runtime
            .run(
                AgentId::new(),
                conversation(),
                LoopConfig::default(),
                Arc::new(TurnAudit::default()),
            )
            .await;
        assert!(
            outcome.is_err()
                || matches!(
                    outcome.unwrap().0.termination_reason,
                    TerminationReason::Error { .. }
                )
        );
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn cancelled_caller_retains_signed_terminal_audit() {
    let project = project(false);
    let provider = Provider::new(vec![Step::Wait]);
    let runtime = runtime(
        project.path(),
        provider.clone(),
        executor(project.path(), false),
        permissive(),
    );
    let audit = Arc::new(TurnAudit::default());
    let owned = audit.clone();
    let principal = AgentId::new();
    let task = tokio::spawn(async move {
        runtime
            .run(principal, conversation(), LoopConfig::default(), owned)
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while provider.calls.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let references = audit.references().unwrap();
            if !references.entries.is_empty() {
                let reference = &references.entries[0];
                let text = std::fs::read_to_string(&reference.path).unwrap();
                if text.contains("Terminated") {
                    let entries = records(reference, principal);
                    assert!(matches!(
                        entries.last().unwrap().event,
                        LoopEvent::Terminated {
                            reason: TerminationReason::Error { .. },
                            ..
                        }
                    ));
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_provider_failure_after_dispatch_does_not_replay_the_turn() {
    let project = project(false);
    let provider = Provider::new(vec![
        Step::Tools(vec![call("list_agents", serde_json::json!({}))]),
        Step::Fail,
    ]);
    let mut orchestrator = Orchestrator::new(
        provider.clone(),
        executor(project.path(), false),
        false,
        permissive(),
    )
    .with_project_root(Ok(project.path().to_owned()));
    let error = orchestrator
        .send("fixture")
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("503") && error.contains("audit run"));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    assert!(orchestrator
        .conversation()
        .messages()
        .iter()
        .any(
            |message| message.role == symbi_runtime::reasoning::conversation::MessageRole::Tool
                && message.tool_name.as_deref() == Some("list_agents")
        ));
    assert_eq!(orchestrator.audit_references().unwrap().entries.len(), 1);
}

#[tokio::test]
async fn broker_workspace_reads_and_search_preserve_scope_and_normalized_cedar_input() {
    let project = project(false);
    std::fs::write(
        project.path().join("data/allowed.txt"),
        "allowed fixture needle",
    )
    .unwrap();
    let canary = project.path().join("host-canary");
    std::fs::write(&canary, "HOST_SECRET_CANARY").unwrap();
    std::os::unix::fs::symlink(&canary, project.path().join("data/link")).unwrap();
    std::fs::hard_link(&canary, project.path().join("data/hardlink")).unwrap();
    let principal = AgentId::new();
    let gate = Arc::new(CedarPolicyGate::deny_by_default());
    gate.add_policy(CedarPolicy { name:"fixture".into(),active:true,source:format!("permit(principal, action, resource); forbid(principal, action == Action::\"tool_call::read_file\", resource) unless {{ principal == Agent::\"{principal}\" && context.invocation.arguments.path == \"allowed.txt\" }};") }).await;
    let provider = Provider::new(vec![
        Step::Tools(vec![
            call("read_file", serde_json::json!({"path":"./allowed.txt"})),
            call("read_file", serde_json::json!({"path":"link"})),
            call("search", serde_json::json!({"query":"needle"})),
            call("read_file", serde_json::json!({"path":"../host-canary"})),
        ]),
        Step::Reply,
    ]);
    let runtime = runtime(
        project.path(),
        provider,
        executor(project.path(), false),
        gate,
    );
    let (_, entries) = run(&runtime, principal).await;
    let observations = observations(&entries);
    assert_eq!(observations.len(), 2);
    assert!(observations
        .iter()
        .all(|result| !result.content.contains("def components(path)")));
    assert!(entries.iter().any(|entry| matches!(
        entry.event,
        LoopEvent::PolicyEvaluated {
            denied_count: 2,
            ..
        }
    )));
    assert_eq!(observations.iter().filter(|o| !o.is_error).count(), 2);
    assert!(observations.iter().any(|o| o.source == "toolclad:read_file"
        && !o.is_error
        && o.content.contains("allowed fixture needle")));
    assert!(observations.iter().any(|o| o.source == "toolclad:search"
        && !o.is_error
        && o.content.contains("allowed.txt:1:")));
    assert!(observations
        .iter()
        .all(|o| !o.content.contains("HOST_SECRET_CANARY")));
}

#[tokio::test]
async fn broker_writes_require_exact_approval_and_keep_the_selected_profile() {
    let project = project(false);
    let provider = Provider::new(vec![
        Step::Tools(vec![call(
            "edit_file",
            serde_json::json!({"path":"./approved","content":"actual approved effect"}),
        )]),
        Step::Reply,
    ]);
    let (gate, queue) = approval_gate();
    let runtime = runtime(
        project.path(),
        provider,
        executor(project.path(), false),
        gate,
    );
    let task = tokio::spawn(async move { run(&runtime, AgentId::new()).await });
    std::fs::write(
        project.path().join("symbiont.toml"),
        "[sandbox]\ntier=\"firecracker\"\n",
    )
    .unwrap();
    let held = resolve(&queue, true).await;
    assert_eq!(
        held.context_snapshot.unwrap()["invocation"]["arguments"]["path"],
        "approved"
    );
    let (_, entries) = task.await.unwrap();
    assert!(observations(&entries).iter().all(|o| !o.is_error));
    assert_eq!(
        std::fs::read_to_string(project.path().join("data/approved")).unwrap(),
        "actual approved effect"
    );
    assert!(!project.path().join("approved").exists());
    assert!(queue
        .resolve_async(
            &held.id,
            Decision::Approve { reason: None },
            Approver {
                surface: Surface::Tui,
                id: "fixture".into(),
                display: "fixture".into()
            }
        )
        .await
        .is_err());
}

#[tokio::test]
async fn broker_existing_edits_bind_approval_and_stop_after_conflicting_effects() {
    for changed in [false, true] {
        let project = project(false);
        let path = project.path().join("data/existing.txt");
        std::fs::write(&path, "prior content").unwrap();
        let provider = Provider::new(vec![
            Step::Tools(vec![call(
                "edit_file",
                serde_json::json!({"path":"existing.txt","content":"approved new content"}),
            )]),
            Step::Reply,
        ]);
        let (gate, queue) = approval_gate();
        let runtime = runtime(
            project.path(),
            provider.clone(),
            executor(project.path(), false),
            gate,
        );
        let task = tokio::spawn(async move { run(&runtime, AgentId::new()).await });
        tokio::time::timeout(Duration::from_secs(4), async {
            while queue.list_pending_async().await.is_empty() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "prior content");
        if changed {
            std::fs::write(&path, "concurrent change").unwrap();
        }
        let held = resolve(&queue, true).await;
        let context = held.context_snapshot.unwrap();
        assert_eq!(
            context["invocation"]["resolved"]["file_access"]["update"],
            serde_json::json!(["existing.txt"])
        );
        assert!(
            context["invocation"]["resolved"]["file_access"]["write"]["previous"]["sha256"]
                .is_string()
        );
        let (result, entries) = task.await.unwrap();
        let observed = observations(&entries);
        assert_eq!(observed.len(), 1);
        if changed {
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "concurrent change");
            assert!(
                observed[0].is_error && observed[0].content.contains("changed since authorization")
            );
            assert!(matches!(
                result.termination_reason,
                TerminationReason::UnconfirmedEffects
            ));
            assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        } else {
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                "approved new content"
            );
            assert!(!observed[0].is_error);
            assert!(matches!(
                result.termination_reason,
                TerminationReason::Completed
            ));
        }
    }
}

#[tokio::test]
#[ignore = "requires Docker and the cached Python fixture image"]
async fn docker_approved_shell_has_no_ambient_credentials_or_host_path_access() {
    let project = project(false);
    let canary = project.path().join("host-canary");
    std::fs::write(&canary, "HOST_SECRET_CANARY").unwrap();
    let command=format!("test ! -e '{}' && test -z \"$ANTHROPIC_API_KEY$OPENAI_API_KEY$SYMBI_DOCKER_AMBIENT_CANARY\" && printf contained > /workspace/shell-result && cat /workspace/shell-result",canary.display());
    let provider = Provider::new(vec![
        Step::Tools(vec![call("shell", serde_json::json!({"command":command}))]),
        Step::Reply,
    ]);
    let (gate, queue) = approval_gate();
    let runtime = runtime(
        project.path(),
        provider,
        executor(project.path(), true),
        gate,
    );
    let task = tokio::spawn(async move { run(&runtime, AgentId::new()).await });
    resolve(&queue, true).await;
    let (_, entries) = task.await.unwrap();
    assert!(observations(&entries).iter().all(|o| !o.is_error));
    assert!(!project.path().join("data/shell-result").exists());
    assert!(observations(&entries)
        .iter()
        .any(|o| !o.is_error && o.content.contains("contained")));
}

#[tokio::test]
async fn fleet_principal_and_registered_tools_are_retained() {
    let project = project(false);
    let bridge = Arc::new(
        repl_core::RuntimeBridge::new()
            .with_project_root(project.path())
            .unwrap(),
    );
    bridge
        .register_agent("reader", "Read only.", vec!["list_agents".into()])
        .await;
    let principal = bridge
        .agent_registry()
        .get_agent("reader")
        .await
        .unwrap()
        .agent_id;
    let provider = Provider::new(vec![
        Step::Tools(vec![
            call("list_agents", serde_json::json!({})),
            call(
                "edit_file",
                serde_json::json!({"path":"unregistered","content":"x"}),
            ),
        ]),
        Step::Reply,
    ]);
    let audit = Arc::new(TurnAudit::default());
    let factory = crate::fleet_runner::FleetRunnerFactory::new(
        provider,
        Arc::new(ProjectConstraints::default()),
        bridge.clone(),
        Arc::new(tokio::sync::RwLock::new(vec![])),
        true,
        permissive(),
    )
    .with_audit(audit.clone());
    let mut runner = factory
        .build(
            "reader",
            &["list_agents".into(), "edit_file".into(), "delegate".into()],
        )
        .await
        .unwrap();
    bridge
        .register_agent("reader", "Replaced.", vec!["edit_file".into()])
        .await;
    let response = runner.send("fixture").await.unwrap();
    let reference = response.audit.unwrap();
    assert_eq!(
        audit.references().unwrap().entries[0].run_id,
        reference.run_id
    );
    let entries = records(&reference, principal);
    assert!(entries.iter().any(|entry| matches!(
        entry.event,
        LoopEvent::PolicyEvaluated {
            denied_count: 1,
            ..
        }
    )));
    let observations = observations(&entries);
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].source, "list_agents");
    assert!(!observations[0].is_error);
    assert!(!project.path().join("data/unregistered").exists());
}

#[tokio::test]
async fn broker_artifact_content_is_exact_and_denial_has_no_effect() {
    for approve in [false, true] {
        let project = project(false);
        let content =
            "\n// exact whitespace and source syntax\npermit(principal, action, resource);\n";
        let provider = Provider::new(vec![
            Step::Tools(vec![call(
                "save_artifact",
                serde_json::json!({
                    "filename":"artifacts/fixture.cedar", "artifact_type":"cedar", "content":content
                }),
            )]),
            Step::Reply,
        ]);
        let (gate, queue) = approval_gate();
        let runtime = runtime(
            project.path(),
            provider,
            executor(project.path(), false),
            gate,
        );
        let task = tokio::spawn(async move { run(&runtime, AgentId::new()).await });
        let held = resolve(&queue, approve).await;
        assert_eq!(
            held.context_snapshot.unwrap()["invocation"]["arguments"]["content"],
            content
        );
        let (_, entries) = task.await.unwrap();
        let destination = project.path().join("data/artifacts/fixture.cedar");
        if approve {
            assert_eq!(std::fs::read_to_string(destination).unwrap(), content);
            assert_eq!(observations(&entries).len(), 1);
            assert!(!observations(&entries)[0].is_error);
        } else {
            assert!(!destination.exists());
            assert!(observations(&entries).is_empty());
            assert!(entries.iter().any(|entry| matches!(
                entry.event,
                LoopEvent::PolicyEvaluated {
                    denied_count: 1,
                    ..
                }
            )));
        }
    }
}

#[tokio::test]
async fn invalid_artifact_is_denied_before_approval() {
    let project = project(false);
    let provider = Provider::new(vec![
        Step::Tools(vec![call(
            "save_artifact",
            serde_json::json!({
                "filename":"invalid.cedar", "artifact_type":"cedar", "content":"this is not a policy"
            }),
        )]),
        Step::Reply,
    ]);
    let (gate, queue) = approval_gate();
    let runtime = runtime(
        project.path(),
        provider,
        executor(project.path(), false),
        gate,
    );
    let (_, entries) = run(&runtime, AgentId::new()).await;
    assert!(queue.list_pending_async().await.is_empty());
    assert!(observations(&entries).is_empty());
    assert!(!project.path().join("data/invalid.cedar").exists());
}

#[tokio::test]
async fn broker_refuses_symlinks_hardlinks_and_special_files() {
    let project = project(false);
    let data = project.path().join("data");
    let canary = project.path().join("host-canary");
    std::fs::write(&canary, "HOST_SECRET_CANARY").unwrap();
    std::os::unix::fs::symlink(&canary, data.join("link")).unwrap();
    std::fs::hard_link(&canary, data.join("hardlink")).unwrap();
    assert!(std::process::Command::new("mkfifo")
        .arg(data.join("fifo"))
        .status()
        .unwrap()
        .success());
    let provider = Provider::new(vec![
        Step::Tools(
            ["link", "hardlink", "fifo"]
                .into_iter()
                .map(|path| call("read_file", serde_json::json!({"path":path})))
                .collect(),
        ),
        Step::Reply,
    ]);
    let runtime = runtime(
        project.path(),
        provider,
        executor(project.path(), false),
        permissive(),
    );
    let (_, entries) = run(&runtime, AgentId::new()).await;
    let results = observations(&entries);
    assert!(results.is_empty());
    assert!(entries.iter().any(|entry| matches!(
        &entry.event,
        LoopEvent::PolicyEvaluated {
            denied_count: 3,
            ..
        }
    )));
    assert_eq!(
        std::fs::read_to_string(canary).unwrap(),
        "HOST_SECRET_CANARY"
    );
}

#[tokio::test]
#[ignore = "requires Docker and the cached Python fixture image"]
async fn docker_cancelled_turn_awaits_actual_removal_before_terminal_audit() {
    let project = project(false);
    let wrapper = project.path().join("docker-fixture");
    let removing = project.path().join("removing");
    std::fs::write(&wrapper, format!("#!/bin/sh\nif [ \"$1\" = rm ]; then printf removing > '{}'; /bin/sleep 1; fi\nexec /usr/bin/docker \"$@\"\n", removing.display())).unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let config_path = project.path().join("symbiont.toml");
    let config = std::fs::read_to_string(&config_path).unwrap();
    let parsed: toml::Value = toml::from_str(&config).unwrap();
    let label = parsed["sandbox"]["docker"]["extra_flags"][0]
        .as_str()
        .unwrap()
        .strip_prefix("--label=")
        .unwrap()
        .to_owned();
    std::fs::write(
        config_path,
        format!(
            "{config}\ndocker_binary = {}\n",
            serde_json::to_string(&wrapper).unwrap()
        ),
    )
    .unwrap();
    let command = "python3 -c 'import os,time; pid=os.fork(); (os.setsid(), open(\"/workspace/ready\",\"w\").write(\"ready\"), time.sleep(60)) if pid==0 else time.sleep(60)'";
    let provider = Provider::new(vec![Step::Tools(vec![call(
        "shell",
        serde_json::json!({"command":command}),
    )])]);
    let (gate, queue) = approval_gate();
    let runtime = runtime(
        project.path(),
        provider,
        executor(project.path(), true),
        gate,
    );
    let audit = Arc::new(TurnAudit::default());
    let retained = audit.clone();
    let principal = AgentId::new();
    let task = tokio::spawn(async move {
        runtime
            .run(principal, conversation(), LoopConfig::default(), retained)
            .await
    });
    resolve(&queue, true).await;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let workers = tokio::process::Command::new("docker")
                .args(["ps", "-q", "--filter", &format!("label={label}")])
                .output()
                .await
                .unwrap();
            assert!(workers.status.success());
            for id in String::from_utf8_lossy(&workers.stdout).split_whitespace() {
                let top = tokio::process::Command::new("docker")
                    .args(["top", id, "-eo", "pid,ppid,comm"])
                    .output()
                    .await
                    .unwrap();
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
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let reference = audit.references().unwrap().entries[0].clone();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(10), async {
        while !removing.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        !std::fs::read_to_string(&reference.path)
            .unwrap()
            .contains("Terminated"),
        "terminal audit must wait for the removal acknowledgement"
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        while !std::fs::read_to_string(&reference.path)
            .unwrap()
            .contains("Terminated")
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let entries = records(&reference, principal);
    assert!(
        matches!(&entries.last().unwrap().event, LoopEvent::Terminated { reason: TerminationReason::Error { message }, .. } if message == "agent execution cancelled")
    );
    let output = std::process::Command::new("docker")
        .args([
            "ps",
            "-a",
            "--filter",
            &format!("label={label}"),
            "--format",
            "{{.ID}}",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        output.stdout.is_empty(),
        "fixture worker or setsid descendant survived terminal audit"
    );
}

#[tokio::test]
async fn app_cancellation_finishes_shared_audit_and_closes_admission() {
    let project = project(false);
    let provider = Provider::new(vec![Step::Wait]);
    let audit = Arc::new(TurnAudit::default());
    let exec = executor(project.path(), false);
    let orchestrator = Orchestrator::new(provider.clone(), exec.clone(), false, permissive())
        .with_project_root(Ok(project.path().to_owned()))
        .with_audit(audit.clone());
    let bridge = Arc::new(
        repl_core::RuntimeBridge::new()
            .with_project_root(project.path())
            .unwrap(),
    );
    let mut app = crate::app::App::new(
        bridge,
        Some(orchestrator),
        Arc::new(tokio::sync::RwLock::new(vec![])),
        None,
    );
    assert!(app.send_to_orchestrator("fixture", "Waiting"));
    tokio::time::timeout(Duration::from_secs(3), async {
        while provider.active.load(Ordering::SeqCst) != 1 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    app.cancel_pending();
    app.shutdown_pending().await.unwrap();
    assert!(!app.is_busy());
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    let reference = audit.references().unwrap().entries[0].clone();
    let first: serde_json::Value = serde_json::from_str(
        std::fs::read_to_string(&reference.path)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    let principal = serde_json::from_value(first["payload"]["entry"]["agent_id"].clone()).unwrap();
    let entries = records(&reference, principal);
    assert!(
        matches!(&entries.last().unwrap().event, LoopEvent::Terminated { reason: TerminationReason::Error { message }, .. } if message == "agent execution cancelled")
    );
    app.on_tick().await;
    assert!(app
        .drain_unflushed()
        .iter()
        .any(|entry| entry.content.contains("Cancellation requested")));
    let runtime = runtime(project.path(), provider.clone(), exec, permissive());
    assert!(runtime
        .run(principal, conversation(), LoopConfig::default(), audit)
        .await
        .unwrap_err()
        .contains("admission is closed"));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
}
