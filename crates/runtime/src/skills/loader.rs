use schemapin::pinning::KeyPinStore;
use schemapin::skill::{load_signature, parse_skill_name, verify_skill_offline};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::config::SkillsConfig;
use super::scanner::{ScanResult, SkillScanner};

/// Verification status of a skill's cryptographic signature.
#[derive(Debug, Clone)]
pub enum SignatureStatus {
    Verified {
        domain: String,
        developer: Option<String>,
    },
    Pinned {
        domain: String,
        developer: Option<String>,
    },
    Unsigned,
    Invalid {
        reason: String,
    },
    Revoked {
        reason: String,
    },
}

impl std::fmt::Display for SignatureStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignatureStatus::Verified { domain, .. } => write!(f, "Verified ({})", domain),
            SignatureStatus::Pinned { domain, .. } => write!(f, "Pinned ({})", domain),
            SignatureStatus::Unsigned => write!(f, "Unsigned"),
            SignatureStatus::Invalid { reason } => write!(f, "Invalid: {}", reason),
            SignatureStatus::Revoked { reason } => write!(f, "Revoked: {}", reason),
        }
    }
}

/// A skill that has been loaded and optionally verified.
#[derive(Debug, Clone)]
pub struct LoadedSkill {
    pub name: String,
    pub path: PathBuf,
    pub signature_status: SignatureStatus,
    pub content: String,
    pub metadata: SkillMetadata,
    pub scan_result: Option<ScanResult>,
}

/// Parsed frontmatter metadata from a SKILL.md file.
#[derive(Debug, Clone)]
pub struct SkillMetadata {
    pub name: String,
    pub description: Option<String>,
    pub raw_frontmatter: HashMap<String, String>,
}

/// Errors that can occur during skill loading.
#[derive(Debug, thiserror::Error)]
pub enum SkillLoadError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("SKILL.md not found in {0}")]
    MissingSkillMd(PathBuf),
    #[error("Signature error: {0}")]
    Signature(String),
}

/// Loads skills from configured paths, verifies signatures, and runs content scanning.
pub struct SkillLoader {
    config: SkillsConfig,
    pin_store: KeyPinStore,
    scanner: Option<SkillScanner>,
}

impl SkillLoader {
    /// Create a new skill loader with the given configuration.
    pub fn new(config: SkillsConfig) -> Self {
        let scanner = if config.scan_enabled {
            let custom_rules = config
                .custom_deny_patterns
                .iter()
                .map(|p| super::scanner::ScanRule::DenyContentPattern(p.clone()))
                .collect();
            Some(SkillScanner::with_custom_rules(custom_rules))
        } else {
            None
        };

        Self {
            config,
            pin_store: KeyPinStore::new(),
            scanner,
        }
    }

    /// Load all skills from configured paths.
    pub fn load_all(&mut self) -> Vec<LoadedSkill> {
        let mut skills = Vec::new();

        for load_path in self.config.load_paths.clone() {
            if !load_path.exists() || !load_path.is_dir() {
                continue;
            }

            if let Ok(entries) = std::fs::read_dir(&load_path) {
                for entry in entries.filter_map(Result::ok) {
                    let path = entry.path();
                    if !path.is_dir() {
                        continue;
                    }
                    // Each subdirectory is a potential skill
                    if path.join("SKILL.md").exists() {
                        match self.load_skill(&path) {
                            Ok(skill) => skills.push(skill),
                            Err(e) => {
                                tracing::warn!("Failed to load skill at {:?}: {}", path, e);
                            }
                        }
                    }
                }
            }
        }

        skills
    }

    /// Load a single skill from a directory.
    pub fn load_skill(&mut self, path: &Path) -> Result<LoadedSkill, SkillLoadError> {
        let skill_md = path.join("SKILL.md");
        if !skill_md.exists() {
            return Err(SkillLoadError::MissingSkillMd(path.to_path_buf()));
        }

        let content = std::fs::read_to_string(&skill_md)?;
        let name = parse_skill_name(path);
        let metadata = parse_frontmatter(&content, &name);
        let signature_status = self.verify_skill(path);

        let scan_result = self.scanner.as_ref().map(|s| s.scan_skill(path));

        Ok(LoadedSkill {
            name,
            path: path.to_path_buf(),
            signature_status,
            content,
            metadata,
            scan_result,
        })
    }

