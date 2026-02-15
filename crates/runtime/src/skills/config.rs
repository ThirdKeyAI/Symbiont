use std::path::PathBuf;

/// Configuration for the skill loader and scanner.
#[derive(Debug, Clone)]
pub struct SkillsConfig {
    /// Directories to search for skills.
    pub load_paths: Vec<PathBuf>,
    /// Reject unsigned skills globally.
    pub require_signed: bool,
    /// Paths exempt from the signing requirement (e.g. repo-local skills).
    pub allow_unsigned_from: Vec<PathBuf>,
    /// Automatically TOFU-pin on first use.
    pub auto_pin: bool,
    /// Enable content scanning (ClawHavoc defense).
    pub scan_enabled: bool,
    /// User-defined deny patterns (regex strings).
    pub custom_deny_patterns: Vec<String>,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        let repo_local = PathBuf::from(".agents/skills");

        let mut user_paths: Vec<PathBuf> = Vec::new();
        if let Some(home) = dirs::home_dir() {
            user_paths.push(home.join(".claude/skills"));
            user_paths.push(home.join(".codex/skills"));
            user_paths.push(home.join(".symbiont/skills"));
        }

        let mut load_paths = vec![repo_local.clone()];
        load_paths.extend(user_paths);

        Self {
            load_paths,
            require_signed: true,
            allow_unsigned_from: vec![repo_local],
            auto_pin: true,
            scan_enabled: true,
            custom_deny_patterns: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_includes_repo_local() {
        let cfg = SkillsConfig::default();
        assert!(cfg
            .load_paths
            .iter()
            .any(|p| p == &PathBuf::from(".agents/skills")));
    }

    #[test]
    fn repo_local_allows_unsigned() {
        let cfg = SkillsConfig::default();
        assert!(cfg
            .allow_unsigned_from
            .contains(&PathBuf::from(".agents/skills")));
    }

    #[test]
    fn default_enables_scanning() {
        let cfg = SkillsConfig::default();
        assert!(cfg.scan_enabled);
        assert!(cfg.require_signed);
        assert!(cfg.auto_pin);
    }
}
