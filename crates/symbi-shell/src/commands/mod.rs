pub mod agents;
pub mod authoring;
pub mod channels;
pub mod deploy;
pub mod operations;
pub mod orchestration;
pub mod registry;
pub mod remote;
pub mod scheduling;
pub mod secrets;
pub mod session;
pub mod tools;

use crate::app::App;

/// Result of a command execution.
#[allow(dead_code)]
pub enum CommandResult {
    /// Output to display.
    Output(String),
    /// Error message.
    Error(String),
    /// No output (command handled internally, e.g. /quit).
    Handled,
}

/// Return per-command help when `args` is `"help"`, `"--help"`, or `"-h"`.
///
/// Without this, commands like `/behavior help` would treat "help" as the
/// description and shove it into the orchestrator prompt, producing
/// garbage output or timing out on MaxIterations.
fn intercept_help(command: &str, args: &str) -> Option<CommandResult> {
    let trimmed = args.trim();
    if trimmed != "help" && trimmed != "--help" && trimmed != "-h" {
        return None;
    }
    let blurb: &str = match command {
        "/spawn" => {
            "/spawn <description>\n  \
             Generate a Symbiont DSL agent from a natural-language description. \
             The orchestrator validates the DSL against project constraints \
             before presenting it for review.\n\n  \
             Example:  /spawn an agent that summarises weekly sales emails"
        }
        "/policy" => {
            "/policy <requirement>\n  \
             Generate a Cedar policy for the described requirement and \
             validate it against project constraints.\n\n  \
             Example:  /policy deny network egress except to *.example.com"
        }
        "/tool" => {
            "/tool <description>\n  \
             Generate a ToolClad TOML manifest (.clad.toml) for a new tool \
             and validate it.\n\n  \
             Example:  /tool shell tool wrapping `jq` that returns JSON"
        }
        "/behavior" => {
            "/behavior <description>\n  \
             Generate a Symbiont DSL behavior definition and validate it \
             against project constraints.\n\n  \
             Example:  /behavior retry on 429 with exponential backoff"
        }
        "/init" => {
            "/init [profile|description]\n  \
             Scaffold a Symbiont project in the current directory. With no \
             args, starts a conversational setup. With a known profile \
             name (minimal, assistant, dev-agent, multi-agent) runs a \
             deterministic scaffold. Any other arg is treated as a free-form \
             description the orchestrator uses to pick a profile."
        }
        "/ask" => "/ask <agent> <message>\n  Send a message to an agent and wait for the reply.",
        "/send" => {
            "/send <agent> <message>\n  Send a message to an agent without waiting for a reply."
        }
        "/run" => "/run <agent-or-workflow> [input]\n  Start or re-run an agent / workflow.",
        "/chain" => {
            "/chain <agent1,agent2,...> <input>\n  Pipe output of each agent into the next."
        }
        "/debate" => {
            "/debate <agent1,agent2,...> <topic>\n  Multi-agent debate on the given topic."
        }
        "/parallel" => {
            "/parallel <agent1,agent2,...> <input>\n  \
             Run agents in parallel with the same input and aggregate results."
        }
        "/race" => {
            "/race <agent1,agent2,...> <input>\n  \
             Run agents in parallel; first successful reply wins, others are cancelled."
        }
        "/exec" => "/exec <command>\n  Execute a shell command inside the sandboxed dev agent.",
        "/monitor" => "/monitor [agent]\n  Stream live status for the given agent (or all agents).",
        "/logs" => "/logs [agent]\n  Show recent logs for the given agent (or all agents).",
        "/doctor" => "/doctor\n  Diagnose the local runtime environment.",
        "/audit" => "/audit [filter]\n  Show recent audit trail entries, optionally filtered.",
        "/cron" => "/cron [list|add|remove|history] …\n  Manage cron-scheduled agent runs.",
        "/tools" => "/tools [list|add|remove] …\n  Manage ToolClad tools available to agents.",
        "/skills" => "/skills [list|install|remove] …\n  Manage skills available to agents.",
        "/verify" => {
            "/verify <artifact>\n  Verify a signed artifact (tool manifest, skill, etc.) \
             against its SchemaPin signature."
        }
        "/channels" => "/channels\n  List registered channel adapters (Slack, Mattermost, …).",
        "/connect" => "/connect <channel> [options]\n  Register a new channel adapter.",
        "/disconnect" => "/disconnect <channel>\n  Remove a channel adapter.",
        "/secrets" => {
            "/secrets [list|set|get|remove] …\n  Manage secrets available to the runtime."
        }
        "/deploy" => "/deploy [local|cloudrun|aws] [options]\n  Deploy the configured agent stack.",
        "/attach" => "/attach <url>\n  Attach this shell to a remote runtime over HTTP.",
        "/detach" => "/detach\n  Detach from the currently attached remote runtime.",
        "/debug" => "/debug <agent>\n  Inspect an agent's internal state for debugging.",
        "/memory" => "/memory <agent> [query]\n  Query an agent's memory.",
        "/pause" => "/pause <agent>\n  Pause the given agent.",
        "/resume-agent" => "/resume-agent <agent>\n  Resume a paused agent.",
        "/stop" => "/stop <agent>\n  Stop the given agent.",
        "/destroy" => "/destroy <agent>\n  Destroy the given agent and its state.",
        "/agents" => "/agents\n  List active agents.",
        "/context" => "/context\n  Show the current context window / token usage.",
        "/compact" => {
            "/compact [limit]\n  Compact the conversation history to fit within a budget."
        }
        "/model" => "/model [name]\n  Show or switch the active inference model.",
        "/cost" => "/cost\n  Show token / API-cost totals for the current session.",
        "/snapshot" => "/snapshot [name]\n  Save a session snapshot.",
        "/resume" => "/resume <snapshot>\n  Restore a saved snapshot into the current session.",
        "/export" => "/export <path>\n  Export the current session transcript to disk.",
        "/new" => "/new\n  Start a new session, discarding the current one.",
        "/status" => "/status\n  Show runtime + session status.",
        "/dsl" => "/dsl\n  Toggle between DSL and orchestrator input modes.",
        "/clear" => "/clear\n  Clear the visible output buffer.",
        "/quit" | "/exit" => "/quit | /exit\n  Exit the shell.",
        _ => return None, // Unknown command: let dispatch produce the real error.
    };
    Some(CommandResult::Output(blurb.to_string()))
}

