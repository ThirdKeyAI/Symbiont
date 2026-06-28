use super::CommandResult;
use crate::app::App;

pub fn ask(app: &mut App, args: &str) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error("Usage: /ask @agent <message>".to_string());
    }
    let prompt = format!(
        "The user wants to send a message to a specific agent and wait for a response. \
         Their request: {}\n\nUse the list_agents tool to find the agent, then relay the message.",
        args
    );
    if app.send_to_orchestrator(&prompt, "Asking agent...") {
        CommandResult::Handled
    } else {
        CommandResult::Error("No inference provider configured.".to_string())
    }
}

pub fn send(app: &mut App, args: &str) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error("Usage: /send @agent <message>".to_string());
    }
    let prompt = format!(
        "The user wants to send a fire-and-forget message to an agent. \
         Their request: {}\n\nUse the list_agents tool to find the agent, then send the message.",
        args
    );
    if app.send_to_orchestrator(&prompt, "Sending...") {
        CommandResult::Handled
    } else {
        CommandResult::Error("No inference provider configured.".to_string())
    }
}

pub fn memory(_app: &mut App, args: &str) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Output(
            "Usage: /memory inspect|compact|purge <agent-id>\n\
             Or press Ctrl+M to toggle memory display in sidebar."
                .to_string(),
        );
    }
    let parts: Vec<&str> = args.splitn(2, ' ').collect();
    match parts[0] {
        "inspect" => {
            let agent_id = parts.get(1).copied().unwrap_or("orchestrator");
            let path = format!("data/agents/{}/memory.md", agent_id);
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    CommandResult::Output(format!("Memory for {}:\n\n{}", agent_id, content))
                }
                Err(_) => {
                    CommandResult::Output(format!("No memory found for agent '{}'", agent_id))
                }
            }
        }
        "compact" => CommandResult::Output(
            "[Memory compaction requires StandardContextManager — use /compact for orchestrator context]"
                .to_string(),
        ),
        "purge" => {
            let agent_id = parts.get(1).copied().unwrap_or("");
            if agent_id.is_empty() {
                return CommandResult::Error("Usage: /memory purge <agent-id>".to_string());
            }
            let path = format!("data/agents/{}", agent_id);
            match std::fs::remove_dir_all(&path) {
                Ok(()) => CommandResult::Output(format!("Purged memory for agent '{}'", agent_id)),
                Err(e) => CommandResult::Error(format!("Failed to purge: {}", e)),
            }
        }
        _ => CommandResult::Error("Usage: /memory inspect|compact|purge <agent-id>".to_string()),
    }
}

pub fn resume_agent(app: &mut App, args: &str) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error("Usage: /resume-agent <agent-id>".to_string());
    }
    let rt = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => return CommandResult::Error("No async runtime available".to_string()),
    };
    let id = match args.parse::<uuid::Uuid>() {
        Ok(id) => id,
        Err(_) => return CommandResult::Error(format!("Invalid agent ID: {}", args)),
    };
    match tokio::task::block_in_place(|| rt.block_on(app.engine().evaluator().resume_agent(id))) {
        Ok(()) => CommandResult::Output(format!("Resumed agent {}", &id.to_string()[..8])),
        Err(e) => CommandResult::Error(e.to_string()),
    }
}

/// `/agents list | load <dir> | reload` — manage the loaded agent fleet.
///
/// `list` (the default) renders the synchronous fleet mirror. `load <dir>`
/// scans the given directory for TOML manifests and registers them;
/// `reload` re-scans the default `./agents` directory.
pub fn agents_command(app: &mut App, args: &str) -> CommandResult {
    let mut parts = args.split_whitespace();
    match parts.next() {
        Some("list") | None => match app.agent_cards.try_read() {
            Ok(cards) if !cards.is_empty() => CommandResult::Output(render_list(&cards)),
            Ok(_) => {
                CommandResult::Output("No agents loaded. Use `/agents load <dir>`.".to_string())
            }
            Err(_) => CommandResult::Error("Agent list busy; try again.".to_string()),
        },
        Some("load") => match parts.next() {
            Some(p) => run_load(app, p),
            None => CommandResult::Error("Usage: /agents load <dir>".to_string()),
        },
        Some("reload") => run_load(app, "agents"),
        Some(other) => CommandResult::Error(format!(
            "Unknown /agents subcommand '{other}'. Available: list, load, reload"
        )),
    }
}

