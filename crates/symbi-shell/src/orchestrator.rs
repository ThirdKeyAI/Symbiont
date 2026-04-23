//! Orchestrator module — ORGA-governed conversational agent.
//!
//! The orchestrator uses the full Observe-Reason-Gate-Act loop instead of
//! raw LLM calls. This ensures every action (including artifact generation)
//! passes through Cedar policy gates and is recorded in the audit journal.

use anyhow::{anyhow, Result};
use std::sync::Arc;
use symbi_runtime::reasoning::conversation::{
    Conversation, ConversationMessage, MessageRole, ToolCall,
};
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
- save_artifact: Save a validated artifact to disk
- list_agents: List all running agents

When the user asks you to create an artifact (agent, policy, tool manifest):
1. Generate the appropriate DSL/Cedar/TOML.
2. Call the matching validate_* tool; if it reports errors, fix the artifact and re-validate until it passes.
3. Decide whether to save immediately or present for review:
   - If the user's request was specific (named a profile, described a concrete need, or told you to scaffold/init/create/save), call save_artifact for each validated file without asking again — their request IS the approval.
   - If the request was exploratory ("what would a policy for X look like?", "sketch something"), present for review and wait for an explicit approve-or-edit reply before calling save_artifact.
4. After saving, briefly tell the user what was saved and suggest the next command to run (for example /spawn <agent-name>).
5. If validation keeps failing after two attempts, stop, show the user the latest artifact plus the validator's errors, and ask how they want to proceed.

Keep responses concise and actionable. You are running inside symbi shell — a terminal-based orchestration interface."#;

/// The orchestrator manages the ORGA-governed conversation loop.
pub struct Orchestrator {
    runner: ReasoningLoopRunner,
    conversation: Conversation,
    model_name: String,
    agent_id: AgentId,
    #[allow(dead_code)] // used by /audit command (Task 25)
    journal: Arc<BufferedJournal>,
    /// The system prompt in effect for this orchestrator — the base
    /// constant plus any runtime addenda (e.g. `--yes` auto-approve).
    /// Stored so `/clear` can rebuild a fresh conversation with the
    /// same prompt instead of dropping back to the default.
    system_prompt: String,
}

/// System-prompt addendum applied when the shell was started with `--yes`.
///
/// Turns the default "ask for review on exploratory requests" rule off:
/// launching with `--yes` declares up front that every save is approved,
/// so the orchestrator should never withhold save_artifact waiting for a
/// "looks good" reply.
const AUTO_APPROVE_ADDENDUM: &str = r#"

The user launched symbi-shell with --yes (auto-approve mode): every save/create/scaffold request is pre-approved. Do not present artifacts for review — call save_artifact immediately for every artifact that passes validation. Skip the "present for review" branch entirely."#;

impl Orchestrator {
    /// Create a new orchestrator with ORGA loop governance.
    ///
    /// `auto_approve` comes from the shell's `--yes` CLI flag; when set,
    /// the system prompt is extended so the model saves without asking.
    pub fn new(
        provider: Arc<dyn InferenceProvider>,
        executor: Arc<dyn ActionExecutor>,
        auto_approve: bool,
    ) -> Self {
        let model_name = provider.default_model().to_string();
        let prompt = if auto_approve {
            format!("{}{}", SYSTEM_PROMPT, AUTO_APPROVE_ADDENDUM)
        } else {
            SYSTEM_PROMPT.to_string()
        };
        let conversation = Conversation::with_system(&prompt);
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
            system_prompt: prompt,
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

        // Remember where this turn starts so we can extract only the
        // tool-call / tool-observation pairs added during *this* turn
        // and not resurface ones from prior turns that are still in the
        // conversation.
        let turn_start = self.conversation.messages().len();

        let config = LoopConfig {
            max_iterations: LOOP_MAX_ITERATIONS,
            max_total_tokens: LOOP_TOKEN_BUDGET,
            context_token_budget: CONTEXT_TOKEN_BUDGET,
            temperature: 0.3,
            ..LoopConfig::default()
        };

        // Transient-error retry: on a 429 / rate-limit / 5xx / timeout,
        // wait briefly and retry once with the same model. Keeps common
        // provider hiccups from failing the whole turn. Multi-model
        // fallback would need a runner-API change; keeping that out of
        // scope until there's a concrete user need.
        const RETRY_BACKOFF: std::time::Duration = std::time::Duration::from_millis(1500);
        let mut result = self
            .runner
            .run(self.agent_id, self.conversation.clone(), config.clone())
            .await;
        if is_transient_error(&result.termination_reason) {
            tracing::warn!(
                "orchestrator: transient error ({}), retrying after {:?}",
                describe_termination(&result.termination_reason),
                RETRY_BACKOFF,
            );
            tokio::time::sleep(RETRY_BACKOFF).await;
            result = self
                .runner
                .run(self.agent_id, self.conversation.clone(), config)
                .await;
        }

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
                    duration_ms: result.duration.as_millis() as u64,
                    tool_calls: extract_tool_calls(&self.conversation, turn_start),
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
            duration_ms: result.duration.as_millis() as u64,
            tool_calls: extract_tool_calls(&self.conversation, turn_start),
        })
    }

    /// Get the model name for display.
    pub fn model_name(&self) -> &str {
        &self.model_name
    }

    /// Get the journal for audit display / live event streaming.
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

    /// Clear conversation history (keep system prompt, including any
    /// `--yes` addendum the shell started with).
    pub fn clear(&mut self) {
        self.conversation = Conversation::with_system(&self.system_prompt);
    }

    /// Borrow the current conversation so the caller can persist it
    /// (session snapshot on `/snapshot` / exit auto-save).
    pub fn conversation(&self) -> &Conversation {
        &self.conversation
    }

    /// Replace the current conversation with one loaded from a saved
    /// session. The caller is responsible for having fed a well-formed
    /// `Conversation` — typically deserialised from the session file.
    /// The existing in-flight history is discarded.
    pub fn set_conversation(&mut self, conversation: Conversation) {
        self.conversation = conversation;
    }
}

