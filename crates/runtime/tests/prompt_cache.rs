//! Real HTTP transport and signed-journal coverage without paid provider calls.
#![cfg(all(unix, feature = "cloud-llm"))]

use async_trait::async_trait;
use axum::{extract::State, routing::post, Json, Router};
use serde_json::{json, Value};
use serial_test::serial;
use std::sync::{Arc, Mutex};
use symbi_runtime::{
    reasoning::{
        circuit_breaker::CircuitBreakerRegistry,
        context_manager::DefaultContextManager,
        conversation::{Conversation, ConversationMessage},
        executor::ActionExecutor,
        inference::*,
        loop_types::*,
        policy_bridge::DefaultPolicyGate,
        prompt_cache::*,
        protected_journal::ProtectedJournal,
        providers::cloud::CloudInferenceProvider,
        reasoning_loop::ReasoningLoopRunner,
        recovery::inspect_run,
        run_audit::RunAuditReference,
    },
    types::AgentId,
};

#[derive(Clone)]
struct Server {
    direct: bool,
    invalid: bool,
    requests: Arc<Mutex<Vec<Value>>>,
}

async fn respond(State(state): State<Server>, Json(body): Json<Value>) -> Json<Value> {
    let mut requests = state.requests.lock().unwrap();
    let first = requests.is_empty();
    requests.push(body);
    if state.direct {
        Json(json!({
            "model":"fixture", "stop_reason":if first {"tool_use"} else {"end_turn"},
            "content":if first {json!([{"type":"tool_use","id":"one","name":"lookup","input":{}}])}
                else {json!([{"type":"text","text":"verified result"}])},
            "usage":{"input_tokens":10,"output_tokens":5,
                "cache_creation_input_tokens":if first {90} else {0},
                "cache_read_input_tokens":if state.invalid {json!(-1)} else {json!(if first {0} else {90})}}
        }))
    } else {
        Json(json!({
            "model":"anthropic/fixture", "choices":[{
                "finish_reason":if first {"tool_calls"} else {"stop"},
                "message":if first {json!({"role":"assistant","content":null,"tool_calls":[{
                    "id":"one","type":"function","function":{"name":"lookup","arguments":"{}"}
                }]})} else {json!({"role":"assistant","content":"verified result"})}
            }],
            "usage":{"prompt_tokens":100,"completion_tokens":5,"total_tokens":105,
                "prompt_tokens_details":{"cached_tokens":if state.invalid {json!(-1)} else {json!(if first {0} else {90})},
                    "cache_write_tokens":if first {90} else {0}}}
        }))
    }
}

struct AuditedTransport {
    inner: CloudInferenceProvider,
    audit: RunAuditReference,
}

#[async_trait]
impl InferenceProvider for AuditedTransport {
    fn prepare_options(&self, options: &mut InferenceOptions) -> Result<(), InferenceError> {
        self.inner.prepare_options(options)
    }
    fn input_token_reservation(
        &self,
        conversation: &Conversation,
        options: &InferenceOptions,
    ) -> Result<u32, InferenceError> {
        self.inner.input_token_reservation(conversation, options)
    }
    async fn complete(
        &self,
        conversation: &Conversation,
        options: &InferenceOptions,
    ) -> Result<InferenceResponse, InferenceError> {
        assert_eq!(options.prompt_cache()?.unwrap().mode, PromptCache::Rolling);
        let key: [u8; 32] = hex::decode(&self.audit.public_key)
            .unwrap()
            .try_into()
            .unwrap();
        let entries =
            ProtectedJournal::verify_run(&self.audit.path, &key, self.audit.run_id).unwrap();
        let LoopEvent::BudgetReservationStarted { reservation } = &entries.last().unwrap().event
        else {
            panic!("transport started without durable budget reservation")
        };
        assert_eq!(
            reservation.request_hash,
            symbi_runtime::reasoning::prepared::digest_json(
                &json!({"conversation":conversation,"options":options})
            )
            .unwrap()
        );
        self.inner.complete(conversation, options).await
    }
    fn provider_name(&self) -> &str {
        self.inner.provider_name()
    }
    fn default_model(&self) -> &str {
        self.inner.default_model()
    }
    fn supports_native_tools(&self) -> bool {
        true
    }
    fn supports_structured_output(&self) -> bool {
        true
    }
}

struct Executor(Arc<Mutex<usize>>);
#[async_trait]
impl ActionExecutor for Executor {
    async fn execute_actions(
        &self,
        actions: &[ProposedAction],
        _: &LoopConfig,
        _: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        actions
            .iter()
            .map(|action| {
                let ProposedAction::ToolCall { name, call_id, .. } = action else {
                    panic!("unexpected action")
                };
                *self.0.lock().unwrap() += 1;
                Observation::tool_result(name.clone(), "fixture evidence")
                    .with_call_id(call_id.clone())
            })
            .collect()
    }
}

