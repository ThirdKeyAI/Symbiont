//! Workflow authority and invocation lifecycle through the real HTTP server.
#![cfg(all(unix, feature = "http-api", feature = "cedar"))]

use async_trait::async_trait;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use symbi_runtime::{
    api::{
        api_keys::{ApiKeyRecord, ApiKeyStore},
        server::{HttpApiConfig, HttpApiServer},
    },
    scheduler::{
        execution::{GovernedAgentExecutor, ScheduledAgentExecutor},
        task_manager::{TaskCompletion, TaskStatus},
        DefaultAgentScheduler, ScheduledTask,
    },
    secrets::SecretsConfig,
    types::{AgentConfig, AgentId, ExecutionMode, SecurityTier},
    AgentRuntime, RuntimeConfig,
};
use tokio::sync::{mpsc, Semaphore};
use tokio_util::sync::CancellationToken;

struct Provider;
#[async_trait]
impl symbi_runtime::reasoning::inference::InferenceProvider for Provider {
    async fn complete(
        &self,
        _: &symbi_runtime::reasoning::conversation::Conversation,
        _: &symbi_runtime::reasoning::inference::InferenceOptions,
    ) -> Result<
        symbi_runtime::reasoning::inference::InferenceResponse,
        symbi_runtime::reasoning::inference::InferenceError,
    > {
        use symbi_runtime::reasoning::inference::*;
        Ok(InferenceResponse {
            content: "fixture complete".into(),
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
            usage: Usage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: 2,
            },
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

struct ObservedInvocation {
    run_id: String,
    config: AgentConfig,
    input: Value,
}

struct ControlledExecutor {
    inner: GovernedAgentExecutor,
    observed: mpsc::UnboundedSender<ObservedInvocation>,
    finish: Arc<Semaphore>,
}

#[async_trait]
impl ScheduledAgentExecutor for ControlledExecutor {
    fn invocation_project(&self) -> Result<&std::path::Path, String> {
        self.inner.invocation_project()
    }

    async fn execute(
        &self,
        task: &ScheduledTask,
        budget: Duration,
        cancellation: CancellationToken,
    ) -> TaskCompletion {
        self.observed
            .send(ObservedInvocation {
                run_id: task.handle.run_id().to_string(),
                config: task.config.clone(),
                input: task.input.clone(),
            })
            .unwrap();
        tokio::select! {
            _ = cancellation.cancelled() => TaskCompletion::new(task, TaskStatus::Terminated, None),
            permit = self.finish.acquire() => {
                permit.unwrap().forget();
                if task.input["fail"] == true {
                    TaskCompletion::new(task, TaskStatus::Failed, Some("fixture refusal".into()))
                } else {
                    self.inner.execute(task,budget,cancellation.clone()).await
                }
            }
        }
    }
}

struct Fixture {
    runtime: Arc<AgentRuntime>,
    client: reqwest::Client,
    base: String,
    admin: String,
    scoped: String,
    agent: AgentId,
    observed: mpsc::UnboundedReceiver<ObservedInvocation>,
    finish: Arc<Semaphore>,
    server: tokio::task::JoinHandle<()>,
    _root: tempfile::TempDir,
}

impl Fixture {
    async fn new(concurrency: usize) -> Self {
        let root = tempfile::tempdir().unwrap();
        let agent = AgentId::new();
        let mut keys = Vec::new();
        let mut wires = Vec::new();
        for (id, scope) in [("admin", None), ("scoped", Some(vec![agent.to_string()]))] {
            let secret = uuid::Uuid::new_v4().to_string();
            keys.push(ApiKeyRecord {
                key_id: id.into(),
                key_hash: ApiKeyStore::hash_key(&secret).unwrap(),
                agent_scope: scope,
                description: "local workflow fixture".into(),
                created_at: "2026-01-01T00:00:00Z".into(),
                revoked: false,
            });
            wires.push(format!("{id}.{secret}"));
        }
        let key_path = root.path().join("keys.json");
        std::fs::write(&key_path, serde_json::to_vec(&keys).unwrap()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }

        let mut config = RuntimeConfig::default();
        config.scheduler.max_concurrent_agents = concurrency;
        config.context_manager.enable_persistence = false;
        config.context_manager.persistence_config.root_data_dir = root.path().join("data");
        config.context_manager.secrets_config =
            SecretsConfig::file_json(root.path().join("secrets.json"));
        config.logging.enabled = false;
        let scheduler_config = config.scheduler.clone();
        let mut runtime = AgentRuntime::new(config).await.unwrap();
        runtime.scheduler.shutdown().await.unwrap();
        let (sender, observed) = mpsc::unbounded_channel();
        let finish = Arc::new(Semaphore::new(0));
        let gate = Arc::new(symbi_runtime::reasoning::CedarPolicyGate::deny_by_default());
        gate.add_policy(symbi_runtime::reasoning::CedarPolicy {
            name: "fixture".into(),
            active: true,
            source: "permit(principal, action, resource);".into(),
        })
        .await;
        let inner = GovernedAgentExecutor::new(root.path())
            .unwrap()
            .with_provider(Arc::new(Provider))
            .with_policy_gate(gate);
        runtime.scheduler = Arc::new(
            DefaultAgentScheduler::new_with_executor(
                scheduler_config,
                None,
                Arc::new(ControlledExecutor {
                    inner,
                    observed: sender,
                    finish: finish.clone(),
                }),
            )
            .await
            .unwrap(),
        );
        runtime
            .scheduler
            .register_agent(AgentConfig {
                id: agent,
                name: "guarded".into(),
                dsl_source: "agent guarded() { policy boundary { deny: true } }".into(),
                execution_mode: ExecutionMode::Ephemeral,
                security_tier: SecurityTier::Tier1,
                resource_limits: Default::default(),
                capabilities: vec![],
                policies: vec![],
                metadata: Default::default(),
                priority: Default::default(),
            })
            .await
            .unwrap();
        let runtime = Arc::new(runtime);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let mut server = HttpApiServer::new(HttpApiConfig {
            bind_address: "127.0.0.1".into(),
            port,
            enable_cors: false,
            enable_tracing: false,
            enable_rate_limiting: false,
            api_keys_file: Some(key_path),
            serve_agents_md: false,
        })
        .with_runtime_provider(runtime.clone());
        let server = tokio::spawn(async move { server.start().await.unwrap() });
        tokio::time::timeout(Duration::from_secs(5), async {
            while tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_err()
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        Self {
            runtime,
            client: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap(),
            base: format!("http://127.0.0.1:{port}/api/v1"),
            admin: wires.remove(0),
            scoped: wires.remove(0),
            agent,
            observed,
            finish,
            server,
            _root: root,
        }
    }

    async fn post(&self, route: &str, key: &str, body: Value) -> (u16, Value) {
        self.post_id(route, key, body, uuid::Uuid::new_v4()).await
    }
    async fn post_id(&self, route: &str, key: &str, body: Value, id: uuid::Uuid) -> (u16, Value) {
        let response = self
            .client
            .post(format!("{}{route}", self.base))
            .bearer_auth(key)
            .header("Idempotency-Key", id.to_string())
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = response.status().as_u16();
        let body = response.text().await.unwrap();
        let value = serde_json::from_str(&body)
            .unwrap_or_else(|error| panic!("HTTP {status} {route}: {body:?}: {error}"));
        (status, value)
    }

    async fn history(&self) -> Value {
        self.client
            .get(format!("{}/agents/{}/history", self.base, self.agent))
            .bearer_auth(&self.scoped)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    async fn shutdown(self) {
        self.runtime.shutdown().await.unwrap();
        self.server.abort();
        let _ = self.server.await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoped_workflow_cannot_replace_registered_source() {
    let fixture = Fixture::new(0).await;
    let original = fixture
        .runtime
        .scheduler
        .get_agent_config(fixture.agent)
        .unwrap();
    let other = AgentId::new();
    let unauthenticated = fixture
        .client
        .post(format!("{}/workflows/execute", fixture.base))
        .json(&json!({"workflow_id": "agent replacement() {}", "parameters": {}}))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), 401);
    for id in [Some(fixture.agent), Some(other), None] {
        let mut request = json!({"workflow_id": "agent replacement() {}", "parameters": {}});
        if let Some(id) = id {
            request["agent_id"] = json!(id);
        }
        let (status, response) = fixture
            .post("/workflows/execute", &fixture.scoped, request)
            .await;
        assert_eq!(status, 403, "{response}");
        assert_eq!(response["code"], "ADMIN_REQUIRED");
        assert_eq!(
            fixture
                .runtime
                .scheduler
                .get_agent_config(fixture.agent)
                .unwrap()
                .dsl_source,
            original.dsl_source
        );
        assert!(fixture.runtime.scheduler.get_agent_config(other).is_none());
        assert_eq!(fixture.history().await["history"], json!([]));
    }
    let (status, result) = fixture
        .post(
            &format!("/agents/{}/execute", fixture.agent),
            &fixture.scoped,
            json!({"input": {"token": "registered-source"}}),
        )
        .await;
    assert_eq!(status, 200, "{result}");
    assert_eq!(result["status"], "queued");
    let run = result["execution_id"].as_str().unwrap();
    assert_ne!(uuid::Uuid::parse_str(run).unwrap(), fixture.agent.0);
    assert_eq!(fixture.history().await["history"][0]["execution_id"], run);
    assert_eq!(
        fixture
            .runtime
            .scheduler
            .get_agent_config(fixture.agent)
            .unwrap()
            .dsl_source,
        original.dsl_source
    );
    let (status, _) = fixture
        .post(
            &format!("/agents/{other}/execute"),
            &fixture.scoped,
            json!({"input": {}}),
        )
        .await;
    assert_eq!(status, 403);
    let (status, admission) = fixture
        .post(
            "/workflows/execute",
            &fixture.admin,
            json!({"workflow_id": "agent fresh() {}", "parameters": {}}),
        )
        .await;
    assert_eq!(status, 200, "{admission}");
    assert_eq!(admission["status"], "queued");
    let created: AgentId = serde_json::from_value(admission["agent_id"].clone()).unwrap();
    assert_ne!(created, fixture.agent);
    assert_eq!(
        fixture
            .runtime
            .scheduler
            .get_agent_config(created)
            .unwrap()
            .name,
        "fresh"
    );
    assert_ne!(admission["agent_id"], admission["execution_id"]);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workflow_returns_run_ids_and_delivers_parameters() {
    let mut fixture = Fixture::new(1).await;
    let mut previous = None;
    for (source, name, fail, terminal) in [
        ("agent selected() {}", "selected", false, "Completed"),
        (
            r#"metadata { name = "chosen" } agent sibling() {} agent chosen() {}"#,
            "chosen",
            true,
            "Unresolved",
        ),
    ] {
        let parameters =
            json!({"token": "workflow-input", "nested": [1, {"ok": true}], "fail": fail});
        let (status, result) = fixture
            .post(
                "/workflows/execute",
                &fixture.admin,
                json!({"workflow_id": source, "parameters": parameters, "agent_id": fixture.agent}),
            )
            .await;
        assert_eq!(status, 200, "{result}");
        assert_eq!(result["status"], "queued", "{result}");
        assert!(result.get("execution_started").is_none());
        let run = result["execution_id"].as_str().unwrap().to_owned();
        assert_ne!(uuid::Uuid::parse_str(&run).unwrap(), fixture.agent.0);
        assert_ne!(previous.as_ref(), Some(&run));
        let observed = tokio::time::timeout(Duration::from_secs(5), fixture.observed.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(observed.run_id, run);
        assert_eq!(observed.input, parameters);
        assert_eq!(observed.config.id, fixture.agent);
        assert_eq!(observed.config.name, name);
        assert_eq!(observed.config.dsl_source, source);
        let history = fixture.history().await;
        let entries = history["history"].as_array().unwrap();
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry["execution_id"] == run)
                .count(),
            1
        );
        assert_eq!(entries[0]["status"], "queued");
        fixture.finish.add_permits(1);
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let history = fixture.history().await;
                let entries = history["history"].as_array().unwrap();
                if entries
                    .iter()
                    .any(|e| e["execution_id"] == run && e["status"] == terminal)
                {
                    assert_eq!(
                        entries.iter().filter(|e| e["execution_id"] == run).count(),
                        2
                    );
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        previous = Some(run);
    }
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_workflow_does_not_replace_registered_source() {
    let fixture = Fixture::new(0).await;
    let original = fixture
        .runtime
        .scheduler
        .get_agent_config(fixture.agent)
        .unwrap();
    for source in [
        "agent broken( {",
        "metadata { description = \"no agent\" }",
        "agent duplicate() {} agent duplicate() {}",
        "agent invalid() { policy boundary { require: true } }",
        r#"metadata { name = "absent" } agent first() {} agent second() {}"#,
    ] {
        let (status, response) = fixture
            .post(
                "/workflows/execute",
                &fixture.admin,
                json!({"workflow_id": source, "parameters": {}, "agent_id": fixture.agent}),
            )
            .await;
        assert_eq!(status, 400, "{response}");
        assert_eq!(response["status"], "invalid_request");
        assert_eq!(
            fixture
                .runtime
                .scheduler
                .get_agent_config(fixture.agent)
                .unwrap()
                .dsl_source,
            original.dsl_source
        );
        assert_eq!(fixture.history().await["history"], json!([]));
    }
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saved_agent_result_is_bound_to_the_verified_api_key() {
    let mut fixture = Fixture::new(1).await;
    let mut config = fixture
        .runtime
        .scheduler
        .get_agent_config(fixture.agent)
        .unwrap();
    config.dsl_source = "agent guarded() {}".into();
    fixture
        .runtime
        .scheduler
        .register_agent(config)
        .await
        .unwrap();
    let path = format!("/agents/{}/execute", fixture.agent);
    let payload = json!({"input":{"token":"private"}});
    let id = uuid::Uuid::new_v4();
    let (status, first) = fixture
        .post_id(&path, &fixture.admin, payload.clone(), id)
        .await;
    assert_eq!(status, 200, "{first}");
    assert_eq!(first["status"], "queued");
    let observed = tokio::time::timeout(Duration::from_secs(5), fixture.observed.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(observed.run_id, first["execution_id"]);
    let (status, active) = fixture
        .post_id(&path, &fixture.admin, payload.clone(), id)
        .await;
    assert_eq!(status, 409);
    assert_eq!(active["status"], "in_progress");
    fixture.finish.add_permits(1);
    let cached = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let (status, result) = fixture
                .post_id(&path, &fixture.admin, payload.clone(), id)
                .await;
            if status == 200 {
                break result;
            }
            assert_eq!(status, 409);
            assert_eq!(result["status"], "in_progress");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(cached["status"], "completed");
    assert_eq!(cached["result"]["output"], "fixture complete");
    assert_eq!(cached["audit"], first["audit"]);
    let (status, other) = fixture
        .post_id(&path, &fixture.scoped, payload.clone(), id)
        .await;
    assert_eq!(status, 409);
    assert_eq!(other["status"], "conflict");
    assert!(other.get("audit").is_none() && other.get("result").is_none());
    let (status, changed) = fixture
        .post_id(&path, &fixture.admin, json!({"input":"changed"}), id)
        .await;
    assert_eq!(status, 409);
    assert_eq!(changed["status"], "conflict");
    assert!(fixture.observed.try_recv().is_err());
    fixture.shutdown().await;
}
