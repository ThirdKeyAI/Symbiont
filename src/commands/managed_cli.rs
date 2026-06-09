//! Deterministic managed-CLI agent execution (Mode B).
//!
//! Agents whose metadata declares `executor = "claude_code"` are run by spawning
//! a governed Claude Code subprocess through the runtime's `CliExecutor`, rather
//! than the ORGA reasoning loop. The subprocess receives the `SYMBIONT_*` env
//! handshake (so the symbi-claude-code plugin defers enforcement to the outer
//! Gate), the plugin loaded via `--plugin-dir`, and the stdio `symbi mcp`
//! back-channel via `--mcp-config`. See `agents/code_reviewer.symbi`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use clap::ArgMatches;
use tokio::time::Duration;

use symbi_runtime::cli_executor::{
    ClaudeCodeAdapter, CliExecutor, CliExecutorConfig, CodeGenRequest,
};
use symbi_runtime::reasoning::conversation::Conversation;
use symbi_runtime::reasoning::loop_types::{LoopDecision, LoopState, ProposedAction};
use symbi_runtime::reasoning::policy_bridge::{DefaultPolicyGate, ReasoningPolicyGate};
use symbi_runtime::types::AgentId;

const DEFAULT_MAX_TURNS: u32 = 12;
const DEFAULT_BUDGET_TOKENS: u64 = 100_000;
const DEFAULT_BUDGET_SECS: u64 = 15 * 60;