/// Dispatch a /command. Returns None if the command is not recognized.
pub fn dispatch(app: &mut App, command: &str, args: &str) -> Option<CommandResult> {
    if let Some(help) = intercept_help(command, args) {
        return Some(help);
    }
    match command {
        "/help" => Some(session::help(app)),
        "/clear" => Some(session::clear(app)),
        "/quit" | "/exit" => Some(session::quit(app)),
        "/dsl" => Some(session::dsl_toggle(app)),
        "/model" => Some(session::model(app, args)),
        "/cost" => Some(session::cost(app)),
        "/status" => Some(session::status(app)),
        "/agents" => Some(agents::agents(app)),
        "/debug" => Some(agents::debug(app, args)),
        "/stop" => Some(agents::stop(app, args)),
        "/pause" => Some(agents::pause(app, args)),
        "/destroy" => Some(agents::destroy(app, args)),

        // Authoring
        "/policy" => Some(authoring::policy(app, args)),
        "/tool" => Some(authoring::tool(app, args)),
        "/behavior" => Some(authoring::behavior(app, args)),
        "/init" => Some(authoring::init(app, args)),

        // Orchestration
        "/run" => Some(orchestration::run(app, args)),
        "/chain" => Some(orchestration::chain(app, args)),
        "/debate" => Some(orchestration::debate(app, args)),
        "/parallel" => Some(orchestration::parallel(app, args)),
        "/race" => Some(orchestration::race(app, args)),
        "/exec" => Some(orchestration::exec(app, args)),

        // Operations
        "/monitor" => Some(operations::monitor(app, args)),
        "/logs" => Some(operations::logs(app, args)),
        "/doctor" => Some(operations::doctor(app)),
        "/audit" => Some(operations::audit(app, args)),

        // Scheduling
        "/cron" => Some(scheduling::cron(app, args)),

        // Tools
        "/tools" => Some(tools::tools(app, args)),
        "/skills" => Some(tools::skills(app, args)),
        "/verify" => Some(tools::verify(app, args)),

        // Channels
        "/channels" => Some(channels::channels(app)),
        "/connect" => Some(channels::connect(app, args)),
        "/disconnect" => Some(channels::disconnect(app, args)),

        // Secrets
        "/secrets" => Some(secrets::secrets(app, args)),

        // Deployment
        "/deploy" => Some(deploy::deploy(app, args)),

        // Remote attach
        "/attach" => Some(remote::attach(app, args)),
        "/detach" => Some(remote::detach(app)),

        // Context management
        "/compact" => Some(session::compact(app, args)),
        "/context" => Some(session::context(app)),

        // Session management
        "/snapshot" => Some(session::snapshot(app, args)),
        "/resume" => Some(session::resume(app, args)),
        "/export" => Some(session::export(app, args)),

        // Session stubs
        "/new" => Some(session::new_session(app)),
        "/branch" | "/copy" => Some(CommandResult::Output(format!(
            "[{} requires session branching — planned for a future release]",
            command
        ))),

        // Agent creation
        "/spawn" => Some(authoring::spawn(app, args)),
        "/ask" => Some(agents::ask(app, args)),
        "/send" => Some(agents::send(app, args)),
        "/memory" => Some(agents::memory(app, args)),
        "/resume-agent" => Some(agents::resume_agent(app, args)),

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unwrap_output(r: CommandResult) -> String {
        match r {
            CommandResult::Output(s) => s,
            other => panic!("expected Output, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn help_intercept_returns_blurb_for_orchestrator_command() {
        let r = intercept_help("/behavior", "help").expect("has help");
        let s = unwrap_output(r);
        assert!(s.starts_with("/behavior"), "got: {}", s);
        assert!(s.contains("<description>"));
    }

    #[test]
    fn help_intercept_matches_dash_h_and_long_flag() {
        assert!(intercept_help("/spawn", "-h").is_some());
        assert!(intercept_help("/spawn", "--help").is_some());
    }

    #[test]
    fn help_intercept_ignores_other_args() {
        // "/behavior help me write X" is a free-form description, not a
        // request for command help.
        assert!(intercept_help("/behavior", "help me retry on 429").is_none());
        assert!(intercept_help("/behavior", "retry logic").is_none());
    }

    #[test]
    fn help_intercept_ignores_empty_args() {
        // Empty args should fall through to the command's own Usage line.
        assert!(intercept_help("/behavior", "").is_none());
    }

    #[test]
    fn help_intercept_unknown_command_returns_none() {
        // Unknown commands: let the normal dispatcher produce the error.
        assert!(intercept_help("/does-not-exist", "help").is_none());
    }

    #[test]
    fn help_intercept_trims_whitespace() {
        assert!(intercept_help("/behavior", "  help  ").is_some());
    }
}
