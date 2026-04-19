#![allow(dead_code)]

use anyhow::Result;
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ProjectConstraints {
    #[serde(default)]
    pub constraints: ConstraintRules,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ConstraintRules {
    pub max_memory: Option<String>,
    pub max_cpu: Option<f64>,
    pub max_tier: Option<String>,
    pub min_security_tier: Option<String>,
    pub required_sandbox: Option<String>,
    pub min_audit_level: Option<String>,
    #[serde(default)]
    pub forbidden_capabilities: Vec<String>,
    pub max_cron_frequency: Option<String>,
    #[serde(default)]
    pub cedar: CedarConstraints,
    #[serde(default)]
    pub toolclad: ToolcladConstraints,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CedarConstraints {
    #[serde(default)]
    pub require_schema_verified: bool,
    #[serde(default)]
    pub require_approval_for_execute: bool,
    #[serde(default)]
    pub forbid_wildcard_principal: bool,
    #[serde(default)]
    pub forbid_wildcard_resource_on: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ToolcladConstraints {
    pub max_risk_tier: Option<String>,
    pub require_evidence_above_tier: Option<String>,
    #[serde(default)]
    pub require_scope_check: bool,
}

impl ProjectConstraints {
    /// Load constraints from a TOML file. Returns defaults if file doesn't exist.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path)?;
        let constraints: ProjectConstraints = toml::from_str(&content)?;
        Ok(constraints)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_load_default_when_missing() {
        let constraints = ProjectConstraints::load(Path::new("/nonexistent")).unwrap();
        assert!(constraints.constraints.forbidden_capabilities.is_empty());
    }

    #[test]
    fn test_load_from_toml() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
[constraints]
max_memory = "512MB"
max_cpu = 2.0
max_tier = "Tier2"
required_sandbox = "strict"
forbidden_capabilities = ["network_raw", "filesystem_write_root"]

[constraints.cedar]
forbid_wildcard_principal = true

[constraints.toolclad]
max_risk_tier = "medium"
require_scope_check = true
"#
        )
        .unwrap();

        let constraints = ProjectConstraints::load(f.path()).unwrap();
        assert_eq!(constraints.constraints.max_memory.as_deref(), Some("512MB"));
        assert_eq!(constraints.constraints.max_cpu, Some(2.0));
        assert!(constraints.constraints.cedar.forbid_wildcard_principal);
        assert!(constraints.constraints.toolclad.require_scope_check);
        assert_eq!(constraints.constraints.forbidden_capabilities.len(), 2);
    }
}
