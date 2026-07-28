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
    ClaudeCodeAdapter, CliExecutor, CliExecutorConfig, CodeGenRequest, LineSink,
};
use symbi_runtime::reasoning::conversation::Conversation;
use symbi_runtime::reasoning::loop_types::{LoopDecision, LoopState, ProposedAction};
use symbi_runtime::reasoning::policy_bridge::ReasoningPolicyGate;
use symbi_runtime::types::AgentId;

/// Policy surface Mode B reads. `symbi run` spawning a managed subprocess is a
/// different blast radius from its in-process reasoning loop, so the two do not
/// share a policy directory.
const MANAGED_CLI_SURFACE: &str = "managed-cli";

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
    // Opt-in only. Unset means `--permission-mode` is omitted and the child
    // applies its own default, which still prompts for anything outside
    // `allowed_tools`; an agent that must run unattended declares
    // `permission_mode = "dontAsk"` in its `metadata { ... }` block and takes
    // that trade-off explicitly.
    let permission_mode = meta_str(meta, "permission_mode");

    // --- require an explicit tool allowlist ---
    //
    // The spawn below is gated once (see the policy-gate check further
    // down), but nothing gates the child afterward: whatever
    // `permission_mode` resolves to applies for the whole session, and when
    // `allowed_tools` is empty `ClaudeCodeAdapter` omits `--allowedTools`
    // entirely (see `cli_executor/adapters/claude_code.rs`), so the child
    // falls back to its own defaults. One gate decision would then be
    // authorizing an unrestricted session, not a bounded one. Per-action
    // gating would require the child to call back into Symbiont's gate,
    // which is a trust-boundary redesign out of scope here — so refuse the
    // spawn instead of silently handing over an unrestricted session.
    if let Err(e) = require_allowed_tools(agent_name, &allowed_tools) {
        eprintln!("✗ {e}");
        std::process::exit(1);
    }

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
            // Name the exact file. Mode B reads the `managed-cli` surface, not
            // `run`, so a policy dropped next to the command the operator typed
            // is loaded by nothing and looks the same as having written none.
            eprintln!(
                "  Permit it in policies/{MANAGED_CLI_SURFACE}/ (read only by Mode B, not by\n  \
                 `symbi run`'s in-process loop), e.g. \
                 policies/{MANAGED_CLI_SURFACE}/claude_code.cedar:"
            );
            eprintln!(
                "      permit(principal, action == Action::\"tool_call::claude_code\", resource);"
            );
            eprintln!("  Or set SYMBI_INSECURE_ALLOW_ALL=1 for local development.");
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
        permission_mode,
        // Stream the child's events so its tool calls can be journalled while
        // it runs. `parse_output` folds the stream back to the same shape the
        // `json` format returns, so the printed result is unchanged.
        stream_json: true,
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

    let mut executor = CliExecutor::new(config);
    if let Some(sink) =
        mode_b_journal_sink(session_id.clone(), PathBuf::from(".symbiont").join("audit"))
    {
        executor = executor.with_stdout_line_sink(sink);
    }
    println!();

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

/// Argument keys whose values are replaced before a tool call is journalled.
///
/// `mask_sensitive_arguments` matches keys exactly, so the common spellings are
/// listed rather than a single canonical one.
const SENSITIVE_ARG_KEYS: &[&str] = &[
    "password",
    "passwd",
    "token",
    "access_token",
    "refresh_token",
    "secret",
    "client_secret",
    "api_key",
    "apiKey",
    "authorization",
    "credential",
    "credentials",
    "private_key",
];

/// Cap on the journalled rendering of one tool call's arguments.
///
/// A `Write` call carries an entire file in `content`, so the raw arguments are
/// unbounded. The audit record needs enough to identify what the child did, not
/// a second copy of the payload.
const MAX_JOURNALLED_ARGS_BYTES: usize = 2_048;

