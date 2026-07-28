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
    /// Directory scanned for `*.cedar` policy files. Files sitting directly in
    /// it are shared: they apply to every surface.
    pub policies_dir: PathBuf,
    /// Name of the surface this gate governs — the subdirectory of
    /// `policies_dir` layered on top of the shared files, i.e.
    /// `<policies_dir>/<surface>/*.cedar`.
    ///
    /// Surfaces exist because one process can hand model output to places with
    /// very different blast radii (a chat coordinator that only emits text vs.
    /// an HTTP endpoint that dispatches tool calls). A grant written into
    /// `<policies_dir>/<surface>/` reaches only that surface, so it cannot
    /// become a latent permission somewhere nobody intended.
    ///
    /// `None` loads the shared files only. The subdirectory is purely
    /// additive: flat files keep applying everywhere, so existing deployments
    /// are unaffected until they opt in by creating one.
    pub surface: Option<String>,
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
    } else if let Some(cedar_gate) =
        try_wire_cedar_policy_gate(&opts.policies_dir, opts.surface.as_deref()).await
    {
        cedar_gate
    } else {
        // Name both directories that were searched. With surface scoping a
        // policy in the wrong one looks identical to no policy at all, and the
        // surface name is not something a caller can guess.
        match opts.surface.as_deref() {
            Some(surface) => tracing::info!(
                "policy gate: fail-closed default (no *.cedar in {} or {}/{}); \
                 put policies for this surface in {}/{}/",
                opts.policies_dir.display(),
                opts.policies_dir.display(),
                surface,
                opts.policies_dir.display(),
                surface
            ),
            None => tracing::info!(
                "policy gate: fail-closed default (no {}/*.cedar found); configure CedarPolicyGate, OpaPolicyGateBridge, or another ReasoningPolicyGate",
                opts.policies_dir.display()
            ),
        }
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
/// what `policies/shell/orchestrator.cedar` contains. Both extensions are the same, so
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

/// The `*.cedar` files sitting directly in `dir`, sorted. A directory that
/// does not exist yields none — an absent policy directory is not an error,
/// the caller simply falls through to fail-closed.
#[cfg(feature = "cedar")]
fn cedar_files_in(dir: &Path) -> Vec<PathBuf> {
    if !dir.is_dir() {
        return Vec::new();
    }
    let mut files: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|p| p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("cedar"))
            .collect(),
        Err(e) => {
            tracing::warn!("unable to read policy directory {}: {}", dir.display(), e);
            return Vec::new();
        }
    };
    files.sort();
    files
}

/// Resolve `<policies_dir>/<surface>`. A surface names a subdirectory, never a
/// path, so anything that could reach outside `policies_dir` is refused rather
/// than joined.
#[cfg(feature = "cedar")]
fn surface_dir(policies_dir: &Path, surface: &str) -> Option<PathBuf> {
    let sane = !surface.is_empty()
        && surface != "."
        && surface != ".."
        && !surface.contains('/')
        && !surface.contains('\\');
    if !sane {
        tracing::warn!(
            "ignoring policy surface {:?}: not a plain directory name",
            surface
        );
        return None;
    }
    Some(policies_dir.join(surface))
}

