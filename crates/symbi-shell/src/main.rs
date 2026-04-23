use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use ratatui::prelude::*;
use ratatui::{TerminalOptions, Viewport};
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

/// Parsed CLI flags — kept minimal so we don't drag in clap for the
/// handful of flags the shell exposes.
struct ShellArgs {
    /// `--resume <id-or-name>` — restore a saved session before the
    /// first render. Accepts a UUID (from a prior auto-save) or a
    /// named snapshot (`/snapshot foo`).
    resume: Option<String>,
    /// `--list-sessions` — print the saved sessions and exit.
    list_sessions: bool,
    /// `--cleanup-sessions` — delete stale session files and exit.
    /// Requires `--older-than` to specify the cutoff.
    cleanup_sessions: bool,
    /// Cutoff for `--list-sessions` (filter) and `--cleanup-sessions`
    /// (delete). Format: `30d` / `12h` / `5m` / `90s`.
    older_than: Option<std::time::Duration>,
    /// `--dry-run` with `--cleanup-sessions` — report what would be
    /// deleted without touching disk.
    dry_run: bool,
    /// `--profile <name>` — isolate session/state dir under
    /// `$HOME/.symbi-<name>/` instead of `$HOME/.symbi/`.
    profile: Option<String>,
    /// `--yes` — pre-approve orchestrator save/create actions so the
    /// shell can be scripted without interactive "looks good" replies.
    auto_approve: bool,
    /// `--theme <name>` — select a built-in theme. User TOML at
    /// `$HOME/.symbi[-<profile>]/theme.toml` still wins if present.
    theme: Option<String>,
}

fn parse_args() -> Result<ShellArgs> {
    let mut resume: Option<String> = None;
    let mut list_sessions = false;
    let mut cleanup_sessions = false;
    let mut older_than: Option<std::time::Duration> = None;
    let mut dry_run = false;
    let mut profile: Option<String> = None;
    let mut auto_approve = false;
    let mut theme: Option<String> = None;

    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            "--version" => {
                println!("symbi-shell {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--list-sessions" => list_sessions = true,
            "--cleanup-sessions" => cleanup_sessions = true,
            "--dry-run" => dry_run = true,
            "-y" | "--yes" => auto_approve = true,
            "--resume" => {
                resume =
                    Some(iter.next().ok_or_else(|| {
                        anyhow::anyhow!("--resume requires a session id or name")
                    })?);
            }
            flag if flag.starts_with("--resume=") => {
                resume = Some(flag.trim_start_matches("--resume=").to_string());
            }
            "--older-than" => {
                let v = iter.next().ok_or_else(|| {
                    anyhow::anyhow!("--older-than requires a duration (e.g. 30d)")
                })?;
                older_than = Some(session::parse_duration(&v)?);
            }
            flag if flag.starts_with("--older-than=") => {
                older_than = Some(session::parse_duration(
                    flag.trim_start_matches("--older-than="),
                )?);
            }
            "--profile" => {
                profile = Some(
                    iter.next()
                        .ok_or_else(|| anyhow::anyhow!("--profile requires a name"))?,
                );
            }
            flag if flag.starts_with("--profile=") => {
                profile = Some(flag.trim_start_matches("--profile=").to_string());
            }
            "--theme" => {
                theme = Some(
                    iter.next()
                        .ok_or_else(|| anyhow::anyhow!("--theme requires a name"))?,
                );
            }
            flag if flag.starts_with("--theme=") => {
                theme = Some(flag.trim_start_matches("--theme=").to_string());
            }
            other => {
                return Err(anyhow::anyhow!("unknown argument: {}", other));
            }
        }
    }
    Ok(ShellArgs {
        resume,
        list_sessions,
        cleanup_sessions,
        older_than,
        dry_run,
        profile,
        auto_approve,
        theme,
    })
}

