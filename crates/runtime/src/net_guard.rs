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

    // Prefer the typed `Host` API — for IPv6 literals, host_str() may
    // return a canonical compressed form that our downstream matchers don't
    // recognise, while host() gives us the concrete `Ipv6Addr`.
    if let Some(parsed_host) = parsed.host() {
        match parsed_host {
            url::Host::Ipv4(v4) => {
                if is_non_public_ip(IpAddr::V4(v4)) {
                    return Err(format!("SSRF: refusing non-public IPv4 {}", v4));
                }
                return Ok(());
            }
            url::Host::Ipv6(v6) => {
                let ip = IpAddr::V6(v6);
                if is_non_public_ip(ip) {
                    return Err(format!("SSRF: refusing non-public IPv6 {}", v6));
                }
                // IPv4-in-IPv6 forms: pull the low 32 bits if the upper
                // 96 bits are zero or ::ffff: and re-check against the
                // IPv4 blocklist.
                let s = v6.segments();
                let upper_zero = s[0] == 0 && s[1] == 0 && s[2] == 0 && s[3] == 0 && s[4] == 0;
                let looks_mapped = upper_zero && (s[5] == 0 || s[5] == 0xffff);
                if looks_mapped {
                    let a = (s[6] >> 8) as u8;
                    let b = (s[6] & 0xff) as u8;
                    let c = (s[7] >> 8) as u8;
                    let d = (s[7] & 0xff) as u8;
                    let embedded = std::net::Ipv4Addr::new(a, b, c, d);
                    if !embedded.is_unspecified() && is_non_public_ip(IpAddr::V4(embedded)) {
                        return Err(format!(
                            "SSRF: refusing IPv6 literal embedding non-public IPv4 {} ({})",
                            v6, embedded
                        ));
                    }
                }
                return Ok(());
            }
            url::Host::Domain(_) => {
                // Fall through to the hostname-based checks below.
            }
        }
    }

    if let Some(host) = parsed.host_str() {
        let host_lc = host.to_ascii_lowercase();

        // Hostnames — explicit blocklist plus any label that resolves to
        // localhost (e.g. `localhost.localdomain`, `localhost.example`).
        if matches!(
            host_lc.as_str(),
            "localhost" | "127.0.0.1" | "::1" | "[::1]" | "metadata.google.internal"
        ) || host_lc == "localhost."
            || host_lc.starts_with("localhost.")
        {
            return Err(format!("SSRF: refusing host '{}'", host));
        }
        if host_lc == "169.254.169.254" {
            return Err("SSRF: cloud metadata endpoint refused".to_string());
        }

        // Obfuscated IPv4 literal forms that `host.parse::<IpAddr>()` will
        // NOT match (dotted-decimal is the only form std::net accepts).
        // We normalise these into dotted-decimal so the private/loopback
        // checks below still fire for e.g. `http://127.1/`, `http://0/`,
        // `http://0x7f000001/`, `http://2130706433/`.
        if let Some(normalised) = normalise_legacy_ipv4(&host_lc) {
            let bad = is_bad_ipv4(&normalised);
            if bad {
                return Err(format!(
                    "SSRF: refusing obfuscated loopback/private IPv4 literal '{}' ({})",
                    host, normalised
                ));
            }
        }

        // IPv6 literals that embed a loopback or private IPv4 mapping (e.g.
        // `::127.0.0.1`, `::ffff:10.0.0.1`).
        if let Some(embedded_v4) = extract_embedded_ipv4(&host_lc) {
            if is_bad_ipv4(&embedded_v4) {
                return Err(format!(
                    "SSRF: refusing IPv6 literal embedding non-public IPv4 '{}'",
                    host
                ));
            }
        }

        if let Ok(ip) = host.parse::<IpAddr>() {
            let bad = is_non_public_ip(ip)
                // `Ipv6Addr::is_loopback` only recognises the canonical
                // `::1`, not IPv4-in-IPv6 forms like `::127.0.0.1` or
                // `::ffff:10.0.0.1`. Pull the embedded IPv4 out of any
                // IPv6 address whose upper 96 bits are zero or `::ffff:`
                // and re-check it against the standard IPv4 blocklist.
                || match ip {
                    IpAddr::V6(v6) => {
                        let s = v6.segments();
                        let upper_zero = s[0] == 0 && s[1] == 0 && s[2] == 0 && s[3] == 0 && s[4] == 0;
                        let looks_mapped = upper_zero && (s[5] == 0 || s[5] == 0xffff);
                        if looks_mapped {
                            let a = (s[6] >> 8) as u8;
                            let b = (s[6] & 0xff) as u8;
                            let c = (s[7] >> 8) as u8;
                            let d = (s[7] & 0xff) as u8;
                            let embedded = std::net::Ipv4Addr::new(a, b, c, d);
                            // Treat ::0.0.0.0 as unspecified rather than a
                            // valid IPv4 (the outer `is_non_public_ip` path
                            // already handled the ::1 loopback).
                            !embedded.is_unspecified()
                                && is_non_public_ip(IpAddr::V4(embedded))
                        } else {
                            false
                        }
                    }
                    IpAddr::V4(_) => false,
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

/// `true` iff `ip` is a private, loopback, link-local, broadcast, or
/// metadata address — i.e. not a public internet destination.
pub fn is_non_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || v4.is_documentation()
                // Explicit AWS/GCP metadata endpoint — `is_link_local` covers
                // this but we want a labelled log on hit.
                || v4.octets() == [169, 254, 169, 254]
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
        }
    }
}

