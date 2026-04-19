use super::CommandResult;
use crate::app::App;
use crate::remote::RemoteConnection;

pub fn attach(app: &mut App, args: &str) -> CommandResult {
    if args.is_empty() {
        if let Some(ref conn) = app.remote {
            return CommandResult::Output(format!(
                "Currently attached to: {}\n\nUse /detach to disconnect.",
                conn.base_url()
            ));
        }
        return CommandResult::Output(
            "Usage:\n\
             /attach <url> [--token <token>]    Connect to a running symbi up\n\
             /attach local                      Attach to http://localhost:8080\n\
             /attach @<agent>                   Attach to a locally deployed agent"
                .to_string(),
        );
    }

    let parts: Vec<&str> = args.split_whitespace().collect();
    let first = parts[0];

    // Parse optional --token
    let token = parts
        .iter()
        .position(|&p| p == "--token")
        .and_then(|i| parts.get(i + 1))
        .map(|t| t.to_string());

    let url = if first == "local" {
        "http://localhost:8080".to_string()
    } else if let Some(agent) = first.strip_prefix('@') {
        // Try to find the deployed agent's port
        match lookup_deployed_agent_port(agent) {
            Some(port) => format!("http://localhost:{}", port),
            None => {
                return CommandResult::Error(format!(
                "No locally deployed agent '{}' found. Use /deploy list to see running containers.",
                agent
            ))
            }
        }
    } else {
        // Treat as explicit URL
        if first.starts_with("http://") || first.starts_with("https://") {
            first.to_string()
        } else {
            format!("http://{}", first)
        }
    };

    let conn = RemoteConnection::new(&url, token);

    // Ping to verify connection
    let rt = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => return CommandResult::Error("No async runtime".to_string()),
    };

    match tokio::task::block_in_place(|| rt.block_on(conn.ping())) {
        Ok(()) => {
            let url_display = conn.base_url().to_string();
            app.remote = Some(conn);
            CommandResult::Output(format!(
                "Attached to {}\n\n\
                 /cron, /channels, /agents, /audit now route through this connection.\n\
                 Use /detach to disconnect.",
                url_display
            ))
        }
        Err(e) => CommandResult::Error(format!("Failed to attach: {}", e)),
    }
}

pub fn detach(app: &mut App) -> CommandResult {
    match app.remote.take() {
        Some(conn) => CommandResult::Output(format!("Detached from {}", conn.base_url())),
        None => CommandResult::Output("Not currently attached.".to_string()),
    }
}

/// Look up the exposed port of a locally deployed agent by inspecting docker.
fn lookup_deployed_agent_port(agent_name: &str) -> Option<u16> {
    let container_name = format!("symbi-{}", agent_name);
    let output = std::process::Command::new("docker")
        .args(["port", &container_name, "8080"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    // docker port output format: "0.0.0.0:9090"
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.lines().next()?;
    line.rsplit(':').next()?.trim().parse().ok()
}
