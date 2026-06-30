//! Loading `.symbi` DSL agents into the shell fleet: parse, fail-closed sandbox
//! gate, and a conservative capability→tool mapping. Produces the same
//! `AgentSpec` the manifest path uses; everything downstream is shared.

use crate::agents::loader::{AgentSource, AgentSpec, LoadError};
use std::path::Path;

/// Map a `.symbi` agent's declared capabilities to the shell's concrete tools.
///
/// Conservative and fail-closed: only known capability tokens grant a tool;
/// anything else (e.g. "analyze", "network_scan") grants nothing. `delegate` is
/// never granted — delegation is orchestrator-only. Case-insensitive; the result
/// is deduped and in a stable order.
pub fn capabilities_to_tools(caps: &[String]) -> Vec<String> {
    let mut read = false;
    let mut write = false;
    let mut exec = false;
    for c in caps {
        match c.to_ascii_lowercase().as_str() {
            "read" | "read_logs" | "read_metrics" | "memory_read" | "filesystem" => read = true,
            "write" | "memory_write" => write = true,
            "execute" | "execute_tests" => exec = true,
            _ => {}
        }
    }
    let mut tools = Vec::new();
    if read {
        tools.push("read_file".to_string());
        tools.push("search".to_string());
    }
    if write {
        tools.push("edit_file".to_string());
    }
    if exec {
        tools.push("shell".to_string());
    }
    tools
}

/// Outcome of loading one agent declaration from a `.symbi` file.
pub enum SymbiLoad {
    Loaded(AgentSpec),
    // `reason` is surfaced in future diagnostic output and test assertions.
    Refused {
        name: String,
        #[allow(dead_code)]
        reason: String,
    },
}

