//! Coordinator session and shared state.
//!
//! Each WebSocket connection gets its own [`CoordinatorSession`] which holds
//! the persistent [`Conversation`] state and drives the [`ReasoningLoopRunner`]
//! for each user message. [`CoordinatorState`] is shared across connections
//! and holds the inference provider, policy gate, and runtime provider.

#[cfg(feature = "http-api")]
use std::sync::Arc;

#[cfg(feature = "http-api")]
use tokio::sync::mpsc;

#[cfg(feature = "http-api")]
use uuid::Uuid;

#[cfg(feature = "http-api")]
use crate::reasoning::circuit_breaker::CircuitBreakerRegistry;
#[cfg(feature = "http-api")]
use crate::reasoning::context_manager::DefaultContextManager;
#[cfg(feature = "http-api")]
use crate::reasoning::conversation::{Conversation, ConversationMessage};
#[cfg(feature = "http-api")]
use crate::reasoning::inference::{InferenceProvider, ToolDefinition};
#[cfg(feature = "http-api")]
use crate::reasoning::loop_types::{JournalEntry, LoopConfig, LoopEvent, TerminationReason};
#[cfg(feature = "http-api")]
use crate::reasoning::policy_bridge::ReasoningPolicyGate;
#[cfg(feature = "http-api")]
use crate::reasoning::reasoning_loop::ReasoningLoopRunner;
#[cfg(feature = "http-api")]
use crate::types::AgentId;

#[cfg(feature = "http-api")]
use super::coordinator_executor::CoordinatorExecutor;
#[cfg(feature = "http-api")]
use super::streaming_journal::StreamingJournal;
#[cfg(feature = "http-api")]
use super::traits::RuntimeApiProvider;
#[cfg(feature = "http-api")]
use super::ws_types::ServerMessage;

/// System prompt for the coordinator agent.
#[cfg(feature = "http-api")]
const COORDINATOR_SYSTEM_PROMPT: &str = "\
You are the Symbiont Coordinator, a meta-agent for the Symbiont runtime.
You help operators monitor, inspect, and manage the agent fleet.
Be concise and factual. Format data clearly.
Every action you propose is policy-evaluated before it runs.
Your actions and delegated agents' internal steps are policy-evaluated and \
recorded in linked protected run journals.";

/// Shared state across all coordinator WebSocket connections.
#[cfg(feature = "http-api")]
pub struct CoordinatorState {
    pub provider: Arc<dyn InferenceProvider>,
    pub policy_gate: Arc<dyn ReasoningPolicyGate>,
    pub runtime_provider: Arc<dyn RuntimeApiProvider>,
    pub tool_definitions: Vec<ToolDefinition>,
    pub loop_config: LoopConfig,
    /// Live RAG retrieval bridge, or `None` when RAG is not configured/available.
    pub knowledge_bridge: Option<Arc<crate::reasoning::knowledge_bridge::KnowledgeBridge>>,
    /// Stable namespace used for all coordinator knowledge store/recall calls.
    /// Generated once per process so knowledge persists across turns and
    /// sessions (single-runtime deployment). Must stay stable across the
    /// process lifetime; if per-agent search filtering is added later, this
    /// id is what identifies the coordinator's own knowledge namespace.
    pub knowledge_agent_id: AgentId,
    /// In-process agent-to-agent delegation handle, or `None` when no `./agents`
    /// registry was configured. Built once at construction via `with_delegation`.
    pub delegation: Option<Arc<dyn crate::reasoning::delegation::DelegationExecutor>>,
    /// Frozen trusted project directory for required per-turn audit storage.
    pub(super) audit_project: Result<std::path::PathBuf, String>,
    registered_delegation:
        Option<Arc<crate::reasoning::delegation_executor::RegisteredDelegationRegistry>>,
}

