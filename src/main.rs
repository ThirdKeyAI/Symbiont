#![allow(clippy::multiple_crate_versions)]

use clap::{Arg, ArgAction, Command};

mod commands;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() {
    let matches = Command::new("symbi")
        .version(VERSION)
        .about("Symbiont - AI Agent Runtime and DSL")
        .subcommand(
            Command::new("up")
                .about("Start the Symbiont runtime with auto-configuration")
                .arg(
                    Arg::new("port")
                        .short('p')
                        .long("port")
                        .value_name("PORT")
                        .help("Runtime API port")
                        .default_value("8080"),
                )
                .arg(
                    Arg::new("http-port")
                        .long("http-port")
                        .value_name("HTTP_PORT")
                        .help("HTTP Input port")
                        .default_value("8081"),
                )
                .arg(
                    Arg::new("http-token")
                        .long("http.token")
                        .value_name("TOKEN")
                        .help("Bearer token for HTTP authentication (use 'env:VAR' for environment variable)"),
                )
                .arg(
                    Arg::new("http-cors")
                        .long("http.cors")
                        .action(ArgAction::SetTrue)
                        .help("Enable CORS with sensible defaults"),
                )
                .arg(
                    Arg::new("http-audit")
                        .long("http.audit")
                        .action(ArgAction::SetTrue)
                        .help("Log all HTTP requests to audit log"),
                )
                .arg(
                    Arg::new("preset")
                        .long("preset")
                        .value_name("PRESET")
                        .help("Use a configuration preset (e.g., dev-simple)"),
                ),
        )
        .subcommand(
            Command::new("doctor")
                .about("Check system health and dependencies")
        )
        .subcommand(
            Command::new("logs")
                .about("Show runtime logs")
                .arg(
                    Arg::new("follow")
                        .short('f')
                        .long("follow")
                        .action(ArgAction::SetTrue)
                        .help("Follow log output in real-time"),
                )
                .arg(
                    Arg::new("lines")
                        .short('n')
                        .long("lines")
                        .value_name("LINES")
                        .help("Number of recent log lines to show")
                        .default_value("50"),
                ),
        )
        .subcommand(
            Command::new("status")
                .about("Show running agents, routes, and I/O handlers")
        )
        .subcommand(
            Command::new("new")
                .about("Create a new project from a template")
                .arg(
                    Arg::new("template")
                        .value_name("TEMPLATE")
                        .help("Template name (webhook-min, webscraper-agent, slm-first, rag-lite)")
                        .required_unless_present("list"),
                )
                .arg(
                    Arg::new("name")
                        .value_name("PROJECT_NAME")
                        .help("Project name")
                        .required(false),
                )
                .arg(
                    Arg::new("list")
                        .long("list")
                        .action(ArgAction::SetTrue)
                        .help("List available templates"),
                ),
        )
        .subcommand(
            Command::new("mcp")
                .about("Start MCP server")
                .arg(
                    Arg::new("port")
                        .short('p')
                        .long("port")
                        .value_name("PORT")
                        .help("Port to bind the server to")
                        .default_value("8080"),
                )
                .arg(
                    Arg::new("host")
                        .short('h')
                        .long("host")
                        .value_name("HOST")
                        .help("Host address to bind to")
                        .default_value("127.0.0.1"),
                ),
        )
        .subcommand(
            Command::new("dsl")
                .about("Parse and execute DSL")
                .arg(
                    Arg::new("file")
                        .short('f')
                        .long("file")
                        .value_name("FILE")
                        .help("DSL file to parse and execute"),
                )
                .arg(
                    Arg::new("content")
                        .short('c')
                        .long("content")
                        .value_name("CONTENT")
                        .help("DSL content to parse directly"),
                ),
        )
        .get_matches();

    match matches.subcommand() {
        Some(("up", sub_matches)) => {
            commands::up::run(sub_matches).await;
        }
        Some(("doctor", _sub_matches)) => {
            commands::doctor::run().await;
        }
        Some(("logs", sub_matches)) => {
            commands::logs::run(sub_matches).await;
        }
        Some(("status", _sub_matches)) => {
            commands::status::run().await;
        }
        Some(("new", sub_matches)) => {
            commands::new::run(sub_matches).await;
        }
        Some(("mcp", sub_matches)) => {
            let port = sub_matches.get_one::<String>("port").unwrap();
            let host = sub_matches.get_one::<String>("host").unwrap();

            println!("Starting Symbiont MCP server on {}:{}", host, port);
            println!("MCP server functionality would be implemented here");
        }
        Some(("dsl", sub_matches)) => {
            if let Some(file) = sub_matches.get_one::<String>("file") {
                println!("Parsing DSL file: {}", file);
                println!("DSL parsing functionality would be implemented here");
            } else if let Some(content) = sub_matches.get_one::<String>("content") {
                println!("Parsing DSL content: {}", content);
                println!("DSL parsing functionality would be implemented here");
            } else {
                eprintln!("Either --file or --content must be provided for DSL command");
                std::process::exit(1);
            }
        }
        _ => {
            println!("Symbiont v{}", VERSION);
            println!("Use --help for available commands");
        }
    }
}