// ---------------------------------------------------------------------------
// SSRF-safe HTTP client factory
// ---------------------------------------------------------------------------

/// DNS resolver for `reqwest` that refuses to return non-public IPs.
///
/// Prevents DNS-rebinding attacks: the lexical check in
/// [`reject_ssrf_url`] inspects the URL string at queue time, but the actual
/// HTTP connection resolves the hostname again at fetch time. A hostile DNS
/// record can respond with a public IP during the lexical check and then
/// switch to `10.0.0.1` (or any other RFC 1918 address) by the time the
/// reqwest connector asks for addresses. This resolver rejects the
/// resolution itself when any returned IP is non-public, closing that gap.
#[derive(Debug, Clone, Default)]
pub struct SsrfSafeResolver;

impl reqwest::dns::Resolve for SsrfSafeResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            // Off-load the synchronous `std::net::ToSocketAddrs` lookup to a
            // blocking task so we don't stall the reqwest runtime on slow
            // DNS.
            let join_result: Result<
                Result<Vec<std::net::SocketAddr>, std::io::Error>,
                tokio::task::JoinError,
            > = tokio::task::spawn_blocking(move || {
                use std::net::ToSocketAddrs;
                // reqwest passes a host without a port; pick 0 so we can
                // still drive getaddrinfo.
                let iter = (host.as_str(), 0u16).to_socket_addrs()?;
                let addrs: Vec<std::net::SocketAddr> = iter.collect();
                Ok::<_, std::io::Error>(addrs)
            })
            .await;

            let addrs: Vec<std::net::SocketAddr> = match join_result {
                Ok(Ok(v)) => v,
                Ok(Err(io_err)) => {
                    let boxed: Box<dyn std::error::Error + Send + Sync> = Box::new(io_err);
                    return Err(boxed);
                }
                Err(join_err) => {
                    let boxed: Box<dyn std::error::Error + Send + Sync> = Box::new(
                        std::io::Error::other(format!("dns join failed: {join_err}")),
                    );
                    return Err(boxed);
                }
            };

            // Drop every non-public address. If nothing public remains, refuse
            // the lookup so reqwest never gets a private IP to dial.
            let filtered: Vec<std::net::SocketAddr> = addrs
                .into_iter()
                .filter(|sa| !is_non_public_ip(sa.ip()))
                .collect();

            if filtered.is_empty() {
                let err: Box<dyn std::error::Error + Send + Sync> =
                    Box::<dyn std::error::Error + Send + Sync>::from(
                        "SSRF: hostname resolves only to non-public IPs — refusing".to_string(),
                    );
                return Err(err);
            }

            let boxed: Box<dyn Iterator<Item = std::net::SocketAddr> + Send> =
                Box::new(filtered.into_iter());
            Ok(reqwest::dns::Addrs::from(boxed))
        })
    }
}

