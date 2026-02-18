//! HTTP middleware for the API server
//!
//! This module contains middleware implementations for request processing,
//! authentication, rate limiting, and other cross-cutting concerns.

#[cfg(feature = "http-api")]
use axum::{extract::Request, http::StatusCode, middleware::Next, response::Response};

#[cfg(feature = "http-api")]
use subtle::ConstantTimeEq;

#[cfg(feature = "http-api")]
use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter,
};

#[cfg(feature = "http-api")]
use std::{
    net::{IpAddr, SocketAddr},
    num::NonZeroU32,
    sync::{Arc, OnceLock},
};

#[cfg(feature = "http-api")]
use dashmap::DashMap;

#[cfg(feature = "http-api")]
use axum::extract::ConnectInfo;

#[cfg(feature = "http-api")]
use std::env;

/// A single CIDR range used for trusted proxy matching.
#[cfg(feature = "http-api")]
#[derive(Debug, Clone)]
struct TrustedProxyCidr {
    addr: IpAddr,
    prefix_len: u8,
}

#[cfg(feature = "http-api")]
impl TrustedProxyCidr {
    fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if let Some((addr_str, prefix_str)) = s.split_once('/') {
            let addr = addr_str.parse::<IpAddr>().ok()?;
            let prefix_len = prefix_str.parse::<u8>().ok()?;
            let max_prefix = if addr.is_ipv4() { 32 } else { 128 };
            if prefix_len > max_prefix {
                return None;
            }
            Some(Self { addr, prefix_len })
        } else {
            let addr = s.parse::<IpAddr>().ok()?;
            let prefix_len = if addr.is_ipv4() { 32 } else { 128 };
            Some(Self { addr, prefix_len })
        }
    }

    fn contains(&self, ip: &IpAddr) -> bool {
        match (self.addr, ip) {
            (IpAddr::V4(net), IpAddr::V4(candidate)) => {
                if self.prefix_len == 0 {
                    return true;
                }
                if self.prefix_len >= 32 {
                    return net == *candidate;
                }
                let mask = u32::MAX << (32 - self.prefix_len);
                (u32::from(net) & mask) == (u32::from(*candidate) & mask)
            }
            (IpAddr::V6(net), IpAddr::V6(candidate)) => {
                if self.prefix_len == 0 {
                    return true;
                }
                if self.prefix_len >= 128 {
                    return net == *candidate;
                }
                let mask = u128::MAX << (128 - self.prefix_len);
                (u128::from(net) & mask) == (u128::from(*candidate) & mask)
            }
            _ => false,
        }
    }
}

/// Set of trusted proxy CIDRs. Only requests originating from these addresses
/// will have their `X-Forwarded-For` / `X-Real-IP` headers respected.
#[cfg(feature = "http-api")]
#[derive(Debug)]
struct TrustedProxies {
    cidrs: Vec<TrustedProxyCidr>,
}

#[cfg(feature = "http-api")]
impl TrustedProxies {
    fn is_trusted(&self, ip: &IpAddr) -> bool {
        self.cidrs.iter().any(|cidr| cidr.contains(ip))
    }
}

#[cfg(feature = "http-api")]
static TRUSTED_PROXIES: OnceLock<TrustedProxies> = OnceLock::new();

/// Initialize the trusted proxy configuration from the `SYMBIONT_TRUSTED_PROXIES`
/// environment variable. The value should be a comma-separated list of IP
/// addresses or CIDR ranges (e.g. `"127.0.0.1,10.0.0.0/8,172.16.0.0/12"`).
///
/// If the variable is unset or empty, **no** proxies are trusted and forwarded
/// headers (`X-Forwarded-For`, `X-Real-IP`) are always ignored — the connecting
/// IP is used directly for rate limiting and logging.
#[cfg(feature = "http-api")]
pub(crate) fn init_trusted_proxies() {
    TRUSTED_PROXIES.get_or_init(|| {
        let cidrs: Vec<TrustedProxyCidr> = env::var("SYMBIONT_TRUSTED_PROXIES")
            .unwrap_or_default()
            .split(',')
            .filter_map(|s| {
                let s = s.trim();
                if s.is_empty() {
                    return None;
                }
                match TrustedProxyCidr::parse(s) {
                    Some(cidr) => {
                        tracing::info!("Trusted proxy: {}", s);
                        Some(cidr)
                    }
                    None => {
                        tracing::warn!("Invalid trusted proxy entry, skipping: {}", s);
                        None
                    }
                }
            })
            .collect();

        if cidrs.is_empty() {
            tracing::info!(
                "No trusted proxies configured — forwarded headers will be ignored. \
                 Set SYMBIONT_TRUSTED_PROXIES to trust proxy headers."
            );
        }

        TrustedProxies { cidrs }
    });
}

