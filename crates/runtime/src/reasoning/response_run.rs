//! A bounded, audited text response from a registered conversational agent.
//! This route never dispatches tools or interprets executable DSL statements.
use super::{
    circuit_breaker::CircuitBreakerRegistry,
    context_manager::DefaultContextManager,
    conversation::{Conversation, ConversationMessage},
    executor::ActionExecutor,
    inference::{InferenceError, InferenceOptions, InferenceProvider, InferenceResponse},
    loop_types::{
        JournalEntry, JournalWriter, LoopConfig, LoopEvent, Observation, ProposedAction,
        TerminationReason,
    },
    policy_bridge::ReasoningPolicyGate,
    prepared::{canonical_json, digest_json, PreparedAction},
    reasoning_loop::ReasoningLoopRunner,
    run_audit::{open_run_journal, RunAuditReference},
    source_policy::SourcePolicyExecutor,
};
use crate::types::AgentId;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

/// Construct from trusted startup configuration and a validated registry entry.
pub struct ResponseRequest {
    pub project: PathBuf,
    pub agent: dsl::ConversationalAgent,
    pub surface: String,
    pub input: String,
    /// Additional caller instructions are input, never runtime authority.
    pub caller_instructions: Option<String>,
    pub provider: Arc<dyn InferenceProvider>,
    pub gate: Arc<dyn ReasoningPolicyGate>,
    pub cancellation: tokio_util::sync::CancellationToken,
}

pub struct ResponseOutcome {
    pub agent_id: AgentId,
    pub audit: Option<RunAuditReference>,
    pub result: Result<String, String>,
}

/// Names share a stable response principal across filename aliases and runs.
pub fn response_agent_id(name: &str) -> AgentId {
    AgentId(uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        format!("symbi://response/{name}").as_bytes(),
    ))
}

/// Retain initialization, cancellation and terminal writes if the caller drops
/// its future. Returning text requires both response authorization and durable
/// terminal audit. It establishes release to this caller, not external delivery.
pub async fn run_response(request: ResponseRequest) -> ResponseOutcome {
    run_owned(request, None).await
}

/// Run through the same inference and response gate, delivering the frozen,
/// formatted response before recording successful terminal completion.
pub async fn run_response_to(
    request: ResponseRequest,
    destination: Arc<dyn super::response_delivery::ResponseDestination>,
) -> ResponseOutcome {
    run_owned(request, Some(destination)).await
}

async fn run_owned(
    request: ResponseRequest,
    destination: Option<Arc<dyn super::response_delivery::ResponseDestination>>,
) -> ResponseOutcome {
    let agent_id = response_agent_id(&request.agent.settings().agent_name);
    if request.input.len() > 64 * 1024
        || request
            .caller_instructions
            .as_ref()
            .is_some_and(|value| value.len() > 64 * 1024)
    {
        return ResponseOutcome {
            agent_id,
            audit: None,
            result: Err("response input exceeds 64 KiB".into()),
        };
    }
    let cancellation = request.cancellation.clone();
    let _cancel_on_drop = cancellation.clone().drop_guard();
    let task = tokio::spawn(async move {
        let (journal, audit) = match open_run_journal(&request.project, agent_id).await {
            Ok(opened) => opened,
            Err(error) => {
                return ResponseOutcome {
                    agent_id,
                    audit: None,
                    result: Err(format!("required response audit unavailable: {error}")),
                }
            }
        };
        let config = LoopConfig {
            max_iterations: 1,
            max_output_tokens: 4096,
            max_total_tokens: 16_384,
            timeout: Duration::from_secs(
                request
                    .agent
                    .settings()
                    .timeout_seconds
                    .unwrap_or(120)
                    .min(120),
            ),
            ..Default::default()
        };
        let metadata = json!({
            "name": request.agent.settings().agent_name,
            "source_hash": digest_json(&json!(request.agent.source())).expect("source is JSON serializable"),
            "declaration_hash": digest_json(&json!(request.agent.settings().agent_source)).expect("source is JSON serializable"),
            "mode": "text_response", "surface": request.surface,
            "tools_enabled": false,
        });
        let source_hash = metadata["source_hash"].as_str().unwrap().to_owned();
        let executor = Arc::new(SourcePolicyExecutor::new(
            Arc::new(ResponseExecutor {
                metadata,
                destination,
            }),
            request.agent.policy().clone(),
        ));
        let mut conversation = Conversation::with_system(format!(
            "You are agent '{}'. Produce a text response to the caller. Tool execution is unavailable on this route.\n\n--- Selected agent definition ---\n{}\n--- End definition ---",
            request.agent.settings().agent_name, request.agent.settings().agent_source));
        if let Some(instructions) = request.caller_instructions {
            conversation.push(ConversationMessage::user(format!(
                "Additional caller instructions:\n{instructions}"
            )));
        }
        conversation.push(ConversationMessage::user(&request.input));
        let runner = ReasoningLoopRunner {
            provider: Arc::new(AuditedProvider {
                inner: request.provider,
                journal: journal.clone(),
                agent_id,
                operation: request.surface,
                source_hash,
            }),
            executor,
            policy_gate: request.gate,
            context_manager: Arc::new(DefaultContextManager::default()),
            circuit_breakers: Arc::new(CircuitBreakerRegistry::default()),
            journal,
            knowledge_bridge: None,
            delegation: None,
        };
        let result = runner
            .run_cancellable(agent_id, conversation, config, cancellation)
            .await;
        let result = match result.termination_reason {
            TerminationReason::Completed => Ok(result.output),
            reason => Err(format!("registered response did not complete: {reason:?}")),
        };
        ResponseOutcome {
            agent_id,
            audit: Some(audit),
            result,
        }
    });
    task.await.unwrap_or_else(|error| ResponseOutcome {
        agent_id,
        audit: None,
        result: Err(format!("response owner failed: {error}")),
    })
}

