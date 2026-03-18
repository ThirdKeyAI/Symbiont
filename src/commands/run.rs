//! `symbi run` — Execute a single agent from the CLI
//!
//! Loads a DSL file, sets up the reasoning loop with cloud inference,
//! runs the ORGA cycle with the provided input, and exits.

use clap::ArgMatches;
use std::path::Path;
use std::sync::Arc;

pub async fn run(matches: &ArgMatches) {
    let file = matches
        .get_one::<String>("agent")
        .expect("agent argument is required");
    let input = matches
        .get_one::<String>("input")
        .cloned()
        .unwrap_or_else(|| "{}".to_string());
    let max_iterations = matches
        .get_one::<String>("max-iterations")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(10);

    // Resolve agent file path
    let agent_path = resolve_agent_path(file);
    let dsl_source = match std::fs::read_to_string(&agent_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("✗ Failed to read '{}': {}", agent_path.display(), e);
            std::process::exit(1);
        }
    };

    let agent_name = agent_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "agent".to_string());

    // Parse DSL to extract metadata
    let description = match dsl::parse_dsl(&dsl_source) {
        Ok(tree) => {
            let meta = dsl::extract_metadata(&tree, &dsl_source);
            meta.get("description").cloned().unwrap_or_default()
        }
        Err(_) => String::new(),
    };

    // Set up inference provider from environment
    let provider = match symbi_runtime::reasoning::providers::cloud::CloudInferenceProvider::from_env()
    {
        Some(p) => Arc::new(p) as Arc<dyn symbi_runtime::reasoning::inference::InferenceProvider>,
        None => {
            eprintln!("✗ No LLM provider configured.");
            eprintln!(
                "  Set one of: OPENROUTER_API_KEY, OPENAI_API_KEY, or ANTHROPIC_API_KEY"
            );
            std::process::exit(1);
        }
    };

    println!("→ Running agent: {} ({})", agent_name, agent_path.display());
    if !description.is_empty() {
        println!("  {}", description);
    }
    println!("→ Input: {}", truncate(&input, 200));
    println!();

    // Build the reasoning loop runner
    use symbi_runtime::reasoning::circuit_breaker::CircuitBreakerRegistry;
    use symbi_runtime::reasoning::context_manager::DefaultContextManager;
    use symbi_runtime::reasoning::conversation::{Conversation, ConversationMessage};
    use symbi_runtime::reasoning::executor::DefaultActionExecutor;
    use symbi_runtime::reasoning::loop_types::{BufferedJournal, LoopConfig};
    use symbi_runtime::reasoning::policy_bridge::DefaultPolicyGate;
    use symbi_runtime::reasoning::reasoning_loop::ReasoningLoopRunner;
    use symbi_runtime::types::AgentId;

    let runner = ReasoningLoopRunner {
        provider,
        policy_gate: Arc::new(DefaultPolicyGate::permissive()),
        executor: Arc::new(DefaultActionExecutor::default()),
        context_manager: Arc::new(DefaultContextManager::default()),
        circuit_breakers: Arc::new(CircuitBreakerRegistry::default()),
        journal: Arc::new(BufferedJournal::new(1000)),
        knowledge_bridge: None,
    };

    // Build conversation from DSL system prompt + user input
    let system_prompt = format!(
        "You are agent '{}'. Follow the governance rules defined in your DSL.\n\n--- Agent DSL ---\n{}\n--- End DSL ---",
        agent_name, dsl_source
    );

    let mut conv = Conversation::with_system(&system_prompt);
    conv.push(ConversationMessage::user(&input));

    let config = LoopConfig {
        max_iterations,
        max_total_tokens: 100_000,
        ..Default::default()
    };

    // Run the ORGA loop
    let result = runner.run(AgentId::new(), conv, config).await;

    // Print results
    println!("{}", result.output);
    eprintln!(
        "\n--- {} iterations, {} tokens, terminated: {:?} ---",
        result.iterations, result.total_usage.total_tokens, result.termination_reason
    );
}

/// Resolve agent path: check direct path, then agents/ directory
fn resolve_agent_path(name: &str) -> std::path::PathBuf {
    let path = Path::new(name);

    // Direct path (with or without .dsl extension)
    if path.exists() {
        return path.to_path_buf();
    }
    if !name.ends_with(".dsl") {
        let with_ext = format!("{}.dsl", name);
        let path_ext = Path::new(&with_ext);
        if path_ext.exists() {
            return path_ext.to_path_buf();
        }
    }

    // Check agents/ directory
    let agents_path = Path::new("agents").join(name);
    if agents_path.exists() {
        return agents_path;
    }
    if !name.ends_with(".dsl") {
        let agents_path_ext = Path::new("agents").join(format!("{}.dsl", name));
        if agents_path_ext.exists() {
            return agents_path_ext;
        }
    }

    // Return original path (will fail with a readable error)
    path.to_path_buf()
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}
