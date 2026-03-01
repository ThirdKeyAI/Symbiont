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
use crate::reasoning::loop_types::{
    BufferedJournal, JournalEntry, LoopConfig, LoopEvent, TerminationReason,
};
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
All actions are policy-evaluated and audit-logged.";

/// Shared state across all coordinator WebSocket connections.
#[cfg(feature = "http-api")]
pub struct CoordinatorState {
    pub provider: Arc<dyn InferenceProvider>,
    pub policy_gate: Arc<dyn ReasoningPolicyGate>,
    pub runtime_provider: Arc<dyn RuntimeApiProvider>,
    pub tool_definitions: Vec<ToolDefinition>,
    pub loop_config: LoopConfig,
}

#[cfg(feature = "http-api")]
impl CoordinatorState {
    /// Create a new coordinator state with default loop config.
    pub fn new(
        provider: Arc<dyn InferenceProvider>,
        policy_gate: Arc<dyn ReasoningPolicyGate>,
        runtime_provider: Arc<dyn RuntimeApiProvider>,
    ) -> Self {
        let tool_definitions = CoordinatorExecutor::tool_definitions();
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
        }
    }
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

    /// Handle a chat message from the user.
    ///
    /// Runs the reasoning loop and streams events to the WebSocket client.
    pub async fn handle_chat(&mut self, content: String) {
        let request_id = Uuid::new_v4().to_string();

        // Push user message into conversation
        self.conversation.push(ConversationMessage::user(&content));

        // Set up streaming journal
        let inner_journal = Arc::new(BufferedJournal::new(500));
        let (journal_tx, mut journal_rx) = mpsc::channel::<JournalEntry>(64);
        let streaming_journal = Arc::new(StreamingJournal::new(inner_journal, journal_tx));

        // Build executor
        let executor = Arc::new(CoordinatorExecutor::new(
            self.state.runtime_provider.clone(),
        ));

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
            knowledge_bridge: None,
        };

        // Spawn the journal→WebSocket bridge task
        let ws_tx = self.ws_tx.clone();
        let bridge_request_id = request_id.clone();
        let bridge_handle = tokio::spawn(async move {
            while let Some(entry) = journal_rx.recv().await {
                let msg = match &entry.event {
                    LoopEvent::ReasoningComplete {
                        actions, usage: _, ..
                    } => {
                        // Report tool call starts
                        for action in actions {
                            if let crate::reasoning::loop_types::ProposedAction::ToolCall {
                                call_id,
                                name,
                                arguments,
                            } = action
                            {
                                let _ = ws_tx
                                    .send(ServerMessage::ToolCallStarted {
                                        request_id: bridge_request_id.clone(),
                                        call_id: call_id.clone(),
                                        tool_name: name.clone(),
                                        arguments: arguments.clone(),
                                    })
                                    .await;
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
                    let _ = ws_tx.send(msg).await;
                }
            }
        });

        // Run the reasoning loop
        let agent_id = AgentId::new();
        tracing::info!(
            session_id = %self.session_id,
            request_id = %request_id,
            "Starting coordinator reasoning loop"
        );

        let result = runner
            .run(agent_id, self.conversation.clone(), config)
            .await;

        // Wait for bridge to drain
        drop(runner);
        let _ = bridge_handle.await;

        // Send final chat chunk
        let _ = self
            .ws_tx
            .send(ServerMessage::ChatChunk {
                request_id: request_id.clone(),
                content: result.output.clone(),
                done: true,
            })
            .await;

        // Check for errors
        if let TerminationReason::Error { ref message } = result.termination_reason {
            let _ = self
                .ws_tx
                .send(ServerMessage::Error {
                    request_id: Some(request_id),
                    code: "LOOP_ERROR".into(),
                    message: message.clone(),
                })
                .await;
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
}
