//! Orchestrator module — ORGA-governed conversational agent.
//!
//! The orchestrator uses the full Observe-Reason-Gate-Act loop instead of
//! raw LLM calls. This ensures every action (including artifact generation)
//! passes through Cedar policy gates and is recorded in the audit journal.

use anyhow::{anyhow, Result};
use std::sync::Arc;
use symbi_runtime::reasoning::conversation::{Conversation, ConversationMessage};
use symbi_runtime::reasoning::executor::ActionExecutor;
use symbi_runtime::reasoning::inference::InferenceProvider;
use symbi_runtime::reasoning::loop_types::{
    BufferedJournal, JournalWriter, LoopConfig, TerminationReason,
};
use symbi_runtime::reasoning::policy_bridge::DefaultPolicyGate;
use symbi_runtime::reasoning::reasoning_loop::ReasoningLoopRunner;
use symbi_runtime::types::AgentId;

/// Token budget for the orchestrator's conversation history.
///
/// We truncate the conversation to this before handing it to the reasoning
/// loop. MUST be strictly smaller than `LOOP_TOKEN_BUDGET` — otherwise the
/// loop's per-run cap is smaller than the input we hand it, and every turn
/// terminates with `MaxTokens` before the model can produce a reply.
const CONTEXT_TOKEN_BUDGET: usize = 80_000;

/// Per-turn token budget for the full reasoning loop.
///
/// Includes conversation history + every tool observation + the final
/// response. Generous enough that a single turn with a handful of tool
/// calls (validate_dsl → fix → re-validate → respond) has headroom.
/// Bounded well under Claude 3.5 Sonnet's 200K context so we stay away
/// from the provider-side hard cap.
const LOOP_TOKEN_BUDGET: u32 = 200_000;

/// Maximum iterations the reasoning loop will make per turn.
///
/// Each tool call is an iteration. Conversational turns with several
/// validation passes (generate → validate → fix → validate again) can
/// easily reach ~10, so we give real headroom before cutting off.
const LOOP_MAX_ITERATIONS: u32 = 20;

const SYSTEM_PROMPT: &str = r#"You are the Symbiont orchestrator, an AI agent that helps users create, manage, and operate AI agents.

You have tools available:
- validate_dsl: Validate Symbiont DSL code against project constraints
- validate_cedar: Validate Cedar policies against project constraints
- validate_toolclad: Validate ToolClad TOML manifests against project constraints
- save_artifact: Save a validated artifact to disk (only after user approval)
- list_agents: List all running agents

When the user asks you to create an artifact (agent, policy, tool manifest):
1. Generate the appropriate DSL/Cedar/TOML
2. Use the validation tool to check it against project constraints
3. If validation fails, fix the issues and re-validate
4. Present the validated artifact to the user for review
5. ONLY call save_artifact after the user explicitly approves (e.g. "looks good", "save it", "yes")
6. NEVER save without user confirmation

Keep responses concise and actionable. You are running inside symbi shell — a terminal-based orchestration interface."#;

/// The orchestrator manages the ORGA-governed conversation loop.
pub struct Orchestrator {
    runner: ReasoningLoopRunner,
    conversation: Conversation,
    model_name: String,
    agent_id: AgentId,
    #[allow(dead_code)] // used by /audit command (Task 25)
    journal: Arc<BufferedJournal>,
}

impl Orchestrator {
    /// Create a new orchestrator with ORGA loop governance.
    pub fn new(provider: Arc<dyn InferenceProvider>, executor: Arc<dyn ActionExecutor>) -> Self {
        let model_name = provider.default_model().to_string();
        let conversation = Conversation::with_system(SYSTEM_PROMPT);
        let journal = Arc::new(BufferedJournal::new(1000));

        let runner = ReasoningLoopRunner::builder()
            .provider(provider)
            .executor(executor)
            .policy_gate(Arc::new(DefaultPolicyGate::new()))
            .journal(Arc::clone(&journal) as Arc<dyn JournalWriter>)
            .build();

        Self {
            runner,
            conversation,
            model_name,
            agent_id: AgentId::new(),
            journal,
        }
    }