/// `/agent use <name|orchestrator>` | `/agent clear [name]` | `/agent [status]`.
pub fn agent_focus_command(app: &mut App, args: &str) -> CommandResult {
    let mut parts = args.split_whitespace();
    match parts.next() {
        Some("use") => match parts.next() {
            Some("orchestrator") | Some("orch") => {
                app.focus_agent = None;
                CommandResult::Output("Now talking to ORCH.".into())
            }
            Some(name) => {
                let known = app
                    .agent_cards
                    .try_read()
                    .map(|c| c.iter().any(|a| a.name == name))
                    .unwrap_or(false);
                if !known {
                    return CommandResult::Error(format!("No agent '{name}'. Try `/agents list`."));
                }
                app.focus_agent = Some(name.to_string());
                CommandResult::Output(format!(
                    "Now talking to @{name}. `/agent clear` to return to ORCH."
                ))
            }
            None => CommandResult::Error("Usage: /agent use <name|orchestrator>".into()),
        },
        Some("clear") => {
            if let Some(name) = parts.next() {
                app.agent_runners.remove(name);
                CommandResult::Output(format!("Cleared conversation thread for @{name}."))
            } else {
                app.focus_agent = None;
                CommandResult::Output("Returned to ORCH.".into())
            }
        }
        None | Some("status") => CommandResult::Output(match &app.focus_agent {
            Some(n) => format!("Talking to @{n}."),
            None => "Talking to ORCH.".into(),
        }),
        Some(o) => CommandResult::Error(format!(
            "Unknown /agent subcommand '{o}'. Available: use, clear, status"
        )),
    }
}

/// Render the loaded fleet for `/agents list` (pure helper so it's unit-testable).
fn render_list(cards: &[crate::agents::AgentCard]) -> String {
    let mut out = String::from("Loaded agents:\n");
    for c in cards {
        out.push_str(&format!("  {} — {}\n", c.name, c.description));
    }
    out
}

/// Scan `dir`, register agents into the runtime, refresh the mirror, and
/// report the outcome. Runs the async loader from this sync handler via the
/// same `block_in_place` + current-runtime-handle pattern other commands use.
fn run_load(app: &mut App, dir: &str) -> CommandResult {
    let rt = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => return CommandResult::Error("No async runtime available".to_string()),
    };
    let bridge = app.runtime_bridge_handle();
    let cards = app.agent_cards.clone();
    let report = tokio::task::block_in_place(|| {
        rt.block_on(crate::agents::load_agents_into(
            std::path::Path::new(dir),
            &bridge,
            &cards,
        ))
    });
    // A (re)load can change an agent's manifest tools or system prompt. Cached
    // per-agent runners froze their tool scope at first use, so drop them all —
    // the next `@name` rebuilds against the freshly registered definition.
    app.agent_runners.clear();
    let mut msg = format!("Loaded {} agent(s) from {dir}.", report.loaded);
    if !report.sandbox_refused.is_empty() {
        msg.push_str(&format!(
            "\n  {} .symbi agent(s) refused (sandbox tier — run via `symbi up`/`symbi run`): {}",
            report.sandbox_refused.len(),
            report.sandbox_refused.join(", ")
        ));
    }
    for c in &report.collisions {
        msg.push_str(&format!("\n  collision (last wins): {c}"));
    }
    for e in &report.errors {
        msg.push_str(&format!("\n  skipped {}: {}", e.path.display(), e.message));
    }
    // If the focused agent is no longer in the freshly rebuilt mirror, drop the
    // stale focus so we don't keep routing to a vanished agent. Skip silently if
    // the mirror lock is momentarily unavailable.
    if let Some(name) = app.focus_agent.clone() {
        if let Ok(cards) = app.agent_cards.try_read() {
            if !cards.iter().any(|c| c.name == name) {
                drop(cards);
                app.focus_agent = None;
                msg.push_str(&format!(
                    "\n  focus on '@{name}' cleared (no longer loaded)"
                ));
            }
        }
    }
    CommandResult::Output(msg)
}

