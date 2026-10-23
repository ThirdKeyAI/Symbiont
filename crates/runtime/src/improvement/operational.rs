//! Explicit operator selection and a stable evaluation profile for queued runs.

use crate::{
    reasoning::{
        conversation::{Conversation, ConversationMessage},
        loop_types::LoopConfig,
    },
    types::AgentConfig,
};
use dsl::AgentExecutionSettings;

pub const WORKFLOW_METADATA: &str = "improvement_workflow";

/// Registration belongs to the operator. Invocation payloads cannot override it.
pub fn workflow(config: &AgentConfig) -> Result<Option<String>, String> {
    let Some(workflow) = config.metadata.get(WORKFLOW_METADATA) else {
        return Ok(None);
    };
    let workflow = normalize(workflow)?;
    if matches!(
        config.execution_mode,
        crate::types::ExecutionMode::External { .. }
    ) {
        return Err("external agents do not support governed improvement selection".into());
    }
    Ok(Some(workflow))
}

fn normalize(value: &str) -> Result<String, String> {
    let workflow = if value.starts_with('"') {
        serde_json::from_str::<String>(value).map_err(|e| e.to_string())?
    } else {
        value.to_owned()
    };
    super::identifier(&workflow)?;
    Ok(workflow)
}

/// Copy only the explicit opt-in from operator-owned source into registration.
pub fn registration_metadata(
    source: &str,
) -> Result<std::collections::HashMap<String, String>, String> {
    let tree = dsl::parse_dsl(source).map_err(|e| e.to_string())?;
    let metadata = dsl::extract_metadata(&tree, source);
    let mut selected = std::collections::HashMap::new();
    if let Some(workflow) = metadata.get(WORKFLOW_METADATA) {
        selected.insert(WORKFLOW_METADATA.into(), normalize(workflow)?);
    }
    Ok(selected)
}

/// Matches the default ordinary CLI profile used to collect acceptance evidence.
/// Tighter route deadlines are enforced by cancellation, independently of this
/// evaluated configuration; queue time must never change the approved hashes.
pub fn profile(settings: &AgentExecutionSettings, input: &str) -> (Conversation, LoopConfig) {
    let mut conversation = Conversation::with_system(format!(
        "You are agent '{}'. Follow the governance rules defined in your DSL.\n\n--- Agent DSL ---\n{}\n--- End DSL ---",
        settings.agent_name, settings.agent_source,
    ));
    conversation.push(ConversationMessage::user(input));
    let config = LoopConfig {
        max_iterations: 10,
        max_total_tokens: 100_000,
        timeout: settings
            .timeout_seconds
            .map(std::time::Duration::from_secs)
            .map_or(LoopConfig::default().timeout, |timeout| {
                timeout.min(LoopConfig::default().timeout)
            }),
        ..Default::default()
    };
    (conversation, config)
}

