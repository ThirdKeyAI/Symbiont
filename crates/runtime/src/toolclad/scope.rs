//! Scope enforcement for ToolClad network arguments.
//!
//! Scope restricts the entire requested IP range or canonical hostname.
//! DNS/egress enforcement must additionally bind a hostname to its actual
//! connection; this lexical check does not grant unrestricted network access.

use serde::Deserialize;
use std::net::IpAddr;
use std::path::Path;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default)]
    pub domains: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

impl Scope {
    /// Missing scope is distinct from an unreadable or malformed scope.
    pub fn load(project_dir: &Path) -> Result<Option<Self>, String> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Document {
            scope: Scope,
        }
        let path = project_dir.join("scope/scope.toml");
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // A dangling symlink is an invalid configured scope, not absence.
                if std::fs::symlink_metadata(&path).is_ok() {
                    return Err(format!("Cannot read scope {}: {error}", path.display()));
                }
                return Ok(None);
            }
            Err(error) => return Err(format!("Cannot read scope {}: {error}", path.display())),
        };
        let document: Document = toml::from_str(&text)
            .map_err(|error| format!("Invalid scope {}: {error}", path.display()))?;
        document.scope.validate()?;
        Ok(Some(document.scope))
    }

    pub fn validate(&self) -> Result<(), String> {
        for entry in self
            .targets
            .iter()
            .chain(&self.domains)
            .chain(&self.exclude)
        {
            Rule::parse(entry)?;
        }
        Ok(())
    }

    pub fn check(&self, target: &str) -> Result<(), String> {
        self.validate()?;
        let requested = Rule::destination(target)?;
        for exclusion in &self.exclude {
            if Rule::parse(exclusion)?.overlaps(&requested) {
                return Err(format!(
                    "Target '{target}' intersects excluded scope '{exclusion}'"
                ));
            }
        }
        for allowed in self.targets.iter().chain(&self.domains) {
            if Rule::parse(allowed)?.contains(&requested) {
                return Ok(());
            }
        }
        Err(format!("Target '{target}' is not in scope"))
    }
}

#[derive(Debug)]
enum Rule {
    Network { v4: bool, first: u128, last: u128 },
    Domain { name: String, descendants: bool },
}

impl Rule {
    fn destination(value: &str) -> Result<Self, String> {
        if value.contains("://") {
            let url = url::Url::parse(value).map_err(|e| format!("Invalid scope URL: {e}"))?;
            if !matches!(url.scheme(), "http" | "https") {
                return Err("Scope URL must use HTTP or HTTPS".into());
            }
            let host = url.host().ok_or("Scope URL has no host")?;
            return Self::parse(&host.to_string());
        }
        if value.starts_with("*.") {
            return Err("A requested destination cannot contain a wildcard".into());
        }
        Self::parse(value)
    }

    fn parse(value: &str) -> Result<Self, String> {
        if value.is_empty() || value.trim() != value || value.contains("://") {
            return Err(format!("Invalid scope entry '{value}'"));
        }
        if let Some((address, prefix)) = value.split_once('/') {
            let ip: IpAddr = address
                .parse()
                .map_err(|_| format!("Invalid CIDR '{value}'"))?;
            let prefix = prefix
                .parse::<u32>()
                .map_err(|_| format!("Invalid CIDR prefix '{value}'"))?;
            return Self::network(ip, prefix);
        }
        if let Ok(ip) = value.parse::<IpAddr>() {
            return Self::network(ip, if ip.is_ipv4() { 32 } else { 128 });
        }
        let (hostname, descendants) = match value.strip_prefix("*.") {
            Some(name) => (name, true),
            None => (value, false),
        };
        // URL's Host parser canonicalizes IDNA names and legacy IPv4 forms.
        let host =
            url::Host::parse(hostname).map_err(|e| format!("Invalid scope host '{value}': {e}"))?;
        match host {
            url::Host::Ipv4(ip) if !descendants => Self::network(IpAddr::V4(ip), 32),
            url::Host::Ipv6(ip) if !descendants => Self::network(IpAddr::V6(ip), 128),
            url::Host::Domain(name) => {
                let name = name.strip_suffix('.').unwrap_or(&name).to_ascii_lowercase();
                if name.len() > 253
                    || name.split('.').any(|label| {
                        label.is_empty()
                            || label.len() > 63
                            || label.starts_with('-')
                            || label.ends_with('-')
                            || !label
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    })
                {
                    return Err(format!("Invalid scope domain '{value}'"));
                }
                Ok(Self::Domain { name, descendants })
            }
            _ => Err(format!("Wildcard IP scope is invalid: '{value}'")),
        }
    }