/// Build a `reqwest::Client` that (a) rejects DNS lookups pointing at
/// private/loopback IPs and (b) disables automatic HTTP redirects.
///
/// The central factory is the mitigation for the DNS-rebinding class of
/// SSRF attacks: lexical URL checks don't survive a later DNS swap, but the
/// resolver filter does. Call sites that accept a URL from any non-fully-
/// trusted source (DSL config, env var, LLM tool output) should use this
/// factory rather than `reqwest::Client::new()`.
///
/// # Notes
/// - Redirects are disabled by default so a trusted endpoint can't bounce
///   requests to an internal target after the fact. Callers that need
///   redirect following should build their own `ClientBuilder` and compose
///   with `SsrfSafeResolver`.
/// - The connect / request timeout is caller-supplied; pick a tight value
///   (5–15 seconds) for user-facing flows.
pub fn build_ssrf_safe_client(
    timeout: std::time::Duration,
) -> Result<reqwest::Client, reqwest::Error> {
    use std::sync::Arc;
    reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .dns_resolver(Arc::new(SsrfSafeResolver))
        .build()
}

/// Same as [`build_ssrf_safe_client`] but allows the caller to extend the
/// default `ClientBuilder` (for example to set a custom redirect policy,
/// add default headers, or enable `gzip`). The SSRF-safe DNS resolver is
/// applied *after* the caller's customisations so it can't be swapped out;
/// other settings (including redirect policy) remain caller-controlled.
///
/// Redirects are safe to follow under this factory because every hop's DNS
/// goes through [`SsrfSafeResolver`], so a redirect to an internal host
/// will be rejected at resolve time just like the original request would be.
pub fn customise_ssrf_safe_client<F>(
    timeout: std::time::Duration,
    customise: F,
) -> Result<reqwest::Client, reqwest::Error>
where
    F: FnOnce(reqwest::ClientBuilder) -> reqwest::ClientBuilder,
{
    use std::sync::Arc;
    let builder = reqwest::Client::builder().timeout(timeout);
    let builder = customise(builder);
    builder.dns_resolver(Arc::new(SsrfSafeResolver)).build()
}

/// Normalise legacy IPv4 literal forms into dotted-decimal so the standard
/// `Ipv4Addr::is_*` checks can see through shorthand like `127.1` or
/// `0x7f000001`. Returns `None` when the input doesn't look like a legacy
/// IPv4 literal (e.g. when it's a DNS name).
fn normalise_legacy_ipv4(host: &str) -> Option<String> {
    // Fast path: already dotted-decimal 4-tuple.
    if host.parse::<std::net::Ipv4Addr>().is_ok() {
        return Some(host.to_string());
    }

    let parse_part = |s: &str| -> Option<u32> {
        if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            u32::from_str_radix(hex, 16).ok()
        } else if s.starts_with('0') && s.len() > 1 && s.chars().all(|c| c.is_ascii_digit()) {
            // Octal: 0-prefixed but only digits 0-7 are valid; permissively
            // accept any digit and let the range check below reject overflow.
            u32::from_str_radix(s, 8).ok()
        } else {
            s.parse::<u32>().ok()
        }
    };

    let parts: Vec<&str> = host.split('.').collect();
    match parts.len() {
        1 => parse_part(parts[0]).map(|n| {
            let o = n.to_be_bytes();
            format!("{}.{}.{}.{}", o[0], o[1], o[2], o[3])
        }),
        2 => {
            let a = parse_part(parts[0])?;
            let rest = parse_part(parts[1])?;
            if a > 0xff || rest > 0xff_ffff {
                return None;
            }
            let o = rest.to_be_bytes();
            Some(format!("{}.{}.{}.{}", a, o[1], o[2], o[3]))
        }
        3 => {
            let a = parse_part(parts[0])?;
            let b = parse_part(parts[1])?;
            let rest = parse_part(parts[2])?;
            if a > 0xff || b > 0xff || rest > 0xffff {
                return None;
            }
            let o = rest.to_be_bytes();
            Some(format!("{}.{}.{}.{}", a, b, o[2], o[3]))
        }
        4 => {
            let parsed: Option<Vec<u32>> = parts.iter().map(|p| parse_part(p)).collect();
            let vals = parsed?;
            if vals.iter().any(|v| *v > 0xff) {
                return None;
            }
            Some(format!("{}.{}.{}.{}", vals[0], vals[1], vals[2], vals[3]))
        }
        _ => None,
    }
}

