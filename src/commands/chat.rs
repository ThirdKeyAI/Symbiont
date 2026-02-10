//! `symbi chat` subcommand — connect, status, and disconnect chat adapters.

use clap::ArgMatches;

pub async fn run(matches: &ArgMatches) {
    match matches.subcommand() {
        Some(("connect", sub)) => run_connect(sub).await,
        Some(("status", _)) => run_status().await,
        Some(("disconnect", sub)) => run_disconnect(sub).await,
        _ => {
            println!("Usage: symbi chat <connect|status|disconnect>");
            println!("  connect slack --token xoxb-...                           Connect to Slack");
            println!("  connect teams --tenant-id ... --client-id ... --client-secret ...  Connect to Teams");
            println!(
                "  connect mattermost --server-url ... --token ...           Connect to Mattermost"
            );
            println!("  status                                                    Show connected adapters");
            println!(
                "  disconnect <platform>                                     Disconnect adapter"
            );
        }
    }
}

async fn run_connect(matches: &ArgMatches) {
    match matches.subcommand() {
        Some(("slack", sub)) => connect_slack(sub).await,
        Some(("teams", sub)) => connect_teams(sub).await,
        Some(("mattermost", sub)) => connect_mattermost(sub).await,
        _ => {
            println!("Usage: symbi chat connect <slack|teams|mattermost>");
            println!();
            println!("Platforms:");
            println!("  slack       --token xoxb-...");
            println!("  teams       --tenant-id ... --client-id ... --client-secret ...");
            println!("  mattermost  --server-url ... --token ...");
        }
    }
}

