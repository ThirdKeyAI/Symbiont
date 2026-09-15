//! Discovery + parsing of agent definitions into a unified `AgentSpec`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Where an ingested agent came from — a TOML manifest or a .symbi DSL file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSource {
    Manifest(PathBuf),
    Symbi(PathBuf),
}

/// The manifest's projected shape — what the orchestrator can delegate to.
#[derive(Debug, Clone)]
pub struct AgentSpec {
    pub name: String,
    pub description: String,
    pub system_prompt: String,
    pub tools: Vec<String>,
    // Retained for routing and future DSL-agent execution phases.
    #[allow(dead_code)]
    pub source: AgentSource,
    pub definition: Option<dsl::ConversationalAgent>,
}

/// A synchronous summary of a loaded agent, used to build the `delegate` tool's
/// description without touching the async registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCard {
    pub name: String,
    pub description: String,
    /// Tool names the agent's manifest grants. Empty = conversational-only.
    pub tools: Vec<String>,
}

/// A single file that failed to load. One bad file never aborts the rest.
#[derive(Debug, Clone)]
pub struct LoadError {
    pub path: PathBuf,
    pub message: String,
}

#[derive(serde::Deserialize)]
struct Manifest {
    name: String,
    description: String,
    system_prompt: String,
    #[serde(default)]
    tools: Vec<String>,
}

/// Parse a single `*.toml` manifest into an `AgentSpec`.
pub fn parse_manifest(path: &Path) -> Result<AgentSpec, LoadError> {
    let err = |m: String| LoadError {
        path: path.to_path_buf(),
        message: m,
    };
    let text = std::fs::read_to_string(path).map_err(|e| err(e.to_string()))?;
    let m: Manifest = toml::from_str(&text).map_err(|e| err(e.to_string()))?;
    if m.name.trim().is_empty() {
        return Err(err("manifest 'name' is empty".into()));
    }
    if m.system_prompt.trim().is_empty() {
        return Err(err("manifest 'system_prompt' is empty".into()));
    }
    Ok(AgentSpec {
        name: m.name,
        description: m.description,
        system_prompt: m.system_prompt,
        tools: m.tools,
        source: AgentSource::Manifest(path.to_path_buf()),
        definition: None,
    })
}

/// Scan a directory for agent definitions: `*.toml` manifests and `*.symbi` /
/// `*.dsl` DSL agents. Returns successfully-parsed specs, a per-file error for
/// each that failed, and the names of agents with unsupported execution requirements.
/// A missing directory yields empty results.
pub fn scan_dir(dir: &Path) -> (Vec<AgentSpec>, Vec<LoadError>, Vec<String>) {
    let mut specs = Vec::new();
    let mut errors = Vec::new();
    let mut refused = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return (specs, errors, refused),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        match path.extension().and_then(|e| e.to_str()) {
            Some("toml") => match parse_manifest(&path) {
                Ok(s) => specs.push(s),
                Err(e) => errors.push(e),
            },
            Some("symbi") | Some("dsl") => match crate::agents::symbi::parse_symbi(&path) {
                Ok(loads) => {
                    for load in loads {
                        match load {
                            crate::agents::symbi::SymbiLoad::Loaded(s) => specs.push(*s),
                            crate::agents::symbi::SymbiLoad::Refused { name, reason } => {
                                errors.push(LoadError {
                                    path: path.clone(),
                                    message: format!("{name}: {reason}"),
                                });
                                refused.push(name)
                            }
                        }
                    }
                }
                Err(e) => errors.push(e),
            },
            _ => {}
        }
    }
    (specs, errors, refused)
}

/// Outcome of a load pass, for surfacing to the user.
#[derive(Debug, Clone, Default)]
pub struct LoadReport {
    pub loaded: usize,
    pub errors: Vec<LoadError>,
    pub collisions: Vec<String>,
    /// Names of `.symbi` agents refused because conversational execution cannot enforce their declared requirements.
    pub sandbox_refused: Vec<String>,
}

