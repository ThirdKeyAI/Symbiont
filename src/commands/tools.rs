//! `symbi tools` — Manage ToolClad tool manifests
//!
//! Subcommands: list, validate, test, schema

use clap::ArgMatches;
use std::collections::HashMap;
use std::path::Path;
use symbi_runtime::reasoning::executor::ActionExecutor;
use symbi_runtime::toolclad::executor::ToolCladExecutor;
use symbi_runtime::toolclad::manifest::{load_manifest, load_manifests_from_dir};

pub async fn run(matches: &ArgMatches) {
    match matches.subcommand() {
        Some(("list", _)) => cmd_list(),
        Some(("validate", sub)) => {
            let file = sub.get_one::<String>("file");
            cmd_validate(file.map(|s| s.as_str()));
        }
        Some(("test", sub)) => {
            let name = sub.get_one::<String>("name").unwrap();
            let args: Vec<String> = sub
                .get_many::<String>("arg")
                .map(|v| v.cloned().collect())
                .unwrap_or_default();
            cmd_test(name, &args);
        }
        Some(("schema", sub)) => {
            let name = sub.get_one::<String>("name").unwrap();
            cmd_schema(name);
        }
        _ => {
            println!("Usage: symbi tools <list|validate|test|schema>");
            println!("  list                        List all discovered tools");
            println!("  validate [file]             Validate manifest(s)");
            println!("  test <name> --arg k=v ...   Dry-run a tool invocation");
            println!("  schema <name>               Output MCP JSON Schema");
        }
    }
}

fn cmd_list() {
    let manifests = load_manifests_from_dir(Path::new("tools"));
    if manifests.is_empty() {
        println!("No tools found in tools/");
        return;
    }

    println!(
        "{:<24} {:<16} {:<8} CEDAR RESOURCE",
        "TOOL", "BINARY", "RISK"
    );
    for (name, m) in &manifests {
        let cedar = m
            .tool
            .cedar
            .as_ref()
            .map(|c| c.resource.as_str())
            .unwrap_or("-");
        println!(
            "{:<24} {:<16} {:<8} {}",
            name, m.tool.binary, m.tool.risk_tier, cedar
        );
    }
}

fn cmd_validate(file: Option<&str>) {
    if let Some(path) = file {
        match load_manifest(Path::new(path)) {
            Ok(m) => {
                println!("{:<40} OK ({})", path, m.tool.name);
                print_manifest_summary(&m);
            }
            Err(e) => {
                eprintln!("{:<40} ERROR: {}", path, e);
                std::process::exit(1);
            }
        }
    } else {
        let dir = Path::new("tools");
        if !dir.exists() {
            eprintln!("No tools/ directory found");
            std::process::exit(1);
        }
        let mut errors = 0;
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path
                    .file_name()
                    .map(|n| n.to_string_lossy().ends_with(".clad.toml"))
                    .unwrap_or(false)
                {
                    match load_manifest(&path) {
                        Ok(m) => println!("{:<40} OK", m.tool.name),
                        Err(e) => {
                            eprintln!("{:<40} ERROR: {}", path.display(), e);
                            errors += 1;
                        }
                    }
                }
            }
        }
        if errors > 0 {
            std::process::exit(1);
        }
    }
}

fn cmd_test(name: &str, arg_strs: &[String]) {
    let manifests = load_manifests_from_dir(Path::new("tools"));
    let manifest = manifests.iter().find(|(n, _)| n == name).map(|(_, m)| m);

    let manifest = match manifest {
        Some(m) => m,
        None => {
            // Try as a file path
            match load_manifest(Path::new(name)) {
                Ok(m) => {
                    do_test(&m, arg_strs);
                    return;
                }
                Err(_) => {
                    eprintln!("Tool '{}' not found in tools/", name);
                    std::process::exit(1);
                }
            }
        }
    };
    do_test(manifest, arg_strs);
}

