#![no_main]

use hmac::{Hmac, Mac};
use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use sha2::Sha256;
use symbi_channel_adapter::adapters::slack::signature::verify_slack_signature;
use symbi_channel_adapter::error::ChannelAdapterError;

type HmacSha256 = Hmac<Sha256>;

#[derive(Arbitrary, Debug)]
struct Input {
    secret: String,
    body: Vec<u8>,
    tamper_signature: bool,
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
    let secret = clamp(input.secret, 128, "test-secret");
    let body = if input.body.is_empty() {
        b"{}".to_vec()
    } else {
        input.body
    };

    // Keep timestamp fresh to exercise signature verification rather than replay rejection.
    let timestamp = chrono::Utc::now().timestamp().to_string();
    let base = format!("v0:{}:{}", timestamp, String::from_utf8_lossy(&body));

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac init");
    mac.update(base.as_bytes());
    let computed = mac.finalize().into_bytes();
    let mut signature = format!("v0={}", hex::encode(computed));

    if input.tamper_signature {
        signature.push('0');
        let result = verify_slack_signature(&secret, &timestamp, &body, &signature);
        assert!(matches!(
            result,
            Err(ChannelAdapterError::SignatureInvalid(_))
        ));
    } else {
        assert!(verify_slack_signature(&secret, &timestamp, &body, &signature).is_ok());
    }
});
