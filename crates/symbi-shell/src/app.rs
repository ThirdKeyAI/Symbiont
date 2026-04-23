// These types and methods are used by the TUI tasks that follow this scaffold.
#![allow(dead_code)]

use crate::commands::{self, CommandResult};
use crate::completion;
use crate::orchestrator::{Orchestrator, OrchestratorResponse};
use crate::session;
use repl_core::{ReplEngine, RuntimeBridge};
use std::sync::Arc;
use throbber_widgets_tui::ThrobberState;
use tokio::sync::oneshot;

/// The two input modes for the shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    /// Bare text goes to orchestrator agent.
    Orchestrator,
    /// Bare text is evaluated as DSL.
    Dsl,
}

/// Scrollable conversation entry.
#[derive(Debug, Clone)]
pub struct OutputEntry {
    /// Who produced this entry.
    pub source: EntrySource,
    /// Rendered text content.
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntrySource {
    User,
    System,
    Agent(String),
    Error,
    /// Dimmed per-turn metadata line (tokens / iterations / duration)
    /// emitted immediately after an agent reply.
    Meta,
    /// A tool invocation inside the ORGA loop — rendered as a card
    /// with `●` header, `⎿`-indented output, and (for edit-shaped
    /// tools) a diff view. The `ToolCallEntry` carries everything
    /// needed to render without a separate state table.
    ToolCall(ToolCallEntry),
    /// Out-of-band agent / runtime notification — e.g. an agent
    /// spawned, a cron job fired, a policy denial from elsewhere in
    /// the runtime, a channel message arrived. Rendered as a dim
    /// single-line banner with an icon keyed on `NoticeKind`.
    Notice {
        kind: NoticeKind,
        /// Short label shown before the content (e.g. "cron:daily",
        /// "agent:writer", "policy").
        source_label: String,
    },
}

/// Notice severity → icon + color in the feed. Mirrors tracing's
/// Info/Warn/Error plus a dedicated Success for completed-work events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoticeKind {
    Info,
    Success,
    Warning,
    Error,
}

/// Rendering state for a single tool invocation.
#[derive(Debug, Clone)]
pub struct ToolCallEntry {
    /// Stable id used to pair live-stream updates with the existing
    /// entry (see the journal polling path).
    pub call_id: String,
    /// Tool name.
    pub name: String,
    /// Short one-line summary of the arguments.
    pub args_summary: String,
    /// Raw JSON arguments — kept for diff rendering on edit tools.
    pub args: String,
    /// Observation body. Empty while the tool is still running.
    pub output: String,
    /// True when the tool has returned.
    pub done: bool,
    /// True when the observation indicates an error.
    pub is_error: bool,
    /// True when the card should render as a file diff instead of
    /// plain output.
    pub is_edit: bool,
    /// Whether the user has expanded this card via Ctrl+O. Cards start
    /// collapsed and truncate to a bounded number of visible lines.
    pub expanded: bool,
    /// Wall-clock at which the card was first pushed in-progress.
    /// Used to compute `duration_ms` when the observation arrives.
    /// `None` for cards that appear post-hoc with no streaming.
    pub started_at: Option<std::time::Instant>,
    /// Wall-clock duration of the tool call in milliseconds, once
    /// known. Populated at finalize from `started_at.elapsed()`.
    pub duration_ms: Option<u64>,
}

// Instant makes this type non-Eq/Hash; callers that need equality
// should compare on (call_id, done) or similar semantic fields.
impl PartialEq for ToolCallEntry {
    fn eq(&self, other: &Self) -> bool {
        self.call_id == other.call_id
            && self.name == other.name
            && self.args_summary == other.args_summary
            && self.args == other.args
            && self.output == other.output
            && self.done == other.done
            && self.is_error == other.is_error
            && self.is_edit == other.is_edit
            && self.expanded == other.expanded
            && self.duration_ms == other.duration_ms
    }
}
impl Eq for ToolCallEntry {}

/// Format an `OrchestratorResponse` as the per-turn meta line.
///
/// Shape: `⎿ 1,273 tokens · 2 iter · 4.2s`. The tokens count is
/// thousands-separated; iteration suffix is `iter` (singular) when the
/// value is 1; duration is seconds with one decimal when ≥1 s,
/// milliseconds otherwise.
pub fn format_response_meta(response: &OrchestratorResponse) -> String {
    let tokens = format_thousands(response.tokens_used);
    // Fixed "iter" label regardless of count — keeps the meta line
    // visually stable as iterations tick up mid-turn.
    let duration = if response.duration_ms >= 1_000 {
        format!("{:.1}s", response.duration_ms as f64 / 1000.0)
    } else {
        format!("{}ms", response.duration_ms)
    };
    format!(
        "⎿ {} tokens · {} iter · {}",
        tokens, response.iterations, duration
    )
}

fn format_thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out.chars().rev().collect()
}

