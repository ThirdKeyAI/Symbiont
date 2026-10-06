//! `symbi run` — Execute a single agent from the CLI
//!
//! Loads a DSL file, sets up the reasoning loop with cloud inference,
//! runs the ORGA cycle with the provided input, and exits.

use clap::ArgMatches;
use std::path::Path;
use std::sync::Arc;

#[cfg(unix)]
pub async fn run(matches: &ArgMatches) {
    use symbi_runtime::reasoning::invocation::{open_invocation, InvocationId, OpenInvocation};
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

    let execution_settings = match dsl::resolve_execution_settings(&dsl_source, &agent_name) {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("✗ Invalid agent execution settings: {error}");
            std::process::exit(1);
        }
    };

    let source_policy = dsl::ExecutionPolicy::parse(&dsl_source, &execution_settings.agent_name)
        .unwrap_or_else(|error| {
            eprintln!("✗ Unsupported agent policy: {error}");
            std::process::exit(1);
        });

    // Metadata is file-wide in the supported DSL grammar.
    let meta = match dsl::parse_dsl(&dsl_source) {
        Ok(tree) => dsl::extract_metadata(&tree, &dsl_source),
        Err(_) => std::collections::HashMap::new(),
    };
    let description = meta.get("description").cloned().unwrap_or_default();

    // Managed-CLI agents (metadata `executor = "claude_code"`) take the
    // deterministic CliExecutor path (Mode B) instead of the ORGA reasoning loop.
    let executor_kind = meta
        .get("executor")
        .map(|v| v.trim().trim_matches('"').to_string());
    if executor_kind.as_deref() == Some("claude_code") {
        if matches.get_one::<String>("improvement").is_some() {
            eprintln!("✗ Improvements currently require ordinary ORGA execution; no managed CLI work started.");
            std::process::exit(1);
        }
        if matches.get_one::<String>("invocation-id").is_some() {
            eprintln!("✗ --invocation-id is not yet supported by managed CLI agents; no execution started.");
            std::process::exit(1);
        }
        #[cfg(feature = "cli-executor")]
        {
            super::managed_cli::run_claude_code(matches, &meta, &input, &source_policy).await;
            return;
        }
        #[cfg(not(feature = "cli-executor"))]
        {
            eprintln!(
                "✗ agent '{}' uses executor=claude_code, which requires the 'cli-executor' feature.",
                agent_name
            );
            eprintln!("  Rebuild with: cargo build --features cli-executor (on by default).");
            std::process::exit(1);
        }
    }

    let project = std::env::current_dir()
        .and_then(|path| path.canonicalize())
        .unwrap_or_else(|error| {
            eprintln!("✗ Cannot resolve the project directory: {error}");
            std::process::exit(1);
        });
    let config = LoopConfig {
        max_iterations,
        max_total_tokens: 100_000,
        timeout: execution_settings
            .timeout_seconds
            .map(std::time::Duration::from_secs)
            .map_or(LoopConfig::default().timeout, |timeout| {
                timeout.min(LoopConfig::default().timeout)
            }),
        ..Default::default()
    };
    // Merely installing the feature or initializing another workflow changes
    // nothing. Only the explicit CLI selection opens the improvement store.
    let improvement = matches.get_one::<String>("improvement").map(|workflow| {
        if std::env::var("SYMBI_INSECURE_ALLOW_ALL").as_deref() == Ok("1") {
            eprintln!("✗ Improvement runs refuse the permissive development policy bypass.");
            std::process::exit(1);
        }
        symbi_runtime::improvement::Store::open(&project, workflow)
            .and_then(|store| {
                store.pin(
                    &execution_settings.agent_name,
                    &execution_settings.agent_source,
                    matches
                        .get_one::<String>("improvement-trial")
                        .map(String::as_str),
                )
            })
            .unwrap_or_else(|error| {
                eprintln!("✗ Improvement selection refused: {error}");
                std::process::exit(1);
            })
    });
    let invocation_id = matches
        .get_one::<String>("invocation-id")
        .map(|value| value.parse::<InvocationId>())
        .transpose()
        .unwrap_or_else(|error| {
            eprintln!("✗ Invalid invocation ID: {error}");
            std::process::exit(1);
        })
        .unwrap_or_else(InvocationId::new_v4);
    eprintln!("Invocation ID: {invocation_id}");
    let mut request = serde_json::json!({"agent": execution_settings.agent_name,
        "source": execution_settings.agent_source, "input": input, "config": config});
    if let Some(pinned) = &improvement {
        request["improvement"] = pinned.identity();
        eprintln!("Improvement version: {}", pinned.candidate_id());
    }
    let invocation = match open_invocation(
        &project,
        "cli:orga",
        invocation_id,
        &request,
        AgentId::new(),
    )
    .await
    {
        Ok(OpenInvocation::Fresh(invocation)) => *invocation,
        Ok(OpenInvocation::Existing(result)) => {
            eprintln!("Existing invocation; no work repeated.");
            show_outcome(result, true)
        }
        Err(error) => {
            eprintln!("✗ Invocation refused: {error}");
            std::process::exit(1);
        }
    };
    let agent_id = invocation.agent_id();
    let journal = invocation.journal();
    print_audit(invocation.audit());
    let executor = match symbi_runtime::reasoning::build_agent_tool_executor(
        &project.join("tools"),
        &execution_settings,
    ) {
        Ok(executor) => executor,
        Err(error) => {
            eprintln!("✗ Agent sandbox selection failed: {error}");
            std::process::exit(1);
        }
    };

    // Set up inference provider from environment
    let executor = Arc::new(
        symbi_runtime::reasoning::source_policy::SourcePolicyExecutor::new(executor, source_policy),
    );

    let provider =
        match symbi_runtime::reasoning::providers::cloud::CloudInferenceProvider::from_env() {
            Some(p) => {
                Arc::new(p) as Arc<dyn symbi_runtime::reasoning::inference::InferenceProvider>
            }
            None => {
                eprintln!("✗ No LLM provider configured.");
                eprintln!("  Set one of: OPENROUTER_API_KEY, OPENAI_API_KEY, or ANTHROPIC_API_KEY");
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
    use symbi_runtime::reasoning::loop_types::LoopConfig;
    use symbi_runtime::reasoning::reasoning_loop::ReasoningLoopRunner;
    use symbi_runtime::types::AgentId;

    // `symbi run` is for trusted local execution. The policy gate defaults
    // to fail-closed (`DefaultPolicyGate::new()`), which denies every
    // proposed tool call and delegation. Operators who explicitly want the
    // unrestricted dev-mode behaviour can opt in via the
    // `SYMBI_INSECURE_ALLOW_ALL=1` env var (matching `symbi up`).
    let insecure_allow_all = std::env::var("SYMBI_INSECURE_ALLOW_ALL").as_deref() == Ok("1");
    if insecure_allow_all {
        eprintln!("\n");
        eprintln!("================================================================");
        eprintln!("WARNING: SYMBI_INSECURE_ALLOW_ALL=1 is set");
        eprintln!("Policy gate is in PERMISSIVE mode for this `symbi run` invocation.");
        eprintln!("Every LLM-proposed tool call and delegation will be allowed.");
        eprintln!("This is only safe for local development. Do NOT use in production.");
        eprintln!("================================================================\n");
    }
    let policy_gate =
        symbi_runtime::reasoning::governed_gate(symbi_runtime::reasoning::GateOptions {
            policies_dir: project.join("policies"),
            surface: Some("run".to_string()),
            insecure_allow_all,
            escalation: super::approval::from_matches(matches)
                .await
                .unwrap_or_else(|error| {
                    eprintln!("✗ Approval initialization failed: {error}");
                    std::process::exit(1);
                }),
        })
        .await;

    let runner = ReasoningLoopRunner {
        provider,
        policy_gate,
        // Real tool execution when `tools/` has ToolClad manifests (shell,
        // HTTP, MCP-proxy, session, browser backends); otherwise falls back
        // to the honest executor that surfaces tool calls as errors rather
        // than fabricating success.
        executor,
        context_manager: Arc::new(DefaultContextManager::default()),
        circuit_breakers: Arc::new(CircuitBreakerRegistry::default()),
        journal,
        knowledge_bridge: None,
        delegation: None,
    };

    // Build conversation from DSL system prompt + user input
    let system_prompt = format!(
        "You are agent '{}'. Follow the governance rules defined in your DSL.\n\n--- Agent DSL ---\n{}\n--- End DSL ---",
        execution_settings.agent_name, execution_settings.agent_source
    );

    let mut conv = Conversation::with_system(&system_prompt);
    conv.push(ConversationMessage::user(&input));

    // Run the ORGA loop
    let result = if let Some(pinned) = &improvement {
        runner
            .run_with_improvement(agent_id, conv, config, pinned)
            .await
            .unwrap_or_else(|error| {
                eprintln!("✗ Improvement execution refused: {error}");
                std::process::exit(1);
            })
    } else {
        runner.run(agent_id, conv, config).await
    };

    let errors: Vec<_> = result
        .conversation
        .messages()
        .iter()
        .filter(|message| {
            message.role == symbi_runtime::reasoning::MessageRole::Tool
                && message.content.starts_with("[Error]")
        })
        .take(8)
        .map(|message| format!("{:?}", truncate(&message.content, 2048)))
        .collect();
    let receipt = CliResult {
        output: result.output,
        iterations: result.iterations,
        total_tokens: result.total_usage.total_tokens,
        termination_reason: result.termination_reason,
    };
    let stored = invocation.finish(serde_json::to_value(receipt).expect("serializable CLI result")).await
        .unwrap_or_else(|error| {
            eprintln!("✗ Invocation receipt unavailable: {error}; reuse the same ID and inspect the original audit.");
            std::process::exit(2);
        });
    if matches!(
        &stored,
        symbi_runtime::reasoning::invocation::ExistingInvocation::Unresolved { .. }
    ) {
        for error in errors {
            eprintln!("Tool error (inspect audit for effect status): {error}");
        }
    }
    show_outcome(stored, false)
}

#[cfg(not(unix))]
pub async fn run(_: &ArgMatches) {
    eprintln!("✗ Protected CLI invocations require a Unix host.");
    std::process::exit(1);
}

#[cfg(unix)]
#[derive(serde::Serialize, serde::Deserialize)]
struct CliResult {
    output: String,
    iterations: u32,
    total_tokens: u32,
    termination_reason: symbi_runtime::reasoning::loop_types::TerminationReason,
}

#[cfg(unix)]
fn print_audit(audit: &symbi_runtime::reasoning::run_audit::RunAuditReference) {
    eprintln!("Audit run: {}", audit.run_id);
    eprintln!("Audit journal: {}", audit.path.display());
    eprintln!("Audit public key: {}", audit.public_key);
}

#[cfg(unix)]
fn show_outcome(
    outcome: symbi_runtime::reasoning::invocation::ExistingInvocation,
    display_audit: bool,
) -> ! {
    use symbi_runtime::reasoning::{invocation::ExistingInvocation, loop_types::TerminationReason};
    match outcome {
        ExistingInvocation::InProgress => {
            eprintln!(
                "Invocation is already in progress; retry with the same ID to inspect its result."
            );
            std::process::exit(2);
        }
        ExistingInvocation::Unresolved { audit } => {
            if display_audit {
                if let Some(audit) = audit {
                    print_audit(&audit);
                }
            }
            eprintln!("Invocation requires reconciliation; no work repeated. Inspect its protected run journal.");
            std::process::exit(2);
        }
        ExistingInvocation::Reconciled { audit, resolution } => {
            if display_audit {
                print_audit(&audit);
            }
            println!(
                "{}",
                serde_json::json!({"status":"reconciled", "resolution":resolution, "work_repeated":false})
            );
            // A reviewed outcome is not the missing original CLI result.
            std::process::exit(3);
        }
        ExistingInvocation::Recorded { audit, result } => {
            if display_audit {
                print_audit(&audit);
            }
            let result: CliResult = serde_json::from_value(result).unwrap_or_else(|error| {
                eprintln!("Invalid saved CLI receipt: {error}; no work repeated.");
                std::process::exit(1);
            });
            println!("{}", result.output);
            eprintln!(
                "\n--- {} iterations, {} tokens, terminated: {:?} ---",
                result.iterations, result.total_tokens, result.termination_reason
            );
            std::process::exit(
                if matches!(result.termination_reason, TerminationReason::Completed) {
                    0
                } else {
                    1
                },
            );
        }
    }
}

/// Resolve agent path: check direct path, then agents/ directory.
/// Tries the bare name, then `.symbi` (canonical), then `.dsl` (legacy).
fn resolve_agent_path(name: &str) -> std::path::PathBuf {
    let path = Path::new(name);

    // Direct path (already has extension or is a literal path).
    if path.exists() {
        return path.to_path_buf();
    }

    let already_extended = name.ends_with(".symbi") || name.ends_with(".dsl");
    let candidate_exts: &[&str] = if already_extended {
        &[]
    } else {
        &["symbi", "dsl"]
    };

    for ext in candidate_exts {
        let with_ext = format!("{}.{}", name, ext);
        let path_ext = Path::new(&with_ext);
        if path_ext.exists() {
            return path_ext.to_path_buf();
        }
    }

    // Check agents/ directory.
    let agents_path = Path::new("agents").join(name);
    if agents_path.exists() {
        return agents_path;
    }
    for ext in candidate_exts {
        let agents_path_ext = Path::new("agents").join(format!("{}.{}", name, ext));
        if agents_path_ext.exists() {
            return agents_path_ext;
        }
    }

    // Return original path (will fail with a readable error)
    path.to_path_buf()
}

fn truncate(s: &str, max: usize) -> &str {
    symbi_runtime::text_util::truncate_utf8(s, max)
}