/// Build the stdout sink that records what the Mode B child does, as it does it.
///
/// The policy gate authorizes the spawn once and has no say over the child
/// afterward, which leaves its tool calls invisible. With `--output-format
/// stream-json` the child emits one JSON event per line, so this sink can turn
/// them into an append-only record at
/// `.symbiont/audit/mode-b-<session>.jsonl` while the run is still going —
/// worth doing live, because a run killed by the wall-clock timeout never
/// returns its buffered stdout at all.
///
/// Journalling never fails the run: a record that cannot be written is dropped
/// with a warning rather than aborting a session that is otherwise fine.
fn mode_b_journal_sink(session_id: String, audit_dir: PathBuf) -> Option<LineSink> {
    if let Err(e) = std::fs::create_dir_all(&audit_dir) {
        eprintln!(
            "⚠ cannot create {} ({e}) — this run will not be journalled",
            audit_dir.display()
        );
        return None;
    }
    let path = audit_dir.join(format!("mode-b-{session_id}.jsonl"));
    let file = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(f) => std::sync::Mutex::new(f),
        Err(e) => {
            eprintln!(
                "⚠ cannot open {} ({e}) — this run will not be journalled",
                path.display()
            );
            return None;
        }
    };

    println!("  journal:    {}", path.display());

    let sink = move |line: &str| {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            return; // non-JSON chatter on stdout is not an audit record
        };
        let mut records: Vec<serde_json::Value> = Vec::new();

        match event.get("type").and_then(|t| t.as_str()) {
            // Tool calls and their results ride inside message content blocks.
            Some("assistant") | Some("user") => {
                let blocks = event
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array());
                for block in blocks.into_iter().flatten() {
                    match block.get("type").and_then(|t| t.as_str()) {
                        Some("tool_use") => {
                            let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                            let masked = symbi_runtime::integrations::mask_sensitive_arguments(
                                block.get("input").unwrap_or(&serde_json::Value::Null),
                                &SENSITIVE_ARG_KEYS
                                    .iter()
                                    .map(|s| s.to_string())
                                    .collect::<Vec<_>>(),
                            );
                            let rendered = masked.to_string();
                            eprintln!("  ↳ tool_use {name}");
                            records.push(serde_json::json!({
                                "event": "tool_use",
                                "tool": name,
                                "tool_use_id": block.get("id"),
                                "arguments": symbi_runtime::text_util::truncate_utf8(
                                    &rendered, MAX_JOURNALLED_ARGS_BYTES),
                                "arguments_truncated": rendered.len() > MAX_JOURNALLED_ARGS_BYTES,
                            }));
                        }
                        Some("tool_result") => {
                            records.push(serde_json::json!({
                                "event": "tool_result",
                                "tool_use_id": block.get("tool_use_id"),
                                "is_error": block.get("is_error").and_then(|e| e.as_bool())
                                    .unwrap_or(false),
                            }));
                        }
                        _ => {}
                    }
                }
            }
            // Terminal event: the child's own summary of the whole session.
            Some("result") => {
                records.push(serde_json::json!({
                    "event": "result",
                    "subtype": event.get("subtype"),
                    "is_error": event.get("is_error"),
                    "num_turns": event.get("num_turns"),
                    "permission_denials": event.get("permission_denials"),
                    "usage": event.get("usage"),
                }));
            }
            _ => {}
        }

        if records.is_empty() {
            return;
        }
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let mut out = String::new();
        for mut record in records {
            if let Some(obj) = record.as_object_mut() {
                obj.insert("ts_ms".to_string(), serde_json::json!(ts));
                obj.insert("session".to_string(), serde_json::json!(session_id));
            }
            out.push_str(&record.to_string());
            out.push('\n');
        }
        // Local append under a mutex. This runs inline on the child's read
        // path, so it must stay fast — a slow sink stalls reading and can trip
        // the idle timeout on a healthy child.
        if let Ok(mut f) = file.lock() {
            use std::io::Write as _;
            let _ = f.write_all(out.as_bytes());
        }
    };

    Some(Arc::new(sink))
}