#[cfg(unix)]
pub async fn pin_registered(
    project: &std::path::Path,
    config: &AgentConfig,
) -> Result<Option<super::PinnedImprovement>, String> {
    let Some(workflow) = workflow(config)? else {
        return Ok(None);
    };
    let project = project.to_owned();
    let config = config.clone();
    tokio::task::spawn_blocking(move || {
        let policy = dsl::ExecutionPolicy::parse(&config.dsl_source, &config.name)?;
        let settings = policy.settings();
        let tree = dsl::parse_dsl(&config.dsl_source).map_err(|e| e.to_string())?;
        let metadata = dsl::extract_metadata(&tree, &config.dsl_source);
        if metadata
            .get("executor")
            .is_some_and(|kind| kind.trim_matches('"') != "orga")
        {
            return Err("governed improvements require an ordinary ORGA agent".into());
        }
        super::Store::open(&project, &workflow)?
            .pin(&settings.agent_name, &settings.agent_source, None)
            .map(Some)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(all(test, unix))]
pub(crate) mod tests {
    use super::*;
    use crate::{
        improvement::*,
        reasoning::{
            circuit_breaker::CircuitBreakerRegistry, context_manager::DefaultContextManager,
            executor::UnavailableToolExecutor, inference::*, loop_types::*,
            policy_bridge::DefaultPolicyGate, protected_journal::ProtectedJournal,
            reasoning_loop::ReasoningLoopRunner,
        },
        types::AgentId,
    };
    use std::{
        path::Path,
        sync::{Arc, Mutex},
    };

    #[derive(Default)]
    pub(crate) struct Provider {
        pub seen: Mutex<Vec<String>>,
    }
    #[async_trait::async_trait]
    impl InferenceProvider for Provider {
        async fn complete(
            &self,
            conversation: &Conversation,
            _: &InferenceOptions,
        ) -> Result<InferenceResponse, InferenceError> {
            self.seen.lock().unwrap().push(
                conversation
                    .messages()
                    .iter()
                    .map(|m| m.content.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            Ok(InferenceResponse {
                content: "expected".into(),
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
        fn configuration_identity(&self) -> Option<String> {
            Some(sha256(b"operational fixture"))
        }
        fn supports_native_tools(&self) -> bool {
            true
        }
        fn supports_structured_output(&self) -> bool {
            false
        }
    }

    pub(crate) async fn release(
        root: &Path,
        config: &AgentConfig,
        provider: Arc<Provider>,
        marker: &str,
        initialize: bool,
    ) -> (String, String) {
        let policy = dsl::ExecutionPolicy::parse(&config.dsl_source, &config.name).unwrap();
        let settings = policy.settings();
        let mut store = if initialize {
            Store::initialize(
                root,
                "claims",
                &settings.agent_name,
                &settings.agent_source,
                AcceptanceSuite {
                    schema_version: 1,
                    minimum_passed: 1,
                    maximum_tokens_per_case: 100,
                    maximum_denied_calls: 0,
                    cases: vec![AcceptanceCase {
                        id: "case".into(),
                        input: "input".into(),
                        expected_output: "expected".into(),
                    }],
                },
            )
            .unwrap()
        } else {
            Store::open(root, "claims").unwrap()
        };
        let candidate = store
            .propose(Proposal {
                instructions: marker.into(),
                rationale: "Operational evidence".into(),
                evidence: vec![],
            })
            .unwrap();
        let pin = store
            .pin(
                &settings.agent_name,
                &settings.agent_source,
                Some(&candidate),
            )
            .unwrap();
        let agent_id = AgentId::new();
        let journal = Arc::new(
            ProtectedJournal::create_run(
                &root.join(".symbiont/governed"),
                agent_id,
                uuid::Uuid::new_v4(),
            )
            .unwrap(),
        );
        let audit = journal.audit_reference().unwrap();
        let runner = ReasoningLoopRunner {
            provider,
            policy_gate: Arc::new(DefaultPolicyGate::new()),
            executor: Arc::new(UnavailableToolExecutor),
            context_manager: Arc::new(DefaultContextManager::default()),
            circuit_breakers: Arc::new(CircuitBreakerRegistry::default()),
            journal,
            knowledge_bridge: None,
            delegation: None,
        };
        let (conversation, config) = profile(settings, "input");
        let result = runner
            .run_with_improvement(agent_id, conversation, config, &pin)
            .await
            .unwrap();
        assert!(matches!(
            result.termination_reason,
            TerminationReason::Completed
        ));
        let (evaluation, report) = store
            .evaluate(
                &candidate,
                &[evaluation::TrialReference {
                    case_id: "case".into(),
                    audit,
                }],
                &evaluation::ExactAnswerEvaluator,
            )
            .unwrap();
        assert!(report.accepted);
        let approval = store
            .approve(&candidate, &evaluation, "Reviewed acceptance evidence")
            .unwrap();
        let expected = store.state().active.clone();
        store
            .activate(&candidate, &approval, expected.as_deref(), false)
            .unwrap();
        (candidate, approval)
    }

    #[test]
    fn opt_in_metadata_is_bounded_and_default_is_unchanged() {
        assert!(registration_metadata("agent claims {}").unwrap().is_empty());
        assert_eq!(normalize("\"claims\"").unwrap(), "claims");
        for invalid in ["", "../claims", "claims/other", "\"claims", " claims "] {
            assert!(normalize(invalid).is_err());
        }
    }
}