/// Response from the orchestrator — one turn of the ORGA loop.
pub struct OrchestratorResponse {
    /// The text content of the response.
    pub content: String,
    /// Total tokens used for this turn (input + output + tool observations).
    pub tokens_used: u64,
    /// Number of ORGA iterations used this turn.
    pub iterations: u32,
    /// Wall-clock milliseconds the loop took for this turn.
    pub duration_ms: u64,
    /// Tool calls the model made this turn, paired with their observations.
    /// Rendered in the UI as `● name(args)` / `⎿ output` cards between
    /// the user message and the assistant reply.
    pub tool_calls: Vec<ToolCallRecord>,
}

/// A single tool invocation recovered from the post-turn conversation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolCallRecord {
    /// Provider-assigned ID (used to pair calls with tool observations).
    pub call_id: String,
    /// Tool name — e.g. `"validate_dsl"`, `"save_artifact"`.
    pub name: String,
    /// Raw JSON arguments string exactly as the model produced it.
    pub args: String,
    /// Short human-readable summary of `args` shown in the card header.
    pub args_summary: String,
    /// Observation / output from the tool. Empty when the tool has
    /// started but has not yet produced a result (live-streaming path).
    pub output: String,
    /// Whether the observation indicates a tool-level error.
    pub is_error: bool,
    /// True when the tool looks like a file edit and the card should
    /// render as a diff rather than a plain `⎿`-indented paragraph.
    pub is_edit: bool,
}

/// Walk a completed `Conversation` (starting at `start`) and extract
/// tool-call / observation pairs so the shell can render one card per
/// tool invocation. `start` is the message index captured before the
/// current turn began, which scopes extraction to this turn's work.
pub fn extract_tool_calls(conversation: &Conversation, start: usize) -> Vec<ToolCallRecord> {
    let messages = conversation.messages();
    let turn = messages.get(start..).unwrap_or(&[]);
    let mut records: Vec<ToolCallRecord> = Vec::new();

    for (i, msg) in turn.iter().enumerate() {
        if !matches!(msg.role, MessageRole::Assistant) || msg.tool_calls.is_empty() {
            continue;
        }
        for call in &msg.tool_calls {
            let observation = find_tool_observation(&turn[i + 1..], &call.id);
            let (output, is_error) = observation
                .map(|m| (m.content.clone(), looks_like_error(&m.content)))
                .unwrap_or_default();
            records.push(ToolCallRecord {
                call_id: call.id.clone(),
                name: call.name.clone(),
                args: call.arguments.clone(),
                args_summary: summarise_args(call),
                output,
                is_error,
                is_edit: is_edit_tool(call),
            });
        }
    }
    records
}

/// Find the `Tool` observation matching `call_id` in the messages that
/// follow the assistant tool_calls message.
fn find_tool_observation<'a>(
    after: &'a [ConversationMessage],
    call_id: &str,
) -> Option<&'a ConversationMessage> {
    after
        .iter()
        .find(|m| matches!(m.role, MessageRole::Tool) && m.tool_call_id.as_deref() == Some(call_id))
}

/// Lightweight heuristic — the runtime doesn't mark tool errors in a
/// dedicated field, so we look for a leading `Error:` / `error` hint.
/// False positives here are harmless: the UI just tints the body red.
fn looks_like_error(content: &str) -> bool {
    let head = content.trim_start();
    head.starts_with("Error") || head.starts_with("error:") || head.starts_with("ERROR")
}

