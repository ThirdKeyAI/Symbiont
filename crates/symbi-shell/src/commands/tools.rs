use super::CommandResult;
use crate::app::App;

pub fn tools(_app: &mut App, args: &str) -> CommandResult {
    if args.is_empty() {
        // List .toml files in tools/
        let dir = std::path::Path::new("tools");
        if !dir.exists() {
            return CommandResult::Output(
                "No tools/ directory. Run /init to create project structure.".to_string(),
            );
        }
        let mut tool_list = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "toml") {
                    tool_list.push(path.file_name().unwrap().to_string_lossy().to_string());
                }
            }
        }
        if tool_list.is_empty() {
            CommandResult::Output("No tool manifests found in tools/".to_string())
        } else {
            let mut out = String::from("Tools:\n");
            for t in &tool_list {
                out.push_str(&format!("  {}\n", t));
            }
            CommandResult::Output(out)
        }
    } else {
        match args.split_whitespace().next().unwrap_or("") {
            "validate" => {
                let file = args.strip_prefix("validate").unwrap_or("").trim();
                if file.is_empty() {
                    return CommandResult::Error(
                        "Usage: /tools validate <manifest.clad.toml>".to_string(),
                    );
                }
                match std::fs::read_to_string(file) {
                    Ok(content) => {
                        let constraints = crate::validation::constraints::ProjectConstraints::load(
                            std::path::Path::new(".symbi/constraints.toml"),
                        )
                        .unwrap_or_default();
                        match crate::validation::toolclad_validator::validate_toolclad(
                            &content,
                            &constraints.constraints.toolclad,
                        ) {
                            Ok(issues) if issues.is_empty() => {
                                CommandResult::Output(format!("{}: validation passed", file))
                            }
                            Ok(issues) => {
                                let mut out = format!("{}: {} issues\n", file, issues.len());
                                for i in &issues {
                                    out.push_str(&format!("  [{:?}] {}\n", i.severity, i.message));
                                }
                                CommandResult::Output(out)
                            }
                            Err(e) => CommandResult::Error(format!("Validation error: {}", e)),
                        }
                    }
                    Err(e) => CommandResult::Error(format!("Failed to read {}: {}", file, e)),
                }
            }
            "test" => CommandResult::Output(format!("[tools test not yet connected: {}]", args)),
            _ => CommandResult::Error(format!("Unknown tools subcommand: {}", args)),
        }
    }
}

pub fn skills(_app: &mut App, args: &str) -> CommandResult {
    if args.is_empty() {
        // List .dsl files in agents/
        let dir = std::path::Path::new("agents");
        if !dir.exists() {
            return CommandResult::Output(
                "No agents/ directory. Run /init to create project structure.".to_string(),
            );
        }
        let mut skill_list = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "dsl") {
                    skill_list.push(path.file_name().unwrap().to_string_lossy().to_string());
                }
            }
        }
        if skill_list.is_empty() {
            CommandResult::Output("No DSL files found in agents/".to_string())
        } else {
            let mut out = String::from("Agent definitions:\n");
            for s in &skill_list {
                out.push_str(&format!("  {}\n", s));
            }
            CommandResult::Output(out)
        }
    } else {
        CommandResult::Output(format!("[skills {} not yet connected]", args))
    }
}

pub fn verify(_app: &mut App, args: &str) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error("Usage: /verify @tool_name".to_string());
    }
    CommandResult::Output(format!(
        "[SchemaPin verification not yet connected for: {}]",
        args
    ))
}