/// Authentication middleware for bearer token validation.
///
/// Authentication strategy (fail-closed):
///
/// 1. If an [`ApiKeyStore`](super::api_keys::ApiKeyStore) extension is present
///    **and** contains at least one record, authentication is performed
///    exclusively against the key store. The legacy env-var path is skipped
///    entirely so a leaked static token cannot bypass per-agent controls.
///
/// 2. If no key store is configured (or it is empty), the middleware falls
///    back to the `SYMBIONT_API_TOKEN` environment variable with
///    constant-time comparison. A deprecation warning is emitted on every
///    successful legacy auth to encourage migration.
///
/// 3. If neither mechanism can authenticate the request, `401 Unauthorized`
///    is returned.
#[cfg(feature = "http-api")]
pub async fn auth_middleware(request: Request, next: Next) -> Result<Response, StatusCode> {
    let auth_value = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    if !auth_value.starts_with("Bearer ") {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let token = &auth_value[7..];

    // --- Primary path: per-agent API key store ---
    let key_store: Option<Arc<super::api_keys::ApiKeyStore>> = request
        .extensions()
        .get::<Arc<super::api_keys::ApiKeyStore>>()
        .cloned();

    if let Some(store) = &key_store {
        if store.has_records() {
            // Key store is the sole authority — do NOT fall through to the
            // legacy env-var token. This prevents a leaked static token from
            // bypassing per-agent, rotatable, Argon2-hashed keys.
            return match store.validate_key(token) {
                Some(validated) => {
                    tracing::info!(
                        "Authenticated via API key store: key_id={}",
                        validated.key_id
                    );
                    Ok(next.run(request).await)
                }
                None => {
                    tracing::warn!("Authentication failed: key not found in API key store");
                    Err(StatusCode::UNAUTHORIZED)
                }
            };
        }
    }

    // --- Legacy fallback: static SYMBIONT_API_TOKEN env var ---
    // Only reachable when no key store with records is configured.
    let expected_token = env::var("SYMBIONT_API_TOKEN").map_err(|_| {
        tracing::error!(
            "No API key store configured and SYMBIONT_API_TOKEN not set — \
             all requests will be rejected. Configure an API key store or set \
             SYMBIONT_API_TOKEN for development."
        );
        StatusCode::UNAUTHORIZED
    })?;

    if !bool::from(token.as_bytes().ct_eq(expected_token.as_bytes())) {
        tracing::warn!("Authentication failed: invalid token provided");
        return Err(StatusCode::UNAUTHORIZED);
    }

    tracing::warn!(
        "Authenticated via legacy SYMBIONT_API_TOKEN — this is deprecated. \
         Migrate to the API key store (--api-keys-file) for per-agent keys, \
         Argon2 hashing, and key rotation."
    );
    Ok(next.run(request).await)
}

/// Global rate limiter store for per-IP rate limiting
#[cfg(feature = "http-api")]
type IpRateLimiter = Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>;
static RATE_LIMITERS: OnceLock<DashMap<IpAddr, IpRateLimiter>> = OnceLock::new();

/// Get or create a rate limiter for a specific IP address
#[cfg(feature = "http-api")]
fn get_rate_limiter_for_ip(ip: IpAddr) -> Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>> {
    let limiters = RATE_LIMITERS.get_or_init(DashMap::new);

    // Check if limiter exists, if not create one
    if let Some(limiter) = limiters.get(&ip) {
        Arc::clone(&limiter)
    } else {
        // Create a rate limiter: 100 requests per minute (roughly 1.67 requests per second)
        let quota = Quota::per_minute(NonZeroU32::new(100).unwrap());
        let limiter = Arc::new(RateLimiter::direct(quota));
        limiters.insert(ip, Arc::clone(&limiter));
        limiter
    }
}

/// Extract the client IP address from a request.
///
/// Uses Axum's [`ConnectInfo`] to obtain the real connecting IP. Forwarded
/// headers (`X-Forwarded-For`, `X-Real-IP`) are only respected when the
/// connecting IP belongs to a trusted proxy (see [`init_trusted_proxies`]).
/// This prevents attackers from spoofing their IP to bypass rate limiting
/// when the server is directly exposed to the internet.
///
/// Returns `None` when the connecting IP cannot be determined **and** no
/// forwarded headers are available from a trusted proxy. Callers should
/// reject these requests rather than falling back to a shared default
/// bucket (which would be a DoS vector).
#[cfg(feature = "http-api")]
fn extract_client_ip(request: &Request) -> Option<IpAddr> {
    let connecting_ip: Option<IpAddr> = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip());

    let from_trusted_proxy = connecting_ip
        .as_ref()
        .and_then(|ip| TRUSTED_PROXIES.get().map(|tp| tp.is_trusted(ip)))
        .unwrap_or(false);

    if from_trusted_proxy {
        // Connection is from a trusted proxy — respect forwarded headers.
        // Take the rightmost X-Forwarded-For entry (appended by our proxy).
        if let Some(forwarded_for) = request.headers().get("x-forwarded-for") {
            if let Ok(forwarded_str) = forwarded_for.to_str() {
                if let Some(last_ip) = forwarded_str.split(',').next_back() {
                    if let Ok(ip) = last_ip.trim().parse::<IpAddr>() {
                        return Some(ip);
                    }
                }
            }
        }

        // Try X-Real-IP header
        if let Some(real_ip) = request.headers().get("x-real-ip") {
            if let Ok(real_ip_str) = real_ip.to_str() {
                if let Ok(ip) = real_ip_str.parse::<IpAddr>() {
                    return Some(ip);
                }
            }
        }
    } else if request.headers().contains_key("x-forwarded-for")
        || request.headers().contains_key("x-real-ip")
    {
        tracing::debug!(
            connecting_ip = ?connecting_ip,
            "Ignoring forwarded headers from untrusted source. \
             Set SYMBIONT_TRUSTED_PROXIES to trust proxy headers.",
        );
    }

    connecting_ip
}

