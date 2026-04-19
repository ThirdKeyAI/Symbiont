#![no_main]

//! Fuzz target for `RemoteCommunicationBus::parse_envelope`.
//!
//! The remote bus pulls inbound messages from another runtime's HTTP API.
//! The envelope parser is the first line of defence against malformed or
//! malicious peer responses, so we fuzz it with arbitrary JSON to ensure:
//! - Parsing never panics (all invalid shapes return `InvalidFormat`).
//! - Valid envelopes round-trip: a well-formed JSON encoding of a
//!   generated envelope parses into an equivalent `SecureMessage`.
//! - Unknown `message_type` values are rejected.
//! - Missing required fields are rejected (no silent defaults).

use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use serde_json::{json, Value};
use symbi_runtime::communication::remote::parse_envelope;
use symbi_runtime::types::MessageType;
use uuid::Uuid;

#[derive(Arbitrary, Debug)]
struct Input {
    mode: Mode,
}

#[derive(Arbitrary, Debug)]
enum Mode {
    /// Arbitrary JSON bytes.
    RawJson(Vec<u8>),
    /// Structured valid envelope — must parse successfully.
    Valid(Envelope),
    /// Valid envelope with one field removed — must fail.
    MissingField { env: Envelope, which: MissingField },
    /// Unknown message_type.
    UnknownType { env: Envelope, ty: String },
    /// Malformed UUIDs.
    BadUuids(Envelope),
}

#[derive(Arbitrary, Debug, Clone)]
struct Envelope {
    id_hi: u64,
    id_lo: u64,
    sender_hi: u64,
    sender_lo: u64,
    recipient_hi: Option<u64>,
    recipient_lo: Option<u64>,
    topic: Option<String>,
    payload: String,
    kind: MessageTypeSpec,
    timestamp_secs: u64,
    ttl_seconds: u64,
}

#[derive(Arbitrary, Debug, Clone)]
enum MessageTypeSpec {
    Direct,
    Publish,
    Subscribe,
    Broadcast,
    Request,
    Response,
}

#[derive(Arbitrary, Debug)]
enum MissingField {
    MessageId,
    Sender,
    Payload,
    MessageType,
    Ttl,
    Timestamp,
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

fn message_type_str(spec: &MessageTypeSpec) -> &'static str {
    match spec {
        MessageTypeSpec::Direct => "direct",
        MessageTypeSpec::Publish => "publish",
        MessageTypeSpec::Subscribe => "subscribe",
        MessageTypeSpec::Broadcast => "broadcast",
        MessageTypeSpec::Request => "request",
        MessageTypeSpec::Response => "response",
    }
}

fn envelope_to_json(env: &Envelope) -> Value {
    let message_id = Uuid::from_u64_pair(env.id_hi, env.id_lo).to_string();
    let sender = Uuid::from_u64_pair(env.sender_hi, env.sender_lo).to_string();
    let recipient = match (env.recipient_hi, env.recipient_lo) {
        (Some(hi), Some(lo)) => json!(Uuid::from_u64_pair(hi, lo).to_string()),
        _ => Value::Null,
    };
    let topic = match env.topic.as_ref() {
        Some(t) => json!(clamp(t.clone(), 128, "topic")),
        None => Value::Null,
    };
    json!({
        "message_id": message_id,
        "sender": sender,
        "recipient": recipient,
        "topic": topic,
        "payload": clamp(env.payload.clone(), 4096, ""),
        "message_type": message_type_str(&env.kind),
        "timestamp_secs": env.timestamp_secs,
        "ttl_seconds": env.ttl_seconds,
    })
}

fn remove_field(mut v: Value, which: &MissingField) -> Value {
    let key = match which {
        MissingField::MessageId => "message_id",
        MissingField::Sender => "sender",
        MissingField::Payload => "payload",
        MissingField::MessageType => "message_type",
        MissingField::Ttl => "ttl_seconds",
        MissingField::Timestamp => "timestamp_secs",
    };
    if let Some(obj) = v.as_object_mut() {
        obj.remove(key);
    }
    v
}

fuzz_target!(|input: Input| {
    match input.mode {
        Mode::RawJson(bytes) => {
            if bytes.len() > 64 * 1024 {
                return;
            }
            // Interpret as JSON; if not parseable, still don't panic at the
            // envelope layer (which takes a Value).
            let Ok(s) = std::str::from_utf8(&bytes) else {
                return;
            };
            if let Ok(v) = serde_json::from_str::<Value>(s) {
                let _ = parse_envelope(&v);
            }
        }
        Mode::Valid(env) => {
            let v = envelope_to_json(&env);
            let parsed = parse_envelope(&v).expect("valid envelope must parse");
            // message_type round-trip check.
            match (&parsed.message_type, &env.kind) {
                (MessageType::Direct(_), MessageTypeSpec::Direct)
                | (MessageType::Publish(_), MessageTypeSpec::Publish)
                | (MessageType::Subscribe(_), MessageTypeSpec::Subscribe)
                | (MessageType::Broadcast, MessageTypeSpec::Broadcast)
                | (MessageType::Request(_), MessageTypeSpec::Request)
                | (MessageType::Response(_), MessageTypeSpec::Response) => {}
                (got, want) => {
                    panic!(
                        "message_type mismatch: got {:?} want {:?}",
                        got,
                        message_type_str(want),
                    );
                }
            }
            // TTL must round-trip exactly.
            assert_eq!(parsed.ttl.as_secs(), env.ttl_seconds);
            // Payload bytes must round-trip.
            assert_eq!(
                &parsed.payload.data[..],
                clamp(env.payload.clone(), 4096, "").as_bytes()
            );
        }
        Mode::MissingField { env, which } => {
            let v = envelope_to_json(&env);
            let mangled = remove_field(v, &which);
            assert!(
                parse_envelope(&mangled).is_err(),
                "envelope missing {:?} must be rejected",
                which
            );
        }
        Mode::UnknownType { env, ty } => {
            let ty = clamp(ty, 64, "mystery_type");
            if [
                "direct",
                "publish",
                "subscribe",
                "broadcast",
                "request",
                "response",
            ]
            .contains(&ty.as_str())
            {
                return;
            }
            let mut v = envelope_to_json(&env);
            if let Some(obj) = v.as_object_mut() {
                obj.insert("message_type".into(), Value::String(ty));
            }
            assert!(
                parse_envelope(&v).is_err(),
                "unknown message_type must be rejected"
            );
        }
        Mode::BadUuids(env) => {
            let mut v = envelope_to_json(&env);
            if let Some(obj) = v.as_object_mut() {
                obj.insert("message_id".into(), Value::String("not-a-uuid".into()));
            }
            assert!(
                parse_envelope(&v).is_err(),
                "envelope with non-UUID message_id must be rejected"
            );
        }
    }
});
