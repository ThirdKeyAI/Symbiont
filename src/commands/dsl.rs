//! `symbi dsl` subcommand — parse and analyze Symbiont DSL files.

/// Validate one DSL source without printing the AST.
///
/// Returns `Ok(())` when the file parses cleanly, or `Err` with a short,
/// single-line reason. Tree-sitter is error-recovering: `parse_dsl` returns a
/// tree even for nonsense input, so a clean parse is only proven by the tree
/// carrying no `ERROR` node.
pub fn check_source(source: &str) -> Result<(), String> {
    let tree = dsl::parse_dsl(source).map_err(|e| e.to_string())?;
    let root = tree.root_node();
    if !root.has_error() {
        return Ok(());
    }
    let diagnostics = dsl::find_errors(root, source, 1);
    match diagnostics.first() {
        Some(d) => Err(format!(
            "parse error at line {}, column {}",
            d.start_line, d.start_col
        )),
        None => Err("parse error".to_string()),
    }
}

/// Run `--check`: print exactly one column-aligned line for the source and
/// return the process exit code (0 valid, 1 invalid). The column width matches
/// `symbi tools validate` so the two verdicts line up in the same terminal.
pub fn check(source: &str, filename: Option<&str>) -> i32 {
    let file_display = filename.unwrap_or("<inline>");
    match check_source(source) {
        Ok(()) => {
            println!("{:<40} OK", file_display);
            0
        }
        Err(e) => {
            println!("{:<40} FAILED: {e}", file_display);
            1
        }
    }
}

/// Run the DSL parse-and-analyze command.
pub fn run(source: &str, filename: Option<&str>) {
    let label = filename.unwrap_or("<inline>");

    // Parse the DSL source
    let tree = match dsl::parse_dsl(source) {
        Ok(tree) => tree,
        Err(e) => {
            eprintln!("Error: failed to parse DSL ({}): {}", label, e);
            std::process::exit(1);
        }
    };

    let root = tree.root_node();

    // Check for parse errors
    let has_errors = root.has_error();
    if has_errors {
        eprintln!("Parse errors in {}:", label);
        dsl::find_errors(root, source, 1);
        eprintln!();
    }

    // Extract metadata
    let metadata = dsl::extract_metadata(&tree, source);
    if !metadata.is_empty() {
        println!("Metadata:");
        for (key, value) in &metadata {
            println!("  {}: {}", key, value);
        }
        println!();
    }

    // Extract with blocks (sandbox / timeout configuration)
    match dsl::extract_with_blocks(&tree, source) {
        Ok(with_blocks) if !with_blocks.is_empty() => {
            println!("With blocks: {}", with_blocks.len());
            for (i, wb) in with_blocks.iter().enumerate() {
                println!("  [{}]", i + 1);
                if let Some(ref tier) = wb.sandbox_tier {
                    println!("    sandbox: {}", tier);
                }
                if let Some(timeout) = wb.timeout {
                    println!("    timeout: {}s", timeout);
                }
                for attr in &wb.attributes {
                    if attr.name != "sandbox" && attr.name != "timeout" {
                        println!("    {}: {}", attr.name, attr.value);
                    }
                }
            }
            println!();
        }
        Err(e) => {
            eprintln!("Warning: failed to extract with blocks: {}", e);
        }
        _ => {}
    }

    // Extract schedule definitions
    match dsl::extract_schedule_definitions(&tree, source) {
        Ok(schedules) if !schedules.is_empty() => {
            println!("Schedules: {}", schedules.len());
            for s in &schedules {
                if let Some(ref cron_expr) = s.cron {
                    println!("  {} (cron: {}, tz: {})", s.name, cron_expr, s.timezone);
                } else if let Some(ref at) = s.at {
                    println!("  {} (at: {}, tz: {})", s.name, at, s.timezone);
                }
                if let Some(ref agent) = s.agent {
                    println!("    agent: {}", agent);
                }
                if let Some(ref policy) = s.policy {
                    println!("    policy: {}", policy);
                }
                if s.one_shot {
                    println!("    one_shot: true");
                }
                if let Some(ref deliver) = s.deliver {
                    println!("    deliver: {}", deliver);
                }
            }
            println!();
        }
        Err(e) => {
            eprintln!("Warning: failed to extract schedules: {}", e);
        }
        _ => {}
    }

    // Extract channel definitions
    match dsl::extract_channel_definitions(&tree, source) {
        Ok(channels) if !channels.is_empty() => {
            println!("Channels: {}", channels.len());
            for ch in &channels {
                println!(
                    "  {} (platform: {})",
                    ch.name,
                    ch.platform.as_deref().unwrap_or("?")
                );
                if let Some(ref ws) = ch.workspace {
                    println!("    workspace: {}", ws);
                }
                if !ch.channels.is_empty() {
                    println!("    channels: {}", ch.channels.join(", "));
                }
                if let Some(ref agent) = ch.default_agent {
                    println!("    default_agent: {}", agent);
                }
                if !ch.policy_rules.is_empty() {
                    println!("    policy rules: {}", ch.policy_rules.len());
                }
                if !ch.data_classification.is_empty() {
                    println!(
                        "    data classification rules: {}",
                        ch.data_classification.len()
                    );
                }
            }
            println!();
        }
        Err(e) => {
            eprintln!("Warning: failed to extract channels: {}", e);
        }
        _ => {}
    }

    // Print AST structure
    println!("AST:");
    dsl::print_ast(root, source, 1);

    if has_errors {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_AGENT: &str = r#"
metadata {
    version = "1.0.0"
    author = "test"
}

agent assistant(query: String) -> String {
    capabilities = ["chat"]

    with memory = "ephemeral", privacy = "standard" {
        return "ok";
    }
}
"#;

    #[test]
    fn check_source_accepts_a_well_formed_agent() {
        assert_eq!(check_source(VALID_AGENT), Ok(()));
    }

    #[test]
    fn check_source_rejects_garbage() {
        // Tree-sitter recovers rather than failing, so this only fails if the
        // ERROR nodes in the recovered tree are inspected.
        let err = check_source("this is not valid dsl {{{\n")
            .expect_err("malformed source must not validate");
        assert!(
            err.starts_with("parse error at line "),
            "unexpected reason: {err}"
        );
    }

    #[test]
    fn check_returns_the_exit_code_matching_the_verdict() {
        assert_eq!(check(VALID_AGENT, Some("good.symbi")), 0);
        assert_eq!(check("this is not valid dsl {{{\n", Some("bad.symbi")), 1);
    }
}