/// Parse a `.symbi` (or legacy `.dsl`) file into one `SymbiLoad`. Uses the
/// tree-sitter `symbi-dsl` parser (the same one `symbi run` uses) so real
/// full-grammar agents parse. A read/syntax error — or a file with no agent
/// declaration — is a `LoadError` (the file is skipped). An agent that declares
/// any `with { sandbox = ... }` tier is `Refused` (the in-process shell provides
/// no isolation). One agent per file.
pub fn parse_symbi(path: &Path) -> Result<Vec<SymbiLoad>, LoadError> {
    let err = |m: String| LoadError {
        path: path.to_path_buf(),
        message: m,
    };
    let text = std::fs::read_to_string(path).map_err(|e| err(e.to_string()))?;

    let tree = dsl::parse_dsl(&text).map_err(|e| err(format!("parse error: {e}")))?;
    if tree.root_node().has_error() {
        return Err(err("syntax error in .symbi file".to_string()));
    }

    let name = dsl::extract_agent_name(&tree, &text)
        .ok_or_else(|| err("no agent declaration found".to_string()))?;

    // Fail-closed sandbox gate: any declared with-block sandbox tier needs
    // isolation symbi-shell (in-process tools) cannot provide.
    let with_blocks = dsl::extract_with_blocks(&tree, &text)
        .map_err(|e| err(format!("with-block parse error: {e}")))?;
    if let Some(tier) = with_blocks.iter().find_map(|w| w.sandbox_tier.as_ref()) {
        return Ok(vec![SymbiLoad::Refused {
            name,
            reason: format!(
                "declares sandbox tier {tier} requiring isolation symbi-shell cannot provide; run it via `symbi up` or `symbi run`"
            ),
        }]);
    }

    let tools = capabilities_to_tools(&dsl::extract_capabilities(&tree, &text));
    let description = dsl::extract_metadata(&tree, &text)
        .get("description")
        .map(|d| d.trim_matches('"').to_string())
        .filter(|d| !d.trim().is_empty())
        .unwrap_or_else(|| format!("{name} (.symbi agent)"));

    Ok(vec![SymbiLoad::Loaded(AgentSpec {
        name,
        description: description.clone(),
        system_prompt: description,
        tools,
        source: AgentSource::Symbi(path.to_path_buf()),
    })])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &std::path::Path, file: &str, body: &str) -> std::path::PathBuf {
        let p = dir.join(file);
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn loads_no_sandbox_agent_with_capabilities() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(
            dir.path(),
            "reader.symbi",
            "metadata { description = \"reads files\" }\nagent reader {\n  capabilities = [\"read\"]\n}\n",
        );
        let loads = parse_symbi(&p).unwrap();
        assert_eq!(loads.len(), 1);
        match &loads[0] {
            SymbiLoad::Loaded(spec) => {
                assert_eq!(spec.name, "reader");
                assert_eq!(spec.description, "reads files");
                assert_eq!(
                    spec.tools,
                    vec!["read_file".to_string(), "search".to_string()]
                );
            }
            SymbiLoad::Refused { .. } => panic!("no-sandbox agent must load"),
        }
    }

    #[test]
    fn domain_capabilities_load_tool_less() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(
            dir.path(),
            "scan.symbi",
            "agent scanner {\n  capabilities = [\"security_scanning\", \"vulnerability_detection\"]\n}\n",
        );
        match &parse_symbi(&p).unwrap()[0] {
            SymbiLoad::Loaded(spec) => assert!(spec.tools.is_empty()),
            SymbiLoad::Refused { .. } => panic!("should load tool-less"),
        }
    }

    #[test]
    fn agent_with_sandbox_tier_is_refused() {
        // Minimal valid with-block that declares a sandbox tier. The with_block
        // grammar rule is: 'with' repeat(with_attribute) block where
        // with_attribute is: identifier '=' value. The block is '{ }' (empty).
        let dir = tempfile::tempdir().unwrap();
        let p = write(
            dir.path(),
            "risky.symbi",
            "agent risky {\n  capabilities = [\"read\"]\n  with sandbox = \"Tier2\" {\n  }\n}\n",
        );
        match &parse_symbi(&p).unwrap()[0] {
            SymbiLoad::Refused { name, reason } => {
                assert_eq!(name, "risky");
                assert!(reason.contains("symbi up") || reason.contains("symbi run"));
            }
            SymbiLoad::Loaded(_) => panic!("sandbox-tier agent must be refused"),
        }
    }

    #[test]
    fn no_agent_declaration_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(
            dir.path(),
            "empty.symbi",
            "metadata { version = \"1.0.0\" }\n",
        );
        assert!(parse_symbi(&p).is_err());
    }

    #[test]
    fn description_falls_back_to_name() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(
            dir.path(),
            "bare.symbi",
            "agent bare {\n  capabilities = [\"read\"]\n}\n",
        );
        match &parse_symbi(&p).unwrap()[0] {
            SymbiLoad::Loaded(spec) => assert!(spec.description.contains("bare")),
            SymbiLoad::Refused { .. } => panic!("should load"),
        }
    }

    #[test]
    fn read_maps_to_read_file_and_search() {
        assert_eq!(
            capabilities_to_tools(&["read".to_string()]),
            vec!["read_file".to_string(), "search".to_string()]
        );
    }

    #[test]
    fn write_and_execute_map() {
        let t = capabilities_to_tools(&["write".to_string(), "execute".to_string()]);
        assert!(t.contains(&"edit_file".to_string()));
        assert!(t.contains(&"shell".to_string()));
    }

    #[test]
    fn unknown_capabilities_grant_nothing() {
        assert!(
            capabilities_to_tools(&["analyze".to_string(), "network_scan".to_string()]).is_empty()
        );
    }

    #[test]
    fn delegate_is_never_granted() {
        assert!(capabilities_to_tools(&["delegate".to_string()]).is_empty());
    }

    #[test]
    fn case_insensitive_and_deduped() {
        let t = capabilities_to_tools(&["READ".to_string(), "read_logs".to_string()]);
        assert_eq!(t, vec!["read_file".to_string(), "search".to_string()]);
    }
}
