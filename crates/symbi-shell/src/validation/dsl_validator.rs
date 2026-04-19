#![allow(dead_code)]

use anyhow::Result;

use super::constraints::ConstraintRules;

/// Validation issue found in a DSL artifact.
#[derive(Debug, Clone)]
pub struct ValidationIssue {
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// Validate a DSL artifact string against project constraints.
pub fn validate_dsl(dsl_text: &str, constraints: &ConstraintRules) -> Result<Vec<ValidationIssue>> {
    let mut issues = Vec::new();

    // Text-based constraint checks run FIRST, regardless of parse success.
    // A forbidden capability in the raw text is a policy violation even if the DSL is malformed.
    let text_lower = dsl_text.to_lowercase();
    for cap in &constraints.forbidden_capabilities {
        if text_lower.contains(&cap.to_lowercase()) {
            issues.push(ValidationIssue {
                severity: Severity::Error,
                message: format!("Forbidden capability referenced: {}", cap),
            });
        }
    }

    // Check sandbox requirement
    if let Some(ref required_sandbox) = constraints.required_sandbox {
        if text_lower.contains("sandbox") {
            let required_lower = required_sandbox.to_lowercase();
            let weak_modes = match required_lower.as_str() {
                "strict" => vec!["permissive", "moderate"],
                "moderate" => vec!["permissive"],
                _ => vec![],
            };
            for weak in weak_modes {
                if text_lower.contains(weak) {
                    issues.push(ValidationIssue {
                        severity: Severity::Error,
                        message: format!(
                            "Sandbox mode '{}' is weaker than required '{}'",
                            weak, required_sandbox
                        ),
                    });
                }
            }
        }
    }

    // Parse the DSL to verify syntax (after text checks so constraint violations are always caught)
    let mut lexer = repl_core::dsl::lexer::Lexer::new(dsl_text);
    let tokens = match lexer.tokenize() {
        Ok(t) => t,
        Err(e) => {
            issues.push(ValidationIssue {
                severity: Severity::Error,
                message: format!("Syntax error: {}", e),
            });
            return Ok(issues);
        }
    };

    let mut parser = repl_core::dsl::parser::Parser::new(tokens);
    if let Err(e) = parser.parse() {
        issues.push(ValidationIssue {
            severity: Severity::Error,
            message: format!("Parse error: {}", e),
        });
    }

    Ok(issues)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::constraints::ConstraintRules;

    fn test_constraints() -> ConstraintRules {
        ConstraintRules {
            forbidden_capabilities: vec!["network_raw".to_string()],
            required_sandbox: Some("strict".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn test_valid_dsl_passes() {
        let dsl = r#"agent TestAgent { name: "Test" version: "1.0" }"#;
        let issues = validate_dsl(dsl, &test_constraints()).unwrap();
        assert!(issues.is_empty(), "Expected no issues, got: {:?}", issues);
    }

    #[test]
    fn test_syntax_error_caught() {
        let dsl = "this is not valid dsl {{{}}}";
        let issues = validate_dsl(dsl, &test_constraints()).unwrap();
        assert!(!issues.is_empty());
        assert_eq!(issues[0].severity, Severity::Error);
    }

    #[test]
    fn test_forbidden_capability_caught() {
        let dsl = r#"agent Bad { name: "Bad" version: "1.0" capabilities { network_raw } }"#;
        let issues = validate_dsl(dsl, &test_constraints()).unwrap();
        let cap_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.message.contains("Forbidden capability"))
            .collect();
        assert!(!cap_issues.is_empty());
    }

    #[test]
    fn test_weak_sandbox_caught() {
        let dsl = r#"agent Bad { name: "Bad" version: "1.0" sandbox: permissive }"#;
        let issues = validate_dsl(dsl, &test_constraints()).unwrap();
        let sandbox_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.message.contains("Sandbox mode"))
            .collect();
        assert!(!sandbox_issues.is_empty());
    }
}