/// Top-level application state.
pub struct App {
    /// Current input mode.
    pub mode: InputMode,
    /// Text currently in the input line.
    pub input: String,
    /// Cursor position within input.
    pub cursor: usize,
    /// Conversation history.
    pub output: Vec<OutputEntry>,
    /// Input history for up/down recall.
    pub history: Vec<String>,
    /// Current position in history (-1 = current input).
    pub history_index: Option<usize>,
    /// Whether the sidebar is visible.
    pub sidebar_visible: bool,
    /// Whether to show memory in the sidebar (toggle with Ctrl+M).
    pub sidebar_show_memory: bool,
    /// Cached memory.md content for sidebar display.
    pub memory_content: Option<String>,
    /// Whether the app should quit.
    pub should_quit: bool,
    /// Active agent count (for footer).
    pub active_agents: usize,
    /// Current model name (for footer).
    pub model_name: String,
    /// Token usage this session (for footer).
    pub tokens_used: u64,
    /// DSL evaluation engine.
    pub engine: ReplEngine,
    /// Completion popup state.
    pub completion_candidates: Vec<completion::Candidate>,
    /// Selected index in completion popup.
    pub completion_index: usize,
    /// Whether completion popup is visible.
    pub completion_visible: bool,
    /// Completion replacement start position.
    pub completion_start: usize,
    /// Known entities for @mention completion (name, kind).
    pub entities: Vec<(String, String)>,
    /// Orchestrator agent (None if no inference provider configured).
    pub orchestrator: Option<Arc<tokio::sync::Mutex<Orchestrator>>>,
    /// Remote connection to a running symbi up instance (when attached).
    pub remote: Option<crate::remote::RemoteConnection>,
    /// Scroll offset for content area (0 = bottom/latest).
    pub scroll_offset: u16,
    /// Throbber state for loading animation.
    pub throbber_state: ThrobberState,
    /// Pending async result from orchestrator.
    pending_result: Option<oneshot::Receiver<Result<OrchestratorResponse, String>>>,
    /// Label shown next to the throbber while busy.
    pub busy_label: String,
    /// Stable UUID for this shell run. Shown in the resume hint on
    /// exit, and used as the filename for the auto-save snapshot.
    pub session_id: String,
    /// Highest journal sequence number we've already consumed for
    /// live-streaming tool-call cards into the feed. Anything ≤ this
    /// has already been rendered; we scan entries > this on each
    /// async tick and push in-progress `ToolCall` cards when we see
    /// a `ReasoningComplete` event whose actions include tool calls.
    pub journal_seen: u64,
    /// Index into `output` marking the first entry that has NOT yet
    /// been flushed into the terminal scrollback via `insert_before`.
    /// Bumped by `drain_unflushed()` each frame. Enables the inline
    /// viewport model: historical entries live in terminal scrollback,
    /// the viewport only paints the input line + popup + footer.
    pub output_flushed: usize,
}

impl App {
    pub fn new(runtime_bridge: Arc<RuntimeBridge>, orchestrator: Option<Orchestrator>) -> Self {
        let engine = ReplEngine::new(runtime_bridge);
        let model_name = orchestrator
            .as_ref()
            .map(|o| o.model_name().to_string())
            .unwrap_or_else(|| "none".to_string());
        let orchestrator = orchestrator.map(|o| Arc::new(tokio::sync::Mutex::new(o)));
        let welcome = if orchestrator.is_some() {
            "Welcome to symbi shell. Type /help for commands, or just talk to the orchestrator."
        } else {
            "Welcome to symbi shell. No inference provider configured — orchestrator disabled.\nSet ANTHROPIC_API_KEY, OPENAI_API_KEY, or OPENROUTER_API_KEY to enable.\nType /help for commands, or /dsl for raw DSL mode."
        };
        Self {
            mode: InputMode::Orchestrator,
            input: String::new(),
            cursor: 0,
            output: vec![OutputEntry {
                source: EntrySource::System,
                content: welcome.to_string(),
            }],
            history: Vec::new(),
            history_index: None,
            sidebar_visible: false,
            sidebar_show_memory: false,
            memory_content: None,
            should_quit: false,
            active_agents: 0,
            model_name,
            tokens_used: 0,
            engine,
            completion_candidates: Vec::new(),
            completion_index: 0,
            completion_visible: false,
            completion_start: 0,
            entities: Vec::new(),
            orchestrator,
            remote: None,
            scroll_offset: 0,
            throbber_state: ThrobberState::default(),
            pending_result: None,
            busy_label: String::new(),
            session_id: uuid::Uuid::new_v4().to_string(),
            journal_seen: 0,
            output_flushed: 0,
        }
    }

    /// Borrow the entries that have accumulated since the last flush
    /// and advance the flush cursor. The caller (the main loop) is
    /// expected to immediately render these into the terminal's
    /// scrollback via `Terminal::insert_before`.
    ///
    /// An in-progress tool-call card (`done == false`) is a hard
    /// boundary — nothing at or after it flushes until the card
    /// becomes `done`. This lets the inline viewport "hold" a
    /// still-running tool card in its live region until the
    /// observation lands, then release the card + everything after
    /// it to scrollback in order.
    pub fn drain_unflushed(&mut self) -> Vec<OutputEntry> {
        if self.output_flushed >= self.output.len() {
            return Vec::new();
        }
        let stop = self.output[self.output_flushed..]
            .iter()
            .position(|e| matches!(&e.source, EntrySource::ToolCall(c) if !c.done))
            .map(|idx| self.output_flushed + idx)
            .unwrap_or(self.output.len());
        if stop == self.output_flushed {
            return Vec::new();
        }
        let pending: Vec<OutputEntry> = self.output[self.output_flushed..stop].to_vec();
        self.output_flushed = stop;
        pending
    }

