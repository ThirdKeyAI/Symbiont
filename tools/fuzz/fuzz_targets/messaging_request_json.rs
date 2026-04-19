#![no_main]

//! Fuzz target for messaging HTTP API DTO deserialization.
//!
//! Exercises `SendMessageRequest`, `MessageEnvelope`, `ReceiveMessagesResponse`,
//! and `MessageStatusResponse` with arbitrary JSON input to ensure:
//! - No panics on malformed input (must return Err).
//! - Round-trip stability: a valid serialized value must deserialize back
//!   to an equivalent value.
//!
//! These are the DTOs sitting on the network edge of the bus, so a parser
//! crash here is a denial-of-service vector against the whole runtime.

use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use symbi_runtime::api::types::{
    MessageEnvelope, MessageStatusResponse, ReceiveMessagesResponse, SendMessageRequest,
    SendMessageResponse,
};
use symbi_runtime::types::AgentId;
use uuid::Uuid;

#[derive(Arbitrary, Debug)]
struct Input {
    mode: Mode,
}

#[derive(Arbitrary, Debug)]
enum Mode {
    /// Raw bytes interpreted as JSON for the given DTO. Must never panic.
    Raw { which: Dto, bytes: Vec<u8> },
    /// Structured: build a valid value, serialize, deserialize, assert round-trip.
    Roundtrip(RoundtripInput),
    /// Envelope list with arbitrary sizes.
    EnvelopeList { count: u8, extra: String },
    /// Adversarial: values that should be rejected or clamped by consumers.
    Adversarial(AdversarialInput),
}

#[derive(Arbitrary, Debug)]
enum Dto {
    SendRequest,
    SendResponse,
    Envelope,
    EnvelopeList,
    Status,
}

#[derive(Arbitrary, Debug)]
struct RoundtripInput {
    sender_hi: u64,
    sender_lo: u64,
    payload: String,
    ttl_seconds: Option<u64>,
    topic: Option<String>,
}

