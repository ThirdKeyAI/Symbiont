use super::CommandResult;
use crate::app::App;

pub fn cron(app: &mut App, args: &str) -> CommandResult {
    // Require an attached remote connection for cron management
    let remote = match app.remote.as_ref() {
        Some(r) => r.clone(),
        None => {
            return CommandResult::Output(
                "Cron management requires a remote connection.\n\n\
                 Options:\n\
                 - Run `symbi up` in another terminal, then: /attach local\n\
                 - Deploy an agent: /deploy @agent local, then: /attach @agent\n\n\
                 See /attach for more options."
                    .to_string(),
            )
        }
    };

    let rt = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => return CommandResult::Error("No async runtime".to_string()),
    };

    let args = args.trim();

    if args.is_empty() {
        match tokio::task::block_in_place(|| rt.block_on(remote.list_schedules())) {
            Ok(value) => format_schedule_list(&value),
            Err(e) => CommandResult::Error(format!("Failed to list schedules: {}", e)),
        }
    } else {
        let parts: Vec<&str> = args.splitn(2, ' ').collect();
        match parts[0] {
            "add" => {
                let desc = parts.get(1).unwrap_or(&"");
                if desc.is_empty() {
                    return CommandResult::Error(
                        "Usage: /cron add <description>\n\
                         Routes through orchestrator to generate a schedule."
                            .to_string(),
                    );
                }
                let prompt = format!(
                    "Generate a Symbiont schedule for:\n\n{}\n\n\
                     Present the schedule JSON for my review. \
                     After I approve, I'll submit it to the runtime.",
                    desc
                );
                if app.send_to_orchestrator(&prompt, "Generating schedule...") {
                    CommandResult::Handled
                } else {
                    CommandResult::Error("No inference provider configured.".to_string())
                }
            }
            "pause" => {
                let id = parts.get(1).unwrap_or(&"");
                if id.is_empty() {
                    return CommandResult::Error("Usage: /cron pause <id>".to_string());
                }
                match tokio::task::block_in_place(|| rt.block_on(remote.pause_schedule(id))) {
                    Ok(_) => CommandResult::Output(format!("Paused schedule {}", id)),
                    Err(e) => CommandResult::Error(format!("Failed to pause: {}", e)),
                }
            }
            "resume" => {
                let id = parts.get(1).unwrap_or(&"");
                if id.is_empty() {
                    return CommandResult::Error("Usage: /cron resume <id>".to_string());
                }
                match tokio::task::block_in_place(|| rt.block_on(remote.resume_schedule(id))) {
                    Ok(_) => CommandResult::Output(format!("Resumed schedule {}", id)),
                    Err(e) => CommandResult::Error(format!("Failed to resume: {}", e)),
                }
            }
            "run" => {
                let id = parts.get(1).unwrap_or(&"");
                if id.is_empty() {
                    return CommandResult::Error("Usage: /cron run <id>".to_string());
                }
                match tokio::task::block_in_place(|| rt.block_on(remote.trigger_schedule(id))) {
                    Ok(_) => CommandResult::Output(format!("Triggered schedule {}", id)),
                    Err(e) => CommandResult::Error(format!("Failed to trigger: {}", e)),
                }
            }
            "history" => {
                let id = parts.get(1).unwrap_or(&"");
                if id.is_empty() {
                    return CommandResult::Error("Usage: /cron history <id>".to_string());
                }
                match tokio::task::block_in_place(|| rt.block_on(remote.schedule_history(id))) {
                    Ok(value) => CommandResult::Output(format!(
                        "History for {}:\n\n{}",
                        id,
                        serde_json::to_string_pretty(&value).unwrap_or_default()
                    )),
                    Err(e) => CommandResult::Error(format!("Failed to get history: {}", e)),
                }
            }
            _ => CommandResult::Error(format!(
                "Unknown cron subcommand: {}\n\
                 Available: add, pause, resume, run, history",
                parts[0]
            )),
        }
    }
}

fn format_schedule_list(value: &serde_json::Value) -> CommandResult {
    let arr = match value.as_array() {
        Some(a) => a,
        None => {
            return CommandResult::Output(format!(
                "Schedules:\n{}",
                serde_json::to_string_pretty(value).unwrap_or_default()
            ))
        }
    };

    if arr.is_empty() {
        return CommandResult::Output("No schedules configured.".to_string());
    }

    let mut out = format!("Schedules ({}):\n\n", arr.len());
    for sched in arr {
        let id = sched.get("id").and_then(|v| v.as_str()).unwrap_or("?");
        let name = sched
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("(unnamed)");
        let status = sched.get("status").and_then(|v| v.as_str()).unwrap_or("?");
        let cron = sched.get("cron").and_then(|v| v.as_str()).unwrap_or("?");
        out.push_str(&format!(
            "  {} — {} ({}) [{}]\n",
            &id[..8.min(id.len())],
            name,
            cron,
            status
        ));
    }
    CommandResult::Output(out)
}