#[cfg(feature = "http-api")]
impl CoordinatorState {
    /// Create a new coordinator state with default loop config.
    pub fn new(
        provider: Arc<dyn InferenceProvider>,
        policy_gate: Arc<dyn ReasoningPolicyGate>,
        runtime_provider: Arc<dyn RuntimeApiProvider>,
    ) -> Self {
        let tool_definitions = CoordinatorExecutor::tool_definitions(&[]);
        Self {
            provider,
            policy_gate,
            runtime_provider,
            tool_definitions,
            loop_config: LoopConfig {
                max_iterations: 10,
                max_total_tokens: 50_000,
                timeout: std::time::Duration::from_secs(120),
                ..Default::default()
            },
            knowledge_bridge: None,
            knowledge_agent_id: AgentId::new(),
            delegation: None,
            registered_delegation: None,
            audit_project: std::env::current_dir()
                .and_then(std::fs::canonicalize)
                .map_err(|error| error.to_string()),
        }
    }

    /// Select audit storage from operator configuration, never chat input.
    pub fn with_audit_project(mut self, project: &std::path::Path) -> Self {
        self.audit_project = std::fs::canonicalize(project).map_err(|error| error.to_string());
        self
    }

    /// Build and attach the live RAG knowledge bridge when RAG is usable
    /// (the `vector-lancedb` feature is built AND an embedding provider is
    /// configured). Otherwise leaves `knowledge_bridge` as `None` after logging
    /// the reason. Async because building the context manager opens the vector
    /// store.
    pub async fn with_rag(mut self, agent_id: &str) -> Self {
        self.knowledge_bridge = build_knowledge_bridge(agent_id).await;
        self
    }

    /// Build the in-process delegation handle from a name→system-prompt registry
    /// (scanned from `./agents`). Sub-loops reuse the coordinator's deps and
    /// open separate protected journals linked to the parent before inference.
    /// No registry entries → still constructs a handle whose every lookup misses
    /// with an honest error; pass an empty map to disable.
    pub fn with_delegation(mut self, registry: std::collections::HashMap<String, String>) -> Self {
        use crate::reasoning::delegation_executor::SubLoopDelegationExecutor;

        // The model picks delegation targets from the tool description, so the
        // advertised names must be exactly the registry keys that resolve.
        let mut names: Vec<String> = registry.keys().cloned().collect();
        names.sort();
        self.tool_definitions = CoordinatorExecutor::tool_definitions(&names);

        let executor: Arc<dyn crate::reasoning::executor::ActionExecutor> =
            Arc::new(CoordinatorExecutor::new(self.runtime_provider.clone()));
        let delegation = SubLoopDelegationExecutor::new_protected(
            self.provider.clone(),
            executor,
            self.policy_gate.clone(),
            Arc::new(DefaultContextManager::default()),
            Arc::new(CircuitBreakerRegistry::default()),
            self.audit_project.clone(),
            registry,
            3,
        );
        self.delegation = Some(delegation);
        self.registered_delegation = None;
        self
    }
    /// Attach canonical sources loaded from bounded, confined project reads.
    pub fn with_registered_delegation(
        mut self,
        registry: crate::reasoning::delegation_executor::RegisteredDelegationRegistry,
    ) -> Self {
        let registry = Arc::new(registry);
        self.tool_definitions = CoordinatorExecutor::tool_definitions(&registry.names());
        self.delegation = Some(
            crate::reasoning::delegation_executor::SubLoopDelegationExecutor::new_registered(
                self.provider.clone(),
                Arc::new(CoordinatorExecutor::new(self.runtime_provider.clone())),
                self.policy_gate.clone(),
                Arc::new(DefaultContextManager::default()),
                Arc::new(CircuitBreakerRegistry::default()),
                self.audit_project.clone(),
                registry.clone(),
                3,
            ),
        );
        self.registered_delegation = Some(registry);
        self
    }
}

