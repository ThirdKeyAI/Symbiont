//! The governed policy-gate ladder shared by every entry point that lets an
//! LLM propose actions: permissive (only when explicitly opted into) ->
//! Cedar (loaded from a policies directory) -> fail-closed default,
//! optionally wrapped in [`EscalationGate`] so flagged actions are held for
//! human approval.
//!
//! This was previously re-implemented at each call site in the binary crate
//! (`up.rs`, `run.rs`, `managed_cli.rs`), with its Cedar half living where no
//! library code could reach it. Consolidating it here lets any consumer of
//! this crate wire the same ladder.
//!
//! Gated on the `cedar` feature this crate already has; without it, the
//! ladder degrades to permissive-or-fail-closed, unchanged from today.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::escalation::{EscalationGate, EscalationGateConfig, EscalationQueue};
use crate::reasoning::policy_bridge::{DefaultPolicyGate, ReasoningPolicyGate};

/// Options controlling how [`governed_gate`] resolves the policy gate.
pub struct GateOptions {
    /// Directory scanned for `*.cedar` policy files.
    pub policies_dir: PathBuf,
    /// Opt into the permissive dev gate (`SYMBI_INSECURE_ALLOW_ALL` /
    /// `--insecure-allow-all`). Callers are responsible for surfacing their
    /// own warning banner before setting this — `governed_gate` does not
    /// print one, since the exact wording differs across call sites.
    pub insecure_allow_all: bool,
    /// When set, wrap the resolved gate so flagged actions are held for
    /// human approval before proceeding.
    pub escalation: Option<(Arc<EscalationQueue>, EscalationGateConfig)>,
}

/// The ladder: permissive (only when explicitly opted into) -> Cedar from
/// `policies_dir` -> fail-closed default. Optionally wrapped in
/// [`EscalationGate`] when `opts.escalation` is set.
pub async fn governed_gate(opts: GateOptions) -> Arc<dyn ReasoningPolicyGate> {
    let gate: Arc<dyn ReasoningPolicyGate> = if opts.insecure_allow_all {
        Arc::new(DefaultPolicyGate::permissive_for_dev_only())
    } else if let Some(cedar_gate) = try_wire_cedar_policy_gate(&opts.policies_dir).await {
        cedar_gate
    } else {
        tracing::info!(
            "policy gate: fail-closed default (no {}/*.cedar found); configure CedarPolicyGate, OpaPolicyGateBridge, or another ReasoningPolicyGate",
            opts.policies_dir.display()
        );
        Arc::new(DefaultPolicyGate::new())
    };

    match opts.escalation {
        Some((queue, config)) => Arc::new(EscalationGate::new(gate, queue, config)),
        None => gate,
    }
}

/// Parse one `*.cedar` policy file into policy entries.
///
/// Such a file is either raw Cedar source or a JSON array of `CedarPolicy`
/// entries — the shape `CedarPolicyGate::reload_policies_from_file` reads, and
/// what `policies/orchestrator.cedar` contains. Both extensions are the same, so
/// sniff the content. Loading a JSON file as if it were Cedar source yields a
/// policy set that cannot parse, and because the gate concatenates active
/// sources at evaluation time, one such file makes every action Deny —
/// including `Respond`, which reaches the operator as a silent, answer-less turn.
///
/// Active entries are validated here rather than at evaluation time. A policy
/// that does not parse cannot be skipped safely (dropping a `forbid` would fail
/// open), so this returns `Err` and the caller declines to wire the gate.
#[cfg(feature = "cedar")]
fn parse_policy_file(
    file_stem: &str,
    contents: String,
) -> Result<Vec<crate::reasoning::CedarPolicy>, String> {
    use crate::reasoning::CedarPolicy;

    let entries: Vec<CedarPolicy> = match serde_json::from_str::<Vec<CedarPolicy>>(&contents) {
        Ok(list) => list,
        Err(_) => vec![CedarPolicy {
            name: file_stem.to_string(),
            source: contents,
            active: true,
        }],
    };

    for policy in &entries {
        if !policy.active {
            continue;
        }
        if let Err(e) = policy.source.parse::<cedar_policy::PolicySet>() {
            return Err(format!(
                "policy '{}' is not valid Cedar: {}",
                policy.name, e
            ));
        }
    }
    Ok(entries)
}