/// Rate limiting middleware using token bucket algorithm
///
/// This middleware implements per-IP rate limiting with a token bucket algorithm.
/// Each IP address gets 100 requests per minute (approximately 1.67 RPS).
///
/// Rate limiters are stored in a global concurrent HashMap and are created
/// on-demand for each unique IP address.
///
/// If the client IP cannot be determined the request is rejected with
/// `400 Bad Request` to avoid funnelling unknown traffic into a single
/// shared bucket (which would be a DoS amplification vector).
#[cfg(feature = "http-api")]
pub async fn rate_limit_middleware(request: Request, next: Next) -> Result<Response, StatusCode> {
    let client_ip = match extract_client_ip(&request) {
        Some(ip) => ip,
        None => {
            tracing::warn!("Rejecting request: could not determine client IP");
            return Err(StatusCode::BAD_REQUEST);
        }
    };

    let rate_limiter = get_rate_limiter_for_ip(client_ip);

    match rate_limiter.check() {
        Ok(_) => Ok(next.run(request).await),
        Err(_) => {
            tracing::warn!("Rate limit exceeded for IP: {}", client_ip);
            Err(StatusCode::TOO_MANY_REQUESTS)
        }
    }
}

/// Enhanced request logging middleware with structured logging
///
/// Logs comprehensive request details including:
/// - HTTP method and URI
/// - Response status code and processing latency
/// - Client IP address and response body size
/// - Uses structured logging with tracing spans for request grouping
#[cfg(feature = "http-api")]
pub async fn logging_middleware(request: Request, next: Next) -> Result<Response, StatusCode> {
    use std::time::Instant;

    // Extract request details
    let method = request.method().clone();
    let uri = request.uri().clone();
    let client_ip =
        extract_client_ip(&request).unwrap_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));

    // Create a structured span for this request
    let span = tracing::info_span!(
        "http_request",
        method = %method,
        uri = %uri,
        client_ip = %client_ip,
        status_code = tracing::field::Empty,
        latency_ms = tracing::field::Empty,
        response_size = tracing::field::Empty,
    );

    let _guard = span.enter();

    // Record start time for latency calculation
    let start_time = Instant::now();

    tracing::info!("Processing request");

    // Process the request
    let response = next.run(request).await;

    // Calculate latency
    let latency = start_time.elapsed();
    let latency_ms = latency.as_millis() as u64;

    // Extract response details
    let status_code = response.status();

    // Try to extract response body size from Content-Length header
    let response_size = response
        .headers()
        .get("content-length")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    // Record additional fields in the span
    span.record("status_code", status_code.as_u16());
    span.record("latency_ms", latency_ms);
    span.record("response_size", response_size);

    // Log completion with all details
    tracing::info!(
        status_code = status_code.as_u16(),
        latency_ms = latency_ms,
        response_size = response_size,
        "Request completed"
    );

    Ok(response)
}