/// Construct a live `KnowledgeBridge` over a real `StandardContextManager`
/// backed by LanceDB, or return `None` (with a loud log) when RAG cannot run.
#[cfg(all(feature = "http-api", feature = "vector-lancedb"))]
async fn build_knowledge_bridge(
    agent_id: &str,
) -> Option<Arc<crate::reasoning::knowledge_bridge::KnowledgeBridge>> {
    use crate::context::embedding::EmbeddingConfig;
    use crate::context::manager::{ContextManagerConfig, StandardContextManager};
    use crate::context::vector_db_factory::VectorBackendConfig;
    use crate::context::vector_db_lance::LanceDbConfig;
    use crate::reasoning::knowledge_bridge::{KnowledgeBridge, KnowledgeConfig};

    let Some(embed_cfg) = EmbeddingConfig::from_env() else {
        tracing::warn!(
            "RAG retrieval disabled: no embedding provider configured (set EMBEDDING_* or \
             OPENAI_API_KEY). Chat will run without knowledge retrieval."
        );
        return None;
    };

    let cfg = ContextManagerConfig {
        enable_vector_db: true,
        vector_backend: Some(VectorBackendConfig::LanceDb(LanceDbConfig {
            vector_dimension: embed_cfg.dimension,
            ..Default::default()
        })),
        ..Default::default()
    };

    match StandardContextManager::new(cfg, agent_id).await {
        Ok(scm) => {
            tracing::info!(
                "RAG knowledge bridge constructed (vector-lancedb + embedding provider); \
                 retrieval will be inactive if the vector backend failed to initialize \
                 (see warnings above)"
            );
            Some(Arc::new(KnowledgeBridge::new(
                Arc::new(scm),
                KnowledgeConfig::default(),
            )))
        }
        Err(e) => {
            tracing::warn!("RAG retrieval disabled: failed to build context manager: {e}");
            None
        }
    }
}

/// Without the `vector-lancedb` feature there is no real vector backend, so RAG
/// stays off honestly.
#[cfg(all(feature = "http-api", not(feature = "vector-lancedb")))]
async fn build_knowledge_bridge(
    _agent_id: &str,
) -> Option<Arc<crate::reasoning::knowledge_bridge::KnowledgeBridge>> {
    tracing::warn!(
        "RAG retrieval disabled: built without the 'vector-lancedb' feature. \
         Rebuild with --features vector-lancedb to enable knowledge retrieval."
    );
    None
}

/// Per-connection session that holds conversation state.
#[cfg(feature = "http-api")]
pub struct CoordinatorSession {
    state: Arc<CoordinatorState>,
    conversation: Conversation,
    ws_tx: mpsc::Sender<ServerMessage>,
    session_id: String,
}

#[cfg(feature = "http-api")]
impl CoordinatorSession {
    /// Create a new session for a WebSocket connection.
    pub fn new(state: Arc<CoordinatorState>, ws_tx: mpsc::Sender<ServerMessage>) -> Self {
        Self {
            state,
            conversation: Conversation::with_system(COORDINATOR_SYSTEM_PROMPT),
            ws_tx,
            session_id: Uuid::new_v4().to_string(),
        }
    }

    /// Start new trusted SDK work. Retain a client UUID and use
    /// `handle_chat_with_id` when retrying a previously submitted message.
    pub async fn handle_chat(&mut self, content: String) {
        self.handle_chat_with_id(Uuid::new_v4(), content).await;
    }

    /// An SDK caller shares the project SDK identity. Network callers receive
    /// identities derived from their validated credentials in the WS handler.
    pub async fn handle_chat_with_id(&mut self, id: Uuid, content: String) {
        #[cfg(unix)]
        {
            use super::chat_invocations::{self, AdmittedChat};
            use crate::reasoning::invocation::OpenInvocation;
            let caller = super::invocations::AuthenticatedCaller::coordinator_sdk();
            match self.state.admit_chat(&caller, id, &content).await {
                Ok(OpenInvocation::Fresh(invocation)) => {
                    if self
                        .send(ServerMessage::AuditOpened {
                            request_id: id.to_string(),
                            audit: invocation.audit().clone(),
                        })
                        .await
                    {
                        self.handle_admitted_cancellable(
                            AdmittedChat {
                                id,
                                content,
                                invocation,
                            },
                            tokio_util::sync::CancellationToken::new(),
                        )
                        .await;
                    }
                }
                Ok(OpenInvocation::Existing(receipt)) => {
                    chat_invocations::existing(&self.ws_tx, id, receipt, true).await;
                }
                Err(error) => {
                    chat_invocations::admission_error(&self.ws_tx, id, &error).await;
                }
            }
        }
        #[cfg(not(unix))]
        {
            let _ = content;
            self.send_error(
                &id.to_string(),
                "AUDIT_UNAVAILABLE",
                "Durable chat admission is unavailable on this platform",
            )
            .await;
        }
    }

