//! Cedar policy auto-generation from ToolClad manifests
//!
//! Generates baseline Cedar policies from manifest metadata (risk tier,
//! Cedar resource/action, human approval requirement).

use super::manifest::Manifest;

/// Generate Cedar policy text for a single manifest.
///
/// The permit matches `resource.base_tool`, which every session and browser
/// sub-command of the manifest shares (the per-call `tool_name` is
/// `<manifest>.<command>` for those). Two forbids carry the approval rules:
/// one for a manifest-level `human_approval`, and one keyed on the call's
/// effective `resource.risk_tier`, so a `high` or `critical` manifest *or*
/// sub-command cannot run without `context.has_human_approval`. A missing
/// approval attribute counts as no approval.
pub fn generate_policy(manifest: &Manifest) -> Option<String> {
    let cedar = manifest.tool.cedar.as_ref()?;
    let tool_name = &manifest.tool.name;
    let resource = &cedar.resource;
    let action = &cedar.action;
    let risk = &manifest.tool.risk_tier;
    let scope =
        format!("    principal,\n    action == {resource}::Action::\"{action}\",\n    resource\n");
    let approved = "context has has_human_approval && context.has_human_approval == true";
    let mut policy = format!(
        "// Auto-generated from {tool_name}.clad.toml (risk_tier = \"{risk}\", human_approval = {approval})\n",
        approval = manifest.tool.human_approval
    );
    if super::manifest::tier_rank(risk) >= 3 {
        policy.push_str(
            "// WARNING: High-risk tool — review and add restrictions before production use\n",
        );
    }
    policy.push_str(&format!(
        "permit (\n{scope})\nwhen {{\n    resource has base_tool && resource.base_tool == \"{tool_name}\"\n}};\n"
    ));
    if manifest.tool.human_approval {
        policy.push_str(&format!(
            "forbid (\n{scope})\nwhen {{\n    resource has base_tool && resource.base_tool == \"{tool_name}\"\n}}\nunless {{\n    {approved}\n}};\n"
        ));
    }
    policy.push_str(&format!(
        "forbid (\n{scope})\nwhen {{\n    resource has base_tool && resource.base_tool == \"{tool_name}\" &&\n    resource has risk_tier && (resource.risk_tier == \"high\" || resource.risk_tier == \"critical\")\n}}\nunless {{\n    {approved}\n}};\n"
    ));
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

    fn make_manifest(name: &str, risk: &str, approval: bool) -> Manifest {
        toml::from_str(&format!(
            r#"
[tool]
name = "{name}"
version = "1.0.0"
binary = "test"
description = "Test tool"
risk_tier = "{risk}"
human_approval = {approval}
[tool.cedar]
resource = "Tool::Test"
action = "execute_tool"
[command]
exec = ["test"]
[output]
format = "text"
"#
        ))
        .unwrap()
    }

    #[test]
    fn test_generate_low_risk() {
        let policy = generate_policy(&make_manifest("whois", "low", false)).unwrap();
        assert!(policy.contains("risk_tier = \"low\""));
        assert!(policy.contains("resource.base_tool == \"whois\""));
        assert!(!policy.contains("tool_name =="));
    }

    #[test]
    fn test_generate_high_risk() {
        let policy = generate_policy(&make_manifest("exploit", "high", false)).unwrap();
        assert!(policy.contains("WARNING: High-risk tool"));
        assert!(policy.contains("forbid"));
        assert!(policy.contains("context.has_human_approval"));
    }

    #[test]
    fn test_generate_approval_required() {
        let policy = generate_policy(&make_manifest("msf", "low", true)).unwrap();
        assert_eq!(policy.matches("forbid").count(), 2);
        assert!(policy.contains("context has has_human_approval"));
    }

    #[cfg(feature = "cedar")]
    #[test]
    fn generated_policies_parse_and_enforce_tiers() {
        use cedar_policy::{
            Authorizer, Context, Decision, Entities, EntityUid, PolicySet, Request,
        };
        use std::str::FromStr;
        let policy = generate_policy(&make_manifest("sess", "low", false)).unwrap();
        let set = PolicySet::from_str(&policy).unwrap();
        let decide = |tier: &str, approved: bool| {
            let entities = Entities::from_json_value(
                serde_json::json!([{
                    "uid": {"type": "Tool::Test", "id": "sess.cmd"},
                    "attrs": {"tool_name": "sess.cmd", "base_tool": "sess", "risk_tier": tier},
                    "parents": []
                }]),
                None,
            )
            .unwrap();
            let request = Request::new(
                EntityUid::from_str("Agent::\"a\"").unwrap(),
                EntityUid::from_str("Tool::Test::Action::\"execute_tool\"").unwrap(),
                EntityUid::from_str("Tool::Test::\"sess.cmd\"").unwrap(),
                Context::from_json_value(serde_json::json!({"has_human_approval": approved}), None)
                    .unwrap(),
                None,
            )
            .unwrap();
            Authorizer::new()
                .is_authorized(&request, &set, &entities)
                .decision()
        };
        // A sub-command matches through base_tool.
        assert_eq!(decide("low", false), Decision::Allow);
        assert_eq!(decide("critical", false), Decision::Deny);
        assert_eq!(decide("critical", true), Decision::Allow);
    }
}