pub fn debug(app: &mut App, args: &str) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error("Usage: /debug <agent-id or @name>".to_string());
    }

    let rt = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => return CommandResult::Error("No async runtime available".to_string()),
    };

    let id = match args.parse::<uuid::Uuid>() {
        Ok(id) => id,
        Err(_) => return CommandResult::Error(format!("Invalid agent ID: {}", args)),
    };

    match tokio::task::block_in_place(|| rt.block_on(app.engine().evaluator().debug_agent(id))) {
        Ok(info) => CommandResult::Output(info),
        Err(e) => CommandResult::Error(e.to_string()),
    }
}

pub fn stop(app: &mut App, args: &str) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error("Usage: /stop <agent-id>".to_string());
    }

    let rt = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => return CommandResult::Error("No async runtime available".to_string()),
    };

    let id = match args.parse::<uuid::Uuid>() {
        Ok(id) => id,
        Err(_) => return CommandResult::Error(format!("Invalid agent ID: {}", args)),
    };

    match tokio::task::block_in_place(|| rt.block_on(app.engine().evaluator().stop_agent(id))) {
        Ok(()) => CommandResult::Output(format!("Stopped agent {}", &id.to_string()[..8])),
        Err(e) => CommandResult::Error(e.to_string()),
    }
}

pub fn pause(app: &mut App, args: &str) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error("Usage: /pause <agent-id>".to_string());
    }

    let rt = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => return CommandResult::Error("No async runtime available".to_string()),
    };

    let id = match args.parse::<uuid::Uuid>() {
        Ok(id) => id,
        Err(_) => return CommandResult::Error(format!("Invalid agent ID: {}", args)),
    };

    match tokio::task::block_in_place(|| rt.block_on(app.engine().evaluator().pause_agent(id))) {
        Ok(()) => CommandResult::Output(format!("Paused agent {}", &id.to_string()[..8])),
        Err(e) => CommandResult::Error(e.to_string()),
    }
}

