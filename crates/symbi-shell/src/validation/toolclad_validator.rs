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

    #[test]
    fn test_valid_manifest() {
        let toml = r#"
[tool]
name = "my_tool"
risk_tier = "low"
"#;
        let issues = validate_toolclad(toml, &test_constraints()).unwrap();
        assert!(issues.is_empty());
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
}
