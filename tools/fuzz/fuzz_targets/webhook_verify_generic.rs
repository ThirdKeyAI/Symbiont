#![no_main]

//! Fuzz target for the generic webhook verification layer (HmacVerifier, JwtVerifier).
//!
//! Exercises:
//! - HMAC-SHA256 verification with various header formats and prefixes
//! - JWT HS256 verification with malformed tokens
//! - WebhookProvider preset verifiers (GitHub, Stripe, Slack, Custom)
//! - Missing headers, wrong headers, case sensitivity

use futures::executor::block_on;
use hmac::{Hmac, Mac};
use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use sha2::Sha256;
use symbi_runtime::http_input::webhook_verify::{
    HmacVerifier, JwtVerifier, SignatureVerifier, WebhookProvider,
};

type HmacSha256 = Hmac<Sha256>;

#[derive(Arbitrary, Debug)]
struct Input {
    mode: VerifyMode,
}

#[derive(Arbitrary, Debug)]
enum VerifyMode {
    /// HMAC verification with structured inputs.
    Hmac {
        secret: String,
        header_name: String,
        prefix: PrefixVariant,
        body: Vec<u8>,
        signature: SigVariant,
    },
    /// JWT verification with structured inputs.
    Jwt {
        secret: String,
        header_name: String,
        token: String,
        issuer: Option<String>,
    },
    /// WebhookProvider preset verifier.
    Provider {
        provider: ProviderVariant,
        secret: String,
        body: Vec<u8>,
        headers: Vec<(String, String)>,
    },
}

#[derive(Arbitrary, Debug)]
enum PrefixVariant {
    None,
    Sha256Eq,
    V0Eq,
    Custom(String),
}

#[derive(Arbitrary, Debug)]
enum SigVariant {
    /// Compute correct HMAC signature.
    Correct,
    /// Tampered signature (one byte flipped).
    Tampered,
    /// Completely wrong value.
    Wrong(String),
    /// Empty string.
    Empty,
    /// Missing header entirely.
    MissingHeader,
}

#[derive(Arbitrary, Debug)]
enum ProviderVariant {
    GitHub,
    Stripe,
    Slack,
    Custom,
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

fn clamp_bytes(mut v: Vec<u8>, max: usize) -> Vec<u8> {
    v.truncate(max);
    v
}

fn compute_hmac(secret: &[u8], body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("hmac init");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

fuzz_target!(|input: Input| {
    match input.mode {
        VerifyMode::Hmac {
            secret, header_name, prefix, body, signature,
        } => {
            let secret = clamp(secret, 256, "test-secret");
            let header_name = clamp(header_name, 64, "x-signature");
            let body = clamp_bytes(body, 4096);

            let prefix_str = match &prefix {
                PrefixVariant::None => None,
                PrefixVariant::Sha256Eq => Some("sha256=".to_string()),
                PrefixVariant::V0Eq => Some("v0=".to_string()),
                PrefixVariant::Custom(s) => Some(clamp(s.clone(), 32, "pfx=")),
            };

            let verifier = HmacVerifier::new(
                secret.as_bytes().to_vec(),
                header_name.clone(),
                prefix_str.clone(),
            );

            let hmac_hex = compute_hmac(secret.as_bytes(), &body);

            let sig_value = match &signature {
                SigVariant::Correct => {
                    let pfx = prefix_str.as_deref().unwrap_or("");
                    format!("{}{}", pfx, hmac_hex)
                }
                SigVariant::Tampered => {
                    let pfx = prefix_str.as_deref().unwrap_or("");
                    let mut h = hmac_hex.clone();
                    let last = h.pop().unwrap_or('0');
                    h.push(if last == 'f' { 'e' } else { 'f' });
                    format!("{}{}", pfx, h)
                }
                SigVariant::Wrong(s) => clamp(s.clone(), 512, "garbage"),
                SigVariant::Empty => String::new(),
                SigVariant::MissingHeader => String::new(), // header won't be added
            };

            let headers: Vec<(String, String)> = if matches!(signature, SigVariant::MissingHeader) {
                vec![]
            } else {
                vec![(header_name.clone(), sig_value)]
            };

            // Must never panic.
            let result = block_on(verifier.verify(&headers, &body));

            // Correct signature must verify.
            if matches!(signature, SigVariant::Correct) {
                assert!(
                    result.is_ok(),
                    "correct HMAC signature must verify",
                );
            }

            // Missing header must fail.
            if matches!(signature, SigVariant::MissingHeader) {
                assert!(
                    result.is_err(),
                    "missing signature header must fail",
                );
            }
        }

        VerifyMode::Jwt {
            secret, header_name, token, issuer,
        } => {
            let secret = clamp(secret, 256, "jwt-secret");
            let header_name = clamp(header_name, 64, "authorization");
            let token = clamp(token, 4096, "invalid.jwt.token");
            let issuer = issuer.map(|s| clamp(s, 128, "issuer"));

            let verifier = JwtVerifier::new_hmac(
                secret.as_bytes().to_vec(),
                header_name.clone(),
                issuer,
            );

            let headers = vec![(header_name, token)];

            // Must never panic — malformed JWTs should return errors.
            let _ = block_on(verifier.verify(&headers, &[]));
        }

        VerifyMode::Provider {
            provider, secret, body, headers,
        } => {
            let secret = clamp(secret, 256, "provider-secret");
            let body = clamp_bytes(body, 4096);
            let headers: Vec<(String, String)> = headers
                .into_iter()
                .take(8)
                .map(|(k, v)| (clamp(k, 64, "header"), clamp(v, 512, "value")))
                .collect();

            let provider = match provider {
                ProviderVariant::GitHub => WebhookProvider::GitHub,
                ProviderVariant::Stripe => WebhookProvider::Stripe,
                ProviderVariant::Slack => WebhookProvider::Slack,
                ProviderVariant::Custom => WebhookProvider::Custom,
            };

            let verifier = provider.verifier(secret.as_bytes());

            // Must never panic.
            let _ = block_on(verifier.verify(&headers, &body));
        }
    }
});
