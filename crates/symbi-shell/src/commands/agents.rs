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

pub fn agents(app: &mut App) -> CommandResult {
    let rt = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => return CommandResult::Error("No async runtime available".to_string()),
    };

    let agents =
        tokio::task::block_in_place(|| rt.block_on(app.engine().evaluator().list_agents()));

    if agents.is_empty() {
        return CommandResult::Output("No agents created.".to_string());
    }

    let mut out = String::from("Agents:\n");
    for agent in &agents {
        out.push_str(&format!(
            "  {} — {} ({:?})\n",
            &agent.id.to_string()[..8],
            agent.definition.name,
            agent.state
        ));
    }
    CommandResult::Output(out)
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
