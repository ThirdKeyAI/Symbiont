//! Change Validator - Validates changes for safety and correctness

use anyhow::Result;
use std::path::Path;

/// Validates changes for safety, syntax, and logical correctness
pub struct ChangeValidator {
    safety_checks_enabled: bool,
    syntax_checks_enabled: bool,
    dependency_checks_enabled: bool,
}

impl ChangeValidator {
    pub fn new(
        _safety_checks_enabled: bool,
        _syntax_checks_enabled: bool,
        _dependency_checks_enabled: bool,
    ) -> Self {
        Self {
            safety_checks_enabled: _safety_checks_enabled,
            syntax_checks_enabled: _syntax_checks_enabled,
            dependency_checks_enabled: _dependency_checks_enabled,
        }
    }

    pub async fn validate_changes(&self, changes: &[FileChange]) -> Result<ValidationReport> {
        let mut safety_issues = vec![];
        let mut syntax_errors = vec![];
        let mut dependency_issues = vec![];

        for change in changes {
            if self.safety_checks_enabled {
                let safety = self.check_safety(&Path::new(&change.file_path), change.content.as_ref().unwrap_or(&String::new())).await?;
                safety_issues.extend(safety.issues);
            }
            if self.syntax_checks_enabled && change.content.is_some() {
                let syntax = self.validate_syntax(&Path::new(&change.file_path), change.content.as_ref().unwrap()).await?;
                syntax_errors.extend(syntax.errors);
            }
        }

        let dependency = if self.dependency_checks_enabled {
            self.check_dependencies(changes).await?
        } else {
            DependencyReport { has_issues: false, issues: vec![] }
        };
        dependency_issues.extend(dependency.issues);

        let impact = self.analyze_impact(changes).await?;

        let is_safe = safety_issues.is_empty() && syntax_errors.is_empty() && dependency_issues.is_empty() && impact.risk_level != RiskLevel::Critical;

        Ok(ValidationReport {
            is_safe,
            safety_issues,
            syntax_errors,
            dependency_issues,
            impact_analysis: impact,
        })
    }

    pub async fn check_safety(&self, file_path: &Path, content: &str) -> Result<SafetyReport> {
        let mut issues = vec![];

        // Check for common safety issues
        if content.contains("unsafe {") {
            issues.push(SafetyIssue {
                severity: Severity::High,
                description: "Use of unsafe block detected".to_string(),
                file_path: file_path.to_string_lossy().to_string(),
                line_number: None,
            });
        }
        if content.contains("panic!") {
            issues.push(SafetyIssue {
                severity: Severity::Medium,
                description: "Potential panic point".to_string(),
                file_path: file_path.to_string_lossy().to_string(),
                line_number: None,
            });
        }

        Ok(SafetyReport {
            is_safe: issues.is_empty(),
            issues,
        })
    }

    pub async fn validate_syntax(&self, file_path: &Path, content: &str) -> Result<SyntaxReport> {
        let mut errors = vec![];

        // Simple syntax checks; in real scenario, use language-specific parser
        if file_path.extension().and_then(|s| s.to_str()) == Some("rs") {
            // Basic Rust syntax checks (simulated)
            if !content.contains("fn main") && content.contains("pub fn") {
                // Missing main or something; dummy check
            }
            if content.matches("{").count() != content.matches("}").count() {
                errors.push(SyntaxError {
                    description: "Mismatched braces".to_string(),
                    file_path: file_path.to_string_lossy().to_string(),
                    line_number: None,
                    column_number: None,
                });
            }
        }

        Ok(SyntaxReport {
            is_valid: errors.is_empty(),
            errors,
        })
    }

    pub async fn check_dependencies(&self, changes: &[FileChange]) -> Result<DependencyReport> {
        let mut issues = vec![];

        // Simulated dependency check
        for change in changes {
            if let Some(content) = &change.content {
                if content.contains("extern crate obsolete;") {
                    issues.push(DependencyIssue {
                        severity: Severity::High,
                        description: "Obsolete dependency detected".to_string(),
                        affected_files: vec![change.file_path.clone()],
                    });
                }
            }
        }

        Ok(DependencyReport {
            has_issues: !issues.is_empty(),
            issues,
        })
    }

    pub async fn analyze_impact(&self, changes: &[FileChange]) -> Result<ImpactAnalysis> {
        let mut affected = vec![];
        let mut breaking = vec![];
        let mut recommendations = vec![];

        for change in changes {
            affected.push(change.file_path.clone());
            if change.change_type == ChangeType::Delete {
                breaking.push(format!("Deletion of {}", change.file_path));
                recommendations.push("Check for dependent code".to_string());
            }
        }

        let risk_level = if breaking.is_empty() { RiskLevel::Low } else { RiskLevel::High };

        Ok(ImpactAnalysis {
            risk_level,
            affected_components: affected,
            breaking_changes: breaking,
            recommendations,
        })
    }
}

/// Re-export FileChange from modifier module
pub use crate::git_tools::FileChange;

/// Comprehensive validation report
pub struct ValidationReport {
    pub is_safe: bool,
    pub safety_issues: Vec<SafetyIssue>,
    pub syntax_errors: Vec<SyntaxError>,
    pub dependency_issues: Vec<DependencyIssue>,
    pub impact_analysis: ImpactAnalysis,
}

/// Safety-related issues
pub struct SafetyReport {
    pub is_safe: bool,
    pub issues: Vec<SafetyIssue>,
}

/// Individual safety issue
pub struct SafetyIssue {
    pub severity: Severity,
    pub description: String,
    pub file_path: String,
    pub line_number: Option<usize>,
}

/// Syntax validation report
pub struct SyntaxReport {
    pub is_valid: bool,
    pub errors: Vec<SyntaxError>,
}

/// Individual syntax error
pub struct SyntaxError {
    pub description: String,
    pub file_path: String,
    pub line_number: Option<usize>,
    pub column_number: Option<usize>,
}

/// Dependency validation report
pub struct DependencyReport {
    pub has_issues: bool,
    pub issues: Vec<DependencyIssue>,
}

/// Individual dependency issue
pub struct DependencyIssue {
    pub severity: Severity,
    pub description: String,
    pub affected_files: Vec<String>,
}

/// Impact analysis of proposed changes
pub struct ImpactAnalysis {
    pub risk_level: RiskLevel,
    pub affected_components: Vec<String>,
    pub breaking_changes: Vec<String>,
    pub recommendations: Vec<String>,
}

/// Severity levels for issues
#[derive(Debug, Clone)]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

/// Risk levels for impact analysis
#[derive(Debug, Clone)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}