/// If the `cedar` feature is compiled in AND `policies_dir` contains at least
/// one `*.cedar` file, construct a [`CedarPolicyGate`] preloaded with each
/// file as a named policy. Files that fail to parse as Cedar syntax are
/// logged and skipped (the gate continues to load the rest); a gate is
/// returned only when at least one policy parses successfully.
///
/// Returns `None` if the `cedar` feature is disabled, `policies_dir` doesn't
/// exist, no `*.cedar` files are present, or every file failed to parse.
/// Callers should fall back to `DefaultPolicyGate::new()` (fail-closed) in
/// that case.
///
/// [`CedarPolicyGate`]: crate::reasoning::CedarPolicyGate
#[cfg(feature = "cedar")]
async fn try_wire_cedar_policy_gate(policies_dir: &Path) -> Option<Arc<dyn ReasoningPolicyGate>> {
    use crate::reasoning::CedarPolicyGate;

    if !policies_dir.is_dir() {
        return None;
    }

    let mut cedar_files: Vec<PathBuf> = match std::fs::read_dir(policies_dir) {
        Ok(rd) => rd
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("cedar"))
            .collect(),
        Err(e) => {
            tracing::warn!("unable to read policies directory: {}", e);
            return None;
        }
    };
    if cedar_files.is_empty() {
        return None;
    }
    cedar_files.sort();

    let gate = CedarPolicyGate::deny_by_default();
    let mut loaded = 0usize;
    for path in cedar_files {
        let source = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("failed to read {}: {}", path.display(), e);
                continue;
            }
        };
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("policy")
            .to_string();

        let entries = match parse_policy_file(&name, source) {
            Ok(entries) => entries,
            Err(e) => {
                tracing::error!(
                    "{} in {}. Refusing to wire the Cedar gate; falling through to the \
                     fail-closed default. Fix or deactivate this policy.",
                    e,
                    path.display()
                );
                return None;
            }
        };

        for policy in entries {
            gate.add_policy(policy).await;
            loaded += 1;
        }
    }
    if loaded == 0 {
        tracing::warn!(
            "found .cedar files under {} but none parsed successfully — falling through to fail-closed default",
            policies_dir.display()
        );
        return None;
    }
    tracing::info!(
        "policy gate: CedarPolicyGate auto-wired from {} policy file(s) under {}",
        loaded,
        policies_dir.display()
    );
    println!(
        "✓ Cedar policy gate wired ({} policy file(s) loaded)",
        loaded
    );
    Some(Arc::new(gate))
}

/// Stub used when the `cedar` feature is disabled. Always returns `None` so
/// the caller falls through to the fail-closed default.
#[cfg(not(feature = "cedar"))]
async fn try_wire_cedar_policy_gate(_policies_dir: &Path) -> Option<Arc<dyn ReasoningPolicyGate>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reasoning::conversation::Conversation;
    use crate::reasoning::loop_types::{LoopDecision, LoopState, ProposedAction};
    use crate::types::AgentId;

    fn tool_call() -> ProposedAction {
        ProposedAction::ToolCall {
            call_id: "c1".into(),
            name: "search".into(),
            arguments: "{}".into(),
        }
    }

    #[tokio::test]
    async fn fail_closed_when_policies_dir_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        let policies_dir = dir.path().join("does-not-exist");

        let gate = governed_gate(GateOptions {
            policies_dir,
            insecure_allow_all: false,
            escalation: None,
        })
        .await;

        let agent_id = AgentId::new();
        let state = LoopState::new(agent_id, Conversation::new());
        let decision = gate.evaluate_action(&agent_id, &tool_call(), &state).await;
        assert!(
            matches!(decision, LoopDecision::Deny { .. }),
            "expected fail-closed deny, got {decision:?}"
        );
    }

    #[cfg(feature = "cedar")]
    #[tokio::test]
    async fn cedar_gate_wired_when_a_valid_policy_file_is_present() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("allow_search.cedar"),
            r#"permit(principal, action == Action::"tool_call::search", resource);"#,
        )
        .unwrap();

        let gate = governed_gate(GateOptions {
            policies_dir: dir.path().to_path_buf(),
            insecure_allow_all: false,
            escalation: None,
        })
        .await;

        let agent_id = AgentId::new();
        let state = LoopState::new(agent_id, Conversation::new());
        let decision = gate.evaluate_action(&agent_id, &tool_call(), &state).await;
        assert!(
            matches!(decision, LoopDecision::Allow),
            "expected the Cedar policy to allow 'search', got {decision:?}"
        );

        // A tool the policy set doesn't mention stays denied: this is a
        // wired Cedar gate, not permissive mode.
        let other = ProposedAction::ToolCall {
            call_id: "c2".into(),
            name: "delete_everything".into(),
            arguments: "{}".into(),
        };
        let decision = gate.evaluate_action(&agent_id, &other, &state).await;
        assert!(matches!(decision, LoopDecision::Deny { .. }));
    }

    #[tokio::test]
    async fn permissive_only_when_insecure_allow_all_is_set() {
        let dir = tempfile::tempdir().unwrap();
        let policies_dir = dir.path().join("does-not-exist");

        // Not reachable without the flag: same absent-policies-dir setup as
        // the fail-closed test above denies the tool call.
        let gate = governed_gate(GateOptions {
            policies_dir: policies_dir.clone(),
            insecure_allow_all: false,
            escalation: None,
        })
        .await;
        let agent_id = AgentId::new();
        let state = LoopState::new(agent_id, Conversation::new());
        let decision = gate.evaluate_action(&agent_id, &tool_call(), &state).await;
        assert!(matches!(decision, LoopDecision::Deny { .. }));

        // With the flag, the same setup allows.
        let gate = governed_gate(GateOptions {
            policies_dir,
            insecure_allow_all: true,
            escalation: None,
        })
        .await;
        let decision = gate.evaluate_action(&agent_id, &tool_call(), &state).await;
        assert!(matches!(decision, LoopDecision::Allow));
    }

    #[tokio::test]
    async fn escalation_wrapping_holds_configured_tools() {
        let dir = tempfile::tempdir().unwrap();
        let policies_dir = dir.path().join("does-not-exist");
        let queue = Arc::new(EscalationQueue::new());

        let gate = governed_gate(GateOptions {
            policies_dir,
            insecure_allow_all: true,
            escalation: Some((
                queue.clone(),
                EscalationGateConfig {
                    require_approval_tools: vec!["search".to_string()],
                    timeout: std::time::Duration::from_millis(50),
                },
            )),
        })
        .await;

        let agent_id = AgentId::new();
        let state = LoopState::new(agent_id, Conversation::new());
        // The inner gate is permissive, but the wrapping should still hold
        // "search" for approval and time out to Deny rather than Allow.
        let decision = gate.evaluate_action(&agent_id, &tool_call(), &state).await;
        assert!(
            matches!(decision, LoopDecision::Deny { .. }),
            "expected the held action to time out to Deny, got {decision:?}"
        );
    }
}