fn is_bad_ipv4(dotted: &str) -> bool {
    if let Ok(v4) = dotted.parse::<std::net::Ipv4Addr>() {
        v4.is_loopback()
            || v4.is_private()
            || v4.is_link_local()
            || v4.is_broadcast()
            || v4.is_unspecified()
            || v4.is_documentation()
    } else {
        false
    }
}

/// Pull an embedded IPv4 address out of an IPv6 literal string (possibly
/// wrapped in `[...]`). Returns `None` when the input is not an IPv6 literal
/// or has no embedded IPv4 component.
fn extract_embedded_ipv4(host: &str) -> Option<String> {
    let s = host.trim_start_matches('[').trim_end_matches(']');
    // Only IPv6 candidates contain ':'; a DNS label will not.
    if !s.contains(':') {
        return None;
    }
    // Find the last ':' and see if what follows parses as an IPv4 literal.
    let tail = s.rsplit(':').next()?;
    if tail.parse::<std::net::Ipv4Addr>().is_ok() {
        Some(tail.to_string())
    } else {
        None
    }
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

    #[test]
    fn rejects_localhost_subdomains() {
        assert!(reject_ssrf_url("http://localhost.localdomain/").is_err());
        assert!(reject_ssrf_url("http://localhost.example/").is_err());
        assert!(reject_ssrf_url("http://LOCALHOST./").is_err());
    }

    #[test]
    fn rejects_shorthand_ipv4() {
        // `127.1` is accepted by most network stacks as 127.0.0.1
        assert!(reject_ssrf_url("http://127.1/").is_err());
        // Hex-encoded 127.0.0.1
        assert!(reject_ssrf_url("http://0x7f000001/").is_err());
        // Decimal packed form of 127.0.0.1 = 2130706433
        assert!(reject_ssrf_url("http://2130706433/").is_err());
        // Hex shorthand for 10.0.0.1 (private)
        assert!(reject_ssrf_url("http://0xa000001/").is_err());
    }

    #[test]
    fn rejects_ipv6_with_embedded_loopback() {
        assert!(reject_ssrf_url("http://[::127.0.0.1]/").is_err());
        assert!(reject_ssrf_url("http://[::ffff:10.0.0.1]/").is_err());
    }

    #[test]
    fn normalise_legacy_ipv4_examples() {
        assert_eq!(normalise_legacy_ipv4("127.1").as_deref(), Some("127.0.0.1"));
        assert_eq!(
            normalise_legacy_ipv4("0x7f000001").as_deref(),
            Some("127.0.0.1")
        );
        assert_eq!(
            normalise_legacy_ipv4("2130706433").as_deref(),
            Some("127.0.0.1")
        );
        // 192.168.1 -> 192.168.0.1
        assert_eq!(
            normalise_legacy_ipv4("192.168.1").as_deref(),
            Some("192.168.0.1")
        );
        // Non-IP strings return None
        assert!(normalise_legacy_ipv4("example.com").is_none());
    }
}