    #[cfg(unix)]
    pub(super) async fn handle_admitted_cancellable(
        &mut self,
        request: super::chat_invocations::AdmittedChat,
        cancellation: tokio_util::sync::CancellationToken,
    ) {
        let super::chat_invocations::AdmittedChat {
            id,
            content,
            invocation,
        } = request;
        let request_id = id.to_string();
        let _cancel_on_drop = cancellation.clone().drop_guard();
        if content.len() > 64 * 1024
            || self.conversation.messages().len() >= 256
            || self
                .conversation
                .messages()
                .iter()
                .map(|message| message.content.len())
                .sum::<usize>()
                > 4 * 1024 * 1024
        {
            self.send_error(
                &request_id,
                "SESSION_LIMIT",
                "Chat input or session history exceeds its limit; open a new session",
            )
            .await;
            return;
        }
        let inner_journal = invocation.journal();
        if cancellation.is_cancelled() {
            return;
        }

        // Push user message into conversation
        self.conversation.push(ConversationMessage::user(&content));

        // Set up streaming journal
        let (journal_tx, mut journal_rx) = mpsc::channel::<JournalEntry>(64);
        let streaming_journal = Arc::new(StreamingJournal::new(inner_journal, journal_tx));

        // Build executor
        let executor: Arc<dyn crate::reasoning::executor::ActionExecutor> = Arc::new(
            CoordinatorExecutor::new(self.state.runtime_provider.clone()),
        );
        let executor = match &self.state.registered_delegation {
            Some(registry) => registry.wrap(executor),
            None => executor,
        };

        // Build loop config with tool definitions
        let mut config = self.state.loop_config.clone();
        config.tool_definitions = self.state.tool_definitions.clone();

        // Build the runner
        let runner = ReasoningLoopRunner {
            provider: self.state.provider.clone(),
            policy_gate: self.state.policy_gate.clone(),
            executor,
            context_manager: Arc::new(DefaultContextManager::default()),
            circuit_breakers: Arc::new(CircuitBreakerRegistry::default()),
            journal: streaming_journal,
            knowledge_bridge: self.state.knowledge_bridge.clone(),
            delegation: self.state.delegation.clone(),
        };

        // Spawn the journal→WebSocket bridge task
        let ws_tx = self.ws_tx.clone();
        let bridge_request_id = request_id.clone();
        let bridge_handle = tokio::spawn(async move {
            while let Some(entry) = journal_rx.recv().await {
                let msg = match &entry.event {
                    LoopEvent::ReasoningComplete { actions, .. } => {
                        // Report tool call starts
                        for action in actions {
                            if let crate::reasoning::loop_types::ProposedAction::ToolCall {
                                call_id,
                                name,
                                arguments,
                            } = action
                            {
                                if let Err(e) = ws_tx.try_send(ServerMessage::ToolCallStarted {
                                    request_id: bridge_request_id.clone(),
                                    call_id: call_id.clone(),
                                    tool_name: name.clone(),
                                    arguments: arguments.clone(),
                                }) {
                                    tracing::debug!(
                                        request_id = %bridge_request_id,
                                        call_id = %call_id,
                                        error = %e,
                                        "WS tool_call_started send failed — client likely disconnected"
                                    );
                                }
                            }
                        }
                        None
                    }
                    LoopEvent::PolicyEvaluated {
                        action_count,
                        denied_count,
                        ..
                    } => {
                        if *denied_count > 0 {
                            Some(ServerMessage::PolicyDecision {
                                request_id: bridge_request_id.clone(),
                                action: format!("{} actions", action_count),
                                decision: "partial_deny".into(),
                                reason: format!("{} denied", denied_count),
                            })
                        } else {
                            Some(ServerMessage::PolicyDecision {
                                request_id: bridge_request_id.clone(),
                                action: format!("{} actions", action_count),
                                decision: "allow".into(),
                                reason: "All actions approved".into(),
                            })
                        }
                    }
                    LoopEvent::ObservationsCollected { .. } => {
                        // Tool results are embedded in the final response
                        None
                    }
                    _ => None,
                };

                if let Some(msg) = msg {
                    if let Err(e) = ws_tx.try_send(msg) {
                        tracing::debug!(
                            request_id = %bridge_request_id,
                            error = %e,
                            "WS policy/bridge send failed — client likely disconnected"
                        );
                    }
                }
            }
        });

        // Run the reasoning loop. The coordinator uses one stable knowledge
        // namespace (set once at process start) so store/recall persist
        // across turns and sessions in this single-runtime deployment; keep
        // this stable if per-agent search filtering is added later.
        let agent_id = self.state.knowledge_agent_id;
        tracing::info!(
            session_id = %self.session_id,
            request_id = %request_id,
            "Starting coordinator reasoning loop"
        );

        let conversation = self.conversation.clone();
        // Retain cleanup and terminal audit even if this session future is
        // dropped. Its drop guard requests cancellation of this owning task.
        let result = tokio::spawn(async move {
            let result = runner
                .run_cancellable(agent_id, conversation, config, cancellation)
                .await;
            // This owner keeps the exclusive claim through retained cleanup and
            // required terminal storage, even if the session is dropped.
            let receipt = invocation
                .finish(super::chat_invocations::saved_result(&result))
                .await;
            (result, receipt)
        })
        .await;

        // Wait for bridge to drain
        if let Err(e) = bridge_handle.await {
            tracing::warn!(
                session_id = %self.session_id,
                error = %e,
                "Coordinator journal->WS bridge task joined with error"
            );
        }

        let (result, receipt) = match result {
            Ok(result) => result,
            Err(error) => {
                tracing::error!(%request_id, %error, "Coordinator run owner failed");
                self.send_error(
                    &request_id,
                    "LOOP_ERROR",
                    "Coordinator run failed; inspect the protected audit",
                )
                .await;
                return;
            }
        };
        let receipt = match receipt {
            Ok(receipt) => receipt,
            Err(error) => {
                tracing::error!(%request_id, %error, "Required coordinator result persistence failed");
                self.send_error(&request_id, "LOOP_ERROR", "The original outcome could not be durably recorded; inspect its audit before new work").await;
                return;
            }
        };
        let completed = matches!(result.termination_reason, TerminationReason::Completed)
            && matches!(
                &receipt,
                crate::reasoning::invocation::ExistingInvocation::Recorded { .. }
            );
        super::chat_invocations::existing(&self.ws_tx, id, receipt, false).await;
        if !completed {
            return;
        }

        // Push assistant response into conversation for context continuity
        self.conversation
            .push(ConversationMessage::assistant(&result.output));

        tracing::info!(
            session_id = %self.session_id,
            iterations = result.iterations,
            tokens = result.total_usage.total_tokens,
            "Coordinator reasoning loop complete"
        );
    }