#[cfg(all(test, feature = "cedar"))]
mod cedar_policy_file_tests {
    use super::parse_policy_file;

    /// `policies/orchestrator.cedar` — the file this repo ships — is a JSON array,
    /// not raw Cedar. Read as raw source it produces an unparseable policy set,
    /// and because the gate concatenates active sources, every action (including
    /// `Respond`) is denied.
    #[test]
    fn json_array_policy_file_is_parsed_as_entries() {
        let contents = r#"[
          {
            "name": "orchestrator_safe_tools",
            "active": true,
            "source": "permit(principal, action == Action::\"tool_call::list_agents\", resource);"
          },
          {
            "name": "disabled_rule",
            "active": false,
            "source": "this is not cedar at all"
          }
        ]"#;
        let entries = parse_policy_file("orchestrator", contents.to_string())
            .expect("a JSON policy file must load as entries");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "orchestrator_safe_tools");
        assert!(entries[0].source.contains("permit("));
        // An inactive entry is not validated, so a broken one doesn't block startup.
        assert!(!entries[1].active);
    }

    #[test]
    fn raw_cedar_policy_file_is_kept_as_source() {
        let contents = "permit(principal, action, resource);";
        let entries =
            parse_policy_file("default", contents.to_string()).expect("raw Cedar must still load");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "default");
        assert_eq!(entries[0].source, contents);
        assert!(entries[0].active);
    }

    #[test]
    fn invalid_cedar_is_rejected_up_front() {
        // Reported to the operator, not deferred to evaluation time where it
        // would silently deny everything.
        let err = parse_policy_file("broken", "not cedar".to_string())
            .expect_err("invalid Cedar must be rejected");
        assert!(
            err.contains("broken"),
            "error should name the policy: {err}"
        );
        assert!(err.contains("not valid Cedar"), "got: {err}");
    }

    /// Guards the real file, so a format change to it cannot silently reintroduce
    /// the deny-everything behavior. `CARGO_MANIFEST_DIR` here is this crate's
    /// directory (`crates/runtime`), two levels below the workspace root where
    /// `policies/` actually lives.
    #[test]
    fn the_shipped_orchestrator_policy_loads() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../policies/orchestrator.cedar");
        if !path.exists() {
            return; // not all checkouts ship it
        }
        let contents = std::fs::read_to_string(&path).expect("read shipped policy");
        let entries = parse_policy_file("orchestrator", contents)
            .expect("the shipped policies/orchestrator.cedar must load and parse as Cedar");
        assert!(!entries.is_empty());
    }
}
