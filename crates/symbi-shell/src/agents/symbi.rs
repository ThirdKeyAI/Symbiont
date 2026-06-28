//! Loading `.symbi` DSL agents into the shell fleet: parse, fail-closed sandbox
//! gate, and a conservative capability→tool mapping. Produces the same
//! `AgentSpec` the manifest path uses; everything downstream is shared.

use crate::agents::loader::{AgentSource, AgentSpec, LoadError};
use repl_core::dsl::ast::{Declaration, SandboxMode, SecurityConfig, SecurityTier};
use repl_core::dsl::{Lexer, Parser};
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

/// Fail-closed sandbox-tier gate. symbi-shell runs its tools in-process and
/// cannot provide container isolation, so it honors only agents that declare no
/// tier, or `Tier1`, and do not request any sandbox (`Permissive` or absent).
/// Both `Strict` and `Moderate` are refused — never silently downgraded.
pub fn shell_can_honor(security: Option<&SecurityConfig>) -> Result<(), String> {
    let Some(sec) = security else {
        return Ok(());
    };
    if matches!(
        sec.sandbox,
        Some(SandboxMode::Strict) | Some(SandboxMode::Moderate)
    ) {
        return Err(
            "requires a sandbox (strict/moderate) that symbi-shell cannot provide; \
             run it via `symbi up` or `symbi run`"
                .to_string(),
        );
    }
    match sec.tier {
        None | Some(SecurityTier::Tier1) => Ok(()),
        Some(SecurityTier::Tier2) | Some(SecurityTier::Tier3) | Some(SecurityTier::Tier4) => Err(
            "requires sandbox isolation (tier 2+) that symbi-shell cannot provide; \
             run it via `symbi up` or `symbi run`"
                .to_string(),
        ),
    }
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

/// Parse a `.symbi` (or legacy `.dsl`) file into one `SymbiLoad` per agent
/// declaration. A read/lex/parse failure — or a file with no agent declaration —
/// is a single `LoadError` (the whole file is skipped). Individual agents that
/// declare a sandbox tier the shell can't honor come back as `Refused`.
pub fn parse_symbi(path: &Path) -> Result<Vec<SymbiLoad>, LoadError> {
    let err = |m: String| LoadError {
        path: path.to_path_buf(),
        message: m,
    };
    let text = std::fs::read_to_string(path).map_err(|e| err(e.to_string()))?;

    let tokens = Lexer::new(&text)
        .tokenize()
        .map_err(|e| err(format!("lex error: {e}")))?;
    let program = Parser::new(tokens)
        .parse()
        .map_err(|e| err(format!("parse error: {e}")))?;

    let mut out = Vec::new();
    for decl in &program.declarations {
        if let Declaration::Agent(def) = decl {
            let security = def.security.as_ref();
            if let Err(reason) = shell_can_honor(security) {
                out.push(SymbiLoad::Refused {
                    name: def.name.clone(),
                    reason,
                });
                continue;
            }
            let caps = security.map(|s| s.capabilities.clone()).unwrap_or_default();
            let tools = capabilities_to_tools(&caps);
            let description = def
                .metadata
                .description
                .clone()
                .filter(|d| !d.trim().is_empty())
                .unwrap_or_else(|| format!("{} (.symbi agent)", def.name));
            // The shared loader applies the tool-aware / no-fab prompt addendum at
            // registration, so the spec carries the raw description as the seed.
            out.push(SymbiLoad::Loaded(AgentSpec {
                name: def.name.clone(),
                description: description.clone(),
                system_prompt: description,
                tools,
                source: AgentSource::Symbi(path.to_path_buf()),
            }));
        }
    }

    if out.is_empty() {
        return Err(err("no agent declaration found".to_string()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use repl_core::dsl::ast::{SandboxMode, SecurityConfig, SecurityTier};

    fn sec(tier: Option<SecurityTier>, sandbox: Option<SandboxMode>) -> SecurityConfig {
        SecurityConfig {
            tier,
            capabilities: vec![],
            sandbox,
        }
    }

    #[test]
    fn none_and_tier1_are_honored() {
        assert!(shell_can_honor(None).is_ok());
        assert!(shell_can_honor(Some(&sec(None, None))).is_ok());
        assert!(shell_can_honor(Some(&sec(Some(SecurityTier::Tier1), None))).is_ok());
    }

    #[test]
    fn tier2_plus_is_refused() {
        for t in [
            SecurityTier::Tier2,
            SecurityTier::Tier3,
            SecurityTier::Tier4,
        ] {
            let e = shell_can_honor(Some(&sec(Some(t), None))).unwrap_err();
            assert!(e.contains("symbi up") || e.contains("symbi run"));
        }
    }

    #[test]
    fn strict_sandbox_is_refused_even_at_tier1() {
        let e = shell_can_honor(Some(&sec(
            Some(SecurityTier::Tier1),
            Some(SandboxMode::Strict),
        )))
        .unwrap_err();
        assert!(e.contains("strict/moderate"));
    }

    #[test]
    fn moderate_sandbox_is_refused() {
        let e = shell_can_honor(Some(&sec(
            Some(SecurityTier::Tier1),
            Some(SandboxMode::Moderate),
        )))
        .unwrap_err();
        assert!(e.contains("strict/moderate"));
        assert!(e.contains("symbi up") || e.contains("symbi run"));
    }

    #[test]
    fn permissive_sandbox_is_honored() {
        assert!(shell_can_honor(Some(&sec(
            Some(SecurityTier::Tier1),
            Some(SandboxMode::Permissive),
        )))
        .is_ok());
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

    use super::SymbiLoad;

    fn write_symbi(dir: &std::path::Path, file: &str, body: &str) -> std::path::PathBuf {
        let p = dir.join(file);
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn parses_tier1_agent_with_capabilities() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_symbi(
            dir.path(),
            "reader.symbi",
            "agent Reader {\n  description: \"reads files\"\n  security {\n    tier: Tier1\n    capabilities: [\"read\"]\n  }\n}\n",
        );
        let loads = parse_symbi(&p).unwrap();
        assert_eq!(loads.len(), 1);
        match &loads[0] {
            SymbiLoad::Loaded(spec) => {
                assert_eq!(spec.name, "Reader");
                assert_eq!(spec.description, "reads files");
                assert_eq!(
                    spec.tools,
                    vec!["read_file".to_string(), "search".to_string()]
                );
            }
            SymbiLoad::Refused { .. } => panic!("Tier1 agent should load"),
        }
    }

    #[test]
    fn tier3_agent_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_symbi(
            dir.path(),
            "risky.symbi",
            "agent Risky {\n  description: \"needs isolation\"\n  security {\n    tier: Tier3\n    capabilities: [\"read\"]\n  }\n}\n",
        );
        let loads = parse_symbi(&p).unwrap();
        match &loads[0] {
            SymbiLoad::Refused { name, reason } => {
                assert_eq!(name, "Risky");
                assert!(reason.contains("symbi up") || reason.contains("symbi run"));
            }
            SymbiLoad::Loaded(_) => panic!("Tier3 agent must be refused"),
        }
    }

    #[test]
    fn agent_without_description_gets_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_symbi(
            dir.path(),
            "bare.symbi",
            "agent Bare {\n  version: \"1.0.0\"\n}\n",
        );
        let loads = parse_symbi(&p).unwrap();
        match &loads[0] {
            SymbiLoad::Loaded(spec) => assert!(spec.description.contains("Bare")),
            SymbiLoad::Refused { .. } => panic!("no security block → honored"),
        }
    }

    #[test]
    fn file_without_agent_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_symbi(dir.path(), "empty.symbi", "\n");
        assert!(parse_symbi(&p).is_err());
    }
}
