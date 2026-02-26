//! Core types for the reasoning loop
//!
//! Defines observations, proposed actions, loop configuration, state,
//! and recovery strategies for the observe-reason-gate-act cycle.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use crate::reasoning::conversation::Conversation;
use crate::reasoning::inference::{ToolDefinition, Usage};
use crate::types::AgentId;

/// An observation that feeds into the reasoning step.
///
/// Observations come from tool results, environment state, policy feedback,
/// or external events. The loop collects them before each reasoning step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    /// Source of the observation (e.g., tool name, "policy_gate", "environment").
    pub source: String,
    /// The observation content.
    pub content: String,
    /// Whether this observation indicates an error.
    pub is_error: bool,
    /// Metadata for logging and auditing.
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

impl Observation {
    /// Create a tool result observation.
    pub fn tool_result(tool_name: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            source: tool_name.into(),
            content: content.into(),
            is_error: false,
            metadata: HashMap::new(),
        }
    }

    /// Create a tool error observation.
    pub fn tool_error(tool_name: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            source: tool_name.into(),
            content: error.into(),
            is_error: true,
            metadata: HashMap::new(),
        }
    }

    /// Create a policy denial observation.
    pub fn policy_denial(reason: impl Into<String>) -> Self {
        Self {
            source: "policy_gate".into(),
            content: reason.into(),
            is_error: true,
            metadata: HashMap::new(),
        }
    }
}

/// An action proposed by the reasoning step, pending policy evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProposedAction {
    /// Call a tool with the given name and arguments.
    ToolCall {
        /// Unique call identifier.
        call_id: String,
        /// Tool name.
        name: String,
        /// JSON-encoded arguments.
        arguments: String,
    },
    /// Delegate work to another agent.
    Delegate {
        /// Target agent identifier or name.
        target: String,
        /// Message to send.
        message: String,
    },
    /// Respond to the user/caller with content (text or structured).
    Respond {
        /// Response content.
        content: String,
    },
    /// Terminate the loop (agent has finished its task).
    Terminate {
        /// Reason for termination.
        reason: String,
        /// Final output.
        output: String,
    },
}

/// The policy gate's decision for a proposed action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LoopDecision {
    /// Allow the action to proceed.
    Allow,
    /// Deny the action with a reason (fed back as observation).
    Deny { reason: String },
    /// Allow with modifications (e.g., parameter redaction).
    Modify {
        modified_action: Box<ProposedAction>,
        reason: String,
    },
}

/// Runtime state of the reasoning loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopState {
    /// Agent identity.
    pub agent_id: AgentId,
    /// Current iteration number (0-indexed).
    pub iteration: u32,
    /// Cumulative token usage.
    pub total_usage: Usage,
    /// The conversation so far.
    pub conversation: Conversation,
    /// Pending observations to process.
    pub pending_observations: Vec<Observation>,
    /// Timestamp when the loop started.
    pub started_at: chrono::DateTime<chrono::Utc>,
    /// Current loop phase for logging.
    pub current_phase: String,
    /// Arbitrary metadata carried across iterations.
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

impl LoopState {
    /// Create initial state for a new loop.
    pub fn new(agent_id: AgentId, conversation: Conversation) -> Self {
        Self {
            agent_id,
            iteration: 0,
            total_usage: Usage::default(),
            conversation,
            pending_observations: Vec::new(),
            started_at: chrono::Utc::now(),
            current_phase: "initialized".into(),
            metadata: HashMap::new(),
        }
    }

    /// Accumulate token usage from an inference response.
    pub fn add_usage(&mut self, usage: &Usage) {
        self.total_usage.prompt_tokens += usage.prompt_tokens;
        self.total_usage.completion_tokens += usage.completion_tokens;
        self.total_usage.total_tokens += usage.total_tokens;
    }

    /// Get elapsed time since loop start.
    pub fn elapsed(&self) -> chrono::Duration {
        chrono::Utc::now() - self.started_at
    }
}