fn provider(direct: bool, base: &str) -> CloudInferenceProvider {
    let names = [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_MODEL",
        "ANTHROPIC_BASE_URL",
        "OPENROUTER_API_KEY",
        "OPENROUTER_MODEL",
        "OPENROUTER_BASE_URL",
        "OPENAI_API_KEY",
        "BEDROCK_MODEL_ID",
    ];
    let previous: Vec<_> = names
        .iter()
        .map(|name| (*name, std::env::var_os(name)))
        .collect();
    for name in names {
        std::env::remove_var(name);
    }
    let kind = if direct { "ANTHROPIC" } else { "OPENROUTER" };
    std::env::set_var(format!("{kind}_API_KEY"), "synthetic-transport-fixture");
    std::env::set_var(
        format!("{kind}_MODEL"),
        if direct {
            "fixture"
        } else {
            "anthropic/fixture"
        },
    );
    std::env::set_var(format!("{kind}_BASE_URL"), base);
    let result = CloudInferenceProvider::from_env();
    for (name, value) in previous {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }
    result.unwrap()
}

#[tokio::test]
#[serial]
async fn rolling_cache_http_loop_preserves_raw_budget_and_signed_contract() {
    for direct in [true, false] {
        for invalid in [false, true] {
            let requests = Arc::new(Mutex::new(Vec::new()));
            let server = Server {
                direct,
                invalid,
                requests: requests.clone(),
            };
            let app = Router::new()
                .route("/v1/messages", post(respond))
                .route("/v1/chat/completions", post(respond))
                .with_state(server);
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}/v1", listener.local_addr().unwrap());
            let serving = tokio::spawn(async { axum::serve(listener, app).await.unwrap() });
            let reports = Arc::new(Mutex::new(Vec::new()));
            let observed = reports.clone();
            let client = provider(direct, &base)
                .with_prompt_cache(PromptCacheConfig {
                    mode: PromptCache::Rolling,
                    ttl: CacheTtl::FiveMinutes,
                })
                .with_cache_usage_observer(Arc::new(move |event| {
                    observed.lock().unwrap().push(event.clone())
                }));
            let root = tempfile::tempdir().unwrap();
            let principal = AgentId::new();
            let journal = Arc::new(
                ProtectedJournal::create_run(
                    &root.path().join("audit"),
                    principal,
                    uuid::Uuid::new_v4(),
                )
                .unwrap(),
            );
            let audit = journal.audit_reference().unwrap();
            let effects = Arc::new(Mutex::new(0));
            let runner = ReasoningLoopRunner {
                provider: Arc::new(AuditedTransport {
                    inner: client,
                    audit: audit.clone(),
                }),
                executor: Arc::new(Executor(effects.clone())),
                policy_gate: Arc::new(DefaultPolicyGate::permissive_for_dev_only()),
                context_manager: Arc::new(DefaultContextManager::default()),
                circuit_breakers: Arc::new(CircuitBreakerRegistry::default()),
                journal,
                knowledge_bridge: None,
                delegation: None,
            };
            let config = LoopConfig {
                max_total_tokens: 20000,
                max_output_tokens: 100,
                max_iterations: 3,
                tool_definitions: vec![ToolDefinition {
                    name: "lookup".into(),
                    description: "Read fixture evidence".into(),
                    parameters: json!({"type":"object"}),
                }],
                ..Default::default()
            };
            let mut conversation = Conversation::with_system("Stable policy");
            conversation.push(ConversationMessage::user("Review fixture"));
            let result = runner.run(principal, conversation, config).await;
            serving.abort();
            let key: [u8; 32] = hex::decode(&audit.public_key).unwrap().try_into().unwrap();
            let recovery = inspect_run(&audit.path, &key, audit.run_id).unwrap();
            let budget = recovery.recovered_budget.unwrap();
            let requests = requests.lock().unwrap();
            let reports = reports.lock().unwrap();
            for request in requests.iter() {
                assert!(request.get("symbi_prompt_cache").is_none());
                assert_eq!(request["cache_control"], json!({"type":"ephemeral"}));
            }
            if invalid {
                assert!(matches!(
                    result.termination_reason,
                    TerminationReason::Error { .. }
                ));
                assert!(result.output.is_empty());
                assert_eq!(*effects.lock().unwrap(), 0);
                assert!(reports.is_empty());
                assert!(budget.scopes[0].uncertain_tokens > 0);
            } else {
                assert!(
                    matches!(result.termination_reason, TerminationReason::Completed),
                    "{:?}",
                    result.termination_reason
                );
                assert_eq!(result.output, "verified result");
                assert_eq!(*effects.lock().unwrap(), 1);
                assert_eq!(requests.len(), 2);
                assert_eq!(requests[0]["messages"][0], requests[1]["messages"][0]);
                assert_eq!(reports.len(), 2);
                assert_eq!(reports[0].reported.cache.cache_creation_tokens, Some(90));
                assert_eq!(reports[1].reported.cache.cache_read_tokens, Some(90));
                assert_eq!(budget.scopes[0].usage.total_tokens, 210);
                assert_eq!(budget.scopes[0].available_tokens, 19790);
                assert_eq!(budget.scopes[0].uncertain_tokens, 0);
                assert!(!recovery.requires_reconciliation);
            }
        }
    }
}