/// Appended to every fleet agent's system prompt. Fleet agents are conversational
/// only — they run against the inference provider with no executor, so any tools
/// named in their manifest are NOT wired for execution yet. Without this, an agent
/// prompted like a scanner will happily *describe* running a command and then
/// fabricate its output (e.g. invented vulnerability findings), which is
/// dangerously misleading for a security tool.
const FLEET_AGENT_CONSTRAINT: &str = "\n\n\
## Execution constraints (read carefully)\n\
You are a conversational fleet agent inside symbi-shell. You CANNOT execute \
commands, run scripts or tools, make network requests, or read/write files — you \
have no shell and no code interpreter. Any tools named in your configuration are \
NOT yet wired for execution.\n\n\
Therefore you MUST NOT:\n\
- claim to have run a command, tool, or scan;\n\
- print or paraphrase command output you did not actually receive;\n\
- invent results, scan findings, file contents, or any data.\n\n\
If a request needs something you cannot run, say so plainly and tell the user \
what to run themselves (or to route it through the orchestrator's governed \
tools). Only report information that is actually present in this conversation.";

/// Augment a manifest's system prompt with the fleet-agent execution constraint.
fn constrained_system_prompt(base: &str) -> String {
    format!("{}{}", base.trim_end(), FLEET_AGENT_CONSTRAINT)
}

/// Addendum for fleet agents that DO have a tool grant. Unlike the tool-less
/// constraint, this tells the agent it may call its governed tools — but must
/// still never fabricate results it did not receive from a tool.
fn tool_aware_system_prompt(base: &str, tools: &[String]) -> String {
    if tools.is_empty() {
        return constrained_system_prompt(base);
    }
    format!(
        "{}\n\n\
## Tool use\n\
You may call these governed tools: {}. Each call is checked by the policy gate \
and some (file edits, shell) require human approval; results come back to you as \
observations. You have no other capabilities. Never claim to have run a tool you \
did not call, and never invent tool output or results — only report what a tool \
actually returned.",
        base.trim_end(),
        tools.join(", "),
    )
}