/// If the `cedar` feature is compiled in AND at least one `*.cedar` file is
/// found, construct a [`CedarPolicyGate`] preloaded with each file as a named
/// policy.
///
/// Two layers are loaded into that one gate:
/// 1. shared — `<policies_dir>/*.cedar`, which apply to every surface;
/// 2. surface-specific — `<policies_dir>/<surface>/*.cedar`, only when
///    `surface` is `Some`.
///
/// The second layer is what keeps a grant written for one surface out of
/// another's gate: a file under `policies/http-input/` is invisible to the
/// coordinator, and vice versa.
///
/// Returns `None` if the `cedar` feature is disabled, neither directory
/// exists, no `*.cedar` files are present, or a policy failed to parse.
/// Callers should fall back to `DefaultPolicyGate::new()` (fail-closed) in
/// that case.
///
/// [`CedarPolicyGate`]: crate::reasoning::CedarPolicyGate
#[cfg(feature = "cedar")]
async fn try_wire_cedar_policy_gate(
    policies_dir: &Path,
    surface: Option<&str>,
) -> Option<Arc<dyn ReasoningPolicyGate>> {
    use crate::reasoning::CedarPolicyGate;

    let mut cedar_files = cedar_files_in(policies_dir);
    if let Some(dir) = surface.and_then(|s| surface_dir(policies_dir, s)) {
        cedar_files.extend(cedar_files_in(&dir));
    }
    if cedar_files.is_empty() {
        return None;
    }

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
        "policy gate: CedarPolicyGate auto-wired from {} policy file(s) under {} (surface: {})",
        loaded,
        policies_dir.display(),
        surface.unwrap_or("<shared only>")
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
async fn try_wire_cedar_policy_gate(
    _policies_dir: &Path,
    _surface: Option<&str>,
) -> Option<Arc<dyn ReasoningPolicyGate>> {
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
            surface: None,
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
            surface: None,
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

    #[cfg(feature = "cedar")]
    fn named_tool_call(name: &str) -> ProposedAction {
        ProposedAction::ToolCall {
            call_id: "c1".into(),
            name: name.into(),
            arguments: "{}".into(),
        }
    }

    /// Lay out a policies dir with one shared policy and one policy per
    /// surface, so each test only has to say which surface it asks for.
    #[cfg(feature = "cedar")]
    fn policies_fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("shared.cedar"),
            r#"permit(principal, action == Action::"tool_call::search", resource);"#,
        )
        .unwrap();

        std::fs::create_dir_all(dir.path().join("coordinator")).unwrap();
        std::fs::write(
            dir.path().join("coordinator/chat.cedar"),
            r#"permit(principal, action == Action::"tool_call::summarize", resource);"#,
        )
        .unwrap();

        std::fs::create_dir_all(dir.path().join("http-input")).unwrap();
        std::fs::write(
            dir.path().join("http-input/tools.cedar"),
            r#"permit(principal, action == Action::"tool_call::edit_file", resource);"#,
        )
        .unwrap();
        dir
    }

    #[cfg(feature = "cedar")]
    #[tokio::test]
    async fn surface_policies_layer_on_top_of_shared_ones() {
        let dir = policies_fixture();
        let gate = governed_gate(GateOptions {
            policies_dir: dir.path().to_path_buf(),
            surface: Some("coordinator".to_string()),
            insecure_allow_all: false,
            escalation: None,
        })
        .await;

        let agent_id = AgentId::new();
        let state = LoopState::new(agent_id, Conversation::new());

        let shared = gate
            .evaluate_action(&agent_id, &named_tool_call("search"), &state)
            .await;
        assert!(
            matches!(shared, LoopDecision::Allow),
            "the shared policy must still apply, got {shared:?}"
        );

        let scoped = gate
            .evaluate_action(&agent_id, &named_tool_call("summarize"), &state)
            .await;
        assert!(
            matches!(scoped, LoopDecision::Allow),
            "policies/coordinator/*.cedar must be layered in, got {scoped:?}"
        );
    }

    /// The isolation property this layering exists for: a grant written for
    /// one surface must not reach another surface's gate.
    #[cfg(feature = "cedar")]
    #[tokio::test]
    async fn another_surfaces_policies_are_not_loaded() {
        let dir = policies_fixture();
        let gate = governed_gate(GateOptions {
            policies_dir: dir.path().to_path_buf(),
            surface: Some("coordinator".to_string()),
            insecure_allow_all: false,
            escalation: None,
        })
        .await;

        let agent_id = AgentId::new();
        let state = LoopState::new(agent_id, Conversation::new());
        let decision = gate
            .evaluate_action(&agent_id, &named_tool_call("edit_file"), &state)
            .await;
        assert!(
            matches!(decision, LoopDecision::Deny { .. }),
            "policies/http-input/*.cedar must not reach the coordinator gate, got {decision:?}"
        );

        // ...and the same file does grant the tool on its own surface, so the
        // deny above is scoping and not a policy that simply never loaded.
        let http_gate = governed_gate(GateOptions {
            policies_dir: dir.path().to_path_buf(),
            surface: Some("http-input".to_string()),
            insecure_allow_all: false,
            escalation: None,
        })
        .await;
        let decision = http_gate
            .evaluate_action(&agent_id, &named_tool_call("edit_file"), &state)
            .await;
        assert!(matches!(decision, LoopDecision::Allow), "got {decision:?}");
    }

    #[cfg(feature = "cedar")]
    #[tokio::test]
    async fn no_surface_loads_shared_policies_only() {
        let dir = policies_fixture();
        let gate = governed_gate(GateOptions {
            policies_dir: dir.path().to_path_buf(),
            surface: None,
            insecure_allow_all: false,
            escalation: None,
        })
        .await;

        let agent_id = AgentId::new();
        let state = LoopState::new(agent_id, Conversation::new());
        let shared = gate
            .evaluate_action(&agent_id, &named_tool_call("search"), &state)
            .await;
        assert!(matches!(shared, LoopDecision::Allow), "got {shared:?}");

        for scoped in ["summarize", "edit_file"] {
            let decision = gate
                .evaluate_action(&agent_id, &named_tool_call(scoped), &state)
                .await;
            assert!(
                matches!(decision, LoopDecision::Deny { .. }),
                "surface: None must load no subdirectory, but {scoped} was {decision:?}"
            );
        }
    }

    #[tokio::test]
    async fn permissive_only_when_insecure_allow_all_is_set() {
        let dir = tempfile::tempdir().unwrap();
        let policies_dir = dir.path().join("does-not-exist");

        // Not reachable without the flag: same absent-policies-dir setup as
        // the fail-closed test above denies the tool call.
        let gate = governed_gate(GateOptions {
            policies_dir: policies_dir.clone(),
            surface: None,
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
            surface: None,
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
            surface: None,
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

    /// `policies/shell/orchestrator.cedar` — the file this repo ships — is a JSON array,
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
            .join("../../policies/shell/orchestrator.cedar");
        if !path.exists() {
            return; // not all checkouts ship it
        }
        let contents = std::fs::read_to_string(&path).expect("read shipped policy");
        let entries = parse_policy_file("orchestrator", contents)
            .expect("the shipped policies/shell/orchestrator.cedar must load and parse as Cedar");
        assert!(!entries.is_empty());
    }
}
