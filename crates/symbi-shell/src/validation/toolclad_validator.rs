#![allow(dead_code)]

use super::constraints::ToolcladConstraints;
use super::dsl_validator::{Severity, ValidationIssue};
use anyhow::Result;

/// Validate a ToolClad TOML manifest against project constraints.
pub fn validate_toolclad(
    toml_text: &str,
    constraints: &ToolcladConstraints,
) -> Result<Vec<ValidationIssue>> {
    let mut issues = Vec::new();

    let doc: toml::Value = match toml::from_str(toml_text) {
        Ok(v) => v,
        Err(e) => {
            issues.push(ValidationIssue {
                severity: Severity::Error,
                message: format!("TOML parse error: {}", e),
            });
            return Ok(issues);
        }
    };

    // The runtime's own loader and lints: one implementation, so an authored
    // manifest is judged exactly as the executor will judge it (unknown keys,
    // interpreter argv[0] without allow_interpreter, egress targets without
    // block_internal, placeholder hygiene, ...).
    match symbi_runtime::toolclad::parse_manifest(toml_text, &std::collections::HashMap::new()) {
        Ok(manifest) => {
            for warning in symbi_runtime::toolclad::manifest::lint(&manifest) {
                issues.push(ValidationIssue {
                    severity: Severity::Warning,
                    message: warning,
                });
            }
        }
        Err(error) => issues.push(ValidationIssue {
            severity: Severity::Error,
            message: format!("manifest does not load: {error}"),
        }),
    }

    let tool = doc.get("tool");

    // Check risk tier
    if let Some(ref max_tier) = constraints.max_risk_tier {
        if let Some(risk_tier) = tool
            .and_then(|t| t.get("risk_tier"))
            .and_then(|v| v.as_str())
        {
            if tier_level(risk_tier) > tier_level(max_tier) {
                issues.push(ValidationIssue {
                    severity: Severity::Error,
                    message: format!(
                        "Risk tier '{}' exceeds project maximum '{}'",
                        risk_tier, max_tier
                    ),
                });
            }
        }
    }

    // Check evidence requirement
    if let Some(ref above_tier) = constraints.require_evidence_above_tier {
        let risk_tier = tool
            .and_then(|t| t.get("risk_tier"))
            .and_then(|v| v.as_str())
            .unwrap_or("low");

        if tier_level(risk_tier) > tier_level(above_tier) {
            let has_evidence = doc
                .get("tool")
                .and_then(|t| t.get("evidence"))
                .and_then(|e| e.get("capture"))
                .and_then(|c| c.as_bool())
                .unwrap_or(false);

            if !has_evidence {
                issues.push(ValidationIssue {
                    severity: Severity::Error,
                    message: format!(
                        "Evidence capture required for tools above '{}' tier",
                        above_tier
                    ),
                });
            }
        }
    }

    // Check scope_check requirement
    if constraints.require_scope_check {
        if let Some(args) = doc.get("args") {
            if let Some(args_table) = args.as_table() {
                for (name, arg) in args_table {
                    let has_schemes = arg.get("schemes").is_some();
                    let scope_check = arg
                        .get("scope_check")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);

                    if has_schemes && !scope_check {
                        issues.push(ValidationIssue {
                            severity: Severity::Warning,
                            message: format!(
                                "Argument '{}' has URL schemes but scope_check is not enabled",
                                name
                            ),
                        });
                    }
                }
            }
        }
    }

    // Anti-pattern: a free-text argument that feeds a privileged downstream
    // decision. Use an enum + Cedar grounding instead (see
    // crates/runtime/src/toolclad/decision.rs).
    if let Some(args) = doc.get("args") {
        if let Some(args_table) = args.as_table() {
            for (name, arg) in args_table {
                let feeds = arg
                    .get("feeds_decision")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if !feeds {
                    continue;
                }
                let ty = arg.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if matches!(
                    ty,
                    "string" | "literal_text" | "agent_summary" | "regex_match"
                ) {
                    issues.push(ValidationIssue {
                        severity: Severity::Error,
                        message: format!(
                            "Argument '{}' is free-text (type '{}') but feeds a privileged decision; \
                             use an enum + Cedar grounding (see crates/runtime/src/toolclad/decision.rs)",
                            name, ty
                        ),
                    });
                }
            }
        }
    }

    Ok(issues)
}