    /// Verify the signature of a skill directory.
    pub fn verify_skill(&mut self, path: &Path) -> SignatureStatus {
        // Try to load the signature file
        let sig = match load_signature(path) {
            Ok(sig) => sig,
            Err(_) => {
                // Check if this path is allowed unsigned
                if self.is_unsigned_allowed(path) {
                    return SignatureStatus::Unsigned;
                }
                if self.config.require_signed {
                    return SignatureStatus::Invalid {
                        reason: "No signature file (.schemapin.sig) found".into(),
                    };
                }
                return SignatureStatus::Unsigned;
            }
        };

        let domain = &sig.domain;

        // Build a discovery document from the signature for offline verification.
        // In a full deployment, you'd resolve via DNS or trust bundle.
        // For offline verification, we need a WellKnownResponse with the public key.
        // Since we don't have a resolver here, we attempt offline verification
        // using a minimal discovery document. In practice, the CLI verify command
        // will accept a --domain flag and resolve properly.

        // For TOFU: use verify_skill_offline with pin_store
        let pin_store = if self.config.auto_pin {
            Some(&mut self.pin_store)
        } else {
            None
        };

        let tool_id = sig.skill_name.clone();

        // Without a resolver, we can still check if the signature file exists
        // and if the skill has been previously pinned. Full verification
        // requires a discovery document with the public key.
        // Return Pinned status if we have a pinned key for this tool+domain.
        if let Some(store) = &pin_store {
            if store.get_tool(&tool_id, domain).is_some() {
                return SignatureStatus::Pinned {
                    domain: domain.clone(),
                    developer: None,
                };
            }
        }

        // Without a discovery document, we can't fully verify.
        // Mark as having a signature but unverified (needs resolver).
        SignatureStatus::Invalid {
            reason: format!(
                "Signature found for domain '{}' but no discovery document available for offline verification",
                domain
            ),
        }
    }

    /// Verify a skill with a provided discovery document (for CLI use).
    pub fn verify_skill_with_discovery(
        &mut self,
        path: &Path,
        discovery: &schemapin::types::discovery::WellKnownResponse,
    ) -> SignatureStatus {
        let sig = match load_signature(path) {
            Ok(sig) => sig,
            Err(_) => {
                return SignatureStatus::Invalid {
                    reason: "No signature file (.schemapin.sig) found".into(),
                };
            }
        };

        let tool_id = sig.skill_name.clone();

        let pin_store = if self.config.auto_pin {
            Some(&mut self.pin_store)
        } else {
            None
        };

        let result = verify_skill_offline(
            path,
            discovery,
            Some(&sig),
            None, // No revocation document for now
            pin_store,
            Some(&tool_id),
        );

        if result.valid {
            let domain = result.domain.clone().unwrap_or_default();
            let developer = result.developer_name.clone();

            if let Some(ref pin_status) = result.key_pinning {
                if pin_status.status == "first_use" {
                    return SignatureStatus::Pinned { domain, developer };
                }
            }

            SignatureStatus::Verified { domain, developer }
        } else {
            let reason = result
                .error_message
                .unwrap_or_else(|| "Verification failed".into());

            if result
                .error_code
                .map(|c| c == schemapin::error::ErrorCode::KeyRevoked)
                .unwrap_or(false)
            {
                SignatureStatus::Revoked { reason }
            } else {
                SignatureStatus::Invalid { reason }
            }
        }
    }

    /// Check if a path is exempted from signing requirements.
    fn is_unsigned_allowed(&self, path: &Path) -> bool {
        for allowed in &self.config.allow_unsigned_from {
            if let (Ok(canonical_allowed), Ok(canonical_path)) =
                (std::fs::canonicalize(allowed), std::fs::canonicalize(path))
            {
                if canonical_path.starts_with(&canonical_allowed) {
                    return true;
                }
            }
            // Fallback: simple prefix check for relative paths
            if path.starts_with(allowed) {
                return true;
            }
        }
        false
    }
}

