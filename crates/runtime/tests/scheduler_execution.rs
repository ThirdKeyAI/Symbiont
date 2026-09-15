//! Real scheduled payloads, tool effects, protected audit and worker cleanup.
#![cfg(all(unix, feature = "cedar", feature = "cron"))]

use async_trait::async_trait;
use serde_json::{json, Value};
use std::{
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use symbi_runtime::{
    reasoning::{
        conversation::{Conversation, MessageRole},
        inference::{
            FinishReason, InferenceError, InferenceOptions, InferenceProvider, InferenceResponse,
            ToolCallRequest, Usage,
        },
        loop_types::LoopEvent,
        protected_journal::ProtectedJournal,
        CedarPolicy, CedarPolicyGate,
    },
    scheduler::{
        cron_scheduler::{CronScheduler, CronSchedulerConfig},
        cron_types::{CronJobDefinition, CronJobStatus, JobRunStatus},
        execution::GovernedAgentExecutor,
        task_manager::{TaskCompletion, TaskStatus},
        AgentScheduler, DefaultAgentScheduler, SchedulerConfig,
    },
    types::{AgentConfig, AgentId, ExecutionMode, ResourceLimits, SecurityTier},
};

struct Provider {
    calls: AtomicUsize,
}
#[async_trait]
impl InferenceProvider for Provider {
    async fn complete(
        &self,
        conversation: &Conversation,
        _: &InferenceOptions,
    ) -> Result<InferenceResponse, InferenceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let system = &conversation.messages()[0].content;
        assert!(system.contains("agent fixture"));
        assert!(!system.contains("agent sibling"));
        let (content, tool_calls, finish_reason) = if let Some(tool) = conversation
            .messages()
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::Tool)
        {
            (tool.content.clone(), vec![], FinishReason::Stop)
        } else {
            let input = conversation
                .messages()
                .iter()
                .find(|m| m.role == MessageRole::User)
                .unwrap();
            let payload: Value = serde_json::from_str(&input.content).unwrap();
            (
                String::new(),
                vec![ToolCallRequest {
                    id: "effect-1".into(),
                    name: "record_payload".into(),
                    arguments: payload.to_string(),
                }],
                FinishReason::ToolCalls,
            )
        };
        Ok(InferenceResponse {
            content,
            tool_calls,
            finish_reason,
            usage: Usage::default(),
            model: "local-fixture".into(),
        })
    }
    fn provider_name(&self) -> &str {
        "local-fixture"
    }
    fn default_model(&self) -> &str {
        "local-fixture"
    }
    fn supports_native_tools(&self) -> bool {
        true
    }
    fn supports_structured_output(&self) -> bool {
        false
    }
}
struct Fixture {
    root: tempfile::TempDir,
    scheduler: Arc<DefaultAgentScheduler>,
    config: AgentConfig,
    provider: Arc<Provider>,
}
impl Fixture {
    async fn new(approval: bool, permit: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let effects = root.path().join("effects");
        std::fs::create_dir(&effects).unwrap();
        std::fs::set_permissions(&effects, std::fs::Permissions::from_mode(0o777)).unwrap();
        let tools = root.path().join("tools");
        std::fs::create_dir(&tools).unwrap();
        std::fs::write(root.path().join("host-canary"), "synthetic-host-canary").unwrap();
        let source = format!(
            r#"from pathlib import Path; import sys, os, json, time; token=sys.argv[1]; delay=float(sys.argv[2]); p=Path("/workspace"); (p/(token+".started")).write_text("started"); print("started",flush=True); time.sleep(delay); result={{"token":token,"uid":os.getuid(),"host_visible":Path("{}").exists(),"ambient_visible":"SYMBI_DOCKER_AMBIENT_CANARY" in os.environ,"memory":Path("/sys/fs/cgroup/memory.max").read_text().strip(),"cpu":Path("/sys/fs/cgroup/cpu.max").read_text().strip()}}; text=json.dumps(result); (p/(token+".json")).write_text(text); print(text)"#,
            root.path().join("host-canary").display()
        );
        let manifest = format!(
            r#"
[tool]
name = "record_payload"
version = "1"
description = "Record a scheduled test payload"
binary = "python3"
timeout_seconds = 25
human_approval = {approval}
[args.token]
position = 1
required = true
type = "string"
[args.delay]
position = 2
required = true
type = "integer"
min = 0
max = 15
[command]
template = '''python3 -c '{source}' {{token}} {{delay}}'''
[output]
format = "text"
"#
        );
        std::fs::write(tools.join("record_payload.clad.toml"), manifest).unwrap();
        std::fs::write(root.path().join("symbiont.toml"),format!(
            "[sandbox]\ntier='firecracker'\n[sandbox.docker]\nimage='python:3.12-slim'\nvolumes=['{}:/workspace:rw']\nmemory_limit='256m'\ncpu_limit=0.5\n",effects.display())).unwrap();
        let gate = Arc::new(CedarPolicyGate::deny_by_default());
        gate.add_policy(CedarPolicy {
            name: "scheduled-fixture".into(),
            active: true,
            source: if permit {
                "permit(principal, action, resource);"
            } else {
                "permit(principal, action == Action::\"respond\", resource);"
            }
            .into(),
        })
        .await;
        let provider = Arc::new(Provider {
            calls: AtomicUsize::new(0),
        });
        let executor = GovernedAgentExecutor::new(root.path())
            .unwrap()
            .with_provider(provider.clone())
            .with_policy_gate(gate);
        let scheduler = Arc::new(
            DefaultAgentScheduler::new_with_executor(
                SchedulerConfig {
                    max_concurrent_agents: 3,
                    task_timeout: Duration::from_secs(30),
                    ..Default::default()
                },
                None,
                Arc::new(executor),
            )
            .await
            .unwrap(),
        );
        let config = AgentConfig { id: AgentId::new(), name: "fixture".into(),
            dsl_source: "agent sibling() { with sandbox = \"firecracker\" {} }\nagent fixture() { with sandbox = \"docker\", timeout = 25.seconds {} }".into(),
            execution_mode: ExecutionMode::Ephemeral, security_tier: SecurityTier::Tier1,
            resource_limits: ResourceLimits { memory_mb: 128, cpu_cores: 0.25, ..Default::default() },
            capabilities: vec![], policies: vec![], metadata: Default::default(), priority: Default::default() };
        Self {
            root,
            scheduler,
            config,
            provider,
        }
    }
    fn effect(&self, token: &str) -> std::path::PathBuf {
        self.root.path().join("effects").join(token)
    }
    fn verify(&self, result: &TaskCompletion) {
        let audit = result.audit.as_ref().expect("protected audit reference");
        let key: [u8; 32] = hex::decode(&audit.public_key).unwrap().try_into().unwrap();
        let entries = ProtectedJournal::verify_run(&audit.path, &key, result.run_id).unwrap();
        assert!(entries.iter().all(|e| e.agent_id == self.config.id));
        assert!(matches!(
            entries.last().unwrap().event,
            LoopEvent::Terminated { .. }
        ));
        assert_eq!(
            std::fs::read_to_string(self.root.path().join("host-canary")).unwrap(),
            "synthetic-host-canary"
        );
    }
}
async fn wait_file(path: &Path) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "requires Docker, python:3.12-slim and the sandbox supervisor"]
async fn scheduled_payloads_have_distinct_results_resources_and_signed_audits() {
    let fixture = Fixture::new(false, true).await;
    fixture
        .scheduler
        .register_agent(fixture.config.clone())
        .await
        .unwrap();
    assert_eq!(fixture.provider.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        fixture
            .scheduler
            .get_agent_status(fixture.config.id)
            .await
            .unwrap()
            .state,
        symbi_runtime::types::AgentState::Ready
    );
    let first = fixture
        .scheduler
        .schedule_invocation(fixture.config.clone(), json!({"token":"first","delay":"0"}))
        .await
        .unwrap();
    let second = fixture
        .scheduler
        .schedule_invocation(
            fixture.config.clone(),
            json!({"token":"second","delay":"0"}),
        )
        .await
        .unwrap();
    let (a, b) = tokio::join!(first.wait(), second.wait());
    assert_ne!(a.run_id, b.run_id);
    assert_ne!(
        a.audit.as_ref().unwrap().path,
        b.audit.as_ref().unwrap().path
    );
    assert_eq!(
        a.audit.as_ref().unwrap().public_key,
        b.audit.as_ref().unwrap().public_key
    );
    for (token, result) in [("first", a), ("second", b)] {
        assert_eq!(result.status, TaskStatus::Completed, "{result:?}");
        assert!(
            fixture.effect(&format!("{token}.json")).exists(),
            "scheduled effect missing: {result:?}"
        );
        let value: Value = serde_json::from_str(
            &std::fs::read_to_string(fixture.effect(&format!("{token}.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(value["token"], token);
        assert_eq!(value["uid"], 65534);
        assert_eq!(value["host_visible"], false);
        assert_eq!(value["ambient_visible"], false);
        assert_eq!(value["memory"], "134217728");
        let cpu: Vec<f64> = value["cpu"]
            .as_str()
            .unwrap()
            .split_whitespace()
            .map(|v| v.parse().unwrap())
            .collect();
        assert_eq!(cpu[0] / cpu[1], 0.25);
        assert!(result.output.as_ref().unwrap().contains(token));
        fixture.verify(&result);
    }
    fixture.scheduler.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires Docker, python:3.12-slim and the sandbox supervisor"]
async fn scheduled_policy_approval_and_selection_denials_have_no_effects() {
    for (approval, permit) in [(false, false), (true, true)] {
        let fixture = Fixture::new(approval, permit).await;
        let result = fixture
            .scheduler
            .execute_agent(
                fixture.config.clone(),
                json!({"token":"denied","delay":"0"}),
            )
            .await
            .unwrap();
        assert!(!fixture.effect("denied.started").exists(), "{result:?}");
        let output = result.output.as_deref().unwrap_or("");
        let expected = if approval { "approval" } else { "denied" };
        assert!(
            output.to_lowercase().contains(expected) || result.status == TaskStatus::Failed,
            "{result:?}"
        );
        assert!(
            !output.contains("schema validation"),
            "wrong refusal: {result:?}"
        );
        fixture.verify(&result);
        let signed = std::fs::read_to_string(&result.audit.as_ref().unwrap().path).unwrap();
        let expected = if approval {
            "required approval relay is unavailable"
        } else {
            "Cedar denied action"
        };
        assert!(
            signed.contains(expected),
            "wrong refusal in audit: {signed}"
        );
        fixture.scheduler.shutdown().await.unwrap();
    }
    let fixture = Fixture::new(false, true).await;
    let mut invalid = fixture.config.clone();
    invalid.dsl_source = "agent fixture() { with sandbox = \"firecracker\" {} }".into();
    let result = fixture
        .scheduler
        .execute_agent(invalid, json!({"token":"unavailable","delay":"0"}))
        .await
        .unwrap();
    assert_eq!(result.status, TaskStatus::Failed);
    assert!(
        result
            .error
            .as_deref()
            .unwrap()
            .contains("Firecracker tier selected without configuration"),
        "{result:?}"
    );
    assert_eq!(fixture.provider.calls.load(Ordering::SeqCst), 0);
    fixture.scheduler.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires Docker, python:3.12-slim and the sandbox supervisor"]
async fn scheduled_inline_policies_bind_source_and_restrict_real_effects() {
    let mut fixture = Fixture::new(false, true).await;
    fixture.config.dsl_source = r#"
policy global { deny: "record_payload" if invocation.arguments.token == "global_denied" }
agent sibling() { policy sibling_only { deny: true } }
agent fixture() {
    with sandbox = "docker", timeout = 25.seconds {}
    policy selected { deny: "record_payload" if invocation.arguments.token == "denied" }
}
"#
    .into();
    let source_hash =
        symbi_runtime::reasoning::prepared::digest_json(&json!(fixture.config.dsl_source)).unwrap();
    for token in ["denied", "global_denied", "allowed"] {
        let result = fixture
            .scheduler
            .execute_agent(fixture.config.clone(), json!({"token":token,"delay":"0"}))
            .await
            .unwrap();
        assert_eq!(result.status, TaskStatus::Completed, "{result:?}");
        assert_eq!(
            fixture.effect(&format!("{token}.started")).exists(),
            token == "allowed"
        );
        assert_eq!(
            fixture.effect(&format!("{token}.json")).exists(),
            token == "allowed"
        );
        fixture.verify(&result);
        let audit = result.audit.as_ref().unwrap();
        let key: [u8; 32] = hex::decode(&audit.public_key).unwrap().try_into().unwrap();
        let entries = ProtectedJournal::verify_run(&audit.path, &key, result.run_id).unwrap();
        let LoopEvent::Started {
            execution_context, ..
        } = &entries[0].event
        else {
            panic!("missing startup context");
        };
        let policy = &execution_context["source_policy"];
        assert_eq!(policy["source_hash"], source_hash);
        assert_eq!(policy["agent_name"], "fixture");
        assert_eq!(policy["policies"], json!(["global", "selected"]));
        let calls: Vec<_> = entries
            .iter()
            .flat_map(|entry| match &entry.event {
                LoopEvent::PolicyEvaluated { approved_calls, .. } => approved_calls.clone(),
                _ => vec![],
            })
            .collect();
        assert!(!calls.is_empty());
        assert!(calls.iter().all(|call| call["source_policy"] == *policy));
        if token == "allowed" {
            let value: Value = serde_json::from_str(
                &std::fs::read_to_string(fixture.effect("allowed.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(value["token"], "allowed");
        } else {
            let name = if token == "denied" {
                "selected"
            } else {
                "global"
            };
            assert!(
                result
                    .output
                    .as_ref()
                    .unwrap()
                    .contains(&format!("inline policy {name}")),
                "{result:?}"
            );
        }
    }
    assert_eq!(fixture.provider.calls.load(Ordering::SeqCst), 6);
    fixture.scheduler.shutdown().await.unwrap();
}

#[tokio::test]
async fn unsupported_scheduled_policy_is_refused_before_inference() {
    let mut fixture = Fixture::new(false, true).await;
    fixture.config.dsl_source = r#"agent fixture() {
        with sandbox = "docker" {}
        policy unsupported { require: true }
    }"#
    .into();
    let result = fixture
        .scheduler
        .execute_agent(
            fixture.config.clone(),
            json!({"token":"unsupported","delay":"0"}),
        )
        .await
        .unwrap();
    assert_eq!(result.status, TaskStatus::Failed, "{result:?}");
    assert!(result
        .error
        .as_ref()
        .unwrap()
        .contains("inline policy unsupported"));
    assert_eq!(fixture.provider.calls.load(Ordering::SeqCst), 0);
    assert!(!fixture.effect("unsupported.started").exists());
    fixture.scheduler.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires Docker, python:3.12-slim and the sandbox supervisor"]
async fn scheduled_cancellation_stops_the_worker_and_records_a_terminal_audit() {
    let fixture = Fixture::new(false, true).await;
    let handle = fixture
        .scheduler
        .schedule_invocation(
            fixture.config.clone(),
            json!({"token":"cancelled","delay":"8"}),
        )
        .await
        .unwrap();
    wait_file(&fixture.effect("cancelled.started")).await;
    handle.cancel();
    let result = tokio::time::timeout(Duration::from_secs(30), handle.wait())
        .await
        .unwrap();
    assert_eq!(result.status, TaskStatus::Terminated, "{result:?}");
    assert!(result.output.is_none());
    fixture.verify(&result);
    tokio::time::sleep(Duration::from_secs(9)).await;
    assert!(!fixture.effect("cancelled.json").exists());
    fixture.scheduler.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires Docker, python:3.12-slim and the sandbox supervisor"]
async fn cron_history_tracks_execution_until_real_payload_completion() {
    let fixture = Fixture::new(false, true).await;
    let cron = CronScheduler::new(
        CronSchedulerConfig {
            job_store_path: Some(fixture.root.path().join("cron.sqlite")),
            tick_interval: Duration::from_millis(50),
            ..Default::default()
        },
        fixture.scheduler.clone(),
    )
    .await
    .unwrap();
    let mut job = CronJobDefinition::new(
        "timer".into(),
        "* * * * * *".into(),
        "UTC".into(),
        fixture.config.clone(),
    );
    job.input = json!({"token":"timer","delay":"2"});
    job.one_shot = true;
    let id = cron.add_job(job).await.unwrap();
    wait_file(&fixture.effect("timer.started")).await;
    let history = cron.get_run_history(id, 10).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].status, JobRunStatus::Running);
    assert!(history[0].completed_at.is_none());
    assert_ne!(
        cron.get_job(id).await.unwrap().status,
        CronJobStatus::Completed
    );
    assert_eq!(cron.check_health().await.unwrap().global_active_runs, 1);
    assert!(cron
        .trigger_now(id)
        .await
        .unwrap_err()
        .to_string()
        .contains("concurrency"));
    let record = tokio::time::timeout(Duration::from_secs(25), async {
        loop {
            let records = cron.get_run_history(id, 10).await.unwrap();
            if records[0].status != JobRunStatus::Running {
                break records[0].clone();
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(record.status, JobRunStatus::Succeeded, "{record:?}");
    let result = record.execution.unwrap();
    fixture.verify(&result);
    assert!(result.output.unwrap().contains("timer"));
    assert!(fixture.effect("timer.json").exists());
    cron.shutdown().await;
    fixture.scheduler.shutdown().await.unwrap();
}