/// Run a `executor = "claude_code"` agent as a governed Mode B subprocess.
pub async fn run_claude_code(
    matches: &ArgMatches,
    agent_name: &str,
    meta: &HashMap<String, String>,
    input: &str,
) {
    // --- resolve configuration ---
    let target_dir = matches
        .get_one::<String>("target")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    if !target_dir.is_dir() {
        eprintln!("✗ target '{}' is not a directory", target_dir.display());
        std::process::exit(1);
    }

    let max_turns = flag_u32(matches, "max-turns").unwrap_or(DEFAULT_MAX_TURNS);
    let budget_tokens = flag_u64(matches, "budget-tokens").unwrap_or(DEFAULT_BUDGET_TOKENS);
    let budget_secs = matches
        .get_one::<String>("budget-timeout")
        .and_then(|s| parse_duration_secs(s))
        .unwrap_or(DEFAULT_BUDGET_SECS);

    let plugin_dir = match resolve_plugin_dir(matches) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("✗ {e}");
            eprintln!("  Set --plugin-dir, SYMBIONT_CLAUDE_PLUGIN_DIR, or place the");
            eprintln!("  symbi-claude-code repo next to the symbiont repo.");
            std::process::exit(1);
        }
    };

    let model = meta_str(meta, "model");
    let allowed_tools = meta_str(meta, "allowed_tools")
        .map(|s| {
            s.split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let system_prompt = meta_str(meta, "system_prompt");

    // Task prompt: explicit `--input`, else a default review instruction.
    let prompt = if input.trim().is_empty() || input.trim() == "{}" {
        format!(
            "Review the code in {}. Prefer the staged and unstaged git diff.",
            target_dir.display()
        )
    } else {
        input.to_string()
    };

    // --- policy Gate: the spawn itself is the privileged action ---
    let agent_id = AgentId::new();
    let session_id = agent_id.to_string();
    let gate = build_policy_gate().await;
    let action = ProposedAction::ToolCall {
        call_id: session_id.clone(),
        name: "claude_code".to_string(),
        arguments: serde_json::json!({
            "agent": agent_name,
            "target": target_dir.display().to_string(),
        })
        .to_string(),
    };
    let state = LoopState::new(agent_id, Conversation::with_system("managed-cli"));
    match gate.evaluate_action(&agent_id, &action, &state).await {
        LoopDecision::Allow => {}
        LoopDecision::Deny { reason } | LoopDecision::Modify { reason, .. } => {
            eprintln!("✗ policy gate denied claude_code spawn: {reason}");
            eprintln!("  Add a Cedar policy allowing it, or set SYMBI_INSECURE_ALLOW_ALL=1.");
            std::process::exit(1);
        }
    }

    // --- build the governed spawn ---
    let mcp_config = serde_json::json!({
        "mcpServers": {
            "symbi": { "type": "stdio", "command": "symbi", "args": ["mcp"] }
        }
    })
    .to_string();

    let adapter = ClaudeCodeAdapter {
        executable_path: "claude".to_string(),
        max_turns: Some(max_turns),
        model,
        allowed_tools,
        disallowed_tools: Vec::new(),
        plugin_dirs: vec![plugin_dir.display().to_string()],
        mcp_config: Some(mcp_config),
        strict_mcp_config: true,
        // NOTE: do NOT pass --bare. It skips reading ~/.claude (credentials
        // included), which breaks subscription-login auth ("Not logged in").
        // --strict-mcp-config already restricts MCP to ours, so --bare is
        // unnecessary here.
        bare: false,
        permission_mode: Some("dontAsk".to_string()),
        append_system_prompt: system_prompt,
        managed: true,
        session_id: Some(session_id.clone()),
        budget_tokens: Some(budget_tokens),
        budget_timeout_secs: Some(budget_secs),
        project_dir: Some(target_dir.display().to_string()),
    };

    let request = CodeGenRequest {
        prompt,
        working_dir: target_dir.clone(),
        target_files: Vec::new(),
        system_context: None,
        model: None,
        options: HashMap::new(),
    };

    let config = CliExecutorConfig {
        max_runtime: Duration::from_secs(budget_secs),
        ..Default::default()
    };

    println!("→ Managed Claude Code run: agent '{agent_name}' (Mode B)");
    println!("  plugin-dir: {}", plugin_dir.display());
    println!("  target:     {}", target_dir.display());
    println!("  bounds:     max-turns={max_turns}, timeout={budget_secs}s, tokens~{budget_tokens}");
    println!("  session:    {session_id}");
    println!();

    let executor = CliExecutor::new(config);
    match executor.execute(&adapter, &request).await {
        Ok(result) => {
            if let Some(json) = &result.parsed_output {
                println!("{}", serde_json::to_string_pretty(json).unwrap_or_default());
            } else {
                println!("{}", result.execution.stdout);
            }
            eprintln!(
                "\n--- managed run {} in {}ms (exit {}) ---",
                if result.success { "ok" } else { "FAILED" },
                result.execution.execution_time_ms,
                result.execution.exit_code
            );
            if !result.success {
                if !result.execution.stderr.is_empty() {
                    eprintln!("{}", result.execution.stderr);
                }
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("✗ managed Claude Code execution failed: {e}");
            std::process::exit(1);
        }
    }
}

fn flag_u32(m: &ArgMatches, key: &str) -> Option<u32> {
    m.get_one::<String>(key).and_then(|s| s.parse().ok())
}

fn flag_u64(m: &ArgMatches, key: &str) -> Option<u64> {
    m.get_one::<String>(key).and_then(|s| s.parse().ok())
}

/// Read a metadata value, stripping the surrounding quotes the DSL parser keeps.
fn meta_str(meta: &HashMap<String, String>, key: &str) -> Option<String> {
    meta.get(key)
        .map(|v| v.trim().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
}

/// Parse a duration like `15m`, `900s`, `2h`, or a bare number of seconds.
fn parse_duration_secs(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(rest) = s.strip_suffix('h') {
        return rest.trim().parse::<u64>().ok().map(|v| v * 3600);
    }
    if let Some(rest) = s.strip_suffix('m') {
        return rest.trim().parse::<u64>().ok().map(|v| v * 60);
    }
    if let Some(rest) = s.strip_suffix('s') {
        return rest.trim().parse::<u64>().ok();
    }
    s.parse::<u64>().ok()
}

/// Resolve the symbi-claude-code plugin directory: explicit flag, then
/// `SYMBIONT_CLAUDE_PLUGIN_DIR`, then a sibling-repo autodetect.
fn resolve_plugin_dir(m: &ArgMatches) -> Result<PathBuf, String> {
    if let Some(p) = m.get_one::<String>("plugin-dir") {
        let pb = PathBuf::from(p);
        return if pb.is_dir() {
            Ok(pb)
        } else {
            Err(format!("plugin-dir '{}' not found", pb.display()))
        };
    }
    if let Ok(p) = std::env::var("SYMBIONT_CLAUDE_PLUGIN_DIR") {
        let pb = PathBuf::from(&p);
        if pb.is_dir() {
            return Ok(pb);
        }
        return Err(format!(
            "SYMBIONT_CLAUDE_PLUGIN_DIR '{}' not found",
            pb.display()
        ));
    }
    for cand in candidate_sibling_dirs() {
        if cand.join(".claude-plugin").is_dir() || cand.join("hooks/hooks.json").is_file() {
            return Ok(cand);
        }
    }
    Err("could not locate the symbi-claude-code plugin".to_string())
}

fn candidate_sibling_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(parent) = cwd.parent() {
            v.push(parent.join("symbi-claude-code"));
        }
    }
    v
}

async fn build_policy_gate() -> Arc<dyn ReasoningPolicyGate> {
    if std::env::var("SYMBI_INSECURE_ALLOW_ALL").as_deref() == Ok("1") {
        eprintln!("WARNING: SYMBI_INSECURE_ALLOW_ALL=1 — policy gate permissive (dev only).");
        Arc::new(DefaultPolicyGate::permissive_for_dev_only())
    } else if let Some(cedar) = super::up::try_wire_cedar_policy_gate().await {
        cedar
    } else {
        Arc::new(DefaultPolicyGate::new())
    }
}