/// Tools whose effect is "write / modify a file or artifact" get the
/// diff-style card. The list is conservative on purpose — everything
/// else renders as plain truncated output.
fn is_edit_tool(call: &ToolCall) -> bool {
    looks_like_edit_tool(&call.name, &call.arguments)
}

/// Same predicate as `is_edit_tool` but taking the name + args string
/// directly — used by the live-streaming path where we have a
/// `ProposedAction::ToolCall` instead of a recorded `ToolCall`.
pub fn looks_like_edit_tool(name: &str, args: &str) -> bool {
    matches!(
        name,
        "save_artifact" | "write_file" | "edit_file" | "apply_edit"
    ) || args.contains("\"old_string\"")
}

/// One-line summary of a tool call's arguments for the card header.
///
/// Heuristic: for Bash-style tools with a `command` field, show the
/// command verbatim (truncated). For edit-style tools, show the file
/// path. For everything else, fall back to "{name}={value}" for the
/// first 2–3 string fields, truncated to ~60 chars total.
fn summarise_args(call: &ToolCall) -> String {
    summarise_tool_args(&call.name, &call.arguments)
}

/// Name + raw-JSON version of [`summarise_args`] for the live-streaming
/// path, where we only have the proposed action (name + JSON args).
pub fn summarise_tool_args(_name: &str, arguments: &str) -> String {
    const MAX_LEN: usize = 80;

    let Ok(value) = serde_json::from_str::<serde_json::Value>(arguments) else {
        // Non-JSON or malformed: show truncated raw string.
        return truncate_one_line(arguments, MAX_LEN);
    };

    let obj = match value.as_object() {
        Some(o) => o,
        None => return truncate_one_line(arguments, MAX_LEN),
    };

    if let Some(cmd) = obj.get("command").and_then(|v| v.as_str()) {
        return truncate_one_line(cmd, MAX_LEN);
    }
    for key in ["path", "file_path", "filename", "artifact_path"] {
        if let Some(p) = obj.get(key).and_then(|v| v.as_str()) {
            return truncate_one_line(p, MAX_LEN);
        }
    }

    let mut parts: Vec<String> = Vec::new();
    for (k, v) in obj.iter().take(3) {
        let rendered = match v {
            serde_json::Value::String(s) => truncate_one_line(s, 30),
            other => other.to_string(),
        };
        parts.push(format!("{}={}", k, rendered));
    }
    truncate_one_line(&parts.join(", "), MAX_LEN)
}

fn truncate_one_line(s: &str, max_len: usize) -> String {
    // Replace newlines with spaces so the header stays single-line.
    let flat: String = s.chars().map(|c| if c == '\n' { ' ' } else { c }).collect();
    if flat.chars().count() <= max_len {
        return flat;
    }
    let take = max_len.saturating_sub(1);
    let truncated: String = flat.chars().take(take).collect();
    format!("{}…", truncated)
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

/// Whether a termination reason looks like a transient provider issue
/// worth retrying. Matches the substring markers we see from the
/// HTTP-backed cloud provider (status codes and common error phrases).
///
/// This is intentionally lenient — a false positive costs one extra
/// round-trip; a false negative leaves the user staring at a failed
/// turn they'd have to re-send anyway.
fn is_transient_error(reason: &TerminationReason) -> bool {
    let TerminationReason::Error { message } = reason else {
        return false;
    };
    let lower = message.to_ascii_lowercase();
    lower.contains("429")
        || lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("503")
        || lower.contains("502")
        || lower.contains("504")
        || lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("connection reset")
        || lower.contains("connection refused")
        || lower.contains("overloaded")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_transient_detects_429() {
        let r = TerminationReason::Error {
            message: "HTTP 429 Too Many Requests".to_string(),
        };
        assert!(is_transient_error(&r));
    }

    #[test]
    fn is_transient_detects_503_overloaded() {
        let r = TerminationReason::Error {
            message: "Upstream provider 503 overloaded".to_string(),
        };
        assert!(is_transient_error(&r));
    }

    #[test]
    fn is_transient_false_for_completed_and_policy() {
        assert!(!is_transient_error(&TerminationReason::Completed));
        assert!(!is_transient_error(&TerminationReason::MaxTokens));
        assert!(!is_transient_error(&TerminationReason::PolicyDenial {
            reason: "denied".to_string()
        }));
    }

    #[test]
    fn is_transient_false_for_non_transient_error() {
        let r = TerminationReason::Error {
            message: "JSON parse failed: unexpected token".to_string(),
        };
        assert!(!is_transient_error(&r));
    }
}
