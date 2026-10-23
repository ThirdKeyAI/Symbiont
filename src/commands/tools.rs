//! `symbi tools` — Manage ToolClad tool manifests
//!
//! Subcommands: list, validate, test, schema, init

use clap::ArgMatches;
use std::collections::HashMap;
use std::path::Path;
use symbi_runtime::reasoning::executor::ActionExecutor;
use symbi_runtime::toolclad::executor::ToolCladExecutor;
use symbi_runtime::toolclad::manifest::{
    load_custom_types, load_manifest_with_types, load_manifests_from_dir_with_types, ArgDef,
    Manifest,
};

/// Project custom types; a malformed `toolclad.toml` is a hard error.
fn custom_types() -> HashMap<String, ArgDef> {
    load_custom_types(Path::new(".")).unwrap_or_else(|e| {
        eprintln!("ERROR: {e}");
        std::process::exit(1);
    })
}

/// Every manifest in `tools/`. Any load failure is reported and exits: a
/// tool must never silently disappear from the listing.
fn load_tools() -> Vec<(String, Manifest)> {
    load_manifests_from_dir_with_types(Path::new("tools"), &custom_types()).unwrap_or_else(|e| {
        eprintln!("ERROR: {e}");
        std::process::exit(1);
    })
}

fn load_manifest(path: &Path) -> Result<Manifest, String> {
    load_manifest_with_types(path, &custom_types())
}

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
        Some(("init", sub)) => {
            let name = sub.get_one::<String>("name").unwrap();
            cmd_init(name);
        }
        _ => {
            println!("Usage: symbi tools <list|validate|test|schema|init>");
            println!("  list                        List all discovered tools");
            println!("  validate [file]             Validate manifest(s)");
            println!("  test <name> --arg k=v ...   Dry-run a tool invocation");
            println!("  schema <name>               Output MCP JSON Schema");
            println!("  init <name>                 Create a starter .clad.toml manifest");
        }
    }
}

