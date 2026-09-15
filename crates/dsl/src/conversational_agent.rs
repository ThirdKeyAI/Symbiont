//! Canonical declarations supported by an ORGA conversational executor.
//!
//! Retain the original bytes and reject executable requirements that this route
//! cannot enforce. A source declaration is not interchangeable with a prompt.

use crate::{parse_dsl, resolve_execution_settings, AgentExecutionSettings};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "SourceSelection", into = "SourceSelection")]
pub struct ConversationalAgent {
    source: String,
    settings: AgentExecutionSettings,
    description: String,
    capabilities: Vec<String>,
    policy: crate::ExecutionPolicy,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceSelection {
    source: String,
    agent_name: String,
}

impl TryFrom<SourceSelection> for ConversationalAgent {
    type Error = String;
    fn try_from(value: SourceSelection) -> Result<Self, String> {
        Self::parse(&value.source, &value.agent_name)
    }
}

impl From<ConversationalAgent> for SourceSelection {
    fn from(value: ConversationalAgent) -> Self {
        Self {
            source: value.source,
            agent_name: value.settings.agent_name,
        }
    }
}

impl ConversationalAgent {
    pub fn parse(source: &str, name: &str) -> Result<Self, String> {
        let settings = resolve_execution_settings(source, name)?;
        let policy = crate::ExecutionPolicy::parse(source, name)?;
        if settings.agent_name != name {
            return Err("canonical registration requires the exact declared agent name".into());
        }
        let tree = parse_dsl(source).map_err(|error| error.to_string())?;
        let mut metadata = std::collections::HashMap::new();
        let mut cursor = tree.root_node().walk();
        for node in tree.root_node().named_children(&mut cursor) {
            match node.kind() {
                "comment" | "agent_definition" | "policy_definition" => {},
                "metadata_block" => {
                    let mut pairs = node.walk();
                    for pair in node.named_children(&mut pairs).filter(|n| n.kind() != "comment") {
                        let key = pair.child(0).ok_or("invalid metadata key")?;
                        let value = pair.child(2).ok_or("invalid metadata value")?;
                        let key = &source[key.byte_range()];
                        if !matches!(key, "description" | "version" | "author" | "executor") {
                            return Err(format!("metadata requirement {key} is unsupported by conversational execution"));
                        }
                        let value: String = serde_json::from_str(&source[value.byte_range()])
                            .map_err(|_| format!("metadata {key} must be a literal string"))?;
                        if metadata.insert(key.to_owned(), value).is_some() {
                            return Err(format!("duplicate metadata key {key}"));
                        }
                    }
                },
                kind => return Err(format!("{kind} requires an executable DSL route; conversational execution cannot enforce it")),
            }
        }
        if metadata
            .get("executor")
            .is_some_and(|value| value != "orga")
        {
            return Err("canonical conversational execution requires executor orga".into());
        }
        let selected = &settings.agent_source;
        let tree = parse_dsl(selected).map_err(|error| error.to_string())?;
        let node = tree
            .root_node()
            .named_child(0)
            .ok_or("missing selected agent")?;
        let mut capabilities = None;
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "identifier" | "comment" | "policy_definition" => {},
                "capabilities_declaration" => {
                    let mut children = child.walk();
                    let array = child.named_children(&mut children).find(|n| n.kind() == "array")
                        .ok_or("missing capabilities array")?;
                    let mut values = array.walk();
                    let caps = array.named_children(&mut values).filter(|n| n.kind() != "comment")
                        .map(|value| serde_json::from_str::<String>(&selected[value.byte_range()])
                            .map_err(|_| "capabilities must contain literal strings".to_owned()))
                        .collect::<Result<Vec<_>, _>>()?;
                    if capabilities.replace(caps).is_some() {
                        return Err("duplicate capabilities declarations".into());
                    }
                },
                "with_block" => {
                    let mut children = child.walk();
                    for item in child.named_children(&mut children) {
                        match item.kind() {
                            "comment" => {},
                            "with_attribute" => {
                                let key = item.child(0).ok_or("missing with attribute")?;
                                let key = &selected[key.byte_range()];
                                if !matches!(key, "sandbox" | "timeout") {
                                    return Err(format!("with requirement {key} is unsupported by conversational execution"));
                                }
                            },
                            "block" => {
                                let mut contents = item.walk();
                                if item.named_children(&mut contents).any(|n| n.kind() != "comment") {
                                    return Err("with-block statements require an executable DSL route".into());
                                }
                            },
                            kind => return Err(format!("unsupported with-block requirement {kind}")),
                        }
                    }
                },
                kind => return Err(format!("{kind} requires an executable DSL route; conversational execution cannot enforce it")),
            }
        }
        Ok(Self {
            source: source.to_owned(),
            description: metadata
                .remove("description")
                .unwrap_or_else(|| format!("{name} (.symbi agent)")),
            settings,
            capabilities: capabilities.unwrap_or_default(),
            policy,
        })
    }

    pub fn source(&self) -> &str {
        &self.source
    }
    pub fn settings(&self) -> &AgentExecutionSettings {
        &self.settings
    }
    pub fn description(&self) -> &str {
        &self.description
    }
    pub fn capabilities(&self) -> &[String] {
        &self.capabilities
    }
    pub fn policy(&self) -> &crate::ExecutionPolicy {
        &self.policy
    }
}

