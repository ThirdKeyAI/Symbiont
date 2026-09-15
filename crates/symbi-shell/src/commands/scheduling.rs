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

    if args.is_empty() || args == "list" {
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
                let values: Vec<_> = parts.get(1).unwrap_or(&"").split_whitespace().collect();
                if values.is_empty() || values.len() > 2 {
                    return CommandResult::Error(
                        "Usage: /cron run <job-id> [invocation-id]".into(),
                    );
                }
                let id = values[0];
                if uuid::Uuid::parse_str(id).is_err() {
                    return CommandResult::Error(
                        "Schedule ID must be a UUID; use /cron list.".into(),
                    );
                }
                let invocation = match values.get(1) {
                    Some(value) => match uuid::Uuid::parse_str(value) {
                        Ok(id) => id,
                        Err(_) => {
                            return CommandResult::Error("Invocation ID must be a UUID.".into())
                        }
                    },
                    None => uuid::Uuid::new_v4(),
                };
                let retry = format!("Retry/status: /cron run {id} {invocation}");
                match tokio::task::block_in_place(|| {
                    rt.block_on(remote.trigger_schedule(id, invocation))
                }) {
                    Ok(value) => format_trigger_result(id, invocation, &value),
                    Err(e) => CommandResult::Error(format!(
                        "Trigger response unavailable: {e}\nInvocation: {invocation}\n{retry}"
                    )),
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
        let id = sched.get("job_id").and_then(|v| v.as_str()).unwrap_or("?");
        let name = sched
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("(unnamed)");
        let status = sched.get("status").and_then(|v| v.as_str()).unwrap_or("?");
        let cron = sched
            .get("cron_expression")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        out.push_str(&format!("  {} — {} ({}) [{}]\n", id, name, cron, status));
    }
    CommandResult::Output(out)
}

fn format_trigger_result(
    job: &str,
    invocation: uuid::Uuid,
    value: &serde_json::Value,
) -> CommandResult {
    let title = match value.get("status").and_then(|s| s.as_str()) {
        Some("queued") => "Schedule queued",
        Some("completed") => "Saved completed result",
        Some("failed") => "Saved execution failure",
        Some("in_progress") => "Invocation is still in progress",
        Some("unresolved") => {
            "Unresolved execution; inspect its audit and effects before reconciliation"
        }
        Some("reconciled") => "Operator-reconciled outcome; original work was not repeated",
        Some("conflict") => "Invocation ID conflicts with a different request",
        _ => "Unrecognized schedule outcome; inspect history before retrying",
    };
    let mut output =
        format!("{title}\nInvocation: {invocation}\nRetry/status: /cron run {job} {invocation}");
    if let Some(review) = value.pointer("/resolution/review") {
        output.push_str(&format!(
            "\nOperator assessment: {}\nRationale: {}",
            review["outcome"], review["rationale"]
        ));
    }
    if let Some(audit) = value.get("audit").filter(|a| a.is_object()) {
        if let Some(run) = audit.get("run_id").and_then(|v| v.as_str()) {
            output.push_str(&format!("\nExecution: {run}"));
        }
        if let Some(path) = audit.get("path").and_then(|v| v.as_str()) {
            output.push_str(&format!("\nJournal: {}", serde_json::json!(path)));
        }
    }
    if let Some(result) = value.get("result") {
        for (field, label) in [("output", "Output"), ("error", "Error")] {
            if let Some(text) = result.get(field).and_then(|v| v.as_str()) {
                let preview = symbi_runtime::text_util::truncate_utf8(text, 1024);
                output.push_str(&format!("\n{label}: {}", serde_json::json!(preview)));
                if preview.len() < text.len() {
                    output.push_str(" (preview)");
                }
            }
        }
        if let Some(tokens) = result
            .pointer("/total_usage/total_tokens")
            .and_then(|v| v.as_u64())
        {
            output.push_str(&format!("\nReported tokens: {tokens}"));
        }
        if let Some(budget) = result.get("budget").filter(|b| b.is_object()) {
            output.push_str(&format!(
                "\nToken budget: available={}, reserved={}, uncertain={}",
                budget["available_tokens"], budget["reserved_tokens"], budget["uncertain_tokens"]
            ));
        }
    }
    output.push_str(&format!("\nFull history: /cron history {job}"));
    CommandResult::Output(output)
}
