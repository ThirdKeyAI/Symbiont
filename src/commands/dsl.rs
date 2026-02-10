//! `symbi dsl` subcommand — parse and analyze Symbiont DSL files.

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
