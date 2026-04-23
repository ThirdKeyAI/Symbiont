use super::CommandResult;
use crate::app::App;
use crate::session;

pub fn help(_app: &mut App) -> CommandResult {
    CommandResult::Output(
        r#"symbi shell commands:

Session:    /help /clear /quit /model /cost /context
            /snapshot /resume /branch /new /export /copy /compact
Agents:     /spawn /agents /ask /send /pause /resume-agent
            /stop /destroy /debug /memory
Authoring:  /policy /tool /behavior /dsl /init
Execute:    /run /chain /debate /parallel /race
Ops:        /status /monitor /logs /doctor /audit
Cron:       /cron [add|pause|resume|run|history]
Tools:      /tools [validate|test] /skills [verify] /verify
Channels:   /channels /connect /disconnect
Secrets:    /secrets [list|set|delete]"#
            .to_string(),
    )
}

pub fn clear(app: &mut App) -> CommandResult {
    app.output.clear();
    app.reset_flush_cursor();
    if let Some(ref orch) = app.orchestrator {
        if let Ok(mut o) = orch.try_lock() {
            o.clear();
        }
    }
    CommandResult::Output("Cleared.".to_string())
}

pub fn quit(app: &mut App) -> CommandResult {
    app.should_quit = true;
    CommandResult::Handled
}

pub fn dsl_toggle(app: &mut App) -> CommandResult {
    app.toggle_dsl_mode();
    CommandResult::Handled
}

pub fn model(app: &mut App, args: &str) -> CommandResult {
    if args.is_empty() {
        CommandResult::Output(format!("Current model: {}", app.model_name))
    } else {
        app.model_name = args.to_string();
        CommandResult::Output(format!("Model set to: {}", app.model_name))
    }
}

pub fn cost(app: &App) -> CommandResult {
    CommandResult::Output(format!("Tokens used this session: {}", app.tokens_used))
}

pub fn context(app: &mut App) -> CommandResult {
    let orch = match app.orchestrator.as_ref() {
        Some(o) => o,
        None => {
            return CommandResult::Output("No orchestrator — no context to display.".to_string())
        }
    };
    let o = match orch.try_lock() {
        Ok(o) => o,
        Err(_) => return CommandResult::Error("Orchestrator is busy.".to_string()),
    };
    let tokens = o.context_tokens();
    let budget = o.context_budget();
    let usage_pct = if budget > 0 {
        (tokens as f64 / budget as f64) * 100.0
    } else {
        0.0
    };
    CommandResult::Output(format!(
        "Context: {} / {} tokens ({:.1}% of budget)",
        tokens, budget, usage_pct
    ))
}

pub fn compact(app: &mut App, _args: &str) -> CommandResult {
    let orch = match app.orchestrator.as_ref() {
        Some(o) => o,
        None => return CommandResult::Error("No orchestrator — nothing to compact.".to_string()),
    };
    let mut o = match orch.try_lock() {
        Ok(o) => o,
        Err(_) => return CommandResult::Error("Orchestrator is busy.".to_string()),
    };
    let (before, after) = o.compact(None);
    if before == after {
        CommandResult::Output(format!(
            "Context already within budget ({} tokens).",
            before
        ))
    } else {
        CommandResult::Output(format!(
            "Compacted: {} -> {} tokens (freed {})",
            before,
            after,
            before - after
        ))
    }
}

pub fn status(app: &App) -> CommandResult {
    CommandResult::Output(format!(
        "Mode: {:?}\nActive agents: {}\nModel: {}\nTokens: {}",
        app.mode, app.active_agents, app.model_name, app.tokens_used
    ))
}

pub fn snapshot(app: &mut App, args: &str) -> CommandResult {
    let name = if args.trim().is_empty() {
        session::DEFAULT_SNAPSHOT_NAME.to_string()
    } else {
        args.trim().to_string()
    };

    let shell_session = app.build_session_snapshot(&name);
    match session::save_session(&name, &shell_session) {
        Ok(path) => {
            CommandResult::Output(format!("Session saved as '{}' at {}", name, path.display()))
        }
        Err(e) => CommandResult::Error(format!("Failed to save session: {}", e)),
    }
}

pub fn resume(app: &mut App, args: &str) -> CommandResult {
    if args.is_empty() {
        // List available sessions
        match session::list_sessions() {
            Ok(sessions) if sessions.is_empty() => {
                CommandResult::Output("No saved sessions found.".to_string())
            }
            Ok(sessions) => {
                let mut out = String::from("Saved sessions:\n");
                for s in &sessions {
                    out.push_str(&format!("  {}\n", s));
                }
                out.push_str("\nUsage: /resume <name>");
                CommandResult::Output(out)
            }
            Err(e) => CommandResult::Error(format!("Failed to list sessions: {}", e)),
        }
    } else {
        match session::load_session(args.trim()) {
            Ok(shell_session) => {
                let entry_count = shell_session.output.len();
                let had_memory = shell_session.conversation.is_some();
                match app.restore_from_session(shell_session) {
                    Ok(()) => CommandResult::Output(format!(
                        "Resumed session '{}' ({} entries{}).",
                        args.trim(),
                        entry_count,
                        if had_memory {
                            ", orchestrator memory restored"
                        } else {
                            ", no orchestrator memory in saved file"
                        }
                    )),
                    Err(e) => CommandResult::Error(format!("Failed to resume: {}", e)),
                }
            }
            Err(e) => CommandResult::Error(format!("Failed to resume: {}", e)),
        }
    }
}

pub fn new_session(app: &mut App) -> CommandResult {
    app.output.clear();
    app.reset_flush_cursor();
    app.history.clear();
    app.tokens_used = 0;
    app.scroll_to_bottom();
    if let Some(ref orch) = app.orchestrator {
        if let Ok(mut o) = orch.try_lock() {
            o.clear();
        }
    }
    // New session gets its own UUID so the exit auto-save doesn't
    // overwrite the previous run's snapshot.
    app.session_id = uuid::Uuid::new_v4().to_string();
    app.output.push(crate::app::OutputEntry {
        source: crate::app::EntrySource::System,
        content: "New session started.".to_string(),
    });
    CommandResult::Handled
}

pub fn export(app: &App, args: &str) -> CommandResult {
    let shell_session = app.build_session_snapshot("export");

    let text = session::export_session(&shell_session);

    if args.is_empty() {
        CommandResult::Output(format!(
            "Session export ({} lines):\n\n{}",
            text.lines().count(),
            text
        ))
    } else {
        match std::fs::write(args, &text) {
            Ok(()) => CommandResult::Output(format!("Exported to {}", args)),
            Err(e) => CommandResult::Error(format!("Failed to export: {}", e)),
        }
    }
}