/// Configuration for a reasoning loop instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopConfig {
    /// Maximum iterations before forced termination.
    pub max_iterations: u32,
    /// Maximum total tokens before forced termination.
    pub max_total_tokens: u32,
    /// Maximum wall-clock time for the entire loop.
    pub timeout: Duration,
    /// Default recovery strategy for tool failures.
    pub default_recovery: RecoveryStrategy,
    /// Per-tool timeout for individual tool calls.
    pub tool_timeout: Duration,
    /// Maximum concurrent tool calls during parallel dispatch.
    pub max_concurrent_tools: usize,
    /// Token budget for context window management.
    pub context_token_budget: usize,
    /// Tool definitions available during this loop run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_definitions: Vec<ToolDefinition>,
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            max_iterations: 25,
            max_total_tokens: 100_000,
            timeout: Duration::from_secs(300),
            default_recovery: RecoveryStrategy::Retry {
                max_attempts: 2,
                base_delay: Duration::from_millis(500),
            },
            tool_timeout: Duration::from_secs(30),
            max_concurrent_tools: 5,
            context_token_budget: 32_000,
            tool_definitions: Vec::new(),
        }
    }
}

/// Strategy for recovering from tool execution failures.
///
/// By default, recovery is deterministic (retry, fallback, cache, dead letter).
/// LLM-driven recovery is opt-in and rate-limited to prevent failure amplification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecoveryStrategy {
    /// Retry with exponential backoff.
    Retry {
        max_attempts: u32,
        base_delay: Duration,
    },
    /// Try alternative tools in order.
    Fallback { alternatives: Vec<String> },
    /// Return a cached result if available and not too stale.
    CachedResult { max_staleness: Duration },
    /// Ask the LLM to propose an alternative approach (opt-in, rate-limited).
    LlmRecovery { max_recovery_attempts: u32 },
    /// Escalate to a human or external system.
    Escalate {
        queue: String,
        context_snapshot: bool,
    },
    /// Send to dead letter queue and continue without the result.
    DeadLetter,
}

/// The final result of a completed reasoning loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopResult {
    /// The final response content.
    pub output: String,
    /// Total iterations executed.
    pub iterations: u32,
    /// Total token usage.
    pub total_usage: Usage,
    /// How the loop terminated.
    pub termination_reason: TerminationReason,
    /// Wall-clock duration.
    pub duration: Duration,
    /// The full conversation history.
    pub conversation: Conversation,
}

/// Why the loop terminated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TerminationReason {
    /// Agent produced a final response (normal completion).
    Completed,
    /// Hit the maximum iteration limit.
    MaxIterations,
    /// Hit the token budget limit.
    MaxTokens,
    /// Hit the timeout.
    Timeout,
    /// Policy denied a critical action with no recovery path.
    PolicyDenial { reason: String },
    /// An unrecoverable error occurred.
    Error { message: String },
}

/// Events emitted during loop execution for observability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LoopEvent {
    /// Loop started.
    Started {
        agent_id: AgentId,
        config: LoopConfig,
    },
    /// Reasoning step completed.
    ReasoningComplete {
        iteration: u32,
        actions: Vec<ProposedAction>,
        usage: Usage,
    },
    /// Policy evaluation completed.
    PolicyEvaluated {
        iteration: u32,
        action_count: usize,
        denied_count: usize,
    },
    /// Tool dispatch completed.
    ToolsDispatched {
        iteration: u32,
        tool_count: usize,
        duration: Duration,
    },
    /// Observations collected.
    ObservationsCollected {
        iteration: u32,
        observation_count: usize,
    },
    /// Loop terminated.
    Terminated {
        reason: TerminationReason,
        iterations: u32,
        total_usage: Usage,
        duration: Duration,
    },
    /// Error recovery triggered.
    RecoveryTriggered {
        iteration: u32,
        tool_name: String,
        strategy: RecoveryStrategy,
        error: String,
    },
}