fn tier_level(tier: &str) -> u8 {
    match tier.to_lowercase().as_str() {
        "low" => 1,
        "medium" => 2,
        "high" => 3,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_constraints() -> ToolcladConstraints {
        ToolcladConstraints {
            max_risk_tier: Some("medium".to_string()),
            require_evidence_above_tier: Some("low".to_string()),
            require_scope_check: true,
        }
    }

    const VALID: &str = r#"
[tool]
name = "my_tool"
version = "1.0.0"
binary = "wc"
description = "Count lines"
risk_tier = "low"

[args.path]
position = 1
required = true
type = "path"

[command]
exec = ["wc", "-l", "{path}"]
end_of_options = "--"

[output]
format = "text"
"#;

    #[test]
    fn test_valid_manifest() {
        let issues = validate_toolclad(VALID, &test_constraints()).unwrap();
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn generated_manifests_go_through_the_runtime_loader() {
        let shell = VALID.replace("binary = \"wc\"", "binary = \"sh\"").replace(
            "exec = [\"wc\", \"-l\", \"{path}\"]",
            "exec = [\"sh\", \"-c\", \"{path}\"]",
        );
        let issues = validate_toolclad(&shell, &test_constraints()).unwrap();
        assert!(issues
            .iter()
            .any(|i| matches!(i.severity, Severity::Error) && i.message.contains("interpreter")));

        let allowed = shell.replace(
            "risk_tier = \"low\"",
            "risk_tier = \"low\"\nallow_interpreter = true",
        );
        let issues = validate_toolclad(&allowed, &test_constraints()).unwrap();
        assert!(issues
            .iter()
            .any(|i| matches!(i.severity, Severity::Warning)
                && i.message.contains("allow_interpreter")));

        let typo = VALID.replace(
            "risk_tier = \"low\"",
            "risk_tier = \"low\"\ntiemout_seconds = 5",
        );
        let issues = validate_toolclad(&typo, &test_constraints()).unwrap();
        assert!(issues.iter().any(|i| i.message.contains("does not load")));
    }

    #[test]
    fn operands_without_end_of_options_and_unscoped_urls_are_flagged() {
        let unfenced = VALID
            .replace("end_of_options = \"--\"\n", "")
            .replace("\"-l\", ", "");
        let issues = validate_toolclad(&unfenced, &test_constraints()).unwrap();
        assert!(
            issues.iter().any(|i| i.message.contains("without `--`")),
            "{issues:?}"
        );

        let url = r#"
[tool]
name = "fetch"
version = "1"
binary = "curl"
description = "d"
[args.url]
position = 1
required = true
type = "url"
block_internal = true
[command]
exec = ["curl", "--", "{url}"]
[output]
format = "text"
"#;
        let issues = validate_toolclad(url, &test_constraints()).unwrap();
        assert!(
            issues
                .iter()
                .any(|i| i.message.contains("without scope_check")),
            "{issues:?}"
        );
        let unguarded = url.replace("block_internal = true\n", "");
        let issues = validate_toolclad(&unguarded, &test_constraints()).unwrap();
        assert!(
            issues.iter().any(|i| i.message.contains("block_internal")),
            "{issues:?}"
        );
    }

    #[test]
    fn test_risk_tier_exceeded() {
        let toml = r#"
[tool]
name = "dangerous"
risk_tier = "high"
"#;
        let issues = validate_toolclad(toml, &test_constraints()).unwrap();
        assert!(issues.iter().any(|i| i.message.contains("exceeds")));
    }

    #[test]
    fn test_evidence_required() {
        let toml = r#"
[tool]
name = "med_tool"
risk_tier = "medium"
"#;
        let issues = validate_toolclad(toml, &test_constraints()).unwrap();
        assert!(issues.iter().any(|i| i.message.contains("Evidence")));
    }

    #[test]
    fn test_invalid_toml() {
        let toml = "this is not { valid toml";
        let issues = validate_toolclad(toml, &test_constraints()).unwrap();
        assert!(issues.iter().any(|i| i.message.contains("TOML parse")));
    }

    #[test]
    fn test_freetext_feeds_decision_is_flagged() {
        let toml = r#"
[tool]
name = "triage_bad"
risk_tier = "low"

[args.summary]
type = "string"
feeds_decision = true
"#;
        let issues = validate_toolclad(toml, &test_constraints()).unwrap();
        assert!(issues
            .iter()
            .any(|i| i.message.contains("feeds a privileged decision")));
    }

    #[test]
    fn test_enum_feeds_decision_is_allowed() {
        let toml = r#"
[tool]
name = "triage_ok"
risk_tier = "low"

[args.severity]
type = "enum"
allowed = ["low", "high", "critical"]
feeds_decision = true
"#;
        let issues = validate_toolclad(toml, &test_constraints()).unwrap();
        assert!(!issues
            .iter()
            .any(|i| i.message.contains("feeds a privileged decision")));
    }

    #[test]
    fn test_example_submit_triage_manifest_passes_lint() {
        let toml = include_str!("../../../../tools/submit_triage.clad.toml");
        let issues = validate_toolclad(toml, &test_constraints()).unwrap();
        // Enum decision args + advisory free-text rationale => no
        // feeds_decision anti-pattern error.
        assert!(
            !issues
                .iter()
                .any(|i| i.message.contains("feeds a privileged decision")),
            "unexpected lint errors: {:?}",
            issues
        );
    }
}