struct ResponseExecutor {
    metadata: Value,
    destination: Option<Arc<dyn super::response_delivery::ResponseDestination>>,
}
#[async_trait]
impl ActionExecutor for ResponseExecutor {
    fn execution_context(&self) -> HashMap<String, Value> {
        let mut context = HashMap::from([("agent_definition".into(), self.metadata.clone())]);
        if let Some(destination) = &self.destination {
            context.insert("response_destination".into(), destination.context());
        }
        context
    }
    fn validate_configuration(&self) -> Result<(), String> {
        if let Some(destination) = &self.destination {
            destination.validate()?;
            if canonical_json(&destination.context())?.len() > 64 * 1024 {
                return Err("response destination exceeds 64 KiB".into());
            }
        }
        Ok(())
    }
    fn prepare_action(
        &self,
        action: &ProposedAction,
        _: &LoopConfig,
    ) -> Result<PreparedAction, String> {
        let ProposedAction::Respond { content } = action else {
            return Err("registered response route permits text responses only".into());
        };
        let prepared = PreparedAction::new(action.clone(), None)?;
        match &self.destination {
            Some(destination) => {
                super::response_delivery::prepare(prepared, destination.clone(), content)
            }
            None => Ok(prepared),
        }
    }
    async fn execute_actions(
        &self,
        _: &[ProposedAction],
        _: &LoopConfig,
        _: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        vec![Observation::tool_error(
            "response",
            "tool execution is unavailable on the response route",
        )]
    }
}

struct AuditedProvider {
    inner: Arc<dyn InferenceProvider>,
    journal: Arc<dyn JournalWriter>,
    agent_id: AgentId,
    operation: String,
    source_hash: String,
}
impl AuditedProvider {
    async fn append(&self, event: LoopEvent) -> Result<(), InferenceError> {
        self.journal
            .append(JournalEntry {
                sequence: self.journal.next_sequence().await,
                timestamp: chrono::Utc::now(),
                agent_id: self.agent_id,
                iteration: 1,
                event,
            })
            .await
            .map_err(|error| {
                InferenceError::Provider(format!("required inference audit failed: {error}"))
            })
    }
}

fn contract(value: &Value, limit: usize) -> Result<(String, u64), InferenceError> {
    let bytes = canonical_json(value).map_err(InferenceError::Provider)?;
    if bytes.len() > limit {
        return Err(InferenceError::Provider(
            "typed inference contract exceeds its byte limit".into(),
        ));
    }
    Ok((
        digest_json(value).map_err(InferenceError::Provider)?,
        bytes.len() as u64,
    ))
}

#[async_trait]
impl InferenceProvider for AuditedProvider {
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
        let call_id = uuid::Uuid::new_v4().to_string();
        let (request_hash, request_bytes) = contract(
            &json!({"conversation":conversation,"options":options}),
            1024 * 1024,
        )?;
        self.append(LoopEvent::DirectInferenceRequested {
            call_id: call_id.clone(),
            operation: self.operation.clone(),
            provider: self.inner.provider_name().into(),
            model: options
                .model
                .clone()
                .unwrap_or_else(|| self.inner.default_model().into()),
            request_hash,
            request_bytes,
            recipient: Some(self.agent_id),
            recipient_definition_hash: Some(self.source_hash.clone()),
            communication: None,
        })
        .await?;
        let response = self.inner.complete(conversation, options).await;
        match &response {
            Ok(response) => {
                let (response_hash, response_bytes) = contract(&json!(response), 4 * 1024 * 1024)?;
                self.append(LoopEvent::DirectInferenceResponseReceived {
                    call_id: call_id.clone(),
                    response_hash: response_hash.clone(),
                    response_bytes,
                })
                .await?;
                self.append(LoopEvent::DirectInferenceFinished {
                    call_id,
                    reason: TerminationReason::Completed,
                    response_hash: Some(response_hash),
                    response_bytes,
                    finish_reason: Some(response.finish_reason.clone()),
                    usage: response.usage.clone(),
                })
                .await?;
            }
            Err(error) => {
                self.append(LoopEvent::DirectInferenceFinished {
                    call_id,
                    reason: TerminationReason::Error {
                        message: error.to_string(),
                    },
                    response_hash: None,
                    response_bytes: 0,
                    finish_reason: None,
                    usage: Default::default(),
                })
                .await?
            }
        }
        response
    }
    fn provider_name(&self) -> &str {
        self.inner.provider_name()
    }
    fn default_model(&self) -> &str {
        self.inner.default_model()
    }
    fn supports_native_tools(&self) -> bool {
        self.inner.supports_native_tools()
    }
    fn supports_structured_output(&self) -> bool {
        self.inner.supports_structured_output()
    }
}

#[cfg(all(test, unix, feature = "cedar"))]
mod tests;
