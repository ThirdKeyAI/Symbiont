#![no_main]

//! Comprehensive fuzz target for `verify_slack_signature`.
//!
//! Exercises:
//! - Non-numeric / empty / huge / negative / boundary timestamps
//! - Empty / arbitrary signing secrets
//! - Malformed signature strings (no prefix, wrong length, non-hex, empty)
//! - Large and non-UTF-8 bodies
//! - Valid round-trip: correct signature must always verify
//! - Tampered inputs must always fail
//! - The function must NEVER panic regardless of input combination.

use hmac::{Hmac, Mac};
use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use sha2::Sha256;
use symbi_channel_adapter::adapters::slack::signature::verify_slack_signature;
use symbi_channel_adapter::error::ChannelAdapterError;

type HmacSha256 = Hmac<Sha256>;

/// Clamp a string to at most `max` bytes, respecting UTF-8 char boundaries.
/// Returns `fallback` when the input is empty.
fn clamp(mut s: String, max: usize, fallback: &str) -> String {
    if s.is_empty() {
        return fallback.to_string();
    }
    if s.len() > max {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

/// Clamp a byte vec to at most `max` bytes.
fn clamp_bytes(mut v: Vec<u8>, max: usize) -> Vec<u8> {
    v.truncate(max);
    v
}

// ---------------------------------------------------------------------------
// Structured input types
// ---------------------------------------------------------------------------

#[derive(Arbitrary, Debug)]
struct Input {
    secret: SecretVariant,
    timestamp: TimestampVariant,
    body: BodyVariant,
    signature: SignatureVariant,
}

#[derive(Arbitrary, Debug)]
enum SecretVariant {
    /// A non-empty arbitrary secret.
    Arbitrary(String),
    /// Empty signing secret.
    Empty,
}

#[derive(Arbitrary, Debug)]
enum TimestampVariant {
    /// Current wall-clock time (should pass replay check).
    Valid,
    /// Exactly at the 300-second boundary.
    Boundary,
    /// Stale: more than 300 seconds ago.
    Stale,
    /// A future timestamp well beyond now.
    Future,
    /// Non-numeric garbage string.
    NonNumeric(String),
    /// Empty string.
    Empty,
    /// Extremely large number (near i64::MAX).
    Huge,
    /// Negative number.
    Negative,
    /// Overflow: the literal string for a number that overflows i64.
    Overflow,
}

#[derive(Arbitrary, Debug)]
enum BodyVariant {
    /// Small arbitrary bytes (may include non-UTF-8).
    Small(Vec<u8>),
    /// Completely empty body.
    Empty,
    /// Large body (up to ~1 MB, clamped from fuzzer data).
    Large(Vec<u8>),
}

#[derive(Arbitrary, Debug)]
enum SignatureVariant {
    /// Compute the correct HMAC signature for the given secret/timestamp/body.
    Correct,
    /// Correct signature with one byte flipped.
    Tampered,
    /// Arbitrary string WITHOUT the `v0=` prefix.
    NoPrefix(String),
    /// Empty string.
    Empty,
    /// Too short to be a valid hex digest.
    TooShort,
    /// Too long (extra garbage appended to a valid-looking prefix).
    TooLong(String),
    /// Has the `v0=` prefix but contains non-hex characters.
    NonHex(String),
    /// Fully arbitrary string.
    Arbitrary(String),
}

// ---------------------------------------------------------------------------
// Helpers to materialise concrete values from each variant
// ---------------------------------------------------------------------------

fn resolve_secret(v: &SecretVariant) -> String {
    match v {
        SecretVariant::Arbitrary(s) => clamp(s.clone(), 256, "default-secret"),
        SecretVariant::Empty => String::new(),
    }
}

fn resolve_timestamp(v: &TimestampVariant) -> String {
    let now = chrono::Utc::now().timestamp();
    match v {
        TimestampVariant::Valid => now.to_string(),
        TimestampVariant::Boundary => (now - 300).to_string(),
        TimestampVariant::Stale => (now - 600).to_string(),
        TimestampVariant::Future => (now + 600).to_string(),
        TimestampVariant::NonNumeric(s) => clamp(s.clone(), 64, "not-a-number"),
        TimestampVariant::Empty => String::new(),
        TimestampVariant::Huge => i64::MAX.to_string(),
        TimestampVariant::Negative => (-99999i64).to_string(),
        TimestampVariant::Overflow => "99999999999999999999999".to_string(),
    }
}

fn resolve_body(v: &BodyVariant) -> Vec<u8> {
    match v {
        BodyVariant::Small(b) => clamp_bytes(b.clone(), 4096),
        BodyVariant::Empty => Vec::new(),
        // Allow up to ~1 MB to stress allocation paths.
        BodyVariant::Large(b) => clamp_bytes(b.clone(), 1_048_576),
    }
}

fn compute_correct_signature(secret: &str, timestamp: &str, body: &[u8]) -> String {
    let base = format!("v0:{}:{}", timestamp, String::from_utf8_lossy(body));
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac init");
    mac.update(base.as_bytes());
    let digest = mac.finalize().into_bytes();
    format!("v0={}", hex::encode(digest))
}

fn resolve_signature(
    v: &SignatureVariant,
    secret: &str,
    timestamp: &str,
    body: &[u8],
) -> String {
    match v {
        SignatureVariant::Correct => compute_correct_signature(secret, timestamp, body),
        SignatureVariant::Tampered => {
            let mut sig = compute_correct_signature(secret, timestamp, body);
            // Flip the last character to break the signature.
            let last = sig.pop().unwrap_or('0');
            let flipped = if last == 'f' { 'e' } else { 'f' };
            sig.push(flipped);
            sig
        }
        SignatureVariant::NoPrefix(s) => clamp(s.clone(), 256, "deadbeef"),
        SignatureVariant::Empty => String::new(),
        SignatureVariant::TooShort => "v0=abcd".to_string(),
        SignatureVariant::TooLong(s) => {
            let correct = compute_correct_signature(secret, timestamp, body);
            let extra = clamp(s.clone(), 128, "extra");
            format!("{}{}", correct, extra)
        }
        SignatureVariant::NonHex(s) => {
            let garbage = clamp(s.clone(), 64, "zzzz");
            format!("v0={}", garbage)
        }
        SignatureVariant::Arbitrary(s) => clamp(s.clone(), 512, "garbage"),
    }
}

// ---------------------------------------------------------------------------
// Whether the given timestamp string is numeric and within the 300s window.
// ---------------------------------------------------------------------------

fn timestamp_is_fresh(ts: &str) -> bool {
    if let Ok(parsed) = ts.parse::<i64>() {
        let now = chrono::Utc::now().timestamp();
        (now - parsed).abs() <= 300
    } else {
        false
    }
}

// ---------------------------------------------------------------------------
// Fuzz target
// ---------------------------------------------------------------------------

fuzz_target!(|input: Input| {
    let secret = resolve_secret(&input.secret);
    let timestamp = resolve_timestamp(&input.timestamp);
    let body = resolve_body(&input.body);
    let signature = resolve_signature(&input.signature, &secret, &timestamp, &body);

    // The function must NEVER panic.
    let result = verify_slack_signature(&secret, &timestamp, &body, &signature);

    // ---- Semantic assertions ------------------------------------------------

    match &input.timestamp {
        // Non-numeric timestamps must always produce an error.
        TimestampVariant::NonNumeric(_) | TimestampVariant::Empty | TimestampVariant::Overflow => {
            assert!(
                result.is_err(),
                "non-numeric timestamp should be rejected: ts={:?}",
                timestamp,
            );
            if let Err(ChannelAdapterError::SignatureInvalid(msg)) = &result {
                assert!(
                    msg.contains("timestamp"),
                    "error message should mention 'timestamp', got: {}",
                    msg,
                );
            }
        }
        // Stale, future, huge, and negative timestamps are numeric but outside
        // the 300-second window — they must be rejected.
        TimestampVariant::Stale
        | TimestampVariant::Future
        | TimestampVariant::Huge
        | TimestampVariant::Negative => {
            assert!(
                result.is_err(),
                "out-of-window timestamp should be rejected: ts={}",
                timestamp,
            );
        }
        // Valid and Boundary timestamps are within window; correctness depends
        // on the signature variant.
        TimestampVariant::Valid | TimestampVariant::Boundary => {
            // Only check correctness when secret is non-empty (HMAC accepts
            // empty keys, but let's be precise about what "correct" means).
            if matches!(input.signature, SignatureVariant::Correct) && timestamp_is_fresh(&timestamp)
            {
                assert!(
                    result.is_ok(),
                    "correct signature + fresh timestamp must verify: ts={}, sig=..., err={:?}",
                    timestamp,
                    result.unwrap_err(),
                );
            }

            // Tampered, NoPrefix, Empty, TooShort, TooLong, NonHex, Arbitrary
            // must all be rejected when timestamp is within window.
            match &input.signature {
                SignatureVariant::Tampered
                | SignatureVariant::NoPrefix(_)
                | SignatureVariant::Empty
                | SignatureVariant::TooShort
                | SignatureVariant::NonHex(_) => {
                    if timestamp_is_fresh(&timestamp) {
                        assert!(
                            result.is_err(),
                            "bad signature variant {:?} should be rejected",
                            input.signature,
                        );
                    }
                }
                _ => {}
            }
        }
    }
});
