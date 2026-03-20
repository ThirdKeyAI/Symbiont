//! Scope enforcement for ToolClad arguments
//!
//! Validates scope_target, ip_address, cidr, and url arguments
//! against a project scope definition (scope/scope.toml).

use std::net::IpAddr;
use std::path::Path;

/// Project scope definition.
#[derive(Debug, Clone, Default)]
pub struct Scope {
    pub targets: Vec<String>,
    pub domains: Vec<String>,
    pub exclude: Vec<String>,
}

impl Scope {
    /// Load scope from scope/scope.toml if it exists.
    pub fn load(project_dir: &Path) -> Option<Self> {
        let path = project_dir.join("scope").join("scope.toml");
        if !path.exists() {
            return None;
        }
        let content = std::fs::read_to_string(&path).ok()?;
        let table: toml::Table = toml::from_str(&content).ok()?;
        let scope = table.get("scope")?;

        let targets = scope
            .get("targets")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let domains = scope
            .get("domains")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let exclude = scope
            .get("exclude")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        Some(Scope {
            targets,
            domains,
            exclude,
        })
    }

    /// Check if a target (IP, CIDR, or hostname) is within scope.
    pub fn check(&self, target: &str) -> Result<(), String> {
        // Check exclusions first
        if self.exclude.contains(&target.to_string()) {
            return Err(format!(
                "Target '{}' is explicitly excluded from scope",
                target
            ));
        }

        // Try as IP address
        if let Ok(ip) = target.parse::<IpAddr>() {
            return self.check_ip(ip, target);
        }

        // Try as CIDR
        if target.contains('/') {
            let parts: Vec<&str> = target.split('/').collect();
            if let Ok(ip) = parts[0].parse::<IpAddr>() {
                return self.check_ip(ip, target);
            }
        }

        // Treat as hostname
        self.check_hostname(target)
    }

    fn check_ip(&self, ip: IpAddr, original: &str) -> Result<(), String> {
        for scope_target in &self.targets {
            if scope_target.contains('/') {
                // CIDR range check
                if ip_in_cidr(ip, scope_target) {
                    return Ok(());
                }
            } else if let Ok(scope_ip) = scope_target.parse::<IpAddr>() {
                if ip == scope_ip {
                    return Ok(());
                }
            }
        }
        Err(format!(
            "Target '{}' is not in scope (allowed: {})",
            original,
            self.targets.join(", ")
        ))
    }

    fn check_hostname(&self, hostname: &str) -> Result<(), String> {
        for domain in &self.domains {
            if domain.starts_with("*.") {
                let suffix = &domain[1..]; // .example.com
                if hostname.ends_with(suffix) || hostname == &domain[2..] {
                    return Ok(());
                }
            } else if hostname == domain {
                return Ok(());
            }
        }
        // Also check if hostname matches any target string exactly
        if self.targets.contains(&hostname.to_string()) {
            return Ok(());
        }
        Err(format!(
            "Target '{}' is not in scope (allowed domains: {})",
            hostname,
            self.domains.join(", ")
        ))
    }
}

/// Check if an IP address falls within a CIDR range.
fn ip_in_cidr(ip: IpAddr, cidr: &str) -> bool {
    let parts: Vec<&str> = cidr.split('/').collect();
    if parts.len() != 2 {
        return false;
    }
    let Ok(network) = parts[0].parse::<IpAddr>() else {
        return false;
    };
    let Ok(prefix_len) = parts[1].parse::<u32>() else {
        return false;
    };

    match (ip, network) {
        (IpAddr::V4(ip4), IpAddr::V4(net4)) => {
            if prefix_len > 32 {
                return false;
            }
            let mask = if prefix_len == 0 {
                0u32
            } else {
                !0u32 << (32 - prefix_len)
            };
            let ip_bits = u32::from(ip4);
            let net_bits = u32::from(net4);
            (ip_bits & mask) == (net_bits & mask)
        }
        (IpAddr::V6(ip6), IpAddr::V6(net6)) => {
            if prefix_len > 128 {
                return false;
            }
            let ip_bits = u128::from(ip6);
            let net_bits = u128::from(net6);
            let mask = if prefix_len == 0 {
                0u128
            } else {
                !0u128 << (128 - prefix_len)
            };
            (ip_bits & mask) == (net_bits & mask)
        }
        _ => false, // Mismatched IP versions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ip_in_cidr() {
        assert!(ip_in_cidr("10.0.1.5".parse().unwrap(), "10.0.1.0/24"));
        assert!(ip_in_cidr("10.0.1.255".parse().unwrap(), "10.0.1.0/24"));
        assert!(!ip_in_cidr("10.0.2.1".parse().unwrap(), "10.0.1.0/24"));
        assert!(ip_in_cidr("192.168.0.1".parse().unwrap(), "192.168.0.0/16"));
    }

    #[test]
    fn test_scope_check_ip() {
        let scope = Scope {
            targets: vec!["10.0.1.0/24".into(), "192.168.1.0/24".into()],
            domains: vec![],
            exclude: vec!["10.0.1.1".into()],
        };
        assert!(scope.check("10.0.1.5").is_ok());
        assert!(scope.check("10.0.1.1").is_err()); // excluded
        assert!(scope.check("10.0.2.1").is_err()); // out of range
        assert!(scope.check("192.168.1.100").is_ok());
    }

    #[test]
    fn test_scope_check_hostname() {
        let scope = Scope {
            targets: vec![],
            domains: vec!["example.com".into(), "*.test.example.com".into()],
            exclude: vec![],
        };
        assert!(scope.check("example.com").is_ok());
        assert!(scope.check("foo.test.example.com").is_ok());
        assert!(scope.check("test.example.com").is_ok());
        assert!(scope.check("evil.com").is_err());
    }

    #[test]
    fn test_scope_check_cidr_target() {
        let scope = Scope {
            targets: vec!["10.0.1.0/24".into()],
            domains: vec![],
            exclude: vec![],
        };
        // CIDR target — check the network address
        assert!(scope.check("10.0.1.0/28").is_ok());
        assert!(scope.check("10.0.2.0/24").is_err());
    }
}