    fn network(ip: IpAddr, prefix: u32) -> Result<Self, String> {
        let (v4, bits, width) = match ip {
            IpAddr::V4(ip) => (true, u32::from(ip) as u128, 32),
            IpAddr::V6(ip) => {
                if let Some(v4) = ip.to_ipv4_mapped() {
                    if prefix < 96 {
                        return Err("IPv4-mapped CIDR prefix must be at least 96".into());
                    }
                    return Self::network(IpAddr::V4(v4), prefix - 96);
                }
                (false, u128::from(ip), 128)
            }
        };
        if prefix > width {
            return Err(format!("Invalid CIDR prefix {prefix}"));
        }
        let host_mask = if width - prefix == 128 {
            u128::MAX
        } else {
            (1u128 << (width - prefix)) - 1
        };
        Ok(Self::Network {
            v4,
            first: bits & !host_mask,
            last: bits | host_mask,
        })
    }

    fn contains(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Network { v4, first, last },
                Self::Network {
                    v4: other_v4,
                    first: other_first,
                    last: other_last,
                },
            ) => v4 == other_v4 && first <= other_first && last >= other_last,
            (
                Self::Domain { name, descendants },
                Self::Domain {
                    name: other_name, ..
                },
            ) => name == other_name || (*descendants && other_name.ends_with(&format!(".{name}"))),
            _ => false,
        }
    }

    fn overlaps(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Network { v4, first, last },
                Self::Network {
                    v4: other_v4,
                    first: other_first,
                    last: other_last,
                },
            ) => v4 == other_v4 && first <= other_last && last >= other_first,
            _ => self.contains(other) || other.contains(self),
        }
    }
}

#[cfg(test)]
fn ip_in_cidr(ip: IpAddr, cidr: &str) -> bool {
    let prefix = if ip.is_ipv4() { 32 } else { 128 };
    matches!((Rule::parse(cidr), Rule::network(ip, prefix)), (Ok(rule), Ok(target)) if rule.contains(&target))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entire_requested_range_must_fit_and_avoid_every_exclusion() {
        let scope = Scope {
            targets: vec!["10.0.1.0/24".into(), "2001:db8:1::/48".into()],
            exclude: vec!["10.0.1.200".into(), "2001:db8:1:8000::/49".into()],
            ..Default::default()
        };
        for target in [
            "10.0.1.0/8",
            "10.0.1.0/24",
            "10.0.1.199/28",
            "2001:db8:1::/32",
            "2001:db8:1::/48",
            "10.0.1.0/33",
            "10.0.1.0/not-a-prefix",
        ] {
            assert!(scope.check(target).is_err(), "{target}");
        }
        for target in [
            "10.0.1.0/28",
            "10.0.1.5",
            "2001:db8:1::/64",
            "::ffff:10.0.1.5",
        ] {
            assert!(scope.check(target).is_ok(), "{target}");
        }
    }

    #[test]
    fn host_exclusions_use_canonical_names_and_label_boundaries() {
        let scope = Scope {
            domains: vec!["*.example.com".into()],
            exclude: vec!["*.private.example.com".into()],
            ..Default::default()
        };
        for target in [
            "https://API.EXAMPLE.COM./path",
            "example.com",
            "a.example.com",
        ] {
            assert!(scope.check(target).is_ok(), "{target}");
        }
        for target in [
            "https://example.com@evil.com/",
            "PRIVATE.EXAMPLE.COM.",
            "https://a.private.example.com/",
            "badexample.com",
            "*.example.com",
            "file://example.com/data",
        ] {
            assert!(scope.check(target).is_err(), "{target}");
        }
    }

    #[test]
    fn malformed_config_cannot_be_treated_as_missing_or_partial_scope() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Scope::load(dir.path()).unwrap().is_none());
        std::fs::create_dir(dir.path().join("scope")).unwrap();
        let path = dir.path().join("scope/scope.toml");
        for content in [
            "[scope]\ntargets = ['10.0.0.0/8', 5]",
            "[scope]\ntargets = ['10.0.0.0/99']",
            "[scope]\ndomains = ['*example.com']",
            "[scope]\nexlcude = ['example.com']",
        ] {
            std::fs::write(&path, content).unwrap();
            assert!(Scope::load(dir.path()).is_err(), "{content}");
        }
    }

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
