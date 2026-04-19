#![allow(dead_code)]

pub mod cedar_validator;
pub mod constraints;
pub mod diff;
pub mod dsl_validator;
pub mod toolclad_validator;

use constraints::ProjectConstraints;
use dsl_validator::{Severity, ValidationIssue};

/// The type of artifact being validated.
#[derive(Debug, Clone, Copy)]
pub enum ArtifactKind {
    Dsl,
    Cedar,
    Toolclad,
}

/// Full validation result for display to the user.
pub struct ValidationResult {
    pub kind: ArtifactKind,
    pub issues: Vec<ValidationIssue>,
    pub diff_lines: Vec<diff::DiffLine>,
    pub has_errors: bool,
    pub has_escalations: bool,
}

/// Run the full validation pipeline on an LLM-generated artifact.
pub fn validate_artifact(
    kind: ArtifactKind,
    new_text: &str,
    old_text: Option<&str>,
    constraints: &ProjectConstraints,
) -> anyhow::Result<ValidationResult> {
    let issues = match kind {
        ArtifactKind::Dsl => dsl_validator::validate_dsl(new_text, &constraints.constraints)?,
        ArtifactKind::Cedar => {
            cedar_validator::validate_cedar(new_text, &constraints.constraints.cedar)?
        }
        ArtifactKind::Toolclad => {
            toolclad_validator::validate_toolclad(new_text, &constraints.constraints.toolclad)?
        }
    };

    let diff_lines = match old_text {
        Some(old) => diff::artifact_diff(old, new_text),
        None => diff::artifact_diff("", new_text),
    };

    let has_errors = issues.iter().any(|i| i.severity == Severity::Error);
    let has_escalations = diff_lines
        .iter()
        .any(|d| d.kind == diff::DiffKind::Escalation);

    Ok(ValidationResult {
        kind,
        issues,
        diff_lines,
        has_errors,
        has_escalations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_valid_dsl() {
        let constraints = ProjectConstraints::default();
        let result = validate_artifact(
            ArtifactKind::Dsl,
            r#"agent Test { name: "Test" version: "1.0" }"#,
            None,
            &constraints,
        )
        .unwrap();
        assert!(!result.has_errors);
    }

    #[test]
    fn test_pipeline_detects_escalation_in_diff() {
        let constraints = ProjectConstraints::default();
        let result = validate_artifact(
            ArtifactKind::Cedar,
            "permit(principal, action, resource);",
            Some(""),
            &constraints,
        )
        .unwrap();
        assert!(result.has_escalations);
    }
}