/// A journal entry for durable execution.
///
/// In Phase 2, entries are emitted via the `JournalWriter` trait.
/// The default implementation is `BufferedJournal` which retains entries
/// in a bounded in-memory ring buffer for observability.
/// Phase 5 adds `DurableJournal` backed by SQLite/Postgres.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    /// Monotonically increasing sequence number.
    pub sequence: u64,
    /// When this entry was created.
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// The agent this entry belongs to.
    pub agent_id: AgentId,
    /// The iteration this entry was created in.
    pub iteration: u32,
    /// The event recorded.
    pub event: LoopEvent,
}

/// Trait for writing journal entries.
///
/// Default implementation is `BufferedJournal`. Phase 5 provides `DurableJournal`.
#[async_trait::async_trait]
pub trait JournalWriter: Send + Sync {
    /// Append an entry to the journal.
    async fn append(&self, entry: JournalEntry) -> Result<(), JournalError>;
    /// Get the next sequence number.
    async fn next_sequence(&self) -> u64;
}

/// In-memory journal that retains entries in a bounded ring buffer.
///
/// Provides observability without requiring durable storage. Events are
/// queryable via `entries()` and consumable via `drain()`. When the buffer
/// reaches capacity, the oldest entries are evicted.
pub struct BufferedJournal {
    sequence: std::sync::atomic::AtomicU64,
    capacity: usize,
    buffer: tokio::sync::Mutex<VecDeque<JournalEntry>>,
}

impl Default for BufferedJournal {
    fn default() -> Self {
        Self::new(1000)
    }
}

impl BufferedJournal {
    /// Create a new buffered journal with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            sequence: std::sync::atomic::AtomicU64::new(0),
            capacity,
            buffer: tokio::sync::Mutex::new(VecDeque::with_capacity(capacity)),
        }
    }

    /// Return all currently buffered entries (oldest first).
    pub async fn entries(&self) -> Vec<JournalEntry> {
        self.buffer.lock().await.iter().cloned().collect()
    }

    /// Consume and return all buffered entries, emptying the buffer.
    pub async fn drain(&self) -> Vec<JournalEntry> {
        self.buffer.lock().await.drain(..).collect()
    }
}

#[async_trait::async_trait]
impl JournalWriter for BufferedJournal {
    async fn append(&self, entry: JournalEntry) -> Result<(), JournalError> {
        self.sequence
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut buf = self.buffer.lock().await;
        if buf.len() >= self.capacity {
            buf.pop_front();
        }
        buf.push_back(entry);
        Ok(())
    }

