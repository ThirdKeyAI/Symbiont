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
    net::IpAddr,
    num::NonZeroU32,
    sync::{Arc, OnceLock},
};

#[cfg(feature = "http-api")]
use dashmap::DashMap;

#[cfg(feature = "http-api")]
use std::env;

/// Authentication middleware for bearer token validation
#[cfg(feature = "http-api")]
pub async fn auth_middleware(request: Request, next: Next) -> Result<Response, StatusCode> {
    // Extract the Authorization header
    let headers = request.headers();
    let auth_header = headers.get("authorization");

    // Check if Authorization header is present
    let auth_value = match auth_header {
        Some(value) => value.to_str().map_err(|_| StatusCode::UNAUTHORIZED)?,
        None => return Err(StatusCode::UNAUTHORIZED),
    };

    // Check if it's a Bearer token
    if !auth_value.starts_with("Bearer ") {
        return Err(StatusCode::UNAUTHORIZED);
    }

    // Extract the token part (after "Bearer ")
    let token = &auth_value[7..];

    // Get the expected token from environment variable (simplified for now)
    let expected_token = env::var("SYMBIONT_API_TOKEN").map_err(|_| {
        tracing::error!("SYMBIONT_API_TOKEN environment variable not set");
        StatusCode::UNAUTHORIZED
    })?;

    // Validate the token using constant-time comparison to prevent timing attacks
    if !bool::from(token.as_bytes().ct_eq(expected_token.as_bytes())) {
        tracing::warn!("Authentication failed: invalid token provided");
        return Err(StatusCode::UNAUTHORIZED);
    }

    // Token is valid, proceed with the request
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

/// Extract client IP address from request
#[cfg(feature = "http-api")]
fn extract_client_ip(request: &Request) -> IpAddr {
    // Try to get real IP from X-Forwarded-For header first (for proxy setups)
    if let Some(forwarded_for) = request.headers().get("x-forwarded-for") {
        if let Ok(forwarded_str) = forwarded_for.to_str() {
            // X-Forwarded-For can contain multiple IPs; take the rightmost
            // (last) entry which is the one added by our trusted proxy,
            // preventing client-side spoofing of earlier entries.
            if let Some(last_ip) = forwarded_str.split(',').next_back() {
                if let Ok(ip) = last_ip.trim().parse::<IpAddr>() {
                    return ip;
                }
            }
        }
    }

    // Try X-Real-IP header
    if let Some(real_ip) = request.headers().get("x-real-ip") {
        if let Ok(real_ip_str) = real_ip.to_str() {
            if let Ok(ip) = real_ip_str.parse::<IpAddr>() {
                return ip;
            }
        }
    }

    // Fallback to connection info or default
    // In a real setup, you'd extract this from the connection info
    // For now, we'll use a default IP as fallback
    "127.0.0.1".parse().unwrap()
}

/// Rate limiting middleware using token bucket algorithm
///
/// This middleware implements per-IP rate limiting with a token bucket algorithm.
/// Each IP address gets 100 requests per minute (approximately 1.67 RPS).
///
/// Rate limiters are stored in a global concurrent HashMap and are created
/// on-demand for each unique IP address.
#[cfg(feature = "http-api")]
pub async fn rate_limit_middleware(request: Request, next: Next) -> Result<Response, StatusCode> {
    // Extract client IP address
    let client_ip = extract_client_ip(&request);

    // Get the rate limiter for this IP
    let rate_limiter = get_rate_limiter_for_ip(client_ip);

    // Check if the request is allowed
    match rate_limiter.check() {
        Ok(_) => {
            // Request is allowed, proceed
            Ok(next.run(request).await)
        }
        Err(_) => {
            // Rate limit exceeded
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
    let client_ip = extract_client_ip(&request);

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