    /// Send a user message through the ORGA loop.
    ///
    /// Automatically compacts the conversation if it exceeds the token budget,
    /// preserving the system prompt and most recent messages.
    pub async fn send(&mut self, user_message: &str) -> Result<OrchestratorResponse> {
        self.conversation
            .push(ConversationMessage::user(user_message));

        // Auto-compact: keep conversation within context budget.
        // Preserves system prompt + most recent messages, drops older middle.
        let tokens_before = self.conversation.estimate_tokens();
        self.conversation.truncate_to_budget(CONTEXT_TOKEN_BUDGET);
        let tokens_after = self.conversation.estimate_tokens();
        if tokens_after < tokens_before {
            tracing::info!(
                "Auto-compacted orchestrator context: {} -> {} tokens",
                tokens_before,
                tokens_after
            );
        }

        let config = LoopConfig {
            max_iterations: LOOP_MAX_ITERATIONS,
            max_total_tokens: LOOP_TOKEN_BUDGET,
            context_token_budget: CONTEXT_TOKEN_BUDGET,
            temperature: 0.3,
            ..LoopConfig::default()
        };

        let result = self
            .runner
            .run(self.agent_id, self.conversation.clone(), config)
            .await;

        // Update our conversation with the full history from the loop.
        self.conversation = result.conversation;

        // Fallback path: if the loop terminated without writing a final
        // `Respond` output (most commonly MaxTokens / MaxIterations /
        // Timeout after a long tool chain), recover the last assistant
        // message from the conversation and surface that instead of
        // erroring. The user gets whatever progress the model made, with
        // a visible note about why it stopped, rather than having to
        // retry from a blank slate.
        if result.output.is_empty() {
            if let Some(msg) = self.conversation.last_assistant_message() {
                let mut content = msg.content.clone();
                if content.trim().is_empty() {
                    content = format!(
                        "(no textual response before loop stopped — {})",
                        describe_termination(&result.termination_reason)
                    );
                } else {
                    content.push_str(&format!(
                        "\n\n_(loop stopped: {}; /compact to free context or rephrase to continue)_",
                        describe_termination(&result.termination_reason),
                    ));
                }
                return Ok(OrchestratorResponse {
                    content,
                    tokens_used: result.total_usage.total_tokens as u64,
                    iterations: result.iterations,
                });
            }
            return Err(anyhow!(
                "Orchestrator produced no output ({}). Try /compact, /clear, or rephrasing.",
                describe_termination(&result.termination_reason),
            ));
        }

        Ok(OrchestratorResponse {
            content: result.output,
            tokens_used: result.total_usage.total_tokens as u64,
            iterations: result.iterations,
        })
    }

    /// Get the model name for display.
    pub fn model_name(&self) -> &str {
        &self.model_name
    }

    /// Get the journal for audit display.
    #[allow(dead_code)] // used by /audit command (Task 25)
    pub fn journal(&self) -> &Arc<BufferedJournal> {
        &self.journal
    }

    /// Get current conversation token estimate.
    pub fn context_tokens(&self) -> usize {
        self.conversation.estimate_tokens()
    }

    /// Get the context token budget.
    pub fn context_budget(&self) -> usize {
        CONTEXT_TOKEN_BUDGET
    }

    /// Manually compact the conversation to a target budget.
    /// Returns (tokens_before, tokens_after).
    pub fn compact(&mut self, budget: Option<usize>) -> (usize, usize) {
        let budget = budget.unwrap_or(CONTEXT_TOKEN_BUDGET / 2);
        let before = self.conversation.estimate_tokens();
        self.conversation.truncate_to_budget(budget);
        let after = self.conversation.estimate_tokens();
        (before, after)
    }

    /// Clear conversation history (keep system prompt).
    pub fn clear(&mut self) {
        self.conversation = Conversation::with_system(SYSTEM_PROMPT);
    }
}

/// Response from the orchestrator.
pub struct OrchestratorResponse {
    /// The text content of the response.
    pub content: String,
    /// Total tokens used for this exchange.
    pub tokens_used: u64,
    /// Number of ORGA iterations used.
    #[allow(dead_code)] // will be displayed in footer/status
    pub iterations: u32,
}

/// Human-readable rendering of a [`TerminationReason`] for error messages.
fn describe_termination(reason: &TerminationReason) -> String {
    match reason {
        TerminationReason::Completed => "completed normally".to_string(),
        TerminationReason::MaxIterations => {
            "reached the iteration limit — too many tool calls this turn".to_string()
        }
        TerminationReason::MaxTokens => {
            "hit the per-turn token budget — context is too large".to_string()
        }
        TerminationReason::Timeout => "timed out".to_string(),
        TerminationReason::PolicyDenial { reason } => format!("policy denied: {}", reason),
        TerminationReason::Error { message } => format!("error: {}", message),
    }
}