fn cmd_list() {
    let manifests = load_tools();
    if manifests.is_empty() {
        println!("No tools found in tools/");
        return;
    }

    println!(
        "{:<24} {:<10} {:<16} {:<8} CEDAR RESOURCE",
        "TOOL", "MODE", "BINARY", "RISK"
    );
    for (name, m) in &manifests {
        let cedar = m
            .tool
            .cedar
            .as_ref()
            .map(|c| c.resource.as_str())
            .unwrap_or("-");
        let binary = if m.source.is_some() {
            "source broker"
        } else if m.tool.binary.is_empty() {
            "-"
        } else {
            &m.tool.binary
        };
        println!(
            "{:<24} {:<10} {:<16} {:<8} {}",
            name, m.tool.mode, binary, m.tool.risk_tier, cedar
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
    let manifests = load_tools();
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

fn do_test(manifest: &Manifest, arg_strs: &[String]) {
    let args = parse_arg_pairs(arg_strs);
    match dry_run(manifest, &args, &custom_types()) {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
            println!("\n  [dry run — command not executed]");
        }
        Err(lines) => {
            for line in lines {
                eprintln!("{line}");
            }
            println!("\n  [dry run — validation failed, command not constructed]");
            std::process::exit(1);
        }
    }
}

/// Validate arguments exactly as the executor does and show the argv that
/// `build_argv` produces — each element is one argv entry, never re-split.
fn dry_run(
    manifest: &Manifest,
    args: &HashMap<String, String>,
    custom: &HashMap<String, ArgDef>,
) -> Result<Vec<String>, Vec<String>> {
    let mut out = vec![
        format!("  Manifest:  {}", manifest.tool.name),
        format!("  Binary:    {}", manifest.tool.binary),
        format!("  Timeout:   {}s", manifest.tool.timeout_seconds),
        format!("  Risk:      {}", manifest.tool.risk_tier),
        String::new(),
    ];
    let exec_mode = symbi_runtime::toolclad::command::exec_was_declared(manifest);
    let mut validated: HashMap<String, String> = HashMap::new();
    let mut failed = false;
    let mut names: Vec<&String> = manifest.args.keys().collect();
    names.sort();
    for arg_name in names {
        let arg_def = &manifest.args[arg_name];
        let value = args.get(arg_name).cloned().unwrap_or_else(|| {
            arg_def
                .default
                .as_ref()
                .map(|d| match d {
                    toml::Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .unwrap_or_default()
        });
        if value.is_empty() && arg_def.required {
            out.push(format!(
                "  ✗ {} ({}): MISSING — required",
                arg_name, arg_def.type_name
            ));
            failed = true;
            continue;
        }
        if value.is_empty() {
            out.push(format!("  ○ {} ({}): <empty>", arg_name, arg_def.type_name));
            validated.insert(arg_name.clone(), value);
            continue;
        }
        match symbi_runtime::toolclad::validator::validate_arg_in(
            arg_def,
            &value,
            Some(custom),
            exec_mode,
        ) {
            Ok(cleaned) => {
                out.push(format!(
                    "  ✓ {} ({}): {} → OK",
                    arg_name, arg_def.type_name, cleaned
                ));
                validated.insert(arg_name.clone(), cleaned);
            }
            Err(e) => {
                out.push(format!(
                    "  ✗ {} ({}): {} → FAILED: {}",
                    arg_name, arg_def.type_name, value, e
                ));
                failed = true;
            }
        }
    }
    if failed {
        return Err(out);
    }
    out.push(String::new());
    if let Some(source) = &manifest.source {
        out.push(format!(
            "  Source:    {:?} (fixed broker; no source read during dry run)",
            source.operation
        ));
    } else if let Some(executor) = &manifest.command.executor {
        out.push(format!("  Executor:  {executor}"));
    } else if manifest.command.exec.is_some() || manifest.command.template.is_some() {
        let injected = symbi_runtime::toolclad::command::Injected {
            scan_id: "<scan-id>".into(),
        };
        match symbi_runtime::toolclad::command::build_argv(manifest, &validated, &injected) {
            Ok(argv) => {
                out.push(format!(
                    "  Argv:      {}",
                    serde_json::to_string(&argv).unwrap_or_default()
                ));
            }
            Err(e) => {
                out.push(format!("  ✗ command construction failed: {e}"));
                return Err(out);
            }
        }
    }
    if let Some(cedar) = &manifest.tool.cedar {
        out.push(format!(
            "  Cedar:     {} / {}",
            cedar.resource, cedar.action
        ));
    }
    Ok(out)
}

fn cmd_schema(name: &str) {
    let manifests = load_tools();
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

fn print_schema(manifest: &Manifest) {
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

fn cmd_init(name: &str) {
    let tools_dir = Path::new("tools");
    if !tools_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(tools_dir) {
            eprintln!("Failed to create tools/ directory: {}", e);
            std::process::exit(1);
        }
    }

    let file_path = tools_dir.join(format!("{}.clad.toml", name));
    if file_path.exists() {
        eprintln!("Manifest already exists: {}", file_path.display());
        eprintln!(
            "Use 'symbi tools validate {}' to check it",
            file_path.display()
        );
        std::process::exit(1);
    }

    // Title-case the name for Cedar resource (e.g., "my_tool" -> "MyTool")
    let title_name: String = name
        .split(['_', '-'])
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => {
                    let upper: String = first.to_uppercase().collect();
                    upper + chars.as_str()
                }
                None => String::new(),
            }
        })
        .collect();

    let template = format!(
        r#"[tool]
name = "{name}"
version = "1.0.0"
binary = "{name}"
description = "TODO: describe what this tool does"
timeout_seconds = 30
risk_tier = "low"

[tool.cedar]
resource = "Tool::{title_name}"
action = "execute_tool"

[args.target]
position = 1
required = true
type = "string"
description = "TODO: describe the target"

[command]
exec = ["{name}", "{{target}}"]

[output]
format = "text"
envelope = true

[output.schema]
type = "object"

[output.schema.properties.raw_output]
type = "string"
description = "Tool output"
"#,
        name = name,
        title_name = title_name,
    );

    if let Err(e) = std::fs::write(&file_path, template) {
        eprintln!("Failed to write {}: {}", file_path.display(), e);
        std::process::exit(1);
    }

    println!("Created {}", file_path.display());
    println!();
    println!("Next steps:");
    println!("  1. Edit {} to configure your tool", file_path.display());
    println!(
        "  2. Run 'symbi tools validate {}' to check it",
        file_path.display()
    );
    println!(
        "  3. Run 'symbi tools test {} --arg target=example' to dry-run",
        name
    );
}

fn print_manifest_summary(m: &Manifest) {
    println!("  Binary:    {}", m.tool.binary);
    println!("  Timeout:   {}s", m.tool.timeout_seconds);
    println!("  Risk:      {}", m.tool.risk_tier);
    println!("  Args:      {}", m.args.len());
    if let Some(cedar) = &m.tool.cedar {
        println!("  Cedar:     {} / {}", cedar.resource, cedar.action);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dry_run_prints_the_real_argv_without_resplitting() {
        let manifest = symbi_runtime::toolclad::parse_manifest(
            r#"
[tool]
name = "fetch"
version = "1"
binary = "printf"
description = "d"
[args.header]
position = 1
type = "string"
allow_whitespace = true
[args.kind]
position = 2
type = "enum"
allowed = ["a", "b"]
[command]
exec = ["printf", "{_kind_flags}", "{header}"]
end_of_options = "--"
[command.mappings.kind]
a = "-x -y"
b = ""
[output]
format = "text"
"#,
            &HashMap::new(),
        )
        .unwrap();
        let mut args = HashMap::new();
        args.insert("header".to_string(), "X-A: one two".to_string());
        args.insert("kind".to_string(), "a".to_string());
        let lines = dry_run(&manifest, &args, &HashMap::new()).unwrap();
        let argv = lines.iter().find(|l| l.contains("Argv:")).unwrap();
        assert!(
            argv.contains(r#"["printf","-x","-y","--","X-A: one two"]"#),
            "{argv}"
        );
        args.insert("header".to_string(), "-H @/etc/passwd".to_string());
        assert!(dry_run(&manifest, &args, &HashMap::new()).is_err());
    }
}
