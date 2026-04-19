// These types and methods are used by the TUI tasks that follow this scaffold.
#![allow(dead_code)]

use crate::commands::{self, CommandResult};
use crate::completion;
use crate::orchestrator::{Orchestrator, OrchestratorResponse};
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
        }
    }

    /// Access the DSL evaluation engine.
    pub fn engine(&self) -> &ReplEngine {
        &self.engine
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

    /// Called on each tick (~100ms) to advance animations and check pending results.
    pub fn on_tick(&mut self) {
        self.throbber_state.calc_next();

        // Check if a pending result has arrived
        if let Some(ref mut rx) = self.pending_result {
            match rx.try_recv() {
                Ok(Ok(response)) => {
                    self.tokens_used += response.tokens_used;
                    self.output.push(OutputEntry {
                        source: EntrySource::Agent("orchestrator".to_string()),
                        content: response.content,
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
