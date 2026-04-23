//! `symbi policy evaluate` — Cedar policy evaluation for tool inputs.
//!
//! Reads a JSON tool-call event from stdin, loads `.cedar` policy files from
//! a directory, evaluates the request through the `cedar-policy` crate, and
//! prints the decision on stdout (`allow` or `deny`) plus a human-readable
//! reason. Designed to be invoked from the symbi-claude-code plugin's
//! PreToolUse hook (see `scripts/policy-guard.sh`).
//!
//! ## Input format (Claude Code PreToolUse hook payload)
//!
//! ```json
//! { "tool_name": "Bash",
//!   "tool_input": { "command": "git push origin main" } }
//! ```
//!
//! ## Cedar request mapping
//!
//! - principal:  `Symbi::Agent::"claude-code"`
//! - action:     `Symbi::Action::"tool_call::<tool_name>"`
//! - resource:   `Symbi::Resource::"default"`
//! - context:    `{ tool_input: <tool_input as Cedar record> }`
//!
//! Policies can branch on `context.tool_input.command`, etc.
//!
//! ## Exit codes
//!
//! - `0`  — emitted decision (allow/deny). Hook reads stdout to decide.
//! - `2`  — invalid input or invalid policies (operator error).
//! - `3`  — built without `cedar` feature; decision falls back to `allow`
//!   with a clear stderr warning so hooks fail-open rather than blanket-blocking
//!   when the operator forgot to enable the feature.

use clap::ArgMatches;
use serde_json::json;
use std::io::Read;
use std::path::PathBuf;

pub async fn run(matches: &ArgMatches) {
    match matches.subcommand() {
        Some(("evaluate", sub)) => cmd_evaluate(sub),
        _ => {
            eprintln!("Usage: symbi policy evaluate --stdin --policies <DIR>");
            std::process::exit(2);
        }
    }
}

fn cmd_evaluate(matches: &ArgMatches) {
    let read_stdin = matches.get_flag("stdin");
    let input_file = matches.get_one::<String>("input").map(PathBuf::from);
    let policies_dir = matches
        .get_one::<String>("policies")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("./policies"));
    let json_only = matches.get_flag("json");

    if !read_stdin && input_file.is_none() {
        eprintln!("symbi policy evaluate requires --stdin or --input <FILE>");
        std::process::exit(2);
    }

    let input_raw = if read_stdin {
        let mut buf = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
            eprintln!("policy evaluate: cannot read stdin: {}", e);
            std::process::exit(2);
        }
        buf
    } else {
        match std::fs::read_to_string(input_file.as_ref().unwrap()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "policy evaluate: cannot read input file {}: {}",
                    input_file.as_ref().unwrap().display(),
                    e
                );
                std::process::exit(2);
            }
        }
    };

    let event: serde_json::Value = match serde_json::from_str(&input_raw) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("policy evaluate: input is not valid JSON: {}", e);
            std::process::exit(2);
        }
    };

    let tool_name = event
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let tool_input = event.get("tool_input").cloned().unwrap_or(json!({}));

    let decision = evaluate(tool_name, &tool_input, &policies_dir);

    let out = json!({
        "decision":     decision.verdict,
        "reason":       decision.reason,
        "tool":         tool_name,
        "policies_dir": policies_dir.display().to_string(),
    });
    let json_text = serde_json::to_string(&out).unwrap_or_else(|_| "{}".into());

    if json_only {
        // Caller wants structured output for programmatic consumption.
        println!("{}", json_text);
    } else {
        // Default: bare verdict on stdout (shell hooks do `[ "$D" = "deny" ]`),
        // structured detail on stderr (visible in logs, lost when hook redirects 2>/dev/null).
        println!("{}", decision.verdict);
        eprintln!("{}", json_text);
    }
}

struct Decision {
    verdict: &'static str,
    reason: String,
}

