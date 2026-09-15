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

    /// Validate the entire rule set even before a destination is available.
    pub fn validate(&self) -> Result<(), String> {
        if self.allowed_domains.len() + self.blocked_domains.len() > 256 {
            return Err("browser scope has too many domain rules".into());
        }
        for rule in self.allowed_domains.iter().chain(&self.blocked_domains) {
            canonical_domain(rule.strip_prefix("*.").unwrap_or(rule))?;
        }
        Ok(())
    }

    /// Check if a URL is allowed by scope rules.
    pub fn check_url(&self, url: &str) -> Result<(), String> {
        let domain =
            extract_domain(url).ok_or_else(|| format!("Cannot extract domain from: {}", url))?;
        self.check_domain(&domain)
    }

    /// Check if a domain is allowed.
    pub fn check_domain(&self, domain: &str) -> Result<(), String> {
        let domain = canonical_domain(domain)?;
        if self.allowed_domains.len() + self.blocked_domains.len() > 256 {
            return Err("browser scope has too many domain rules".into());
        }
        let rules = |values: &[String]| {
            values
                .iter()
                .map(|rule| match rule.strip_prefix("*.") {
                    Some(domain) => canonical_domain(domain).map(|domain| format!("*.{domain}")),
                    None => canonical_domain(rule),
                })
                .collect::<Result<Vec<_>, _>>()
        };
        let blocked_domains = rules(&self.blocked_domains)?;
        let allowed_domains = rules(&self.allowed_domains)?;
        // Check blocked first
        for blocked in &blocked_domains {
            if domain_matches(&domain, blocked) {
                return Err(format!(
                    "Domain '{}' is blocked by scope rule '{}'",
                    domain, blocked
                ));
            }
        }

        // If no allowed list, check allow_external
        if allowed_domains.is_empty() {
            return if self.allow_external {
                Ok(())
            } else {
                Err("No allowed domains configured and allow_external is false".to_string())
            };
        }

        // Check allowed
        for allowed in &allowed_domains {
            if domain_matches(&domain, allowed) {
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

/// Parse the same URL representation used by the browser and HTTP broker.
/// This is a lexical check; the network broker must also enforce DNS/IP scope.
pub(super) fn parse_browser_url(value: &str) -> Result<url::Url, String> {
    if value.len() > 8192
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b'\\')
        || value.trim() != value
    {
        return Err("browser URL contains ambiguous characters or exceeds its limit".into());
    }
    let parsed = url::Url::parse(value).map_err(|_| "invalid browser URL")?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("browser URL requires HTTP(S), a host and no credentials".into());
    }
    canonical_domain(parsed.host_str().unwrap())?;
    Ok(parsed)
}

fn canonical_domain(value: &str) -> Result<String, String> {
    let host = url::Host::parse(value).map_err(|_| "invalid browser scope domain")?;
    let value = host.to_string();
    if let url::Host::Domain(_) = host {
        let value = value.strip_suffix('.').unwrap_or(&value);
        if value.is_empty()
            || value.len() > 253
            || value.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || !label
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            })
        {
            return Err("invalid browser scope domain".into());
        }
        Ok(value.into())
    } else {
        Ok(value)
    }
}

fn extract_domain(url: &str) -> Option<String> {
    let parsed = parse_browser_url(url).ok()?;
    canonical_domain(parsed.host_str()?).ok()
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
        assert!(checker.check_url("HTTPS://ADMIN.EXAMPLE.COM./").is_err());
        assert!(checker.check_url("https://ADMIN%2eEXAMPLE.COM/").is_err());
    }

    #[test]
    fn browser_url_and_scope_rules_use_canonical_hosts() {
        let mut checker = BrowserScopeChecker::new(&BrowserScopeDef {
            allowed_domains: vec!["EXAMPLE.COM.".into(), "bücher.example".into()],
            blocked_domains: vec![],
            allow_external: false,
        });
        assert!(checker
            .check_url("https://example.com/?next=https://other.test")
            .is_ok());
        assert!(checker.check_url("https://xn--bcher-kva.example/").is_ok());
        for url in [
            "file://example.com/etc/passwd",
            "javascript://example.com/1",
            "https://example.com@evil.test/",
            "https://user@example.com/",
            "https://example.com\\@evil.test/",
            " https://example.com/",
            "https://exam\nple.com/",
            "https://example.com../",
        ] {
            assert!(checker.check_url(url).is_err(), "{url}");
        }
        checker.allow_external = true;
        checker.blocked_domains = vec!["example.com/path".into()];
        assert!(checker.check_url("https://other.test/").is_err());
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