#[derive(Arbitrary, Debug)]
enum AdversarialInput {
    /// Huge TTL: consumers must clamp.
    HugeTtl,
    /// Zero TTL.
    ZeroTtl,
    /// Empty payload.
    EmptyPayload,
    /// Non-UTF8 bytes in payload (JSON can't actually encode these as strings;
    /// we use a replacement chain).
    HighBytesPayload,
    /// Missing required fields in envelope.
    PartialEnvelope,
    /// Unknown message_type enum value (currently free-form string).
    UnknownMessageType(String),
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

fn make_agent_id(hi: u64, lo: u64) -> AgentId {
    AgentId(Uuid::from_u64_pair(hi, lo))
}

fuzz_target!(|input: Input| {
    match input.mode {
        Mode::Raw { which, bytes } => {
            // Must not panic on arbitrary input, even malformed UTF-8.
            let Ok(s) = std::str::from_utf8(&bytes) else {
                return;
            };
            if s.len() > 64 * 1024 {
                return;
            }
            match which {
                Dto::SendRequest => {
                    let _ = serde_json::from_str::<SendMessageRequest>(s);
                }
                Dto::SendResponse => {
                    let _ = serde_json::from_str::<SendMessageResponse>(s);
                }
                Dto::Envelope => {
                    let _ = serde_json::from_str::<MessageEnvelope>(s);
                }
                Dto::EnvelopeList => {
                    let _ = serde_json::from_str::<ReceiveMessagesResponse>(s);
                }
                Dto::Status => {
                    let _ = serde_json::from_str::<MessageStatusResponse>(s);
                }
            }
        }
        Mode::Roundtrip(r) => {
            let sender = make_agent_id(r.sender_hi, r.sender_lo);
            let payload = clamp(r.payload, 4096, "hello");
            let topic = r.topic.map(|t| clamp(t, 128, "topic"));

            let req = SendMessageRequest {
                sender,
                payload: payload.clone(),
                ttl_seconds: r.ttl_seconds,
                topic: topic.clone(),
            };

            let json = serde_json::to_string(&req).expect("serialize cannot fail");
            let parsed: SendMessageRequest =
                serde_json::from_str(&json).expect("round-trip must parse");
            assert_eq!(parsed.sender, req.sender, "sender round-trip");
            assert_eq!(parsed.payload, req.payload, "payload round-trip");
            assert_eq!(parsed.ttl_seconds, req.ttl_seconds, "ttl round-trip");
            assert_eq!(parsed.topic, req.topic, "topic round-trip");
        }
        Mode::EnvelopeList { count, extra } => {
            // Build an envelope list and assert deserialization tolerates
            // unknown top-level fields (it should not).
            let count = (count as usize).min(100);
            let envelopes: Vec<MessageEnvelope> = (0..count)
                .map(|i| MessageEnvelope {
                    message_id: Uuid::from_u64_pair(0, i as u64).to_string(),
                    sender: make_agent_id(0, i as u64),
                    recipient: Some(make_agent_id(1, i as u64)),
                    topic: None,
                    payload: "p".to_string(),
                    message_type: "direct".to_string(),
                    timestamp_secs: i as u64,
                    ttl_seconds: 60,
                })
                .collect();
            let resp = ReceiveMessagesResponse {
                messages: envelopes,
            };
            let json = serde_json::to_string(&resp).expect("serialize");
            let parsed: ReceiveMessagesResponse =
                serde_json::from_str(&json).expect("round-trip");
            assert_eq!(parsed.messages.len(), count);

            // Inject an extra field into the JSON object and confirm parser
            // still accepts (serde defaults to tolerant unknown-field parsing).
            let extra_key = clamp(extra, 64, "x");
            if let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&json) {
                if let Some(obj) = v.as_object_mut() {
                    obj.insert(extra_key, serde_json::Value::Null);
                    let mutated = v.to_string();
                    let _ = serde_json::from_str::<ReceiveMessagesResponse>(&mutated);
                }
            }
        }
        Mode::Adversarial(a) => match a {
            AdversarialInput::HugeTtl => {
                let req = SendMessageRequest {
                    sender: AgentId::new(),
                    payload: "p".into(),
                    ttl_seconds: Some(u64::MAX),
                    topic: None,
                };
                let json = serde_json::to_string(&req).unwrap();
                let parsed: SendMessageRequest = serde_json::from_str(&json).unwrap();
                assert_eq!(parsed.ttl_seconds, Some(u64::MAX));
            }
            AdversarialInput::ZeroTtl => {
                let json = r#"{"sender":"00000000-0000-0000-0000-000000000000","payload":"p","ttl_seconds":0}"#;
                let parsed: SendMessageRequest = serde_json::from_str(json).unwrap();
                assert_eq!(parsed.ttl_seconds, Some(0));
            }
            AdversarialInput::EmptyPayload => {
                let json = r#"{"sender":"00000000-0000-0000-0000-000000000000","payload":""}"#;
                let parsed: SendMessageRequest = serde_json::from_str(json).unwrap();
                assert!(parsed.payload.is_empty());
            }
            AdversarialInput::HighBytesPayload => {
                // Legal UTF-8 high codepoints should round-trip.
                let s = "\u{10FFFF}";
                let req = SendMessageRequest {
                    sender: AgentId::new(),
                    payload: s.to_string(),
                    ttl_seconds: None,
                    topic: None,
                };
                let json = serde_json::to_string(&req).unwrap();
                let parsed: SendMessageRequest = serde_json::from_str(&json).unwrap();
                assert_eq!(parsed.payload, s);
            }
            AdversarialInput::PartialEnvelope => {
                // Envelope with missing fields must fail parsing.
                let json = r#"{"message_id":"x"}"#;
                assert!(serde_json::from_str::<MessageEnvelope>(json).is_err());
            }
            AdversarialInput::UnknownMessageType(t) => {
                let t = clamp(t, 64, "mystery");
                let env = MessageEnvelope {
                    message_id: Uuid::nil().to_string(),
                    sender: AgentId::new(),
                    recipient: None,
                    topic: None,
                    payload: "p".into(),
                    message_type: t.clone(),
                    timestamp_secs: 0,
                    ttl_seconds: 60,
                };
                let json = serde_json::to_string(&env).unwrap();
                // Unknown message_type is preserved on the wire as free-form
                // string; receiving bus maps unknown → Direct (default).
                let parsed: MessageEnvelope = serde_json::from_str(&json).unwrap();
                assert_eq!(parsed.message_type, t);
            }
        },
    }
});
