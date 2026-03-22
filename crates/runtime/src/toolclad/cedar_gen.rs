//! Cedar policy auto-generation from ToolClad manifests
//!
//! Generates baseline Cedar policies from manifest metadata (risk tier,
//! Cedar resource/action, human approval requirement).

use super::manifest::Manifest;

/// Generate Cedar policy text for a single manifest.
pub fn generate_policy(manifest: &Manifest) -> Option<String> {
    let cedar = manifest.tool.cedar.as_ref()?;
    let tool_name = &manifest.tool.name;
    let resource = &cedar.resource;
    let action = &cedar.action;

    let policy = if manifest.tool.human_approval {
        format!(
            r#"// Auto-generated from {tool_name}.clad.toml (risk_tier = "{risk}", human_approval = true)
permit (
    principal,
    action == {resource}::Action::"{action}",
    resource
)
when {{
    resource.tool_name == "{tool_name}" &&
    context.has_human_approval == true
}};
"#,
            risk = manifest.tool.risk_tier,
        )
    } else if manifest.tool.risk_tier == "high" {
        format!(
            r#"// Auto-generated from {tool_name}.clad.toml (risk_tier = "high")
// WARNING: High-risk tool — review and add restrictions before production use
permit (
    principal,
    action == {resource}::Action::"{action}",
    resource
)
when {{
    resource.tool_name == "{tool_name}"
}};
"#,
        )
    } else {
        format!(
            r#"// Auto-generated from {tool_name}.clad.toml (risk_tier = "{risk}")
permit (
    principal,
    action == {resource}::Action::"{action}",
    resource
)
when {{
    resource.tool_name == "{tool_name}"
}};
"#,
            risk = manifest.tool.risk_tier,
        )
    };

    Some(policy)
}

/// Generate Cedar policies for all manifests.
pub fn generate_policies(manifests: &[(String, Manifest)]) -> String {
    let mut output = String::from("// ToolClad auto-generated Cedar policies\n");
    output.push_str("// Generated from tools/*.clad.toml manifests\n");
    output.push_str("// Review and customize before production use.\n\n");

    for (_, manifest) in manifests {
        if let Some(policy) = generate_policy(manifest) {
            output.push_str(&policy);
            output.push('\n');
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolclad::manifest::{CedarMeta, ToolMeta};

    fn make_manifest(name: &str, risk: &str, approval: bool) -> Manifest {
        Manifest {
            tool: ToolMeta {
                name: name.to_string(),
                version: "1.0.0".to_string(),
                binary: "test".to_string(),
                mode: "oneshot".to_string(),
                description: "Test tool".to_string(),
                timeout_seconds: 30,
                risk_tier: risk.to_string(),
                human_approval: approval,
                cedar: Some(CedarMeta {
                    resource: "Tool::Test".to_string(),
                    action: "execute_tool".to_string(),
                }),
                evidence: None,
            },
            args: Default::default(),
            command: super::super::manifest::CommandDef {
                template: Some("test".to_string()),
                executor: None,
                defaults: Default::default(),
                mappings: Default::default(),
                conditionals: Default::default(),
            },
            output: super::super::manifest::OutputDef {
                format: "text".to_string(),
                parser: None,
                envelope: true,
                schema: serde_json::json!({}),
            },
            http: None,
            mcp: None,
            session: None,
            browser: None,
        }
    }

    #[test]
    fn test_generate_low_risk() {
        let m = make_manifest("whois", "low", false);
        let policy = generate_policy(&m).unwrap();
        assert!(policy.contains("risk_tier = \"low\""));
        assert!(policy.contains("tool_name == \"whois\""));
        assert!(!policy.contains("human_approval"));
    }

    #[test]
    fn test_generate_high_risk() {
        let m = make_manifest("exploit", "high", false);
        let policy = generate_policy(&m).unwrap();
        assert!(policy.contains("WARNING: High-risk tool"));
    }

    #[test]
    fn test_generate_approval_required() {
        let m = make_manifest("msf", "high", true);
        let policy = generate_policy(&m).unwrap();
        assert!(policy.contains("has_human_approval"));
    }
}
