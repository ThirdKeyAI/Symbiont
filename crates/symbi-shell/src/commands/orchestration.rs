use super::CommandResult;
use crate::app::App;

pub fn run(app: &mut App, args: &str) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error("Usage: /run <behavior> [args]".to_string());
    }
    let prompt = format!(
        "Execute the following behavior or DSL expression:\n\n{}\n\nRun it and report the results.",
        args
    );
    if app.send_to_orchestrator(&prompt, "Running...") {
        CommandResult::Handled
    } else {
        CommandResult::Error("No inference provider configured.".to_string())
    }
}

pub fn chain(app: &mut App, args: &str) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error("Usage: /chain <description>".to_string());
    }
    let prompt = format!(
        "Set up and execute a multi-step chain for:\n\n{}\n\nBreak it into sequential steps, execute each, and report results.",
        args
    );
    if app.send_to_orchestrator(&prompt, "Running chain...") {
        CommandResult::Handled
    } else {
        CommandResult::Error("No inference provider configured.".to_string())
    }
}

pub fn debate(app: &mut App, args: &str) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error("Usage: /debate <topic>".to_string());
    }
    let prompt = format!(
        "Start a writer/critic debate on:\n\n{}\n\nAlternate between writer and critic roles for 3 rounds, then present a synthesis.",
        args
    );
    if app.send_to_orchestrator(&prompt, "Debating...") {
        CommandResult::Handled
    } else {
        CommandResult::Error("No inference provider configured.".to_string())
    }
}

pub fn parallel(app: &mut App, args: &str) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error("Usage: /parallel <tasks>".to_string());
    }
    let prompt = format!(
        "Run these tasks in parallel and report all results:\n\n{}",
        args
    );
    if app.send_to_orchestrator(&prompt, "Running parallel...") {
        CommandResult::Handled
    } else {
        CommandResult::Error("No inference provider configured.".to_string())
    }
}

pub fn race(app: &mut App, args: &str) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error("Usage: /race <tasks>".to_string());
    }
    let prompt = format!("Race these tasks — return the first result:\n\n{}", args);
    if app.send_to_orchestrator(&prompt, "Racing...") {
        CommandResult::Handled
    } else {
        CommandResult::Error("No inference provider configured.".to_string())
    }
}

pub fn exec(_app: &mut App, args: &str) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error("Usage: /exec <command>".to_string());
    }
    CommandResult::Output(format!("[CLI executor not yet connected: {}]", args))
}
