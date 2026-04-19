use super::CommandResult;
use crate::app::App;

pub fn monitor(app: &mut App, args: &str) -> CommandResult {
    let subcmd = if args.is_empty() { "stats" } else { args };
    match subcmd {
        "stats" => {
            let stats = app.engine().evaluator().monitor().get_stats();
            CommandResult::Output(format!(
                "Executions: {} total, {} ok, {} failed\nAvg duration: {:?}",
                stats.total_executions,
                stats.successful_executions,
                stats.failed_executions,
                stats.average_duration,
            ))
        }
        "clear" => {
            app.engine().evaluator().monitor().clear_traces();
            CommandResult::Output("Monitoring data cleared.".to_string())
        }
        _ => CommandResult::Error(format!("Unknown monitor subcommand: {}", subcmd)),
    }
}

pub fn logs(_app: &mut App, args: &str) -> CommandResult {
    let follow = args.contains("-f");
    let log_paths = [".symbiont/audit/", "logs/"];
    for dir in &log_paths {
        let path = std::path::Path::new(dir);
        if path.exists() {
            if let Ok(entries) = std::fs::read_dir(path) {
                let mut files: Vec<_> = entries.filter_map(|e| e.ok()).collect();
                files.sort_by_key(|e| e.file_name());
                if let Some(latest) = files.last() {
                    match std::fs::read_to_string(latest.path()) {
                        Ok(content) => {
                            let lines: Vec<&str> = content.lines().collect();
                            let show = if follow {
                                &lines[..]
                            } else {
                                &lines[lines.len().saturating_sub(20)..]
                            };
                            return CommandResult::Output(format!(
                                "Logs from {}:\n\n{}",
                                latest.path().display(),
                                show.join("\n")
                            ));
                        }
                        Err(e) => {
                            return CommandResult::Error(format!("Failed to read log: {}", e))
                        }
                    }
                }
            }
        }
    }
    CommandResult::Output("No log files found in .symbiont/audit/ or logs/".to_string())
}

pub fn doctor(_app: &mut App) -> CommandResult {
    let mut checks: Vec<String> = Vec::new();

    // Check symbiont.toml
    if std::path::Path::new("symbiont.toml").exists() {
        checks.push("symbiont.toml: found".to_string());
    } else {
        checks.push("symbiont.toml: MISSING (run /init to create)".to_string());
    }

    // Check constraints
    if std::path::Path::new(".symbi/constraints.toml").exists() {
        checks.push("constraints.toml: found".to_string());
    } else {
        checks.push("constraints.toml: MISSING".to_string());
    }

    // Check directories
    for dir in &["agents/", "policies/", "tools/"] {
        if std::path::Path::new(dir).exists() {
            checks.push(dir.to_string());
        }
    }

    // Check API key
    let has_key = std::env::var("ANTHROPIC_API_KEY").is_ok()
        || std::env::var("OPENAI_API_KEY").is_ok()
        || std::env::var("OPENROUTER_API_KEY").is_ok();
    if has_key {
        checks.push("Inference provider: configured".to_string());
    } else {
        checks.push("Inference provider: NOT CONFIGURED".to_string());
    }

    let mut out = String::from("System health:\n\n");
    for check in &checks {
        let icon = if check.contains("MISSING") || check.contains("NOT ") {
            "x"
        } else {
            "ok"
        };
        out.push_str(&format!("  [{}] {}\n", icon, check));
    }
    CommandResult::Output(out)
}

pub fn audit(app: &mut App, args: &str) -> CommandResult {
    let orchestrator = match app.orchestrator.as_ref() {
        Some(o) => o,
        None => {
            return CommandResult::Error(
                "No orchestrator — no audit journal available.".to_string(),
            )
        }
    };

    // Try to lock without blocking the event loop
    let orch = match orchestrator.try_lock() {
        Ok(o) => o,
        Err(_) => {
            return CommandResult::Error(
                "Orchestrator is busy — try again after the current request completes.".to_string(),
            )
        }
    };

    let limit: usize = args.trim().parse().unwrap_or(20);

    // entries() is async but we need sync here — use block_in_place
    let rt = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => return CommandResult::Error("No async runtime".to_string()),
    };

    let entries = tokio::task::block_in_place(|| rt.block_on(orch.journal().entries()));

    if entries.is_empty() {
        return CommandResult::Output("Audit journal is empty.".to_string());
    }

    let mut out = format!(
        "Audit journal ({} entries, showing last {}):\n\n",
        entries.len(),
        limit
    );

    for entry in entries.iter().rev().take(limit).rev() {
        let event_desc = match &entry.event {
            symbi_runtime::reasoning::loop_types::LoopEvent::Started { .. } => {
                "ORGA loop started".to_string()
            }
            symbi_runtime::reasoning::loop_types::LoopEvent::ReasoningComplete {
                iteration,
                actions,
                usage,
            } => {
                format!(
                    "Reasoning complete: iter={}, actions={}, tokens={}",
                    iteration,
                    actions.len(),
                    usage.total_tokens
                )
            }
            symbi_runtime::reasoning::loop_types::LoopEvent::PolicyEvaluated {
                iteration,
                action_count,
                denied_count,
            } => {
                let status = if *denied_count > 0 { "DENIED" } else { "OK" };
                format!(
                    "Policy gate: iter={}, actions={}, denied={} [{}]",
                    iteration, action_count, denied_count, status
                )
            }
            symbi_runtime::reasoning::loop_types::LoopEvent::ToolsDispatched {
                iteration,
                tool_count,
                duration,
            } => {
                format!(
                    "Tools dispatched: iter={}, tools={}, duration={:?}",
                    iteration, tool_count, duration
                )
            }
            symbi_runtime::reasoning::loop_types::LoopEvent::ObservationsCollected {
                iteration,
                observation_count,
            } => {
                format!(
                    "Observations: iter={}, count={}",
                    iteration, observation_count
                )
            }
            symbi_runtime::reasoning::loop_types::LoopEvent::Terminated {
                reason,
                iterations,
                total_usage,
                ..
            } => {
                format!(
                    "Terminated: {:?} after {} iters, {} tokens",
                    reason, iterations, total_usage.total_tokens
                )
            }
            symbi_runtime::reasoning::loop_types::LoopEvent::RecoveryTriggered {
                iteration,
                tool_name,
                error,
                ..
            } => {
                format!(
                    "Recovery: iter={}, tool={}, error={}",
                    iteration, tool_name, error
                )
            }
            #[allow(unreachable_patterns)]
            other => format!("{:?}", other),
        };

        out.push_str(&format!(
            "  [{}] seq={} iter={} {}\n",
            entry.timestamp.format("%H:%M:%S"),
            entry.sequence,
            entry.iteration,
            event_desc,
        ));
    }

    CommandResult::Output(out)
}