fn print_help() {
    println!(
        "symbi-shell — interactive agent orchestration shell\n\
         \n\
         USAGE:\n    symbi-shell [FLAGS]\n\
         \n\
         FLAGS:\n\
             --resume <id|name>     Resume a saved session (UUID from a prior exit,\n\
                                    or a name set via /snapshot).\n\
             --list-sessions        List saved sessions and exit. Combine with\n\
                                    --older-than to filter to stale sessions.\n\
             --cleanup-sessions     Delete sessions older than --older-than and exit.\n\
                                    Add --dry-run to preview without deleting.\n\
             --older-than <dur>     Age cutoff for --list-sessions / --cleanup-sessions.\n\
                                    Accepts 30d / 12h / 90m / 45s.\n\
             --dry-run              With --cleanup-sessions, print names but do not\n\
                                    delete anything.\n\
             --profile <name>       Isolate session/state dir under\n\
                                    $HOME/.symbi-<name>/ for parallel workspaces.\n\
         -y, --yes                  Pre-approve orchestrator save/create actions\n\
                                    so scripted flows don't block on confirmation.\n\
             --theme <name>         Select a built-in theme (default-dark,\n\
                                    solarized-dark, high-contrast). A user file at\n\
                                    $HOME/.symbi[-<profile>]/theme.toml overrides this.\n\
             --version              Print version and exit.\n\
         -h, --help                 Show this help and exit.\n\
         \n\
         Sessions are stored under $HOME/.symbi/sessions/ (or $HOME/.symbi-<profile>/\n\
         when --profile is set). The SYMBIONT_SESSION_DIR env var overrides both.\n\
         On clean exit the shell auto-saves to <uuid>.json and prints the resume command.\n"
    );
}

/// Detect whether we're running inside the Zellij terminal multiplexer.
///
/// Zellij sets `$ZELLIJ` in every child pane. It does not implement
/// DECSTBM scroll regions, which ratatui's inline `insert_before`
/// relies on to flush settled entries into the host terminal's
/// scrollback. Detection today is used only to warn the user that
/// above-viewport scrollback may be missing; a full Zellij-safe
/// render path is task #103.
fn running_inside_zellij() -> bool {
    std::env::var("ZELLIJ").is_ok()
}

