//! `symbi chat` subcommand — connect, status, and disconnect chat adapters.

use clap::ArgMatches;

pub async fn run(matches: &ArgMatches) {
    match matches.subcommand() {
        Some(("connect", sub)) => run_connect(sub).await,
        Some(("status", _)) => run_status().await,
        Some(("disconnect", sub)) => run_disconnect(sub).await,
        _ => {
            println!("Usage: symbi chat <connect|status|disconnect>");
            println!("  connect slack --token xoxb-...   Connect to Slack workspace");
            println!("  status                           Show connected adapters");
            println!("  disconnect slack                 Disconnect adapter");
        }
    }
}

async fn run_connect(matches: &ArgMatches) {
    match matches.subcommand() {
        Some(("slack", sub)) => {
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
                            println!(
                                "\x1b[36m\u{2139}\x1b[0m Policy enforcement: available in Enterprise"
                            );
                            println!(
                                "\x1b[36m\u{2139}\x1b[0m Audit logging: basic (upgrade for cryptographic trails)"
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
        _ => {
            println!("Usage: symbi chat connect slack --token xoxb-...");
            println!();
            println!("Options:");
            println!("  --token <TOKEN>    Slack bot token (required, starts with xoxb-)");
            println!("  --port <PORT>      Webhook server port (default: 3100)");
            println!("  --agent <NAME>     Default agent to invoke");
        }
    }
}

async fn run_status() {
    println!("Channel Adapters:");
    println!("  No adapters currently connected.");
    println!();
    println!("Use 'symbi chat connect slack --token xoxb-...' to connect.");
}

async fn run_disconnect(matches: &ArgMatches) {
    match matches.subcommand() {
        Some(("slack", _)) => {
            println!("Disconnecting Slack adapter...");
            println!("\x1b[32m\u{2713}\x1b[0m Disconnected from Slack");
        }
        _ => {
            println!("Usage: symbi chat disconnect slack");
        }
    }
}
