#![no_main]

//! Fuzz target for Mattermost webhook token verification.
//!
//! Exercises constant-time token comparison with arbitrary inputs:
//! empty secrets, empty tokens, length mismatches, and valid round-trips.

use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use symbi_channel_adapter::adapters::mattermost::signature::verify_webhook_token;

#[derive(Arbitrary, Debug)]
struct Input {
    secret: SecretVariant,
    token: TokenVariant,
}

#[derive(Arbitrary, Debug)]
enum SecretVariant {
    /// Non-empty arbitrary secret.
    Arbitrary(String),
    /// Empty secret (should fail with Config error).
    Empty,
}

#[derive(Arbitrary, Debug)]
enum TokenVariant {
    /// Token that exactly matches the secret (valid round-trip).
    Matching,
    /// Completely different token.
    Different(String),
    /// Empty token.
    Empty,
    /// Token with same length but different content.
    SameLength,
    /// Very long token.
    Long(String),
    /// Token that is a prefix/suffix of the secret.
    Partial,
}

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

fuzz_target!(|input: Input| {
    let secret = match &input.secret {
        SecretVariant::Arbitrary(s) => clamp(s.clone(), 256, "default-secret"),
        SecretVariant::Empty => String::new(),
    };

    let token = match &input.token {
        TokenVariant::Matching => secret.clone(),
        TokenVariant::Different(s) => clamp(s.clone(), 256, "wrong-token"),
        TokenVariant::Empty => String::new(),
        TokenVariant::SameLength => {
            // Generate a string of the same length but flipped bytes
            secret
                .chars()
                .map(|c| if c == 'a' { 'b' } else { 'a' })
                .collect()
        }
        TokenVariant::Long(s) => clamp(s.clone(), 4096, "a]".repeat(2048).as_str()),
        TokenVariant::Partial => {
            if secret.len() > 1 {
                secret[..secret.len() / 2].to_string()
            } else {
                String::new()
            }
        }
    };

    // Function must never panic.
    let result = verify_webhook_token(&secret, &token);

    // --- Semantic assertions ---

    // Empty configured secret must always return an error.
    if secret.is_empty() {
        assert!(
            result.is_err(),
            "empty configured secret must be rejected",
        );
        return;
    }

    // Matching token must always succeed.
    if matches!(input.token, TokenVariant::Matching) && !secret.is_empty() {
        assert!(
            result.is_ok(),
            "matching token must verify: secret={:?}",
            secret,
        );
    }

    // Different/empty/partial tokens must fail (when secret is non-empty).
    match &input.token {
        TokenVariant::Empty | TokenVariant::Partial => {
            assert!(
                result.is_err(),
                "empty/partial token must be rejected",
            );
        }
        TokenVariant::SameLength if !secret.is_empty() => {
            // Same length but different content — should fail unless
            // the flip happens to produce the same string (all 'a's).
            if token != secret {
                assert!(
                    result.is_err(),
                    "same-length different token must be rejected",
                );
            }
        }
        _ => {}
    }
});