    async fn send(&self, message: ServerMessage) -> bool {
        matches!(
            tokio::time::timeout(std::time::Duration::from_secs(2), self.ws_tx.send(message)).await,
            Ok(Ok(()))
        )
    }

    async fn send_error(&self, request_id: &str, code: &str, message: &str) {
        self.send(ServerMessage::Error {
            request_id: Some(request_id.into()),
            code: code.into(),
            message: message.into(),
        })
        .await;
    }
}

#[cfg(all(test, feature = "http-api"))]
mod rag_wiring_tests {
    use super::*;

    #[tokio::test]
    #[serial_test::serial(embedding_env)]
    async fn build_knowledge_bridge_returns_none_without_embedding_provider() {
        // Clear every env var that EmbeddingConfig::from_env() reads so
        // from_env() is guaranteed to return None regardless of ambient env.
        for k in [
            "EMBEDDING_API_KEY",
            "OPENAI_API_KEY",
            "EMBEDDING_API_BASE_URL",
            "OPENAI_API_BASE_URL",
            "EMBEDDING_PROVIDER",
            "EMBEDDING_MODEL",
            "VECTOR_DIMENSION",
        ] {
            std::env::remove_var(k);
        }
        let bridge = build_knowledge_bridge("test-agent").await;
        assert!(
            bridge.is_none(),
            "with no embedding provider configured, RAG must stay off (None)"
        );
    }
}
