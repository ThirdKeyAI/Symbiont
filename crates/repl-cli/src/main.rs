use anyhow::{anyhow, Result};
use rustyline::Editor;
use std::env;

mod client;
mod server;
mod session_manager;

use client::Client;
use session_manager::SessionManager;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() > 1 && args[1] == "--stdio" {
        return server::run();
    }

    let mut rl = Editor::<()>::new()?;
    let mut client = Client::new()?;
    let mut session_manager = SessionManager::new();
    println!("Symbiont REPL (Client/Server Mode)");

    loop {
        let readline = rl.readline(">> ");
        match readline {
            Ok(line) => {
                rl.add_history_entry(line.as_str());
                session_manager.record_command(&line);

                if line.starts_with(':') {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    let cmd = parts.first().unwrap_or(&"");
                    let arg = parts.get(1).unwrap_or(&"");

                    let result = match *cmd {
                        ":snapshot" => session_manager
                            .snapshot(arg)
                            .map(|_| "Session saved.".to_string()),
                        ":restore" => session_manager
                            .restore(arg)
                            .map(|_| "Session restored.".to_string()),
                        ":record" if *arg == "on" => {
                            let path = parts
                                .get(2)
                                .ok_or_else(|| anyhow!("Usage: :record on <file>"))?;
                            session_manager
                                .start_recording(path)
                                .map(|_| format!("Recording to {}", path))
                        }
                        ":record" if *arg == "off" => {
                            session_manager.stop_recording();
                            Ok("Stopped recording.".to_string())
                        }
                        ":memory" => {
                            let subcmd = parts.get(1).copied().unwrap_or("help");
                            match subcmd {
                                "inspect" => {
                                    let agent_id = parts.get(2).ok_or_else(|| {
                                        anyhow!("Usage: :memory inspect <agent-id>")
                                    })?;
                                    let path = format!("data/agents/{}/memory.md", agent_id);
                                    match std::fs::read_to_string(&path) {
                                        Ok(content) => Ok(content),
                                        Err(e) => Ok(format!(
                                            "Could not read memory for {}: {}",
                                            agent_id, e
                                        )),
                                    }
                                }
                                "compact" => {
                                    let agent_id = parts.get(2).ok_or_else(|| {
                                        anyhow!("Usage: :memory compact <agent-id>")
                                    })?;
                                    Ok(format!("Compacted memory for agent {}", agent_id))
                                }
                                "purge" => {
                                    let agent_id = parts.get(2).ok_or_else(|| {
                                        anyhow!("Usage: :memory purge <agent-id>")
                                    })?;
                                    let path = format!("data/agents/{}", agent_id);
                                    match std::fs::remove_dir_all(&path) {
                                        Ok(()) => {
                                            Ok(format!("Purged all memory for agent {}", agent_id))
                                        }
                                        Err(e) => Ok(format!("Could not purge: {}", e)),
                                    }
                                }
                                _ => Ok("Commands: :memory inspect|compact|purge <agent-id>"
                                    .to_string()),
                            }
                        }
                        ":webhook" => {
                            let subcmd = parts.get(1).copied().unwrap_or("help");
                            match subcmd {
                                "list" => {
                                    Ok("Webhook definitions: (parse DSL files to list)".to_string())
                                }
                                _ => Ok("Commands: :webhook list|add|remove|test|logs".to_string()),
                            }
                        }
                        _ => Ok(format!("Unrecognized command: {}", line)),
                    };

                    match result {
                        Ok(msg) => {
                            println!("{}", msg);
                            session_manager.record_output(&msg);
                        }
                        Err(e) => {
                            eprintln!("Error: {}", e);
                            session_manager.record_output(&e.to_string());
                        }
                    }
                } else {
                    match client.evaluate(&line) {
                        Ok(output) => {
                            println!("{}", output);
                            session_manager.record_output(&output);
                        }
                        Err(e) => {
                            eprintln!("Error: {}", e);
                            session_manager.record_output(&e.to_string());
                        }
                    }
                }
            }
            Err(_) => break,
        }
    }

    Ok(())
}
