#![allow(clippy::multiple_crate_versions)]

use clap::{Arg, Command};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() {
    let matches = Command::new("symbi")
        .version(VERSION)
        .about("Symbiont - AI Agent Runtime and DSL")
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