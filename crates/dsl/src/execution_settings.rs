//! Strict selection of one agent's process-level execution settings.

use crate::{extract_with_blocks, parse_dsl, SandboxTier};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentExecutionSettings {
    pub agent_name: String,
    pub agent_source: String,
    pub sandbox_tier: Option<SandboxTier>,
    pub timeout_seconds: Option<u64>,
}

/// Resolve settings from the selected definition, never from another agent,
/// comments, or prompt text. A single definition may be addressed by its file
/// name. Multiple definitions require an exact, unique declared name.
///
/// A reasoning/managed process has one boundary and one lifetime. Conflicting
/// block settings cannot be faithfully applied to that process and are errors.
pub fn resolve_execution_settings(
    source: &str,
    selector: &str,
) -> Result<AgentExecutionSettings, String> {
    if source.len() > 1024 * 1024 {
        return Err("agent definition exceeds 1 MiB".into());
    }
    let tree = parse_dsl(source).map_err(|e| format!("cannot parse agent definition: {e}"))?;
    let root = tree.root_node();
    if root.has_error() {
        return Err(
            "agent definition contains syntax errors; execution settings are unavailable".into(),
        );
    }
    let mut cursor = root.walk();
    let mut agents = Vec::new();
    for node in root
        .named_children(&mut cursor)
        .filter(|n| n.kind() == "agent_definition")
    {
        let mut names = node.walk();
        let name = node
            .named_children(&mut names)
            .find(|n| n.kind() == "identifier")
            .ok_or("agent definition has no name")?;
        agents.push((&source[name.byte_range()], node));
    }
    let mut unique = std::collections::HashSet::new();
    if agents.iter().any(|(name, _)| !unique.insert(*name)) {
        return Err("agent definitions contain duplicate names".into());
    }
    let (name, node) = if agents.len() == 1 {
        agents[0]
    } else {
        *agents
            .iter()
            .find(|(name, _)| *name == selector)
            .ok_or("agent selector does not identify a unique definition")?
    };
    let selected_source = &source[node.byte_range()];
    let selected_tree = parse_dsl(selected_source).map_err(|e| e.to_string())?;
    let blocks = extract_with_blocks(&selected_tree, selected_source)?;
    let mut settings = AgentExecutionSettings {
        agent_name: name.into(),
        agent_source: selected_source.into(),
        sandbox_tier: None,
        timeout_seconds: None,
    };
    for block in blocks {
        for field in ["sandbox", "timeout"] {
            if block.attributes.iter().filter(|a| a.name == field).count() > 1 {
                return Err(format!(
                    "duplicate {field} attribute in agent execution settings"
                ));
            }
        }
        if let Some(tier) = block.sandbox_tier {
            if settings
                .sandbox_tier
                .as_ref()
                .is_some_and(|previous| previous != &tier)
            {
                return Err("conflicting sandbox selections within one agent process".into());
            }
            settings.sandbox_tier = Some(tier);
        }
        if let Some(timeout) = block.timeout {
            if !(1..=86400).contains(&timeout) {
                return Err("agent timeout must be between 1 second and 1 day".into());
            }
            if settings
                .timeout_seconds
                .is_some_and(|previous| previous != timeout)
            {
                return Err("conflicting timeout selections within one agent process".into());
            }
            settings.timeout_seconds = Some(timeout);
        }
    }
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_declared_agent_without_importing_other_agents_settings() {
        let source = r#"agent first() { with sandbox = "docker", timeout = 4.seconds {} }
agent second() { with sandbox = "gvisor", timeout = 8.seconds {} }"#;
        let selected = resolve_execution_settings(source, "second").unwrap();
        assert_eq!(selected.sandbox_tier, Some(SandboxTier::GVisor));
        assert_eq!(selected.timeout_seconds, Some(8));
        assert!(!selected.agent_source.contains("agent first"));
        assert!(resolve_execution_settings(source, "file_alias").is_err());
        assert!(resolve_execution_settings("agent same() {} agent same() {}", "same").is_err());
    }

    #[test]
    fn single_agent_file_alias_and_repeated_consistent_blocks_are_supported() {
        let selected = resolve_execution_settings(
            r#"metadata { description = "sandbox = firecracker" }
agent declared() { with sandbox = "Tier1" {} with sandbox = "docker" {} }"#,
            "file_alias",
        )
        .unwrap();
        assert_eq!(selected.agent_name, "declared");
        assert_eq!(selected.sandbox_tier, Some(SandboxTier::Docker));
        assert_eq!(
            resolve_execution_settings("agent empty() {}", "empty")
                .unwrap()
                .sandbox_tier,
            None
        );
    }

    #[test]
    fn malformed_unknown_duplicate_and_conflicting_settings_never_fall_back() {
        for source in [
            "agent broken() { with sandbox = }",
            r#"agent a() { with sandbox = "typo" {} }"#,
            r#"agent a() { with sandbox = "docker", sandbox = "gvisor" {} }"#,
            r#"agent a() { with sandbox = "docker" {} with sandbox = "gvisor" {} }"#,
            "agent a() { with timeout = 0 {} }",
            "agent a() { with timeout = 86401 {} }",
            "agent a() { with timeout = 3 {} with timeout = 4 {} }",
            "metadata { description = \"no agent\" }",
        ] {
            assert!(resolve_execution_settings(source, "a").is_err(), "{source}");
        }
    }
}