    /// Return the still-unflushed tail. The main loop renders these
    /// inside the inline viewport (above the input line) so users see
    /// in-progress tool cards live before they settle into scrollback.
    pub fn live_tail(&self) -> &[OutputEntry] {
        &self.output[self.output_flushed..]
    }

    /// Reset the flush cursor — used when `/clear` / `/new` wipes the
    /// visible transcript so subsequent entries stream fresh into
    /// scrollback.
    pub fn reset_flush_cursor(&mut self) {
        self.output_flushed = 0;
    }

    /// Access the DSL evaluation engine.
    pub fn engine(&self) -> &ReplEngine {
        &self.engine
    }

    /// Insert or update a tool-call card keyed on `call_id`.
    ///
    /// On the live-streaming path we push cards while tools are still
    /// running (empty output, `done=false`); when the turn completes
    /// and the post-hoc walk yields the finalized record, this matches
    /// the existing entry and updates it in place. When no card exists
    /// yet (streaming disabled or this is the first observation), a new
    /// entry is appended.
    pub fn upsert_tool_call_card(&mut self, record: &crate::orchestrator::ToolCallRecord) {
        if let Some(existing) = self.output.iter_mut().find_map(|e| match &mut e.source {
            EntrySource::ToolCall(card) if card.call_id == record.call_id => Some(card),
            _ => None,
        }) {
            existing.output = record.output.clone();
            existing.done = true;
            existing.is_error = record.is_error;
            existing.is_edit = record.is_edit;
            // Finalize per-tool duration if we streamed an in-progress
            // card when the tool started. Post-hoc-only cards stay
            // `None`; the renderer falls back to showing the body.
            if let Some(start) = existing.started_at.take() {
                existing.duration_ms = Some(start.elapsed().as_millis() as u64);
            }
            // Keep the user's expand/collapse preference.
            return;
        }
        self.output.push(OutputEntry {
            source: EntrySource::ToolCall(ToolCallEntry {
                call_id: record.call_id.clone(),
                name: record.name.clone(),
                args_summary: record.args_summary.clone(),
                args: record.args.clone(),
                output: record.output.clone(),
                done: true,
                is_error: record.is_error,
                is_edit: record.is_edit,
                expanded: false,
                started_at: None,
                duration_ms: None,
            }),
            content: String::new(),
        });
    }

    /// Toggle the expanded state of the most recent tool-call card.
    /// Returns true when a card was found and toggled.
    pub fn toggle_last_tool_card(&mut self) -> bool {
        for entry in self.output.iter_mut().rev() {
            if let EntrySource::ToolCall(card) = &mut entry.source {
                card.expanded = !card.expanded;
                return true;
            }
        }
        false
    }

    /// Drain new `JournalEntry`s from the orchestrator's buffered
    /// journal and surface them in the feed.
    ///
    /// The most interesting event for UX is `ReasoningComplete`, which
    /// carries the assistant's proposed actions *before* tools execute.
    /// Every `ProposedAction::ToolCall` from a new iteration becomes an
    /// in-progress `ToolCall` card — when the turn completes, the
    /// post-hoc walk in `upsert_tool_call_card` finalizes them with
    /// the observation output.
    pub async fn stream_journal_events(&mut self) {
        let Some(orch_arc) = self.orchestrator.clone() else {
            return;
        };
        let journal = {
            let Ok(guard) = orch_arc.try_lock() else {
                return;
            };
            guard.journal().clone()
        };
        let entries = journal.entries().await;
        if entries.is_empty() {
            return;
        }

        for entry in &entries {
            if entry.sequence <= self.journal_seen {
                continue;
            }
            self.journal_seen = entry.sequence;

            use symbi_runtime::reasoning::loop_types::LoopEvent;
            match &entry.event {
                LoopEvent::ReasoningComplete { actions, .. } => {
                    for action in actions {
                        if let Some(card) = action_to_inprogress_card(action) {
                            self.push_inprogress_card(card);
                        }
                    }
                }
                LoopEvent::RecoveryTriggered {
                    tool_name, error, ..
                } => {
                    // Surface tool-level recovery attempts as notices
                    // so users see the orchestrator is re-trying.
                    self.output.push(OutputEntry {
                        source: EntrySource::Notice {
                            kind: NoticeKind::Warning,
                            source_label: format!("tool:{}", tool_name),
                        },
                        content: format!("retrying after error: {}", error),
                    });
                }
                _ => {}
            }
        }
    }