/// Parse YAML frontmatter from a SKILL.md file.
fn parse_frontmatter(content: &str, fallback_name: &str) -> SkillMetadata {
    let mut raw_frontmatter = HashMap::new();
    let mut name = fallback_name.to_string();
    let mut description = None;

    // Check for YAML frontmatter delimited by ---
    if let Some(after_open) = content.strip_prefix("---") {
        if let Some(end) = after_open.find("---") {
            let fm_content = &after_open[..end];
            for line in fm_content.lines() {
                let line = line.trim();
                if let Some(idx) = line.find(':') {
                    let key = line[..idx].trim().to_string();
                    let value = line[idx + 1..].trim().to_string();
                    if key == "name" && !value.is_empty() {
                        name = value.clone();
                    }
                    if key == "description" && !value.is_empty() {
                        description = Some(value.clone());
                    }
                    raw_frontmatter.insert(key, value);
                }
            }
        }
    }

    SkillMetadata {
        name,
        description,
        raw_frontmatter,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_frontmatter_with_name() {
        let content = "---\nname: my-skill\ndescription: A test skill\n---\n# Content";
        let meta = parse_frontmatter(content, "fallback");
        assert_eq!(meta.name, "my-skill");
        assert_eq!(meta.description.as_deref(), Some("A test skill"));
    }

    #[test]
    fn parse_frontmatter_fallback() {
        let content = "# No frontmatter here";
        let meta = parse_frontmatter(content, "fallback");
        assert_eq!(meta.name, "fallback");
        assert!(meta.description.is_none());
    }

    #[test]
    fn parse_frontmatter_empty() {
        let content = "---\n---\n# Empty frontmatter";
        let meta = parse_frontmatter(content, "fallback");
        assert_eq!(meta.name, "fallback");
    }

    #[test]
    fn load_skill_from_tempdir() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("test-skill");
        std::fs::create_dir(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: test-skill\ndescription: A test\n---\n# Test Skill\nHello.",
        )
        .unwrap();

        let config = SkillsConfig {
            load_paths: vec![],
            require_signed: false,
            allow_unsigned_from: vec![dir.path().to_path_buf()],
            auto_pin: false,
            scan_enabled: true,
            custom_deny_patterns: vec![],
        };
        let mut loader = SkillLoader::new(config);
        let skill = loader.load_skill(&skill_dir).unwrap();
        assert_eq!(skill.name, "test-skill");
        assert!(matches!(skill.signature_status, SignatureStatus::Unsigned));
        assert!(skill.scan_result.is_some());
        assert!(skill.scan_result.unwrap().passed);
    }

    #[test]
    fn load_skill_missing_skill_md() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("empty-skill");
        std::fs::create_dir(&skill_dir).unwrap();

        let config = SkillsConfig::default();
        let mut loader = SkillLoader::new(config);
        assert!(loader.load_skill(&skill_dir).is_err());
    }

    #[test]
    fn load_all_from_empty_paths() {
        let config = SkillsConfig {
            load_paths: vec![PathBuf::from("/nonexistent/path")],
            require_signed: false,
            allow_unsigned_from: vec![],
            auto_pin: false,
            scan_enabled: false,
            custom_deny_patterns: vec![],
        };
        let mut loader = SkillLoader::new(config);
        let skills = loader.load_all();
        assert!(skills.is_empty());
    }

    #[test]
    fn load_all_discovers_skills() {
        let dir = tempfile::tempdir().unwrap();
        // Create two skill directories
        for name in &["skill-a", "skill-b"] {
            let skill_dir = dir.path().join(name);
            std::fs::create_dir(&skill_dir).unwrap();
            std::fs::write(
                skill_dir.join("SKILL.md"),
                format!("---\nname: {}\n---\n# {}", name, name),
            )
            .unwrap();
        }

        let config = SkillsConfig {
            load_paths: vec![dir.path().to_path_buf()],
            require_signed: false,
            allow_unsigned_from: vec![dir.path().to_path_buf()],
            auto_pin: false,
            scan_enabled: false,
            custom_deny_patterns: vec![],
        };
        let mut loader = SkillLoader::new(config);
        let skills = loader.load_all();
        assert_eq!(skills.len(), 2);
    }

    #[test]
    fn unsigned_allowed_check() {
        let dir = tempfile::tempdir().unwrap();
        let allowed = dir.path().join("allowed");
        std::fs::create_dir(&allowed).unwrap();
        let skill_dir = allowed.join("my-skill");
        std::fs::create_dir(&skill_dir).unwrap();

        let config = SkillsConfig {
            load_paths: vec![],
            require_signed: true,
            allow_unsigned_from: vec![allowed.clone()],
            auto_pin: false,
            scan_enabled: false,
            custom_deny_patterns: vec![],
        };
        let loader = SkillLoader::new(config);
        assert!(loader.is_unsigned_allowed(&skill_dir));
    }
}