/// Return all declared names after strict syntax and duplicate-name validation.
pub fn conversational_agent_names(source: &str) -> Result<Vec<String>, String> {
    if source.len() > 1024 * 1024 {
        return Err("agent definition exceeds 1 MiB".into());
    }
    let tree = parse_dsl(source).map_err(|error| error.to_string())?;
    if tree.root_node().has_error() {
        return Err("syntax error in canonical agent source".into());
    }
    let mut cursor = tree.root_node().walk();
    let names = tree
        .root_node()
        .named_children(&mut cursor)
        .filter(|n| n.kind() == "agent_definition")
        .map(|node| {
            let mut children = node.walk();
            let name = node
                .named_children(&mut children)
                .find(|n| n.kind() == "identifier")
                .map(|name| source[name.byte_range()].to_owned())
                .ok_or_else(|| "missing agent name".to_owned());
            name
        })
        .collect::<Result<Vec<_>, _>>()?;
    if names.is_empty() {
        return Err("no agent declaration found".into());
    }
    if names.iter().collect::<std::collections::HashSet<_>>().len() != names.len() {
        return Err("agent definitions contain duplicate names".into());
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_contract_preserves_source_and_does_not_import_sibling_grants() {
        let source = r#"metadata { description = "fixture", executor = "orga" }
agent reader() { capabilities = ["read"] with sandbox = "docker", timeout = 3.seconds {} }
agent writer() { capabilities = ["write"] with sandbox = "gvisor" {} }"#;
        assert_eq!(
            conversational_agent_names(source).unwrap(),
            ["reader", "writer"]
        );
        let contract = ConversationalAgent::parse(source, "reader").unwrap();
        assert_eq!(contract.capabilities(), ["read"]);
        assert_eq!(contract.settings().timeout_seconds, Some(3));
        assert_eq!(contract.source(), source);
        let encoded = serde_json::to_value(&contract).unwrap();
        assert_eq!(
            serde_json::from_value::<ConversationalAgent>(encoded)
                .unwrap()
                .source(),
            source
        );
        assert!(ConversationalAgent::parse("agent declared() {}", "alias").is_err());
    }

    #[test]
    fn unsupported_or_ambiguous_requirements_never_become_prompt_only() {
        for source in [
            "agent a() {} agent a() {}",
            "policy p { audit: true } agent a() {}",
            "agent a() { policy p { require: true } }",
            "agent a(value: String) {}",
            "agent a() { function work() { return 1 } }",
            "agent a() { with timeout = 1 { return 1 } }",
            "agent a() { with network = true {} }",
            "agent a() { capabilities = [read] }",
            "agent a() { capabilities = [\"read\"] capabilities = [\"write\"] }",
            "metadata { executor = \"claude_code\" } agent a() {}",
            "metadata { timeout = 9 } agent a() {}",
            "metadata { executor = \"orga\", executor = \"orga\" } agent a() {}",
        ] {
            assert!(ConversationalAgent::parse(source, "a").is_err(), "{source}");
        }
    }
}