/// Scan `dir`, register unambiguous specs into the bridge's registry, rebuild the synchronous `cards` mirror, and collect sandbox
/// refusals. Per-file parse errors are collected, never fatal.
pub async fn load_agents_into(
    dir: &Path,
    bridge: &Arc<repl_core::RuntimeBridge>,
    cards: &Arc<RwLock<Vec<AgentCard>>>,
) -> LoadReport {
    let (specs, errors, sandbox_refused) = scan_dir(dir);

    // Ambiguous names grant no authority. Do not select by directory order.
    let mut by_name: std::collections::HashMap<String, AgentSpec> =
        std::collections::HashMap::new();
    let mut collisions = Vec::new();
    for spec in specs {
        if by_name.insert(spec.name.clone(), spec.clone()).is_some()
            && !collisions.contains(&spec.name)
        {
            collisions.push(spec.name.clone());
        }
    }

    for name in &collisions {
        by_name.remove(name);
        bridge.agent_registry().remove_agent(name).await;
    }
    for name in &sandbox_refused {
        by_name.remove(name);
        bridge.agent_registry().remove_agent(name).await;
    }

    let previous: Vec<_> = cards
        .read()
        .await
        .iter()
        .map(|card| card.name.clone())
        .collect();
    for name in previous {
        if !by_name.contains_key(&name) {
            bridge.agent_registry().remove_agent(&name).await;
        }
    }
    let mut new_cards = Vec::new();
    for spec in by_name.values() {
        if let Some(definition) = &spec.definition {
            bridge
                .agent_registry()
                .register_canonical(
                    definition.clone(),
                    tool_aware_system_prompt(&spec.system_prompt, &spec.tools),
                    spec.tools.clone(),
                )
                .await;
        } else {
            bridge
                .register_agent(
                    &spec.name,
                    &tool_aware_system_prompt(&spec.system_prompt, &spec.tools),
                    spec.tools.clone(),
                )
                .await;
        }
        new_cards.push(AgentCard {
            name: spec.name.clone(),
            description: spec.description.clone(),
            tools: spec.tools.clone(),
        });
    }
    new_cards.sort_by(|a, b| a.name.cmp(&b.name));
    let loaded = new_cards.len();
    *cards.write().await = new_cards;

    LoadReport {
        loaded,
        errors,
        collisions,
        sandbox_refused,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        p
    }

    #[test]
    fn parses_valid_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(
            dir.path(),
            "researcher.toml",
            "name = \"researcher\"\ndescription = \"finds sources\"\nsystem_prompt = \"You research.\"\ntools = [\"search\"]\n",
        );
        let spec = parse_manifest(&p).unwrap();
        assert_eq!(spec.name, "researcher");
        assert_eq!(spec.description, "finds sources");
        assert_eq!(spec.system_prompt, "You research.");
        assert_eq!(spec.tools, vec!["search".to_string()]);
        assert_eq!(spec.source, AgentSource::Manifest(p));
    }

    #[test]
    fn missing_required_field_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "bad.toml", "name = \"x\"\n");
        assert!(parse_manifest(&p).is_err());
    }

    #[test]
    fn malformed_toml_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "bad.toml", "this is = = not toml");
        assert!(parse_manifest(&p).is_err());
    }

    #[test]
    fn scan_dir_loads_manifests_and_symbi_isolates_bad() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "a.toml",
            "name=\"a\"\ndescription=\"d\"\nsystem_prompt=\"p\"\n",
        );
        write(dir.path(), "broken.toml", "= = =");
        // a .symbi file using full grammar is now parsed and loaded
        write(
            dir.path(),
            "b.symbi",
            "metadata { description = \"d\" }\nagent b {\n  capabilities = [\"read\"]\n}\n",
        );
        let (specs, errors, _refused) = scan_dir(dir.path());
        let names: Vec<_> = specs.iter().map(|s| s.name.clone()).collect();
        assert!(names.contains(&"a".to_string()), "manifest loads");
        assert!(
            names.contains(&"b".to_string()),
            ".symbi agent is registered"
        );
        assert_eq!(errors.len(), 1, "the broken manifest is the only error");
    }

    #[test]
    fn scan_dir_preserves_selected_tier_symbi() {
        let dir = tempfile::tempdir().unwrap();
        // Declared tier remains bound to the canonical contract.
        write(
            dir.path(),
            "risky.symbi",
            "agent Risky {\n  capabilities = [\"read\"]\n  with sandbox = \"Tier2\" {\n  }\n}\n",
        );
        let (specs, _errors, refused) = scan_dir(dir.path());
        assert_eq!(specs.len(), 1);
        assert!(refused.is_empty());
        assert_eq!(
            specs[0]
                .definition
                .as_ref()
                .unwrap()
                .settings()
                .sandbox_tier,
            Some(dsl::SandboxTier::GVisor)
        );
    }

    #[tokio::test]
    async fn load_registers_agents_rebuilds_cards_and_dedupes() {
        use std::sync::Arc;
        use tokio::sync::RwLock;
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "a.toml",
            "name=\"a\"\ndescription=\"first\"\nsystem_prompt=\"p\"\n",
        );
        write(
            dir.path(),
            "b.toml",
            "name=\"b\"\ndescription=\"second\"\nsystem_prompt=\"p\"\n",
        );
        // A second declaration of a makes that name unavailable.
        write(
            dir.path(),
            "a2.toml",
            "name=\"a\"\ndescription=\"override\"\nsystem_prompt=\"p\"\n",
        );
        // a .symbi file using full grammar is now loaded (not deferred)
        write(
            dir.path(),
            "z.symbi",
            "metadata { description = \"d\" }\nagent z {\n  capabilities = [\"read\"]\n}\n",
        );

        let bridge = Arc::new(repl_core::RuntimeBridge::new_permissive_for_dev());
        let cards = Arc::new(RwLock::new(Vec::<AgentCard>::new()));
        let report = load_agents_into(dir.path(), &bridge, &cards).await;

        assert_eq!(report.loaded, 2, "b + z; ambiguous a is refused");
        assert_eq!(report.collisions, vec!["a".to_string()]);
        assert!(report.sandbox_refused.is_empty());
        let names: Vec<_> = cards.read().await.iter().map(|c| c.name.clone()).collect();
        assert!(!names.contains(&"a".to_string()) && names.contains(&"b".to_string()));
        assert!(bridge.agent_registry().get_agent("a").await.is_none());
        assert!(
            names.contains(&"z".to_string()),
            ".symbi agent is now registered"
        );
    }

    #[test]
    fn constraint_appended_and_forbids_fabrication() {
        let out = constrained_system_prompt("You are a vulnerability scanner.");
        // base prompt preserved
        assert!(out.starts_with("You are a vulnerability scanner."));
        // the no-fabrication guardrails are present
        assert!(out.contains("CANNOT execute"));
        assert!(out.contains("MUST NOT"));
        assert!(out.contains("invent results"));
    }

    #[tokio::test]
    async fn registered_agent_prompt_carries_constraint() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "scanner.toml",
            "name=\"scanner_agent\"\ndescription=\"scanner\"\nsystem_prompt=\"You scan for issues.\"\n",
        );
        let bridge = Arc::new(repl_core::RuntimeBridge::new_permissive_for_dev());
        let cards = Arc::new(RwLock::new(Vec::<AgentCard>::new()));
        load_agents_into(dir.path(), &bridge, &cards).await;

        let sys = bridge.agent_system_prompt("scanner_agent").await.unwrap();
        assert!(sys.contains("You scan for issues."));
        assert!(sys.contains("CANNOT execute"));
    }

    #[test]
    fn tool_aware_prompt_tool_less_keeps_no_fab() {
        let out = tool_aware_system_prompt("You scan.", &[]);
        assert!(out.contains("CANNOT execute")); // PR #74 constraint
    }

    #[test]
    fn tool_aware_prompt_with_tools_lists_them_and_forbids_fabrication() {
        let out = tool_aware_system_prompt(
            "You review code.",
            &["read_file".to_string(), "search".to_string()],
        );
        assert!(out.starts_with("You review code."));
        assert!(out.contains("read_file, search"));
        assert!(out.contains("never invent tool output"));
        assert!(!out.contains("CANNOT execute")); // not the tool-less constraint
    }

    #[tokio::test]
    async fn card_carries_manifest_tools() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "r.toml",
            "name=\"reviewer\"\ndescription=\"d\"\nsystem_prompt=\"p\"\ntools=[\"read_file\",\"search\"]\n",
        );
        let bridge = Arc::new(repl_core::RuntimeBridge::new_permissive_for_dev());
        let cards = Arc::new(RwLock::new(Vec::<AgentCard>::new()));
        load_agents_into(dir.path(), &bridge, &cards).await;
        let c = cards.read().await;
        let r = c.iter().find(|c| c.name == "reviewer").unwrap();
        assert_eq!(r.tools, vec!["read_file".to_string(), "search".to_string()]);
    }

    #[test]
    fn scan_missing_dir_is_empty_not_error() {
        let (specs, errors, refused) = scan_dir(std::path::Path::new("/no/such/dir/here"));
        assert!(specs.is_empty());
        assert!(errors.is_empty());
        assert!(refused.is_empty());
    }

    /// Syntactically broken .symbi must return Err (covers the has_error() branch).
    #[test]
    fn malformed_symbi_is_parse_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "broken.symbi", "agent { oops");
        let result = crate::agents::symbi::parse_symbi(&p);
        assert!(result.is_err(), "malformed .symbi must return Err");
    }

    /// Every committed agents/*.symbi must load-or-refuse — never parse-fail.
    #[test]
    fn real_committed_symbi_agents_all_parse() {
        let agents_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../agents");
        if !agents_dir.is_dir() {
            return; // not in a full checkout; skip rather than fail
        }
        let mut checked = 0;
        for entry in std::fs::read_dir(&agents_dir).unwrap().flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("symbi") {
                checked += 1;
                let result = crate::agents::symbi::parse_symbi(&path);
                assert!(
                    result.is_ok(),
                    "real agent {:?} failed to parse: {:?}",
                    path.file_name().unwrap(),
                    result.err()
                );
            }
        }
        assert!(checked > 0, "expected to find committed .symbi agents");
    }

    #[tokio::test]
    async fn loads_symbi_agent_into_fleet() {
        let dir = tempfile::tempdir().unwrap();
        // Full grammar: metadata block + agent block with capabilities array.
        write(
            dir.path(),
            "reader.symbi",
            "metadata { description = \"reads\" }\nagent Reader {\n  capabilities = [\"read\"]\n}\n",
        );
        let bridge = Arc::new(repl_core::RuntimeBridge::new_permissive_for_dev());
        let cards = Arc::new(RwLock::new(Vec::<AgentCard>::new()));
        let report = load_agents_into(dir.path(), &bridge, &cards).await;
        assert_eq!(report.loaded, 1);
        let c = cards.read().await;
        let r = c.iter().find(|c| c.name == "Reader").unwrap();
        assert_eq!(r.tools, vec!["read_file".to_string(), "search".to_string()]);
    }

    #[tokio::test]
    async fn symbi_and_manifest_collision_grants_no_authority() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "a.toml",
            "name=\"dup\"\ndescription=\"toml\"\nsystem_prompt=\"p\"\n",
        );
        // Full grammar: simple agent block with capabilities array.
        write(
            dir.path(),
            "a.symbi",
            "agent dup {\n  capabilities = [\"read\"]\n}\n",
        );
        let bridge = Arc::new(repl_core::RuntimeBridge::new_permissive_for_dev());
        let cards = Arc::new(RwLock::new(Vec::<AgentCard>::new()));
        let report = load_agents_into(dir.path(), &bridge, &cards).await;
        assert_eq!(report.loaded, 0, "ambiguous name is unavailable");
        assert_eq!(report.collisions, vec!["dup".to_string()]);
        assert!(bridge.agent_registry().get_agent("dup").await.is_none());
    }

    #[tokio::test]
    async fn reload_removes_contracts_with_missing_or_unsupported_source() {
        let dir = tempfile::tempdir().unwrap();
        let source = "agent retained() { capabilities = [\"read\"] with sandbox = \"docker\" {} }";
        write(dir.path(), "retained.symbi", source);
        let bridge = Arc::new(repl_core::RuntimeBridge::new());
        let cards = Arc::new(RwLock::new(Vec::<AgentCard>::new()));
        assert_eq!(
            load_agents_into(dir.path(), &bridge, &cards).await.loaded,
            1
        );
        let original = bridge.agent_registry().get_agent("retained").await.unwrap();
        write(
            dir.path(),
            "retained.symbi",
            "agent retained() { policy mandatory { require: true } }",
        );
        let report = load_agents_into(dir.path(), &bridge, &cards).await;
        assert_eq!(report.loaded, 0);
        assert_eq!(report.sandbox_refused, ["retained"]);
        assert!(report.errors[0]
            .message
            .contains("require rules require an unsupported interpreter"));
        assert!(bridge
            .agent_registry()
            .get_agent("retained")
            .await
            .is_none());
        assert!(cards.read().await.is_empty());
        assert_eq!(original.definition.unwrap().source(), source);
        write(dir.path(), "retained.symbi", source);
        assert_eq!(
            load_agents_into(dir.path(), &bridge, &cards).await.loaded,
            1
        );
        std::fs::remove_file(dir.path().join("retained.symbi")).unwrap();
        assert_eq!(
            load_agents_into(dir.path(), &bridge, &cards).await.loaded,
            0
        );
        assert!(bridge
            .agent_registry()
            .get_agent("retained")
            .await
            .is_none());
    }
}
