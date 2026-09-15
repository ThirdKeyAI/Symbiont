//! Required journals and retained cancellation ownership for direct DSL inference.

use super::reasoning_builtins::{append_event, ReasoningBuiltinContext};
use crate::error::{ReplError, Result};
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use symbi_runtime::reasoning::{
    agent_registry::RegisteredAgent,
    conversation::Conversation,
    inference::{InferenceOptions, InferenceResponse, Usage},
    loop_types::{LoopEvent, TerminationReason},
    prepared::{canonical_json, digest_json},
    run_audit::RunAuditReference,
};
use symbi_runtime::types::{AgentId, MessageType};

const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_DISPLAY_REFERENCES: usize = 256;

pub(crate) struct InferenceExchange {
    pub recipient: RegisteredAgent,
    pub request_type: MessageType,
    pub response_type: Option<MessageType>,
    pub message: Option<String>,
}

pub(crate) struct PendingInference {
    owner: tokio::task::JoinHandle<Result<InferenceResponse>>,
    cancellation: tokio::sync::oneshot::Sender<()>,
}

impl PendingInference {
    pub(crate) async fn wait(self) -> Result<InferenceResponse> {
        let outcome = self.owner.await.map_err(|error| {
            ReplError::Execution(format!("Direct inference owner failed: {error}"))
        });
        drop(self.cancellation);
        outcome.and_then(|result| result)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct InferenceAuditReference {
    pub agent_id: AgentId,
    pub operation: String,
    pub call_id: String,
    pub audit: RunAuditReference,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AuditReferenceSnapshot {
    pub entries: VecDeque<InferenceAuditReference>,
    /// Older references omitted from this display; their journals are retained.
    pub omitted: u64,
}

#[derive(Default)]
pub struct AuditReferenceLog(Mutex<AuditReferenceSnapshot>);

impl AuditReferenceLog {
    pub fn snapshot(&self) -> Result<AuditReferenceSnapshot> {
        self.0.lock().map(|log| log.clone()).map_err(|_| {
            ReplError::Execution("Inference audit reference display is unavailable".into())
        })
    }

    fn push(&self, reference: InferenceAuditReference) -> Result<()> {
        let mut log = self.0.lock().map_err(|_| {
            ReplError::Execution("Inference audit reference display is unavailable".into())
        })?;
        if log.entries.len() == MAX_DISPLAY_REFERENCES {
            log.entries.pop_front();
            log.omitted = log.omitted.saturating_add(1);
        }
        log.entries.push_back(reference);
        Ok(())
    }
}

fn encoded_contract(value: &impl Serialize, max_bytes: usize) -> Result<(String, u64)> {
    let value = serde_json::to_value(value).map_err(|e| ReplError::Execution(e.to_string()))?;
    let encoded = canonical_json(&value).map_err(ReplError::Execution)?;
    if encoded.len() > max_bytes {
        return Err(ReplError::Execution(format!(
            "Direct inference contract exceeds {max_bytes} bytes"
        )));
    }
    let hash = digest_json(&value).map_err(ReplError::Execution)?;
    Ok((hash, encoded.len() as u64))
}

impl ReasoningBuiltinContext {
    /// Execute one typed provider call with required pre-effect and terminal
    /// records. A recipient must be the same snapshot used for authorization.
    /// Journals describe this call, not completion of a surrounding pattern.
    pub(crate) async fn infer(
        &self,
        operation: &str,
        conversation: &Conversation,
        options: &InferenceOptions,
        exchange: Option<InferenceExchange>,
    ) -> Result<InferenceResponse> {
        self.start_inference(operation, conversation, options, exchange)
            .await?
            .wait()
            .await
    }

    /// Return an owned call only after its required pre-effect records sync.
    pub(crate) async fn start_inference(
        &self,
        operation: &str,
        conversation: &Conversation,
        options: &InferenceOptions,
        exchange: Option<InferenceExchange>,
    ) -> Result<PendingInference> {
        let started = Instant::now();
        let provider = self
            .provider
            .clone()
            .ok_or_else(|| ReplError::Execution("No inference provider configured".into()))?;
        let mut config = self.reasoning_config.clone().unwrap_or_default();
        if config.timeout.is_zero() || config.timeout > Duration::from_secs(86400) {
            return Err(ReplError::Execution(
                "Direct inference deadline must be positive and at most one day".into(),
            ));
        }
        let mut options = options.clone();
        options.max_tokens = options
            .max_tokens
            .min(config.max_output_tokens)
            .min(config.max_total_tokens);
        if options.max_tokens == 0 || !options.temperature.is_finite() {
            return Err(ReplError::Execution(
                "Invalid direct inference options or token budget".into(),
            ));
        }
        // This scope contains one provider call. It never executes returned tools.
        config.max_iterations = 1;
        let conversation = conversation.clone();
        let request = serde_json::json!({"conversation": conversation, "options": options});
        let (request_hash, request_bytes) = encoded_contract(&request, MAX_REQUEST_BYTES)?;
        let recipient = exchange.as_ref().map(|exchange| &exchange.recipient);
        let recipient_definition_hash = recipient
            .map(|agent| encoded_contract(agent, MAX_REQUEST_BYTES).map(|v| v.0))
            .transpose()?;
        let communication = exchange
            .as_ref()
            .map(|exchange| {
                exchange
                    .message
                    .as_ref()
                    .map(|message| {
                        encoded_contract(message, MAX_REQUEST_BYTES).map(|(hash, _)| hash)
                    })
                    .transpose()
                    .map(|hash| {
                        serde_json::json!({
                            "request_type": exchange.request_type,
                            "response_type": exchange.response_type,
                            "enqueue_request": exchange.message.is_some(),
                            "message_hash": hash,
                        })
                    })
            })
            .transpose()?;
        let agent_id = self.sender_agent_id.unwrap_or_default();
        let call_id = uuid::Uuid::new_v4().to_string();
        let operation = operation.to_owned();
        let requested = LoopEvent::DirectInferenceRequested {
            call_id: call_id.clone(),
            operation: operation.clone(),
            provider: provider.provider_name().into(),
            model: options
                .model
                .clone()
                .unwrap_or_else(|| provider.default_model().into()),
            request_hash,
            request_bytes,
            recipient: recipient.map(|agent| agent.agent_id),
            recipient_definition_hash,
            communication,
        };
        let deadline = tokio::time::Instant::from_std(started) + config.timeout;
        let (cancellation, mut cancelled) = tokio::sync::oneshot::channel::<()>();
        let (ready, startup) = tokio::sync::oneshot::channel();
        let bus = self.comm_bus.clone();
        let ctx = self.clone();
        // The retained owner finishes the audit after its caller disappears.
        // Dropping the caller closes cancellation; dropping JoinHandle alone
        // would leave the provider running without a cancellation signal.
        let owner = tokio::spawn(async move {
            // Initialization can contain blocking filesystem work. Retain its
            // owner too, so caller cancellation cannot orphan a new journal.
            let (journal, reference) = ctx.journal(agent_id).await?;
            let outcome = async {
                if let Some(audit) = &reference {
                    ctx.audit_references.push(InferenceAuditReference {
                        agent_id,
                        operation,
                        call_id: call_id.clone(),
                        audit: audit.clone(),
                    })?;
                }
                append_event(
                    journal.as_ref(),
                    agent_id,
                    LoopEvent::Started {
                        agent_id,
                        config: Box::new(config),
                        execution_context: Default::default(),
                    },
                )
                .await?;
                append_event(journal.as_ref(), agent_id, requested).await?;
                let _ = ready.send(());
                let (outcome, mut reason) = tokio::select! {
                    biased;
                    _ = &mut cancelled => (
                        Err(ReplError::Execution("Direct inference caller cancelled".into())),
                        TerminationReason::Error { message: "caller cancelled".into() },
                    ),
                    _ = tokio::time::sleep_until(deadline) => (
                        Err(ReplError::Execution("Direct inference deadline expired".into())),
                        TerminationReason::Timeout,
                    ),
                    result = async {
                        if let (Some(bus), Some(exchange)) = (&bus, &exchange) {
                            if let Some(message) = &exchange.message {
                            let request = bus.create_internal_message(agent_id, exchange.recipient.agent_id,
                                bytes::Bytes::from(message.clone()), exchange.request_type.clone(),
                                deadline.saturating_duration_since(tokio::time::Instant::now()));
                            bus.send_message(request).await.map_err(|error| ReplError::Execution(format!("Required communication enqueue failed: {error}")))?;
                            }
                        }
                        let response = provider.complete(&conversation, &options).await
                            .map_err(|error| ReplError::Execution(format!("Direct inference failed: {error}")))?;
                        Ok::<_, ReplError>(response)
                    } => match result {
                        Ok(response) => (Ok(response), TerminationReason::Completed),
                        Err(error) => (
                            Err(error),
                            TerminationReason::Error { message: "inference or communication call failed".into() },
                        ),
                    },
                };
                let mut response_hash = None;
                let mut response_bytes = 0;
                let mut usage = Usage::default();
                let mut finish_reason = None;
                let mut outcome = outcome.and_then(|response| {
                    let (hash, bytes) = encoded_contract(&response, MAX_RESPONSE_BYTES)?;
                    response_hash = Some(hash);
                    response_bytes = bytes;
                    usage = response.usage.clone();
                    finish_reason = Some(response.finish_reason.clone());
                    Ok(response)
                });
                if let Ok(response) = &outcome {
                    // Await required writes outside cancellation selection, so an
                    // already-started append cannot land after the terminal record.
                    append_event(
                        journal.as_ref(),
                        agent_id,
                        LoopEvent::DirectInferenceResponseReceived {
                            call_id: call_id.clone(),
                            response_hash: response_hash.clone().unwrap(),
                            response_bytes,
                        },
                    )
                    .await?;
                    let delivery = tokio::select! {
                        biased;
                        _ = &mut cancelled => Err((ReplError::Execution("Direct inference caller cancelled".into()), TerminationReason::Error { message: "caller cancelled".into() })),
                        _ = tokio::time::sleep_until(deadline) => Err((ReplError::Execution("Direct inference deadline expired".into()), TerminationReason::Timeout)),
                        result = async {
                            if let (Some(bus), Some(exchange)) = (&bus, &exchange) {
                                if let Some(response_type) = &exchange.response_type {
                                    let message = bus.create_internal_message(exchange.recipient.agent_id, agent_id,
                                        bytes::Bytes::from(response.content.clone()), response_type.clone(),
                                        deadline.saturating_duration_since(tokio::time::Instant::now()));
                                    bus.send_message(message).await.map_err(|error| ReplError::Execution(format!("Required communication response enqueue failed: {error}")))?;
                                }
                            }
                            Ok::<_, ReplError>(())
                        } => result.map_err(|error| (error, TerminationReason::Error { message: "communication response failed".into() })),
                    };
                    if let Err((error, termination)) = delivery {
                        outcome = Err(error);
                        reason = termination;
                    }
                }
                if outcome.is_err() && matches!(reason, TerminationReason::Completed) {
                    reason = TerminationReason::Error {
                        message: "invalid or oversized provider response".into(),
                    };
                }
                append_event(
                    journal.as_ref(),
                    agent_id,
                    LoopEvent::DirectInferenceFinished {
                        call_id,
                        reason: reason.clone(),
                        response_hash,
                        response_bytes,
                        finish_reason,
                        usage: usage.clone(),
                    },
                )
                .await?;
                append_event(
                    journal.as_ref(),
                    agent_id,
                    LoopEvent::Terminated {
                        reason,
                        iterations: 1,
                        total_usage: usage,
                        duration: started.elapsed(),
                    },
                )
                .await?;
                outcome
            }.await;
            outcome.map_err(|error| match reference {
                Some(audit) => ReplError::Execution(format!(
                    "{error}; audit run {} at {} (public key {})",
                    audit.run_id,
                    audit.path.display(),
                    audit.public_key
                )),
                None => error,
            })
        });
        let pending = PendingInference {
            owner,
            cancellation,
        };
        if startup.await.is_err() {
            return Err(pending.wait().await.err().unwrap_or_else(|| {
                ReplError::Execution(
                    "Direct inference owner ended before required startup records".into(),
                )
            }));
        }
        Ok(pending)
    }
}

#[cfg(all(test, unix))]
#[path = "inference_audit_tests.rs"]
mod tests;
