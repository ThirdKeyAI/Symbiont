//! Secrets injection for ToolClad templates
//!
//! Replaces `{_secret:name}` placeholders with values from environment
//! variables. Integration with Vault/OpenBao is deferred to the Symbiont
//! secrets backend.

/// Replace all `{_secret:name}` placeholders in a string with resolved values.
///
/// Resolution order:
/// 1. `TOOLCLAD_SECRET_{NAME}` environment variable (uppercase)
/// 2. Error if not found
pub fn inject_secrets(template: &str) -> Result<String, String> {
    let re = regex::Regex::new(r"\{_secret:([a-zA-Z0-9_]+)\}").unwrap();
    let mut result = template.to_string();
    for cap in re.captures_iter(template) {
        let full_match = &cap[0];
        let secret_name = &cap[1];
        let env_key = format!("TOOLCLAD_SECRET_{}", secret_name.to_uppercase());
        let value = std::env::var(&env_key).map_err(|_| {
            format!(
                "Secret '{}' not found (set {} environment variable)",
                secret_name, env_key
            )
        })?;
        result = result.replace(full_match, &value);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_secrets() {
        assert_eq!(inject_secrets("hello world").unwrap(), "hello world");
    }

    #[test]
    fn test_inject_from_env() {
        std::env::set_var("TOOLCLAD_SECRET_TEST_TOKEN", "abc123");
        let result = inject_secrets("Bearer {_secret:test_token}").unwrap();
        assert_eq!(result, "Bearer abc123");
        std::env::remove_var("TOOLCLAD_SECRET_TEST_TOKEN");
    }

    #[test]
    fn test_missing_secret() {
        std::env::remove_var("TOOLCLAD_SECRET_NONEXISTENT");
        assert!(inject_secrets("{_secret:nonexistent}").is_err());
    }
}