pub fn destroy(app: &mut App, args: &str) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error("Usage: /destroy <agent-id>".to_string());
    }

    let rt = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => return CommandResult::Error("No async runtime available".to_string()),
    };

    let id = match args.parse::<uuid::Uuid>() {
        Ok(id) => id,
        Err(_) => return CommandResult::Error(format!("Invalid agent ID: {}", args)),
    };

    match tokio::task::block_in_place(|| rt.block_on(app.engine().evaluator().destroy_agent(id))) {
        Ok(()) => CommandResult::Output(format!("Destroyed agent {}", &id.to_string()[..8])),
        Err(e) => CommandResult::Error(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::AgentCard;
    use std::sync::Arc;

    /// Build a minimal `App` for command-level unit tests, mirroring the
    /// `test_app` helper in `app.rs` (private to that module).
    fn test_app() -> App {
        App::new(
            Arc::new(repl_core::RuntimeBridge::new_permissive_for_dev()),
            None,
            Arc::new(tokio::sync::RwLock::new(Vec::new())),
            None,
        )
    }

    fn push_card(app: &App, name: &str) {
        app.agent_cards.blocking_write().push(AgentCard {
            name: name.into(),
            description: format!("{name} desc"),
            tools: vec![],
        });
    }

    fn unwrap_output(r: CommandResult) -> String {
        match r {
            CommandResult::Output(s) => s,
            CommandResult::Error(e) => panic!("expected Output, got Error: {e}"),
            CommandResult::Handled => panic!("expected Output, got Handled"),
        }
    }

    #[test]
    fn agent_focus_use_known_sets_focus() {
        let mut app = test_app();
        push_card(&app, "worker");
        let r = agent_focus_command(&mut app, "use worker");
        assert_eq!(app.focus_agent.as_deref(), Some("worker"));
        assert!(matches!(r, CommandResult::Output(_)));
    }

    #[test]
    fn agent_focus_use_unknown_errors() {
        let mut app = test_app();
        push_card(&app, "worker");
        app.focus_agent = Some("worker".into());
        let r = agent_focus_command(&mut app, "use ghost");
        assert!(matches!(r, CommandResult::Error(_)));
        // Focus unchanged.
        assert_eq!(app.focus_agent.as_deref(), Some("worker"));
    }

    #[test]
    fn agent_focus_clear_resets() {
        let mut app = test_app();
        app.focus_agent = Some("worker".into());
        let r = agent_focus_command(&mut app, "clear");
        assert_eq!(app.focus_agent, None);
        assert!(matches!(r, CommandResult::Output(_)));
    }

    #[test]
    fn agent_focus_status_reports() {
        let mut app = test_app();
        app.focus_agent = Some("worker".into());
        let s = unwrap_output(agent_focus_command(&mut app, "status"));
        assert!(s.contains("worker"));
        // Empty args also reports status.
        let s2 = unwrap_output(agent_focus_command(&mut app, ""));
        assert!(s2.contains("worker"));
    }

    #[test]
    fn render_list_includes_all_names() {
        let cards = vec![
            AgentCard {
                name: "a".into(),
                description: "first".into(),
                tools: vec![],
            },
            AgentCard {
                name: "b".into(),
                description: "second".into(),
                tools: vec![],
            },
        ];
        let out = render_list(&cards);
        assert!(out.contains("a") && out.contains("b"));
    }

    // Minimal provider so we can build a real per-agent runner to insert.
    struct Mock;
    #[async_trait::async_trait]
    impl symbi_runtime::reasoning::inference::InferenceProvider for Mock {
        async fn complete(
            &self,
            _c: &symbi_runtime::reasoning::conversation::Conversation,
            _o: &symbi_runtime::reasoning::inference::InferenceOptions,
        ) -> Result<
            symbi_runtime::reasoning::inference::InferenceResponse,
            symbi_runtime::reasoning::inference::InferenceError,
        > {
            Ok(symbi_runtime::reasoning::inference::InferenceResponse {
                content: "ok".into(),
                tool_calls: vec![],
                finish_reason: symbi_runtime::reasoning::inference::FinishReason::Stop,
                usage: symbi_runtime::reasoning::inference::Usage::default(),
                model: "mock".into(),
            })
        }
        fn provider_name(&self) -> &str {
            "mock"
        }
        fn default_model(&self) -> &str {
            "mock"
        }
        fn supports_native_tools(&self) -> bool {
            false
        }
        fn supports_structured_output(&self) -> bool {
            false
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reload_clears_cached_agent_runners() {
        let mut app = test_app();
        // Seed a cached per-agent runner.
        let runner = crate::orchestrator::Orchestrator::for_agent(
            Arc::new(Mock),
            Arc::new(symbi_runtime::reasoning::executor::DefaultActionExecutor::default()),
            Arc::new(
                symbi_runtime::reasoning::policy_bridge::DefaultPolicyGate::permissive_for_dev_only(
                ),
            ),
            "You are a test agent.",
        );
        app.agent_runners
            .insert("worker".into(), Arc::new(tokio::sync::Mutex::new(runner)));
        assert_eq!(app.agent_runners.len(), 1);

        // A load (here, an empty temp dir) must invalidate cached runners so the
        // next `@name` rebuilds against the freshly registered definition.
        let dir = tempfile::tempdir().unwrap();
        let _ = run_load(&mut app, dir.path().to_str().unwrap());
        assert!(
            app.agent_runners.is_empty(),
            "reload/load must clear cached per-agent runners"
        );
    }
}
