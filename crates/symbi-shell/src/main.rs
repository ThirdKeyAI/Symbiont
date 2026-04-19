use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::prelude::*;
use std::io::stdout;
use std::sync::Arc;
use std::time::{Duration, Instant};

mod app;
mod commands;
mod completion;
mod deploy;
mod orchestrator;
mod orchestrator_executor;
mod remote;
mod secrets_store;
mod session;
mod ui;
mod validation;

use app::App;

const TICK_RATE: Duration = Duration::from_millis(100);

#[tokio::main]
async fn main() -> Result<()> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    // symbi shell is a local interactive dev tool; opt into permissive bus
    // semantics explicitly so deploying a multi-agent DSL composition works
    // out of the box. Production deployments must use the server crate with
    // an explicit policy instead of this shell.
    let runtime_bridge = Arc::new(repl_core::RuntimeBridge::new_permissive_for_dev());

    // Load project constraints for artifact validation
    let constraints = Arc::new(
        validation::constraints::ProjectConstraints::load(std::path::Path::new(
            ".symbi/constraints.toml",
        ))
        .unwrap_or_default(),
    );

    // Auto-detect inference provider and create ORGA-governed orchestrator
    let orch = if let Some(provider) =
        symbi_runtime::reasoning::providers::cloud::CloudInferenceProvider::from_env()
    {
        let provider = Arc::new(provider);
        runtime_bridge.set_inference_provider(Arc::clone(&provider)
            as Arc<dyn symbi_runtime::reasoning::inference::InferenceProvider>);

        // Create the orchestrator's action executor with validation tools
        let engine = Arc::new(repl_core::ReplEngine::new(Arc::clone(&runtime_bridge)));
        let executor = Arc::new(orchestrator_executor::OrchestratorExecutor::new(
            Arc::clone(&constraints),
            engine,
        ));

        Some(orchestrator::Orchestrator::new(provider, executor))
    } else {
        None
    };

    let mut app = App::new(runtime_bridge, orch);
    let result = run_loop(&mut terminal, &mut app).await;

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;

    result
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;

        let timeout = TICK_RATE.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                handle_key(app, key).await;
            }
        }

        // Tick: advance throbber animation and check for pending results
        if last_tick.elapsed() >= TICK_RATE {
            app.on_tick();
            last_tick = Instant::now();
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

async fn handle_key(app: &mut App, key: KeyEvent) {
    // Ignore most keys while a request is pending (except Ctrl+C to cancel)
    if app.is_busy() {
        if let (KeyCode::Char('c'), KeyModifiers::CONTROL) = (key.code, key.modifiers) {
            app.cancel_pending();
        }
        return;
    }

    match (key.code, key.modifiers) {
        // Quit
        (KeyCode::Char('d'), KeyModifiers::CONTROL) => app.should_quit = true,
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            if app.input.is_empty() {
                app.should_quit = true;
            } else {
                app.input.clear();
                app.cursor = 0;
            }
        }

        // Submit input / accept completion.
        //
        // If the popup is visible but the highlighted candidate is
        // already fully typed (e.g. the user typed "/exit" — the popup
        // still shows "/exit" as the sole suggestion), accepting would
        // be a no-op. Treat that press as a submit so the user doesn't
        // have to hit Enter twice.
        (KeyCode::Enter, _) => {
            if app.completion_visible && !app.completion_accept_is_noop() {
                app.accept_completion();
            } else {
                if app.completion_visible {
                    app.dismiss_completion();
                }
                let text = app.submit_input();
                if !text.is_empty() {
                    app.handle_input(&text).await;
                }
            }
        }

        // Up/Down — navigate completion popup, or history when popup hidden
        (KeyCode::Up, _) => {
            if app.completion_visible {
                app.completion_up();
            } else {
                app.history_up();
            }
        }
        (KeyCode::Down, _) => {
            if app.completion_visible {
                app.completion_down();
            } else {
                app.history_down();
            }
        }

        // Editing
        (KeyCode::Backspace, _) => {
            if app.cursor > 0 {
                app.cursor -= 1;
                app.input.remove(app.cursor);
                app.trigger_completion();
            }
        }
        (KeyCode::Delete, _) => {
            if app.cursor < app.input.len() {
                app.input.remove(app.cursor);
                app.trigger_completion();
            }
        }
        (KeyCode::Left, _) => {
            if app.cursor > 0 {
                app.cursor -= 1;
                app.dismiss_completion();
            }
        }
        (KeyCode::Right, _) => {
            if app.cursor < app.input.len() {
                app.cursor += 1;
                app.dismiss_completion();
            }
        }
        (KeyCode::Home, _) => {
            app.cursor = 0;
            app.dismiss_completion();
        }
        (KeyCode::End, _) => {
            app.cursor = app.input.len();
            app.dismiss_completion();
        }

        // Toggle sidebar
        (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
            app.sidebar_visible = !app.sidebar_visible;
        }
        // Toggle memory in sidebar
        (KeyCode::Char('m'), KeyModifiers::CONTROL) => {
            app.toggle_sidebar_memory();
        }

        // Tab — also accepts completion (alternative to Enter)
        (KeyCode::Tab, _) => {
            if app.completion_visible {
                app.accept_completion();
            } else {
                app.trigger_completion();
            }
        }
        // Escape — dismiss completion
        (KeyCode::Esc, _) => {
            app.dismiss_completion();
        }

        // Scroll content
        (KeyCode::PageUp, _) => app.scroll_up(10),
        (KeyCode::PageDown, _) => app.scroll_down(10),

        // Regular character input — auto-trigger completion on / and @
        (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            app.input.insert(app.cursor, c);
            app.cursor += 1;
            if c == '/' || c == '@' || app.completion_visible {
                app.trigger_completion();
            }
        }

        _ => {}
    }
}
