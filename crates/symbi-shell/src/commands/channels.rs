use super::CommandResult;
use crate::app::App;

/// Shared "please attach first" blurb. Written in one place so all three
/// channel commands show consistent, actionable guidance rather than
/// making channels look broken.
fn need_remote_message(action: &str) -> String {
    format!(
        "{action} needs the runtime HTTP API, which the shell doesn't embed.\n\
         \n\
         Channels (Slack / Mattermost / Teams) are managed by the full runtime.\n\
         Start it in another terminal with:\n\
         \n    symbi up\n\
         \n\
         then attach this shell:\n\
         \n    /attach local\n\
         \n\
         If the runtime is already running somewhere else, use\n\
         `/attach <url>` with the full base URL. This is optional — the\n\
         shell's other commands keep working without attaching.",
    )
}

pub fn channels(app: &mut App) -> CommandResult {
    let remote = match app.remote.as_ref() {
        Some(r) => r.clone(),
        None => return CommandResult::Output(need_remote_message("Listing channels")),
    };

    let rt = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => return CommandResult::Error("No async runtime".to_string()),
    };

    match tokio::task::block_in_place(|| rt.block_on(remote.list_channels())) {
        Ok(value) => format_channel_list(&value),
        Err(e) => CommandResult::Error(format!("Failed to list channels: {}", e)),
    }
}

pub fn connect(app: &mut App, args: &str) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error(
            "Usage: /connect <slack|teams|mattermost> <description>".to_string(),
        );
    }

    if app.remote.is_none() {
        return CommandResult::Output(need_remote_message("Connecting a channel"));
    }

    let prompt = format!(
        "The user wants to connect a chat adapter:\n\n{}\n\n\
         Generate the channel configuration JSON for POST /api/v1/channels. \
         Include adapter type, auth credentials from secrets, and routing rules. \
         Present the config for review before I submit it.",
        args
    );

    if app.send_to_orchestrator(&prompt, "Configuring channel...") {
        CommandResult::Handled
    } else {
        CommandResult::Error("No inference provider configured.".to_string())
    }
}

pub fn disconnect(app: &mut App, args: &str) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error("Usage: /disconnect <channel-id>".to_string());
    }

    let remote = match app.remote.as_ref() {
        Some(r) => r.clone(),
        None => return CommandResult::Output(need_remote_message("Disconnecting a channel")),
    };

    let rt = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => return CommandResult::Error("No async runtime".to_string()),
    };

    let path = format!("/api/v1/channels/{}/stop", args);
    match tokio::task::block_in_place(|| rt.block_on(remote.post(&path, None))) {
        Ok(_) => CommandResult::Output(format!("Stopped channel {}", args)),
        Err(e) => CommandResult::Error(format!("Failed to stop channel: {}", e)),
    }
}

fn format_channel_list(value: &serde_json::Value) -> CommandResult {
    let arr = match value.as_array() {
        Some(a) => a,
        None => {
            return CommandResult::Output(format!(
                "Channels:\n{}",
                serde_json::to_string_pretty(value).unwrap_or_default()
            ))
        }
    };

    if arr.is_empty() {
        return CommandResult::Output("No channels configured.".to_string());
    }

    let mut out = format!("Channels ({}):\n\n", arr.len());
    for ch in arr {
        let id = ch.get("id").and_then(|v| v.as_str()).unwrap_or("?");
        let adapter = ch.get("adapter").and_then(|v| v.as_str()).unwrap_or("?");
        let status = ch.get("status").and_then(|v| v.as_str()).unwrap_or("?");
        out.push_str(&format!(
            "  {} — {} [{}]\n",
            &id[..8.min(id.len())],
            adapter,
            status
        ));
    }
    CommandResult::Output(out)
}
