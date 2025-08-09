use anyhow::{Result, anyhow};
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
                        ":snapshot" => session_manager.snapshot(arg).map(|_| "Session saved.".to_string()),
                        ":restore" => session_manager.restore(arg).map(|_| "Session restored.".to_string()),
                        ":record" if *arg == "on" => {
                            let path = parts.get(2).ok_or_else(|| anyhow!("Usage: :record on <file>"))?;
                            session_manager.start_recording(path).map(|_| format!("Recording to {}", path))
                        },
                        ":record" if *arg == "off" => {
                            session_manager.stop_recording();
                            Ok("Stopped recording.".to_string())
                        },
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