//! Network-guardrail helpers shared across outbound-HTTP call sites.
//!
//! Centralises SSRF (private-IP / loopback / cloud-metadata) and
//! TLS/loopback-policy checks so every caller using `reqwest` or `ureq`
//! against a user- or manifest-supplied URL has the same filter.

use std::net::IpAddr;

/// Reject URLs that would allow SSRF to private ranges, loopback, cloud
/// metadata services, link-local, or non-http(s) schemes.
///
/// Returns `Ok(())` when the URL is safe to fetch, `Err(reason)` otherwise.
/// Only performs lexical checks — DNS resolution is not performed here, so
/// callers that want hardening against DNS rebinding must also pin the
/// resolved IP or use a pre-resolved IP in the URL.
pub fn reject_ssrf_url(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("Invalid URL '{}': {}", url, e))?;

    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!(
            "SSRF: only http/https schemes allowed, got '{}'",
            parsed.scheme()
        ));
    }

    if let Some(host) = parsed.host_str() {
        if matches!(
            host,
            "localhost" | "127.0.0.1" | "::1" | "[::1]" | "metadata.google.internal"
        ) {
            return Err(format!("SSRF: refusing host '{}'", host));
        }
        if host == "169.254.169.254" {
            return Err("SSRF: cloud metadata endpoint refused".to_string());
        }

        if let Ok(ip) = host.parse::<IpAddr>() {
            let bad = match ip {
                IpAddr::V4(v4) => {
                    v4.is_loopback()
                        || v4.is_private()
                        || v4.is_link_local()
                        || v4.is_broadcast()
                        || v4.is_unspecified()
                        || v4.is_documentation()
                }
                IpAddr::V6(v6) => {
                    v6.is_loopback()
                        || v6.is_unspecified()
                        || v6.is_unique_local()
                        || v6.is_unicast_link_local()
                }
            };
            if bad {
                return Err(format!("SSRF: refusing non-public IP {}", ip));
            }
        }
    } else {
        return Err("SSRF: URL has no host".to_string());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_loopback() {
        assert!(reject_ssrf_url("http://127.0.0.1/pub").is_err());
        assert!(reject_ssrf_url("https://localhost").is_err());
        assert!(reject_ssrf_url("http://[::1]:8080").is_err());
    }

    #[test]
    fn rejects_metadata() {
        assert!(reject_ssrf_url("http://169.254.169.254/latest").is_err());
        assert!(reject_ssrf_url("http://metadata.google.internal").is_err());
    }

    #[test]
    fn rejects_private() {
        assert!(reject_ssrf_url("http://10.0.0.5/x").is_err());
        assert!(reject_ssrf_url("http://192.168.1.1").is_err());
        assert!(reject_ssrf_url("http://172.16.0.3").is_err());
    }

    #[test]
    fn rejects_non_http_scheme() {
        assert!(reject_ssrf_url("file:///etc/passwd").is_err());
        assert!(reject_ssrf_url("gopher://x").is_err());
    }

    #[test]
    fn rejects_url_that_fails_to_parse() {
        // Not-a-URL is rejected at parse time.
        assert!(reject_ssrf_url("not a url").is_err());
    }

    #[test]
    fn allows_public_https() {
        assert!(reject_ssrf_url("https://example.com/pub").is_ok());
        assert!(reject_ssrf_url("https://8.8.8.8").is_ok());
    }
}