/// Security headers middleware
///
/// Adds essential security headers to all HTTP responses:
/// - Strict-Transport-Security: Enforces HTTPS connections
/// - X-Content-Type-Options: Prevents MIME type sniffing
/// - X-Frame-Options: Prevents clickjacking attacks
/// - Content-Security-Policy: Restricts resource loading
#[cfg(feature = "http-api")]
pub async fn security_headers_middleware(
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    use axum::http::HeaderValue;

    // Process the request
    let mut response = next.run(request).await;

    // Add security headers to the response
    let headers = response.headers_mut();

    headers.insert(
        "strict-transport-security",
        HeaderValue::from_static("max-age=63072000; includeSubDomains; preload"),
    );

    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );

    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));

    headers.insert(
        "content-security-policy",
        HeaderValue::from_static("default-src 'self'; frame-ancestors 'none'"),
    );

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    #[test]
    fn cidr_parse_ipv4_exact() {
        let cidr = TrustedProxyCidr::parse("10.0.0.1").unwrap();
        assert_eq!(cidr.prefix_len, 32);
        assert!(cidr.contains(&IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(!cidr.contains(&IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2))));
    }

    #[test]
    fn cidr_parse_ipv4_slash_24() {
        let cidr = TrustedProxyCidr::parse("192.168.1.0/24").unwrap();
        assert!(cidr.contains(&IpAddr::V4(Ipv4Addr::new(192, 168, 1, 0))));
        assert!(cidr.contains(&IpAddr::V4(Ipv4Addr::new(192, 168, 1, 255))));
        assert!(!cidr.contains(&IpAddr::V4(Ipv4Addr::new(192, 168, 2, 1))));
    }

    #[test]
    fn cidr_parse_ipv4_slash_8() {
        let cidr = TrustedProxyCidr::parse("10.0.0.0/8").unwrap();
        assert!(cidr.contains(&IpAddr::V4(Ipv4Addr::new(10, 255, 255, 255))));
        assert!(!cidr.contains(&IpAddr::V4(Ipv4Addr::new(11, 0, 0, 1))));
    }

    #[test]
    fn cidr_parse_ipv4_slash_0_matches_all() {
        let cidr = TrustedProxyCidr::parse("0.0.0.0/0").unwrap();
        assert!(cidr.contains(&IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4))));
        assert!(cidr.contains(&IpAddr::V4(Ipv4Addr::new(255, 255, 255, 255))));
    }

    #[test]
    fn cidr_ipv4_does_not_match_ipv6() {
        let cidr = TrustedProxyCidr::parse("0.0.0.0/0").unwrap();
        assert!(!cidr.contains(&IpAddr::V6(Ipv6Addr::LOCALHOST)));
    }

    #[test]
    fn cidr_parse_ipv6() {
        let cidr = TrustedProxyCidr::parse("::1").unwrap();
        assert!(cidr.contains(&IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(!cidr.contains(&IpAddr::V6(Ipv6Addr::UNSPECIFIED)));
    }

    #[test]
    fn cidr_rejects_invalid_prefix() {
        assert!(TrustedProxyCidr::parse("10.0.0.0/33").is_none());
        assert!(TrustedProxyCidr::parse("::1/129").is_none());
    }

    #[test]
    fn cidr_rejects_garbage() {
        assert!(TrustedProxyCidr::parse("not-an-ip").is_none());
        assert!(TrustedProxyCidr::parse("").is_none());
    }

    #[test]
    fn trusted_proxies_empty_trusts_nothing() {
        let tp = TrustedProxies { cidrs: vec![] };
        assert!(!tp.is_trusted(&IpAddr::V4(Ipv4Addr::LOCALHOST)));
    }

    #[test]
    fn trusted_proxies_matches_configured_ranges() {
        let tp = TrustedProxies {
            cidrs: vec![
                TrustedProxyCidr::parse("127.0.0.1").unwrap(),
                TrustedProxyCidr::parse("172.16.0.0/12").unwrap(),
            ],
        };
        assert!(tp.is_trusted(&IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
        assert!(tp.is_trusted(&IpAddr::V4(Ipv4Addr::new(172, 17, 0, 1))));
        assert!(!tp.is_trusted(&IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
    }
}
