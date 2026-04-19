#![allow(dead_code)]

use super::constraints::CedarConstraints;
use super::dsl_validator::{Severity, ValidationIssue};
use anyhow::Result;

/// Validate a Cedar policy string against project constraints.
pub fn validate_cedar(
    cedar_text: &str,
    constraints: &CedarConstraints,
) -> Result<Vec<ValidationIssue>> {
    let mut issues = Vec::new();

    // Check for wildcard principal in permit rules
    if constraints.forbid_wildcard_principal {
        for line in cedar_text.lines() {
            let trimmed = line.trim().to_lowercase();
            if trimmed.starts_with("permit") && trimmed.contains("principal,") {
                issues.push(ValidationIssue {
                    severity: Severity::Error,
                    message:
                        "Wildcard principal in permit rule is forbidden by project constraints"
                            .to_string(),
                });
            }
        }
    }

    // Check for wildcard resource on sensitive actions
    for action in &constraints.forbid_wildcard_resource_on {
        let action_lower = action.to_lowercase();
        for line in cedar_text.lines() {
            let trimmed = line.trim().to_lowercase();
            if trimmed.contains(&format!("action == \"{}\"", action_lower))
                && trimmed.contains("resource,")
            {
                issues.push(ValidationIssue {
                    severity: Severity::Error,
                    message: format!(
                        "Wildcard resource on action '{}' is forbidden by project constraints",
                        action
                    ),
                });
            }
        }
    }

    // Check for required conditions
    if constraints.require_schema_verified {
        let has_schema_check = cedar_text.to_lowercase().contains("schema_verified");
        if !has_schema_check {
            issues.push(ValidationIssue {
                severity: Severity::Warning,
                message: "Policy does not check schema_verified condition (required by project)"
                    .to_string(),
            });
        }
    }

    if constraints.require_approval_for_execute {
        let text_lower = cedar_text.to_lowercase();
        if text_lower.contains("execute") && !text_lower.contains("approved") {
            issues.push(ValidationIssue {
                severity: Severity::Warning,
                message: "Execute action present without approval condition (required by project)"
                    .to_string(),
            });
        }
    }

    Ok(issues)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_constraints() -> CedarConstraints {
        CedarConstraints {
            require_schema_verified: true,
            require_approval_for_execute: true,
            forbid_wildcard_principal: true,
            forbid_wildcard_resource_on: vec!["execute".to_string(), "delegate".to_string()],
        }
    }

    #[test]
    fn test_valid_policy() {
        let cedar = r#"
permit (
    principal == Agent::"Monitor",
    action == "read",
    resource == Tool::"healthcheck"
) when {
    context.schema_verified == true
};
"#;
        let issues = validate_cedar(cedar, &test_constraints()).unwrap();
        let errors: Vec<_> = issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .collect();
        assert!(errors.is_empty());
    }

    #[test]
    fn test_wildcard_principal_caught() {
        let cedar = r#"permit (principal, action == "read", resource == Tool::"x");"#;
        let issues = validate_cedar(cedar, &test_constraints()).unwrap();
        assert!(issues
            .iter()
            .any(|i| i.message.contains("Wildcard principal")));
    }

    #[test]
    fn test_missing_schema_verified() {
        let cedar = r#"permit (principal == Agent::"X", action == "read", resource == Tool::"y");"#;
        let issues = validate_cedar(cedar, &test_constraints()).unwrap();
        assert!(issues.iter().any(|i| i.message.contains("schema_verified")));
    }

    #[test]
    fn test_execute_without_approval() {
        let cedar = r#"
permit (
    principal == Agent::"X",
    action == "execute",
    resource == Tool::"y"
) when { context.schema_verified == true };
"#;
        let issues = validate_cedar(cedar, &test_constraints()).unwrap();
        assert!(issues.iter().any(|i| i.message.contains("approval")));
    }
}