/// When `--profile <name>` is given, point the sessions dir at
/// `$HOME/.symbi-<name>/sessions` by setting `SYMBIONT_SESSION_DIR`
/// unless the env var is already explicitly set (which wins). This is
/// the least-invasive way to thread the profile through every session
/// read/write without changing every helper signature.
fn apply_profile(profile: &str) {
    if std::env::var(session::SESSION_DIR_ENV).is_ok() {
        return;
    }
    if let Some(mut home) = dirs::home_dir() {
        home.push(format!(".symbi-{}", profile));
        home.push("sessions");
        std::env::set_var(session::SESSION_DIR_ENV, home);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!("run `symbi-shell --help` for usage");
            std::process::exit(2);
        }
    };

    if let Some(profile) = args.profile.as_deref() {
        apply_profile(profile);
    }

    // Install the theme before any UI module touches a color.
    // Resolution order: user TOML → --theme → $SYMBI_THEME → default.
    let theme_spec = match ui::theme::resolve(args.theme.as_deref()) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(2);
        }
    };
    ui::theme::init(theme_spec);

    let in_zellij = running_inside_zellij();

    if args.list_sessions {
        return handle_list_sessions(args.older_than);
    }

    if args.cleanup_sessions {
        return handle_cleanup_sessions(args.older_than, args.dry_run);
    }

    // Inline viewport: ratatui draws a bounded region at the bottom
    // of the terminal (input + popup + throbber) while the rest of the
    // terminal stays "native". New transcript entries (user input,
    // agent replies, tool cards, notices, meta lines) are pushed above
    // the viewport via `Terminal::insert_before` so they flow into the
    // terminal's real scrollback. Closing the shell leaves the full
    // transcript behind in scrollback.
    //
    // Height budget: 12 popup rows + 1 border + 1 input + 1 footer = 15.
    const INLINE_VIEWPORT_ROWS: u16 = 15;
    enable_raw_mode()?;
    let mut terminal = Terminal::with_options(
        CrosstermBackend::new(stdout()),
        TerminalOptions {
            viewport: Viewport::Inline(INLINE_VIEWPORT_ROWS),
        },
    )?;

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

        Some(orchestrator::Orchestrator::new(
            provider,
            executor,
            args.auto_approve,
        ))
    } else {
        None
    };

    let mut app = App::new(runtime_bridge, orch);

    // Make the flags we're running under visible in the transcript so
    // the user can confirm they took effect without digging in docs.
    if args.auto_approve {
        app.output.push(app::OutputEntry {
            source: app::EntrySource::System,
            content: "Auto-approve (--yes) enabled — orchestrator saves without asking."
                .to_string(),
        });
    }
    if let Some(ref profile) = args.profile {
        app.output.push(app::OutputEntry {
            source: app::EntrySource::System,
            content: format!(
                "Profile '{}' — sessions under {}",
                profile,
                session::session_dir().display()
            ),
        });
    }
    if in_zellij {
        // Zellij doesn't implement DECSTBM scroll regions, which ratatui's
        // `Terminal::insert_before` relies on to flush settled entries into
        // real scrollback. The TUI still functions but the transcript will
        // not persist above the live viewport the way it does in
        // iTerm2/Kitty/Alacritty/tmux. A Zellij-safe render path using
        // raw-newline scrolling is filed as task #103.
        app.push_notice(
            app::NoticeKind::Warning,
            "zellij".to_string(),
            "Detected Zellij — scrollback above the inline viewport may not render. Use a native terminal or tmux for full fidelity until the Zellij-safe render path lands.".to_string(),
        );
    }

    // Best-effort resume: if the caller asked for a specific session and
    // it loads, restore it; otherwise show a visible system note and
    // continue with a fresh session rather than hard-failing.
    let mut resume_status: Option<String> = None;
    if let Some(ref requested) = args.resume {
        match session::load_session(requested) {
            Ok(saved) => {
                let entries = saved.output.len();
                let had_memory = saved.conversation.is_some();
                match app.restore_from_session(saved) {
                    Ok(()) => {
                        resume_status = Some(format!(
                            "Resumed '{}' — {} entries{}",
                            requested,
                            entries,
                            if had_memory {
                                ", memory restored"
                            } else {
                                ", no memory in saved file"
                            }
                        ));
                    }
                    Err(e) => {
                        resume_status = Some(format!("Could not restore '{}': {}", requested, e));
                    }
                }
            }
            Err(e) => {
                resume_status = Some(format!("Could not resume '{}': {}", requested, e));
            }
        }
    }
    if let Some(msg) = resume_status {
        app.output.push(app::OutputEntry {
            source: app::EntrySource::System,
            content: msg,
        });
    }

    let session_id = app.session_id.clone();
    let result = run_loop(&mut terminal, &mut app).await;

    // Auto-save before restoring the terminal so a crashed save doesn't
    // leave the terminal in a weird state either way.
    let snapshot = app.build_session_snapshot(&session_id);
    let save_result = session::save_session(&session_id, &snapshot);

    // Before dropping raw mode, push a final blank line below the
    // inline viewport so the shell's prompt lands on a fresh row —
    // otherwise the viewport rectangle remains in place visually
    // until the terminal redraws. `clear()` scrolls the viewport
    // region off; then we disable raw mode so the parent shell takes
    // back input handling.
    let _ = terminal.clear();
    drop(terminal);
    disable_raw_mode()?;

    // Resume hint is printed to stdout AFTER dropping the terminal
    // so the user actually sees it in their terminal scrollback.
    match save_result {
        Ok(path) => {
            println!();
            // OSC-8: in a supporting terminal the path is clickable and
            // opens the session file in the user's default handler; in
            // a plain terminal the link text reads identically.
            let display = path.display().to_string();
            let link = if ui::osc8::stdout_is_tty() {
                ui::osc8::file_link(&path, &display)
            } else {
                display
            };
            println!("Session saved to {}", link);
            println!();
            println!("Resume this session with:");
            println!("    symbi-shell --resume {}", session_id);
        }
        Err(e) => {
            eprintln!("\nsymbi-shell: failed to auto-save session: {}", e);
            eprintln!("Your transcript was not persisted. Run `/snapshot <name>` next time to save manually.");
        }
    }

    result
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    let mut last_tick = Instant::now();

    loop {
        // Flush any entries that have settled since the last frame
        // into terminal scrollback. This is what makes the shell feel
        // "inline" — tool cards, agent replies, notices flow upward
        // into the real terminal buffer rather than being redrawn in
        // an alternate-screen rectangle on every frame.
        let pending = app.drain_unflushed();
        if !pending.is_empty() {
            let lines = ui::content::render_entries_to_lines(&pending);
            // Height passed to insert_before is the number of rows we
            // need above the viewport. Line wrapping isn't accounted
            // for here; long lines will truncate to their first row in
            // scrollback. This matches how inline TUIs typically flush.
            let height = lines.len() as u16;
            if height > 0 {
                use ratatui::layout::Rect;
                use ratatui::widgets::{Paragraph, Wrap};
                terminal.insert_before(height, |buf| {
                    let area = Rect::new(0, 0, buf.area.width, height);
                    Paragraph::new(lines)
                        .wrap(Wrap { trim: false })
                        .render(area, buf);
                })?;
            }
        }

        terminal.draw(|frame| ui::draw(frame, app))?;

        let timeout = TICK_RATE.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) => handle_key(app, key).await,
                // Resize: ratatui's inline viewport needs an explicit
                // clear+redraw to re-anchor below the new screen
                // height, otherwise a shrink leaves the viewport
                // offscreen and a grow leaves stale content above it.
                Event::Resize(_, _) => {
                    let _ = terminal.clear();
                }
                _ => {} // Mouse, focus, paste — ignored today.
            }
        }

        // Tick: advance throbber animation and check for pending results
        if last_tick.elapsed() >= TICK_RATE {
            app.on_tick().await;
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
        (KeyCode::Backspace, _) if app.cursor > 0 => {
            app.cursor -= 1;
            app.input.remove(app.cursor);
            app.trigger_completion();
        }
        (KeyCode::Delete, _) if app.cursor < app.input.len() => {
            app.input.remove(app.cursor);
            app.trigger_completion();
        }
        (KeyCode::Left, _) if app.cursor > 0 => {
            app.cursor -= 1;
            app.dismiss_completion();
        }
        (KeyCode::Right, _) if app.cursor < app.input.len() => {
            app.cursor += 1;
            app.dismiss_completion();
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

        // Expand / collapse the most recent tool-call card. Mirrors the
        // "… +N more (ctrl+o)" hint rendered in collapsed card bodies.
        (KeyCode::Char('o'), KeyModifiers::CONTROL) => {
            app.toggle_last_tool_card();
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

/// Implement the `--list-sessions [--older-than <dur>]` early-exit path.
fn handle_list_sessions(older_than: Option<std::time::Duration>) -> Result<()> {
    let sessions = session::list_sessions_with_metadata()
        .map_err(|e| anyhow::anyhow!("Failed to list sessions: {}", e))?;

    if sessions.is_empty() {
        println!("No saved sessions.");
        return Ok(());
    }

    let now = std::time::SystemTime::now();
    let filtered: Vec<&session::SessionInfo> = match older_than {
        Some(age) => {
            let cutoff = now
                .checked_sub(age)
                .ok_or_else(|| anyhow::anyhow!("cutoff underflow — duration too large"))?;
            sessions.iter().filter(|s| s.modified <= cutoff).collect()
        }
        None => sessions.iter().collect(),
    };

    if filtered.is_empty() {
        println!("No sessions match the filter.");
        return Ok(());
    }

    println!("Saved sessions ({}):", filtered.len());
    for info in filtered {
        let age = now
            .duration_since(info.modified)
            .unwrap_or_default()
            .as_secs();
        println!("  {}  ({})", info.name, format_age(age));
    }
    Ok(())
}

/// Implement the `--cleanup-sessions --older-than <dur> [--dry-run]`
/// early-exit path.
fn handle_cleanup_sessions(older_than: Option<std::time::Duration>, dry_run: bool) -> Result<()> {
    let cutoff = older_than.ok_or_else(|| {
        anyhow::anyhow!("--cleanup-sessions requires --older-than <duration> (e.g. 30d)")
    })?;
    let report = session::cleanup_sessions(cutoff, dry_run)
        .map_err(|e| anyhow::anyhow!("Cleanup failed: {}", e))?;

    let verb = if report.dry_run {
        "Would remove"
    } else {
        "Removed"
    };
    if report.removed.is_empty() {
        println!("No sessions older than cutoff. {} kept.", report.kept);
    } else {
        println!("{} {} session(s):", verb, report.removed.len());
        for name in &report.removed {
            println!("  {}", name);
        }
        println!("{} kept.", report.kept);
    }
    Ok(())
}

fn format_age(secs: u64) -> String {
    if secs < 60 {
        format!("{}s ago", secs)
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}