#[cfg(feature = "cedar")]
fn evaluate(
    tool_name: &str,
    tool_input: &serde_json::Value,
    policies_dir: &std::path::Path,
) -> Decision {
    use cedar_policy::{
        Authorizer, Context, Decision as CedarDecision, EntityUid, Policy, PolicyId, PolicySet,
        Request,
    };
    use std::str::FromStr;

    if !policies_dir.exists() {
        return Decision {
            verdict: "allow",
            reason: format!(
                "policies directory '{}' does not exist; allowing by default (operator should create policies)",
                policies_dir.display()
            ),
        };
    }

    let mut policy_set = PolicySet::new();
    let mut loaded_count = 0usize;
    let entries = match std::fs::read_dir(policies_dir) {
        Ok(e) => e,
        Err(e) => {
            return Decision {
                verdict: "deny",
                reason: format!(
                    "cannot read policies directory '{}': {}",
                    policies_dir.display(),
                    e
                ),
            };
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("cedar") {
            continue;
        }
        let source = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                return Decision {
                    verdict: "deny",
                    reason: format!("cannot read policy {}: {}", path.display(), e),
                };
            }
        };
        // Cedar can parse multiple policies from one file; assign deterministic
        // ids derived from the filename so a syntax error names the offender.
        let id_base = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("policy");
        match PolicySet::from_str(&source) {
            Ok(set) => {
                for (idx, p) in set.policies().enumerate() {
                    let new_id = PolicyId::from_str(&format!("{}_{}", id_base, idx))
                        .unwrap_or_else(|_| p.id().clone());
                    let renamed = Policy::parse(Some(new_id), p.to_string());
                    if let Ok(pol) = renamed {
                        if let Err(e) = policy_set.add(pol) {
                            return Decision {
                                verdict: "deny",
                                reason: format!("policy add failed for {}: {}", path.display(), e),
                            };
                        }
                        loaded_count += 1;
                    }
                }
            }
            Err(e) => {
                return Decision {
                    verdict: "deny",
                    reason: format!("Cedar parse error in {}: {}", path.display(), e),
                };
            }
        }
    }

    if loaded_count == 0 {
        return Decision {
            verdict: "allow",
            reason: format!(
                "no .cedar policies loaded from '{}'; allowing by default",
                policies_dir.display()
            ),
        };
    }

    let principal = match EntityUid::from_str(r#"Symbi::Agent::"claude-code""#) {
        Ok(u) => u,
        Err(e) => {
            return Decision {
                verdict: "deny",
                reason: format!("internal: failed to build principal: {}", e),
            };
        }
    };
    let action_str = format!(r#"Symbi::Action::"tool_call::{}""#, escape_eid(tool_name));
    let action = match EntityUid::from_str(&action_str) {
        Ok(u) => u,
        Err(e) => {
            return Decision {
                verdict: "deny",
                reason: format!("internal: failed to build action '{}': {}", action_str, e),
            };
        }
    };
    let resource = match EntityUid::from_str(r#"Symbi::Resource::"default""#) {
        Ok(u) => u,
        Err(e) => {
            return Decision {
                verdict: "deny",
                reason: format!("internal: failed to build resource: {}", e),
            };
        }
    };

    let context_value = json!({ "tool_input": tool_input });
    let context = match Context::from_json_value(context_value, None) {
        Ok(c) => c,
        Err(e) => {
            return Decision {
                verdict: "deny",
                reason: format!("internal: failed to build Cedar context: {}", e),
            };
        }
    };

    let request = match Request::new(principal, action, resource, context, None) {
        Ok(r) => r,
        Err(e) => {
            return Decision {
                verdict: "deny",
                reason: format!("internal: failed to build Cedar request: {}", e),
            };
        }
    };

    let entities = cedar_policy::Entities::empty();
    let response = Authorizer::new().is_authorized(&request, &policy_set, &entities);
    let determining: Vec<String> = response
        .diagnostics()
        .reason()
        .map(|p| p.to_string())
        .collect();

    match response.decision() {
        CedarDecision::Allow => Decision {
            verdict: "allow",
            reason: if determining.is_empty() {
                format!(
                    "allowed by default (no policies forbid this; {} loaded)",
                    loaded_count
                )
            } else {
                format!("allow policies matched: {}", determining.join(", "))
            },
        },
        CedarDecision::Deny => Decision {
            verdict: "deny",
            reason: if determining.is_empty() {
                format!(
                    "denied by default-deny ({} policies loaded, none allow this)",
                    loaded_count
                )
            } else {
                format!("deny policies matched: {}", determining.join(", "))
            },
        },
    }
}

#[cfg(not(feature = "cedar"))]
fn evaluate(
    _tool_name: &str,
    _tool_input: &serde_json::Value,
    _policies_dir: &std::path::Path,
) -> Decision {
    eprintln!(
        "warning: symbi was built without the 'cedar' feature; policy evaluation is a no-op (allowing). \
         Rebuild with `cargo build --features cedar` to enforce Cedar policies."
    );
    Decision {
        verdict: "allow",
        reason: "cedar feature disabled at build time; allowing".to_string(),
    }
}

#[cfg(feature = "cedar")]
fn escape_eid(s: &str) -> String {
    // Cedar EID strings live inside double quotes; we escape backslash and quote.
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
