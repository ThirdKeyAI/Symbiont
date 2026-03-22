//! Browser state types for CDP-based browser sessions.

use serde::{Deserialize, Serialize};

/// Page state inferred from CDP inspection.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PageState {
    pub url: String,
    pub title: String,
    pub domain: String,
    pub has_forms: bool,
    pub is_authenticated: bool,
    pub page_loaded: bool,
    pub tab_count: u32,
}

/// Browser lifecycle status.
#[derive(Debug, Clone, PartialEq)]
pub enum BrowserStatus {
    Connecting,
    Ready,
    Busy,
    TimedOut,
    Terminated,
}

/// Tab info from Chrome debug endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabInfo {
    pub id: String,
    pub url: String,
    pub title: String,
    #[serde(rename = "type")]
    pub tab_type: String,
    #[serde(rename = "webSocketDebuggerUrl")]
    pub ws_url: Option<String>,
}

/// Browser scope checker -- validates URLs against allowed/blocked domains.
pub struct BrowserScopeChecker {
    pub allowed_domains: Vec<String>,
    pub blocked_domains: Vec<String>,
    pub allow_external: bool,
}

impl BrowserScopeChecker {
    pub fn new(scope: &super::manifest::BrowserScopeDef) -> Self {
        Self {
            allowed_domains: scope.allowed_domains.clone(),
            blocked_domains: scope.blocked_domains.clone(),
            allow_external: scope.allow_external,
        }
    }

    /// Check if a URL is allowed by scope rules.
    pub fn check_url(&self, url: &str) -> Result<(), String> {
        let domain =
            extract_domain(url).ok_or_else(|| format!("Cannot extract domain from: {}", url))?;
        self.check_domain(&domain)
    }

    /// Check if a domain is allowed.
    pub fn check_domain(&self, domain: &str) -> Result<(), String> {
        // Check blocked first
        for blocked in &self.blocked_domains {
            if domain_matches(domain, blocked) {
                return Err(format!(
                    "Domain '{}' is blocked by scope rule '{}'",
                    domain, blocked
                ));
            }
        }

        // If no allowed list, check allow_external
        if self.allowed_domains.is_empty() {
            return if self.allow_external {
                Ok(())
            } else {
                Err("No allowed domains configured and allow_external is false".to_string())
            };
        }

        // Check allowed
        for allowed in &self.allowed_domains {
            if domain_matches(domain, allowed) {
                return Ok(());
            }
        }

        if self.allow_external {
            Ok(())
        } else {
            Err(format!(
                "Domain '{}' not in allowed domains: {}",
                domain,
                self.allowed_domains.join(", ")
            ))
        }
    }
}

/// Extract domain from a URL.
fn extract_domain(url: &str) -> Option<String> {
    let after_scheme = url.split("://").nth(1)?;
    let domain = after_scheme.split('/').next()?;
    let domain = domain.split(':').next()?; // strip port
    Some(domain.to_string())
}

/// Check if a domain matches a pattern (supports wildcard *.example.com).
fn domain_matches(domain: &str, pattern: &str) -> bool {
    if pattern.starts_with("*.") {
        let suffix = &pattern[1..]; // .example.com
        domain.ends_with(suffix) || domain == &pattern[2..]
    } else {
        domain == pattern
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolclad::manifest::BrowserScopeDef;

    #[test]
    fn test_extract_domain() {
        assert_eq!(
            extract_domain("https://example.com/path"),
            Some("example.com".to_string())
        );
        assert_eq!(
            extract_domain("http://localhost:8080/"),
            Some("localhost".to_string())
        );
        assert_eq!(extract_domain("not-a-url"), None);
    }

    #[test]
    fn test_domain_matches_exact() {
        assert!(domain_matches("example.com", "example.com"));
        assert!(!domain_matches("other.com", "example.com"));
    }

    #[test]
    fn test_domain_matches_wildcard() {
        assert!(domain_matches("sub.example.com", "*.example.com"));
        assert!(domain_matches("example.com", "*.example.com"));
        assert!(!domain_matches("evil.com", "*.example.com"));
    }

    #[test]
    fn test_scope_checker_allowed() {
        let scope = BrowserScopeDef {
            allowed_domains: vec!["*.example.com".to_string()],
            blocked_domains: vec![],
            allow_external: false,
        };
        let checker = BrowserScopeChecker::new(&scope);
        assert!(checker.check_url("https://app.example.com/page").is_ok());
        assert!(checker.check_url("https://evil.com/page").is_err());
    }

    #[test]
    fn test_scope_checker_blocked() {
        let scope = BrowserScopeDef {
            allowed_domains: vec!["*.example.com".to_string()],
            blocked_domains: vec!["admin.example.com".to_string()],
            allow_external: false,
        };
        let checker = BrowserScopeChecker::new(&scope);
        assert!(checker.check_url("https://app.example.com").is_ok());
        assert!(checker.check_url("https://admin.example.com").is_err());
    }

    #[test]
    fn test_scope_checker_allow_external() {
        let scope = BrowserScopeDef {
            allowed_domains: vec!["example.com".to_string()],
            blocked_domains: vec![],
            allow_external: true,
        };
        let checker = BrowserScopeChecker::new(&scope);
        assert!(checker.check_url("https://example.com").is_ok());
        assert!(checker.check_url("https://other.com").is_ok()); // allow_external
    }

    #[test]
    fn test_scope_checker_no_external() {
        let scope = BrowserScopeDef {
            allowed_domains: vec![],
            blocked_domains: vec![],
            allow_external: false,
        };
        let checker = BrowserScopeChecker::new(&scope);
        assert!(checker.check_url("https://any.com").is_err());
    }

    #[test]
    fn test_page_state_default() {
        let ps = PageState::default();
        assert!(!ps.has_forms);
        assert!(!ps.is_authenticated);
        assert!(ps.url.is_empty());
    }
}