async fn connect_slack(sub: &ArgMatches) {
    let token = sub.get_one::<String>("token").expect("--token is required");

    // Validate token format
    if !token.starts_with("xoxb-") {
        eprintln!("Error: Bot token must start with 'xoxb-'");
        eprintln!("Get one at https://api.slack.com/apps → OAuth & Permissions");
        std::process::exit(1);
    }

    let port: u16 = sub
        .get_one::<String>("port")
        .map(|p| p.parse().expect("invalid port"))
        .unwrap_or(3100);

    let default_agent = sub.get_one::<String>("agent").cloned();

    println!("Connecting to Slack...");

    // Verify the token by calling auth.test
    let client = reqwest::Client::new();
    match client
        .post("https://slack.com/api/auth.test")
        .bearer_auth(token)
        .send()
        .await
    {
        Ok(resp) => {
            if let Ok(auth) = resp.json::<serde_json::Value>().await {
                if auth["ok"].as_bool() == Some(true) {
                    let workspace = auth["team"].as_str().unwrap_or("unknown");
                    let bot_user = auth["user"].as_str().unwrap_or("unknown");

                    println!(
                        "\x1b[32m\u{2713}\x1b[0m Connected to workspace \"{}\"",
                        workspace
                    );
                    println!(
                        "\x1b[32m\u{2713}\x1b[0m Bot user: {} ({})",
                        bot_user,
                        auth["user_id"].as_str().unwrap_or("?")
                    );

                    if let Some(ref agent) = default_agent {
                        println!("\x1b[32m\u{2713}\x1b[0m Default agent: \"{}\"", agent);
                    }

                    println!(
                        "\x1b[32m\u{2713}\x1b[0m Webhook server will listen on port {}",
                        port
                    );

                    println!();
                    println!("Configure your Slack app's Event Subscriptions URL to:");
                    println!("  https://<your-host>:{}/slack/events", port);
                    println!("Slash command URL:");
                    println!("  https://<your-host>:{}/slack/commands", port);
                } else {
                    let error = auth["error"].as_str().unwrap_or("unknown error");
                    eprintln!("Error: Slack rejected the token: {}", error);
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("Error: Could not reach Slack API: {}", e);
            std::process::exit(1);
        }
    }
}

async fn connect_teams(sub: &ArgMatches) {
    let tenant_id = sub
        .get_one::<String>("tenant-id")
        .expect("--tenant-id is required");
    let client_id = sub
        .get_one::<String>("client-id")
        .expect("--client-id is required");
    let client_secret = sub
        .get_one::<String>("client-secret")
        .expect("--client-secret is required");
    let bot_id = sub.get_one::<String>("bot-id");

    println!("Connecting to Microsoft Teams...");

    // Verify credentials by acquiring an OAuth2 token
    let client = reqwest::Client::new();
    let token_url = format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
        tenant_id
    );

    let params = [
        ("grant_type", "client_credentials"),
        ("client_id", client_id.as_str()),
        ("client_secret", client_secret.as_str()),
        ("scope", "https://api.botframework.com/.default"),
    ];

    match client.post(&token_url).form(&params).send().await {
        Ok(resp) => {
            if resp.status().is_success() {
                println!(
                    "\x1b[32m\u{2713}\x1b[0m Azure AD credentials validated (tenant: {})",
                    tenant_id
                );
                println!("\x1b[32m\u{2713}\x1b[0m Application ID: {}", client_id);

                if let Some(id) = bot_id {
                    println!("\x1b[32m\u{2713}\x1b[0m Bot ID: {}", id);
                }

                println!();
                println!("To complete setup:");
                println!("  1. Register a Bot Channel Registration in Azure Portal");
                println!(
                    "  2. Set the messaging endpoint to: https://<your-host>:3200/teams/messages"
                );
                println!("  3. Add the bot to a Teams channel");
                println!("  4. @mention the bot to invoke an agent");
            } else {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                eprintln!(
                    "Error: Azure AD rejected credentials ({}): {}",
                    status, body
                );
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Error: Could not reach Azure AD: {}", e);
            std::process::exit(1);
        }
    }
}

async fn connect_mattermost(sub: &ArgMatches) {
    let server_url = sub
        .get_one::<String>("server-url")
        .expect("--server-url is required");
    let token = sub.get_one::<String>("token").expect("--token is required");

    println!("Connecting to Mattermost at {}...", server_url);

    // Verify token via /api/v4/users/me
    let client = reqwest::Client::new();
    let url = format!("{}/api/v4/users/me", server_url.trim_end_matches('/'));

    match client.get(&url).bearer_auth(token).send().await {
        Ok(resp) => {
            if resp.status().is_success() {
                if let Ok(me) = resp.json::<serde_json::Value>().await {
                    let username = me["username"].as_str().unwrap_or("unknown");
                    let user_id = me["id"].as_str().unwrap_or("?");

                    println!("\x1b[32m\u{2713}\x1b[0m Connected to {}", server_url);
                    println!(
                        "\x1b[32m\u{2713}\x1b[0m Bot user: {} ({})",
                        username, user_id
                    );

                    println!();
                    println!("To complete setup:");
                    println!("  1. Go to Integrations → Outgoing Webhooks → Add Outgoing Webhook");
                    println!(
                        "  2. Set the callback URL to: https://<your-host>:3300/mattermost/webhook"
                    );
                    println!("  3. Set a trigger word (e.g. @symbi)");
                    println!("  4. Save and note the webhook token for --mm.webhook-secret");
                } else {
                    println!("\x1b[32m\u{2713}\x1b[0m Token accepted");
                }
            } else {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                eprintln!(
                    "Error: Mattermost rejected the token ({}): {}",
                    status, body
                );
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Error: Could not reach Mattermost at {}: {}", server_url, e);
            std::process::exit(1);
        }
    }
}

async fn run_status() {
    println!("Channel Adapter Status");
    println!();

    // Probe known adapter ports to report live status
    let adapters: Vec<(&str, u16)> = vec![("Slack", 3100), ("Teams", 3200), ("Mattermost", 3300)];

    let mut found_any = false;
    for (name, port) in &adapters {
        match tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port)).await {
            Ok(_) => {
                println!(
                    "  \x1b[32m\u{2713}\x1b[0m {} adapter — listening on :{}",
                    name, port
                );
                found_any = true;
            }
            Err(_) => {
                println!("  \x1b[90m\u{2022}\x1b[0m {} adapter — not running", name);
            }
        }
    }

    println!();
    if found_any {
        println!("Adapters are started via 'symbi up' with the appropriate flags.");
        println!("Use Ctrl+C on the running process to stop all adapters.");
    } else {
        println!("No adapters currently running.");
        println!();
        println!("Start adapters with 'symbi up':");
        println!("  symbi up --slack.token xoxb-...");
        println!(
            "  symbi up --teams.tenant-id ... --teams.client-id ... --teams.client-secret ..."
        );
        println!("  symbi up --mm.server-url ... --mm.token ...");
    }
}

async fn run_disconnect(matches: &ArgMatches) {
    match matches.subcommand() {
        Some((platform, _)) => {
            let port = match platform {
                "slack" => 3100,
                "teams" => 3200,
                "mattermost" => 3300,
                _ => {
                    println!("Usage: symbi chat disconnect <slack|teams|mattermost>");
                    return;
                }
            };

            // Check if the adapter is actually running
            match tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port)).await {
                Ok(_) => {
                    println!(
                        "The {} adapter is running on :{} as part of the 'symbi up' process.",
                        platform, port
                    );
                    println!();
                    println!("To stop it, press Ctrl+C on the running 'symbi up' process.");
                    println!(
                        "Individual adapter hot-disconnect will be supported in a future release."
                    );
                }
                Err(_) => {
                    println!("The {} adapter is not currently running.", platform);
                }
            }
        }
        _ => {
            println!("Usage: symbi chat disconnect <slack|teams|mattermost>");
        }
    }
}