fn do_test(manifest: &symbi_runtime::toolclad::manifest::Manifest, arg_strs: &[String]) {
    let args = parse_arg_pairs(arg_strs);

    println!("  Manifest:  {}", manifest.tool.name);
    println!("  Binary:    {}", manifest.tool.binary);
    println!("  Timeout:   {}s", manifest.tool.timeout_seconds);
    println!("  Risk:      {}", manifest.tool.risk_tier);
    println!();

    // Validate each argument
    let mut validated: HashMap<String, String> = HashMap::new();
    let mut has_error = false;

    for (arg_name, arg_def) in &manifest.args {
        let value = args.get(arg_name).cloned().unwrap_or_else(|| {
            arg_def
                .default
                .as_ref()
                .map(|d| d.to_string().trim_matches('"').to_string())
                .unwrap_or_default()
        });

        if value.is_empty() && arg_def.required {
            eprintln!(
                "  ✗ {} ({}): MISSING — required",
                arg_name, arg_def.type_name
            );
            has_error = true;
            continue;
        }

        if value.is_empty() {
            println!("  ○ {} ({}): <empty>", arg_name, arg_def.type_name);
            validated.insert(arg_name.clone(), value);
            continue;
        }

        match symbi_runtime::toolclad::validator::validate_arg(arg_def, &value) {
            Ok(cleaned) => {
                println!("  ✓ {} ({}): {} → OK", arg_name, arg_def.type_name, cleaned);
                validated.insert(arg_name.clone(), cleaned);
            }
            Err(e) => {
                eprintln!(
                    "  ✗ {} ({}): {} → FAILED: {}",
                    arg_name, arg_def.type_name, value, e
                );
                has_error = true;
            }
        }
    }

    if has_error {
        println!("\n  [dry run — validation failed, command not constructed]");
        std::process::exit(1);
    }

    // Build command
    let _executor = ToolCladExecutor::new(vec![(manifest.tool.name.clone(), manifest.clone())]);
    // Use the internal build_command logic via a simple approach
    println!();
    if let Some(template) = &manifest.command.template {
        let mut cmd = template.clone();
        // Apply defaults
        for (key, val) in &manifest.command.defaults {
            let placeholder = format!("{{{}}}", key);
            if cmd.contains(&placeholder) && !validated.contains_key(key) {
                cmd = cmd.replace(&placeholder, val.to_string().trim_matches('"'));
            }
        }
        // Apply mappings
        for (arg_name, mapping) in &manifest.command.mappings {
            if let Some(arg_value) = validated.get(arg_name) {
                if let Some(flags) = mapping.get(arg_value) {
                    cmd = cmd.replace(&format!("{{_{}_flags}}", arg_name), flags);
                    cmd = cmd.replace("{_scan_flags}", flags);
                }
            }
        }
        // Interpolate args
        for (key, value) in &validated {
            cmd = cmd.replace(&format!("{{{}}}", key), value);
        }
        // Clean placeholders
        cmd = cmd
            .replace("{_output_file}", "/dev/null")
            .replace("{_scan_id}", "<scan-id>")
            .replace("{_evidence_dir}", "<evidence-dir>");
        let cmd = cmd.split_whitespace().collect::<Vec<_>>().join(" ");
        println!("  Command:   {}", cmd);
    } else if let Some(executor) = &manifest.command.executor {
        println!("  Executor:  {}", executor);
    }

    if let Some(cedar) = &manifest.tool.cedar {
        println!("  Cedar:     {} / {}", cedar.resource, cedar.action);
    }

    println!("\n  [dry run — command not executed]");
}

fn cmd_schema(name: &str) {
    let manifests = load_manifests_from_dir(Path::new("tools"));
    let manifest = manifests.iter().find(|(n, _)| n == name).map(|(_, m)| m);

    let manifest = match manifest {
        Some(m) => m,
        None => match load_manifest(Path::new(name)) {
            Ok(m) => {
                print_schema(&m);
                return;
            }
            Err(_) => {
                eprintln!("Tool '{}' not found", name);
                std::process::exit(1);
            }
        },
    };
    print_schema(manifest);
}

fn print_schema(manifest: &symbi_runtime::toolclad::manifest::Manifest) {
    let executor = ToolCladExecutor::new(vec![(manifest.tool.name.clone(), manifest.clone())]);
    let defs = executor.tool_definitions();
    if let Some(td) = defs.first() {
        let schema = serde_json::json!({
            "name": td.name,
            "description": td.description,
            "inputSchema": td.parameters,
        });
        println!("{}", serde_json::to_string_pretty(&schema).unwrap());
    }
}

fn parse_arg_pairs(args: &[String]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for arg in args {
        if let Some((key, value)) = arg.split_once('=') {
            map.insert(key.to_string(), value.to_string());
        }
    }
    map
}

fn print_manifest_summary(m: &symbi_runtime::toolclad::manifest::Manifest) {
    println!("  Binary:    {}", m.tool.binary);
    println!("  Timeout:   {}s", m.tool.timeout_seconds);
    println!("  Risk:      {}", m.tool.risk_tier);
    println!("  Args:      {}", m.args.len());
    if let Some(cedar) = &m.tool.cedar {
        println!("  Cedar:     {} / {}", cedar.resource, cedar.action);
    }
}