/// Refuse to spawn a Mode B session when the agent's DSL declares no
/// `allowed_tools`.
///
/// Symbiont's policy gate evaluates the spawn itself (once, up front); it
/// has no way to evaluate the child's tool calls after that — Mode B runs
/// `--permission-mode dontAsk` for the life of the session. The only
/// in-session restriction available is the child's own `--allowedTools`
/// flag, sourced from this DSL metadata. An empty list is not "no
/// restriction configured yet", it is "the child gets its own unrestricted
/// defaults" (`ClaudeCodeAdapter` omits `--allowedTools` entirely when the
/// vec is empty) — so refuse rather than let that pass silently.
fn require_allowed_tools(
    agent_name: &str,
    allowed_tools: &[String],
) -> std::result::Result<(), String> {
    if allowed_tools.is_empty() {
        return Err(format!(
            "agent '{agent_name}' declares no `allowed_tools`; refusing to spawn an unrestricted \
             Mode B session.\n  Add `allowed_tools = \"Tool1,Tool2,...\"` to the `metadata {{ ... }}` \
             block of the agent's .symbi file (see agents/code_reviewer.symbi for an example).\n  \
             Symbiont's policy gate authorizes the spawn itself, once; it cannot restrict what the \
             child does for the rest of the session — `allowed_tools` becomes the child's own \
             --allowedTools allowlist, the only in-session restriction that exists after spawn."
        ));
    }
    Ok(())
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
    let insecure_allow_all = std::env::var("SYMBI_INSECURE_ALLOW_ALL").as_deref() == Ok("1");
    if insecure_allow_all {
        eprintln!("WARNING: SYMBI_INSECURE_ALLOW_ALL=1 — policy gate permissive (dev only).");
    }
    symbi_runtime::reasoning::governed_gate(symbi_runtime::reasoning::GateOptions {
        policies_dir: PathBuf::from("policies"),
        surface: Some(MANAGED_CLI_SURFACE.to_string()),
        insecure_allow_all,
        escalation: None,
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_when_allowed_tools_is_empty() {
        let err = require_allowed_tools("code_reviewer", &[])
            .expect_err("an agent with no allowed_tools must be refused");
        assert!(
            err.contains("allowed_tools"),
            "error must name the missing field, got: {err}"
        );
        assert!(
            err.contains("metadata"),
            "error must point at the DSL metadata block, got: {err}"
        );
        assert!(
            err.contains("code_reviewer"),
            "error must name the offending agent, got: {err}"
        );
        assert!(
            err.to_lowercase().contains("cannot restrict"),
            "error must say Symbiont cannot restrict the session after spawn, got: {err}"
        );
    }

    #[test]
    fn proceeds_when_allowed_tools_is_declared() {
        let tools = vec!["Read".to_string(), "Grep".to_string()];
        require_allowed_tools("code_reviewer", &tools)
            .expect("an agent that declares allowed_tools must proceed past the check");
    }

    /// Feed the sink the event shapes a real `--output-format stream-json` run
    /// emits and check what lands in the journal.
    #[test]
    fn journal_records_tool_calls_and_masks_sensitive_arguments() {
        let dir = tempfile::tempdir().unwrap();
        let session = "sess-abc".to_string();
        let sink = mode_b_journal_sink(session.clone(), dir.path().to_path_buf())
            .expect("a writable audit dir must yield a sink");

        sink(r#"{"type":"system","subtype":"init","session_id":"s1"}"#);
        sink(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1",
                "name":"Bash","input":{"command":"ls","api_key":"sk-live-SHOULD-NOT-APPEAR"}}]}}"#,
        );
        sink(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true}]}}"#,
        );
        sink("this line is not JSON and must be ignored");
        sink(
            r#"{"type":"result","subtype":"success","is_error":false,"num_turns":2,"permission_denials":["Write"]}"#,
        );
        drop(sink);

        let body = std::fs::read_to_string(dir.path().join("mode-b-sess-abc.jsonl")).unwrap();
        let records: Vec<serde_json::Value> = body
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("every journal line must be valid JSON"))
            .collect();

        let kinds: Vec<&str> = records
            .iter()
            .map(|r| r["event"].as_str().unwrap())
            .collect();
        assert_eq!(
            kinds,
            vec!["tool_use", "tool_result", "result"],
            "system events and non-JSON noise are not audit records"
        );

        assert_eq!(records[0]["tool"], "Bash");
        assert_eq!(records[0]["tool_use_id"], "t1");
        assert_eq!(records[0]["session"], "sess-abc");
        assert!(records[0]["ts_ms"].as_u64().unwrap() > 0);

        let args = records[0]["arguments"].as_str().unwrap();
        assert!(
            !args.contains("sk-live-SHOULD-NOT-APPEAR"),
            "a sensitive argument value must never reach the journal, got: {args}"
        );
        assert!(args.contains("[REDACTED:api_key]"), "got: {args}");
        assert!(
            args.contains("ls"),
            "non-sensitive arguments stay legible: {args}"
        );

        assert_eq!(records[1]["is_error"], true);
        assert_eq!(records[2]["permission_denials"][0], "Write");
        assert_eq!(records[2]["num_turns"], 2);
    }

    /// A `Write` call carries a whole file in `content`; the journal records
    /// what the child did, not a second copy of the payload.
    #[test]
    fn journal_truncates_oversize_arguments() {
        let dir = tempfile::tempdir().unwrap();
        let sink = mode_b_journal_sink("s".to_string(), dir.path().to_path_buf()).unwrap();
        let huge = "x".repeat(MAX_JOURNALLED_ARGS_BYTES * 2);
        sink(&format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","id":"t1","name":"Write","input":{{"content":"{huge}"}}}}]}}}}"#
        ));
        drop(sink);

        let body = std::fs::read_to_string(dir.path().join("mode-b-s.jsonl")).unwrap();
        let record: serde_json::Value = serde_json::from_str(body.lines().next().unwrap()).unwrap();
        assert_eq!(record["arguments_truncated"], true);
        assert!(
            record["arguments"].as_str().unwrap().len() <= MAX_JOURNALLED_ARGS_BYTES,
            "oversize arguments must be capped"
        );
    }
}