    async fn next_sequence(&self) -> u64 {
        self.sequence.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Errors from the journal system.
#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    #[error("Journal write failed: {0}")]
    WriteFailed(String),
    #[error("Journal read failed: {0}")]
    ReadFailed(String),
    #[error("Journal sequence error: expected {expected}, got {actual}")]
    SequenceError { expected: u64, actual: u64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_observation_constructors() {
        let tool_result = Observation::tool_result("search", "found 5 results");
        assert_eq!(tool_result.source, "search");
        assert!(!tool_result.is_error);

        let tool_error = Observation::tool_error("search", "timeout");
        assert!(tool_error.is_error);

        let denial = Observation::policy_denial("not authorized");
        assert_eq!(denial.source, "policy_gate");
        assert!(denial.is_error);
    }

    #[test]
    fn test_loop_state_new() {
        let state = LoopState::new(AgentId::new(), Conversation::with_system("test"));
        assert_eq!(state.iteration, 0);
        assert_eq!(state.total_usage.total_tokens, 0);
        assert!(state.pending_observations.is_empty());
    }

    #[test]
    fn test_loop_state_add_usage() {
        let mut state = LoopState::new(AgentId::new(), Conversation::new());
        state.add_usage(&Usage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
        });
        state.add_usage(&Usage {
            prompt_tokens: 200,
            completion_tokens: 80,
            total_tokens: 280,
        });
        assert_eq!(state.total_usage.prompt_tokens, 300);
        assert_eq!(state.total_usage.completion_tokens, 130);
        assert_eq!(state.total_usage.total_tokens, 430);
    }

    #[test]
    fn test_loop_config_default() {
        let config = LoopConfig::default();
        assert_eq!(config.max_iterations, 25);
        assert_eq!(config.max_total_tokens, 100_000);
        assert_eq!(config.max_concurrent_tools, 5);
    }

    #[test]
    fn test_proposed_action_variants() {
        let tc = ProposedAction::ToolCall {
            call_id: "c1".into(),
            name: "search".into(),
            arguments: "{}".into(),
        };
        assert!(matches!(tc, ProposedAction::ToolCall { .. }));

        let respond = ProposedAction::Respond {
            content: "done".into(),
        };
        assert!(matches!(respond, ProposedAction::Respond { .. }));

        let terminate = ProposedAction::Terminate {
            reason: "finished".into(),
            output: "result".into(),
        };
        assert!(matches!(terminate, ProposedAction::Terminate { .. }));
    }

    #[test]
    fn test_recovery_strategy_serde() {
        let retry = RecoveryStrategy::Retry {
            max_attempts: 3,
            base_delay: Duration::from_millis(100),
        };
        let json = serde_json::to_string(&retry).unwrap();
        let _restored: RecoveryStrategy = serde_json::from_str(&json).unwrap();

        let llm = RecoveryStrategy::LlmRecovery {
            max_recovery_attempts: 1,
        };
        let json = serde_json::to_string(&llm).unwrap();
        assert!(json.contains("LlmRecovery"));
    }

    fn make_journal_entry(seq: u64, iteration: u32) -> JournalEntry {
        JournalEntry {
            sequence: seq,
            timestamp: chrono::Utc::now(),
            agent_id: AgentId::new(),
            iteration,
            event: LoopEvent::Started {
                agent_id: AgentId::new(),
                config: LoopConfig::default(),
            },
        }
    }

    #[tokio::test]
    async fn test_buffered_journal_retains_entries() {
        let journal = BufferedJournal::new(100);
        assert_eq!(journal.next_sequence().await, 0);

        journal.append(make_journal_entry(0, 0)).await.unwrap();
        journal.append(make_journal_entry(1, 1)).await.unwrap();

        assert_eq!(journal.next_sequence().await, 2);

        let entries = journal.entries().await;
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].sequence, 0);
        assert_eq!(entries[1].sequence, 1);
    }

    #[tokio::test]
    async fn test_buffered_journal_overflow_evicts_oldest() {
        let journal = BufferedJournal::new(3);

        for i in 0..5u64 {
            journal
                .append(make_journal_entry(i, i as u32))
                .await
                .unwrap();
        }

        let entries = journal.entries().await;
        assert_eq!(entries.len(), 3);
        // Oldest two (seq 0, 1) were evicted
        assert_eq!(entries[0].sequence, 2);
        assert_eq!(entries[1].sequence, 3);
        assert_eq!(entries[2].sequence, 4);
    }

    #[tokio::test]
    async fn test_buffered_journal_drain_empties_buffer() {
        let journal = BufferedJournal::new(100);

        journal.append(make_journal_entry(0, 0)).await.unwrap();
        journal.append(make_journal_entry(1, 1)).await.unwrap();

        let drained = journal.drain().await;
        assert_eq!(drained.len(), 2);

        // Buffer is now empty
        let entries = journal.entries().await;
        assert!(entries.is_empty());

        // Sequence counter is not reset by drain
        assert_eq!(journal.next_sequence().await, 2);
    }

    #[tokio::test]
    async fn test_buffered_journal_entries_returns_all() {
        let journal = BufferedJournal::new(100);

        for i in 0..10u64 {
            journal
                .append(make_journal_entry(i, i as u32))
                .await
                .unwrap();
        }

        let entries = journal.entries().await;
        assert_eq!(entries.len(), 10);
        for (idx, entry) in entries.iter().enumerate() {
            assert_eq!(entry.sequence, idx as u64);
        }
    }

    #[test]
    fn test_loop_event_serde() {
        let event = LoopEvent::Terminated {
            reason: TerminationReason::Completed,
            iterations: 5,
            total_usage: Usage {
                prompt_tokens: 1000,
                completion_tokens: 500,
                total_tokens: 1500,
            },
            duration: Duration::from_secs(10),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("Terminated"));
    }
}