    /// Push an out-of-band notice into the feed.
    ///
    /// Use this for events the orchestrator didn't generate —
    /// background agent lifecycle, cron triggers, inbound channel
    /// messages, etc. — that the user should see interleaved with the
    /// conversation but not attributed to the model.
    pub fn push_notice(
        &mut self,
        kind: NoticeKind,
        source_label: impl Into<String>,
        content: impl Into<String>,
    ) {
        self.output.push(OutputEntry {
            source: EntrySource::Notice {
                kind,
                source_label: source_label.into(),
            },
            content: content.into(),
        });
        self.scroll_to_bottom();
    }

    /// Add an in-progress `ToolCall` card iff a card for the same
    /// `call_id` isn't already in the feed.
    fn push_inprogress_card(&mut self, card: ToolCallEntry) {
        if self.output.iter().any(|e| {
            matches!(
                &e.source,
                EntrySource::ToolCall(c) if c.call_id == card.call_id
            )
        }) {
            return;
        }
        self.output.push(OutputEntry {
            source: EntrySource::ToolCall(card),
            content: String::new(),
        });
        self.scroll_to_bottom();
    }

    /// Build a `ShellSession` snapshot of current state, ready to hand
    /// to `session::save_session`. Includes the orchestrator's full
    /// conversation so `/resume` restores model memory, not just the
    /// visible transcript.
    pub fn build_session_snapshot(&self, name: &str) -> session::ShellSession {
        let conversation = self.orchestrator.as_ref().and_then(|arc| {
            arc.try_lock()
                .ok()
                .and_then(|o| serde_json::to_value(o.conversation()).ok())
        });
        session::ShellSession {
            version: session::SESSION_SCHEMA_VERSION,
            name: name.to_string(),
            session_id: self.session_id.clone(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            mode: format!("{:?}", self.mode),
            model_name: Some(self.model_name.clone()),
            output: self
                .output
                .iter()
                .map(session::SerializedEntry::from)
                .collect(),
            input_history: self.history.clone(),
            tokens_used: self.tokens_used,
            conversation,
        }
    }

    /// Restore app state from a saved `ShellSession`. Takes ownership of
    /// the session so it can hand the `conversation` JSON off to the
    /// orchestrator without an extra clone.
    pub fn restore_from_session(
        &mut self,
        shell_session: session::ShellSession,
    ) -> anyhow::Result<()> {
        // Visible transcript + input history + token counter.
        self.output = shell_session
            .output
            .iter()
            .map(|e| e.to_output_entry())
            .collect();
        self.history = shell_session.input_history;
        self.tokens_used = shell_session.tokens_used;
        if !shell_session.session_id.is_empty() {
            self.session_id = shell_session.session_id;
        }
        // All restored entries are considered "already printed" —
        // they flush to scrollback on the next frame so the user sees
        // their previous transcript above the viewport immediately.
        self.output_flushed = 0;
        self.scroll_to_bottom();

        // Orchestrator memory, when the file carries it and we actually
        // have an orchestrator to accept it.
        if let (Some(orch), Some(conv_json)) =
            (self.orchestrator.as_ref(), shell_session.conversation)
        {
            let conversation: symbi_runtime::reasoning::conversation::Conversation =
                serde_json::from_value(conv_json).map_err(|e| {
                    anyhow::anyhow!("saved conversation failed to deserialise: {}", e)
                })?;
            // Best-effort: if the mutex is contended we can't restore
            // synchronously; log and move on with the visible
            // transcript alone.
            if let Ok(mut guard) = orch.try_lock() {
                guard.set_conversation(conversation);
            } else {
                tracing::warn!("orchestrator busy — skipped restoring conversation memory");
            }
        }
        Ok(())
    }

    /// Whether the app is waiting for an async operation.
    pub fn is_busy(&self) -> bool {
        self.pending_result.is_some()
    }

    /// Cancel a pending async operation.
    pub fn cancel_pending(&mut self) {
        if self.pending_result.take().is_some() {
            self.busy_label.clear();
            self.output.push(OutputEntry {
                source: EntrySource::System,
                content: "Cancelled.".to_string(),
            });
        }
    }

    /// Called on each tick (~100ms) to advance animations, check pending
    /// results, and stream live journal events (in-progress tool cards).
    pub async fn on_tick(&mut self) {
        self.throbber_state.calc_next();
        self.stream_journal_events().await;

        // Check if a pending result has arrived
        if let Some(ref mut rx) = self.pending_result {
            match rx.try_recv() {
                Ok(Ok(response)) => {
                    self.tokens_used += response.tokens_used;
                    let meta = format_response_meta(&response);

                    // Replace any live-streamed in-progress cards for
                    // this turn with their finalized versions (the
                    // post-hoc walk knows the tool output, the stream
                    // only knew that the tool had started). When no
                    // in-progress card exists for a call_id, we push
                    // the finalized card fresh.
                    for record in &response.tool_calls {
                        self.upsert_tool_call_card(record);
                    }

                    self.output.push(OutputEntry {
                        source: EntrySource::Agent("orchestrator".to_string()),
                        content: response.content,
                    });
                    self.output.push(OutputEntry {
                        source: EntrySource::Meta,
                        content: meta,
                    });
                    self.pending_result = None;
                    self.busy_label.clear();
                    self.scroll_to_bottom();
                }
                Ok(Err(e)) => {
                    self.output.push(OutputEntry {
                        source: EntrySource::Error,
                        content: format!("Orchestrator error: {}", e),
                    });
                    self.pending_result = None;
                    self.busy_label.clear();
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    // Still waiting
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.output.push(OutputEntry {
                        source: EntrySource::Error,
                        content: "Request was dropped".to_string(),
                    });
                    self.pending_result = None;
                    self.busy_label.clear();
                }
            }
        }
    }

    /// Scroll up in the content area.
    pub fn scroll_up(&mut self, lines: u16) {
        self.scroll_offset = self.scroll_offset.saturating_add(lines);
    }

    /// Scroll down in the content area (towards latest).
    pub fn scroll_down(&mut self, lines: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    /// Reset scroll to bottom (latest output).
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
    }

    /// Toggle memory display in sidebar and reload content.
    pub fn toggle_sidebar_memory(&mut self) {
        self.sidebar_show_memory = !self.sidebar_show_memory;
        if self.sidebar_show_memory {
            self.reload_memory();
            if !self.sidebar_visible {
                self.sidebar_visible = true;
            }
        }
    }

    /// Reload memory.md content from disk.
    pub fn reload_memory(&mut self) {
        // Look for memory in common locations
        let paths = [
            "data/agents/orchestrator/memory.md",
            ".symbiont/memory.md",
            ".symbi/memory.md",
        ];
        for path in &paths {
            if let Ok(content) = std::fs::read_to_string(path) {
                self.memory_content = Some(content);
                return;
            }
        }
        self.memory_content = None;
    }

    /// Toggle between Orchestrator and DSL modes.
    pub fn toggle_dsl_mode(&mut self) {
        self.mode = match self.mode {
            InputMode::Orchestrator => InputMode::Dsl,
            InputMode::Dsl => InputMode::Orchestrator,
        };
        let msg = match self.mode {
            InputMode::Dsl => "Entered DSL mode. Type /dsl or /exit to return.",
            InputMode::Orchestrator => "Returned to orchestrator mode.",
        };
        self.output.push(OutputEntry {
            source: EntrySource::System,
            content: msg.to_string(),
        });
    }

    /// Push user input into history and return it.
    pub fn submit_input(&mut self) -> String {
        let text = self.input.drain(..).collect::<String>();
        self.cursor = 0;
        self.history_index = None;
        if !text.is_empty() {
            self.history.push(text.clone());
        }
        text
    }

    /// Get the prompt string for the current mode.
    pub fn prompt(&self) -> &str {
        match self.mode {
            InputMode::Orchestrator => "> ",
            InputMode::Dsl => "dsl> ",
        }
    }

    /// Handle submitted input: dispatch to /command or record as DSL/orchestrator input.
    pub async fn handle_input(&mut self, text: &str) {
        // Auto-scroll to bottom on new input
        self.scroll_to_bottom();

        // Record user input in output
        self.output.push(OutputEntry {
            source: EntrySource::User,
            content: text.to_string(),
        });

        // Special case: /exit in DSL mode returns to orchestrator
        if self.mode == InputMode::Dsl && text == "/exit" {
            self.toggle_dsl_mode();
            return;
        }

        // /command dispatch
        if text.starts_with('/') {
            let (cmd, args) = match text.find(' ') {
                Some(pos) => (&text[..pos], text[pos + 1..].trim()),
                None => (text, ""),
            };
            match commands::dispatch(self, cmd, args) {
                Some(CommandResult::Output(msg)) => {
                    self.output.push(OutputEntry {
                        source: EntrySource::System,
                        content: msg,
                    });
                }
                Some(CommandResult::Error(msg)) => {
                    self.output.push(OutputEntry {
                        source: EntrySource::Error,
                        content: msg,
                    });
                }
                Some(CommandResult::Handled) => {}
                None => {
                    self.output.push(OutputEntry {
                        source: EntrySource::System,
                        content: format!(
                            "Unknown command: {}. Type /help for available commands.",
                            cmd
                        ),
                    });
                }
            }
            return;
        }

        // DSL mode: evaluate expression
        if self.mode == InputMode::Dsl {
            let rt = match tokio::runtime::Handle::try_current() {
                Ok(handle) => handle,
                Err(_) => {
                    self.output.push(OutputEntry {
                        source: EntrySource::Error,
                        content: "No async runtime available".to_string(),
                    });
                    return;
                }
            };
            let result = tokio::task::block_in_place(|| rt.block_on(self.engine.evaluate(text)));
            match result {
                Ok(output) => {
                    self.output.push(OutputEntry {
                        source: EntrySource::System,
                        content: output,
                    });
                }
                Err(e) => {
                    self.output.push(OutputEntry {
                        source: EntrySource::Error,
                        content: e.to_string(),
                    });
                }
            }
            return;
        }

        // Orchestrator mode: send to LLM (async, non-blocking)
        self.send_to_orchestrator(text, "Thinking...");
    }

    /// Send a message to the orchestrator asynchronously.
    /// The response will arrive via `on_tick()` polling the pending_result channel.
    /// Returns false if no orchestrator is configured.
    pub fn send_to_orchestrator(&mut self, message: &str, busy_label: &str) -> bool {
        let orchestrator = match self.orchestrator.as_ref() {
            Some(o) => Arc::clone(o),
            None => {
                self.output.push(OutputEntry {
                    source: EntrySource::Error,
                    content: "No inference provider configured. Set ANTHROPIC_API_KEY, OPENAI_API_KEY, or OPENROUTER_API_KEY.\nUse /dsl for raw DSL mode.".to_string(),
                });
                return false;
            }
        };

        let (tx, rx) = oneshot::channel();
        let message = message.to_string();
        self.busy_label = busy_label.to_string();
        self.pending_result = Some(rx);

        tokio::spawn(async move {
            let mut orch = orchestrator.lock().await;
            let result = orch.send(&message).await.map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        true
    }

    /// Navigate input history upward.
    pub fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let idx = match self.history_index {
            None => self.history.len() - 1,
            Some(0) => return,
            Some(i) => i - 1,
        };
        self.history_index = Some(idx);
        self.input = self.history[idx].clone();
        self.cursor = self.input.len();
    }

    /// Navigate input history downward.
    pub fn history_down(&mut self) {
        match self.history_index {
            None => (),
            Some(i) if i >= self.history.len() - 1 => {
                self.history_index = None;
                self.input.clear();
                self.cursor = 0;
            }
            Some(i) => {
                self.history_index = Some(i + 1);
                self.input = self.history[i + 1].clone();
                self.cursor = self.input.len();
            }
        }
    }

    /// Trigger completion based on current input.
    pub fn trigger_completion(&mut self) {
        let dsl_mode = self.mode == InputMode::Dsl;
        let (start, candidates) =
            completion::complete(&self.input, self.cursor, &self.entities, dsl_mode);
        self.completion_start = start;
        self.completion_candidates = candidates;
        self.completion_index = 0;
        self.completion_visible = !self.completion_candidates.is_empty();
    }

    /// Accept the currently selected completion.
    pub fn accept_completion(&mut self) {
        if let Some(candidate) = self.completion_candidates.get(self.completion_index) {
            let replacement = candidate.replacement.clone();
            self.input
                .replace_range(self.completion_start..self.cursor, &replacement);
            self.cursor = self.completion_start + replacement.len();
        }
        self.dismiss_completion();
    }

    /// Returns true when accepting the currently highlighted completion
    /// would leave the input unchanged — i.e. the user has already typed
    /// the full suggestion. The Enter handler uses this to decide whether
    /// to submit the line or "accept" a no-op completion first.
    ///
    /// Without this check, typing `/exit` + Enter required pressing Enter
    /// twice: the first press would "accept" `/exit` (no-op, but still
    /// dismisses the popup), and the second would actually submit.
    pub fn completion_accept_is_noop(&self) -> bool {
        if !self.completion_visible {
            return false;
        }
        let Some(candidate) = self.completion_candidates.get(self.completion_index) else {
            return true; // Nothing highlighted → accepting changes nothing.
        };
        // Guard against an out-of-range window (shouldn't happen, but we
        // don't want to index-panic from a stale completion_start/cursor).
        let start = self.completion_start;
        let end = self.cursor;
        if start > end || end > self.input.len() {
            return false;
        }
        self.input[start..end] == candidate.replacement
    }

    /// Move selection up in the completion popup.
    pub fn completion_up(&mut self) {
        if !self.completion_candidates.is_empty() {
            if self.completion_index > 0 {
                self.completion_index -= 1;
            } else {
                self.completion_index = self.completion_candidates.len() - 1;
            }
        }
    }

    /// Move selection down in the completion popup.
    pub fn completion_down(&mut self) {
        if !self.completion_candidates.is_empty() {
            if self.completion_index < self.completion_candidates.len() - 1 {
                self.completion_index += 1;
            } else {
                self.completion_index = 0;
            }
        }
    }

    /// Dismiss the completion popup.
    pub fn dismiss_completion(&mut self) {
        self.completion_visible = false;
        self.completion_candidates.clear();
    }

    /// Refresh entity list from engine.
    pub fn refresh_entities(&mut self) {
        let rt = match tokio::runtime::Handle::try_current() {
            Ok(h) => h,
            Err(_) => return,
        };
        let items = tokio::task::block_in_place(|| rt.block_on(self.engine.completion_items()));
        self.entities = items
            .into_iter()
            .map(|(name, kind)| (name, kind.to_string()))
            .collect();
    }
}

/// Convert a proposed action from the reasoning loop into an
/// in-progress `ToolCallEntry` (no observation yet). Non-tool actions
/// return `None` — `Respond` / `Delegate` / `Terminate` are already
/// surfaced through the regular orchestrator response path.
fn action_to_inprogress_card(
    action: &symbi_runtime::reasoning::loop_types::ProposedAction,
) -> Option<ToolCallEntry> {
    use symbi_runtime::reasoning::loop_types::ProposedAction;
    let ProposedAction::ToolCall {
        name,
        arguments,
        call_id,
    } = action
    else {
        return None;
    };
    let args_string = arguments.to_string();
    let args_summary = crate::orchestrator::summarise_tool_args(name, &args_string);
    let is_edit = crate::orchestrator::looks_like_edit_tool(name, &args_string);
    Some(ToolCallEntry {
        call_id: call_id.clone(),
        name: name.clone(),
        args_summary,
        args: args_string,
        output: String::new(),
        done: false,
        is_error: false,
        is_edit,
        expanded: false,
        started_at: Some(std::time::Instant::now()),
        duration_ms: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app() -> App {
        App::new(Arc::new(RuntimeBridge::new_permissive_for_dev()), None)
    }

    #[test]
    fn test_new_app_starts_in_orchestrator_mode() {
        let app = test_app();
        assert_eq!(app.mode, InputMode::Orchestrator);
        assert_eq!(app.prompt(), "> ");
    }

    #[test]
    fn test_toggle_dsl_mode() {
        let mut app = test_app();
        app.toggle_dsl_mode();
        assert_eq!(app.mode, InputMode::Dsl);
        assert_eq!(app.prompt(), "dsl> ");
        app.toggle_dsl_mode();
        assert_eq!(app.mode, InputMode::Orchestrator);
    }

    #[test]
    fn test_submit_input_clears_and_records_history() {
        let mut app = test_app();
        app.input = "hello world".to_string();
        app.cursor = 11;
        let text = app.submit_input();
        assert_eq!(text, "hello world");
        assert!(app.input.is_empty());
        assert_eq!(app.cursor, 0);
        assert_eq!(app.history, vec!["hello world"]);
    }

    #[test]
    fn test_submit_empty_input_not_added_to_history() {
        let mut app = test_app();
        let text = app.submit_input();
        assert_eq!(text, "");
        assert!(app.history.is_empty());
    }

    #[tokio::test]
    async fn test_handle_input_quit() {
        let mut app = test_app();
        app.handle_input("/quit").await;
        assert!(app.should_quit);
    }

    #[tokio::test]
    async fn test_handle_input_dsl_toggle() {
        let mut app = test_app();
        app.handle_input("/dsl").await;
        assert_eq!(app.mode, InputMode::Dsl);
        app.handle_input("/exit").await;
        assert_eq!(app.mode, InputMode::Orchestrator);
    }

    #[tokio::test]
    async fn test_handle_input_help() {
        let mut app = test_app();
        app.handle_input("/help").await;
        let last = app.output.last().unwrap();
        assert!(last.content.contains("/spawn"));
        assert_eq!(last.source, EntrySource::System);
    }

    #[tokio::test]
    async fn test_handle_input_unknown_command() {
        let mut app = test_app();
        app.handle_input("/nonexistent").await;
        let last = app.output.last().unwrap();
        assert!(last.content.contains("Unknown command"));
    }

    #[tokio::test]
    async fn test_handle_input_records_user_entry() {
        let mut app = test_app();
        app.handle_input("hello").await;
        // With no orchestrator, should show error about missing provider
        let user_entry = &app.output[1];
        assert_eq!(user_entry.source, EntrySource::User);
        assert_eq!(user_entry.content, "hello");
    }

    #[test]
    fn test_history_navigation() {
        let mut app = test_app();
        app.history = vec!["first".into(), "second".into(), "third".into()];
        app.history_up();
        assert_eq!(app.input, "third");
        app.history_up();
        assert_eq!(app.input, "second");
        app.history_down();
        assert_eq!(app.input, "third");
        app.history_down();
        assert!(app.input.is_empty());
    }

    // ── Completion-Enter interaction ──────────────────────────────────
    //
    // Regression tests for the "Enter twice to exit" bug: when the
    // completion popup is visible but the highlighted candidate is
    // already fully typed, Enter should submit, not "accept".

    fn set_input(app: &mut App, s: &str) {
        app.input = s.to_string();
        app.cursor = s.len();
    }

    #[test]
    fn completion_accept_is_noop_when_popup_hidden() {
        let app = test_app();
        assert!(!app.completion_accept_is_noop());
    }

    #[test]
    fn completion_accept_is_noop_when_input_matches_candidate() {
        let mut app = test_app();
        set_input(&mut app, "/exit");
        app.completion_visible = true;
        app.completion_start = 0;
        app.completion_candidates = vec![crate::completion::Candidate {
            display: "/exit".into(),
            replacement: "/exit".into(),
            score: 0,
            summary: None,
            category: None,
        }];
        app.completion_index = 0;
        assert!(
            app.completion_accept_is_noop(),
            "accepting /exit when /exit is already typed must be a no-op"
        );
    }

    #[test]
    fn completion_accept_is_not_noop_when_input_is_prefix() {
        let mut app = test_app();
        set_input(&mut app, "/ex");
        app.completion_visible = true;
        app.completion_start = 0;
        app.completion_candidates = vec![crate::completion::Candidate {
            display: "/exit".into(),
            replacement: "/exit".into(),
            score: 0,
            summary: None,
            category: None,
        }];
        app.completion_index = 0;
        assert!(
            !app.completion_accept_is_noop(),
            "Enter should accept /exit when only /ex has been typed"
        );
    }

    #[test]
    fn format_response_meta_renders_expected_shape() {
        let r = OrchestratorResponse {
            content: String::new(),
            tokens_used: 1273,
            iterations: 2,
            duration_ms: 4200,
            tool_calls: vec![],
        };
        assert_eq!(format_response_meta(&r), "⎿ 1,273 tokens · 2 iter · 4.2s");
    }

    #[test]
    fn format_response_meta_uses_millis_under_one_second() {
        let r = OrchestratorResponse {
            content: String::new(),
            tokens_used: 42,
            iterations: 1,
            duration_ms: 850,
            tool_calls: vec![],
        };
        assert_eq!(format_response_meta(&r), "⎿ 42 tokens · 1 iter · 850ms");
    }

    #[test]
    fn format_thousands_separator() {
        assert_eq!(format_thousands(0), "0");
        assert_eq!(format_thousands(999), "999");
        assert_eq!(format_thousands(1_000), "1,000");
        assert_eq!(format_thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn push_notice_adds_entry_with_kind_and_label() {
        let mut app = test_app();
        app.push_notice(NoticeKind::Success, "cron:daily", "fired");
        let last = app.output.last().unwrap();
        match &last.source {
            EntrySource::Notice { kind, source_label } => {
                assert_eq!(*kind, NoticeKind::Success);
                assert_eq!(source_label, "cron:daily");
            }
            other => panic!("expected Notice, got {:?}", other),
        }
        assert_eq!(last.content, "fired");
    }

    #[test]
    fn upsert_tool_call_card_inserts_then_updates() {
        use crate::orchestrator::ToolCallRecord;
        let mut app = test_app();

        let rec = ToolCallRecord {
            call_id: "c1".into(),
            name: "validate_dsl".into(),
            args: "{}".into(),
            args_summary: "agent=writer".into(),
            output: "".into(),
            is_error: false,
            is_edit: false,
        };
        app.upsert_tool_call_card(&rec);
        assert!(matches!(
            app.output.last().map(|e| &e.source),
            Some(EntrySource::ToolCall(c)) if c.call_id == "c1" && c.output.is_empty()
        ));

        // Finalize with output — must update the existing entry, not
        // create a second one.
        let final_rec = ToolCallRecord {
            output: "OK: valid".into(),
            ..rec
        };
        app.upsert_tool_call_card(&final_rec);
        let tool_entries = app
            .output
            .iter()
            .filter(|e| matches!(&e.source, EntrySource::ToolCall(_)))
            .count();
        assert_eq!(tool_entries, 1);
        match &app.output.last().unwrap().source {
            EntrySource::ToolCall(c) => {
                assert_eq!(c.call_id, "c1");
                assert_eq!(c.output, "OK: valid");
                assert!(c.done);
            }
            _ => panic!("expected ToolCall"),
        }
    }

    #[test]
    fn toggle_last_tool_card_flips_expanded_state() {
        use crate::orchestrator::ToolCallRecord;
        let mut app = test_app();
        app.upsert_tool_call_card(&ToolCallRecord {
            call_id: "c1".into(),
            name: "bash".into(),
            args: "{\"command\":\"ls\"}".into(),
            args_summary: "ls".into(),
            output: (0..20).map(|i| format!("line{}\n", i)).collect(),
            is_error: false,
            is_edit: false,
        });
        let initial_expanded = matches!(
            &app.output.last().unwrap().source,
            EntrySource::ToolCall(c) if c.expanded
        );
        assert!(!initial_expanded);
        assert!(app.toggle_last_tool_card());
        assert!(matches!(
            &app.output.last().unwrap().source,
            EntrySource::ToolCall(c) if c.expanded
        ));
        assert!(app.toggle_last_tool_card());
        assert!(matches!(
            &app.output.last().unwrap().source,
            EntrySource::ToolCall(c) if !c.expanded
        ));
    }

    #[test]
    fn toggle_last_tool_card_returns_false_without_tool_cards() {
        let mut app = test_app();
        app.output.push(OutputEntry {
            source: EntrySource::System,
            content: "no tool cards here".into(),
        });
        assert!(!app.toggle_last_tool_card());
    }

    #[test]
    fn completion_accept_is_noop_handles_out_of_range_window() {
        // Stale completion state shouldn't panic or claim no-op when
        // the window is no longer valid against the current input.
        let mut app = test_app();
        set_input(&mut app, "hi");
        app.completion_visible = true;
        app.completion_start = 10;
        app.completion_candidates = vec![crate::completion::Candidate {
            display: "hello".into(),
            replacement: "hello".into(),
            score: 0,
            summary: None,
            category: None,
        }];
        app.completion_index = 0;
        assert!(!app.completion_accept_is_noop());
    }
}
