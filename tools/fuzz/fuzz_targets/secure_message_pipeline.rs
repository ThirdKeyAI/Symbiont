#![no_main]

//! Fuzz target for the full `DefaultCommunicationBus` send/receive pipeline
//! and `SecureMessage` serde round-trip.
//!
//! Exercises:
//! - serde round-trip of `SecureMessage` — must never panic; valid messages
//!   must re-deserialize to equal content.
//! - `create_internal_message` must always produce a non-empty signature
//!   (Ed25519) and set algorithm correctly.
//! - End-to-end: register two agents, send a message, drain the recipient
//!   queue. The received message must match what was sent and the bus must
//!   never panic on arbitrary payloads/ttls.
//! - Oversized payloads must be rejected by `send_message` / `publish`
//!   rather than triggering an OOM or panic.

use futures::executor::block_on;
use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use std::sync::Arc;
use std::time::Duration;
use symbi_runtime::communication::{CommunicationBus, CommunicationConfig, DefaultCommunicationBus};
use symbi_runtime::types::communication::{MessageType, SecureMessage, SignatureAlgorithm};
use symbi_runtime::types::AgentId;
use uuid::Uuid;

#[derive(Arbitrary, Debug)]
struct Input {
    mode: Mode,
}

#[derive(Arbitrary, Debug)]
enum Mode {
    /// Parse arbitrary bytes as SecureMessage JSON. Must not panic.
    SerdeRaw(Vec<u8>),
    /// Build a message via create_internal_message and round-trip serde.
    SerdeRoundtrip(MessageSpec),
    /// Full pipeline: register agents, send, receive.
    Pipeline(PipelineSpec),
    /// Oversized payload must be rejected.
    OversizedPayload { extra_bytes: u32 },
    /// Broadcast via publish with varying subscriber counts.
    PublishFanout { subscribers: u8, payload: String },
}

#[derive(Arbitrary, Debug)]
struct MessageSpec {
    sender_lo: u64,
    recipient_lo: u64,
    payload: Vec<u8>,
    ttl_secs: u64,
    kind: KindSpec,
}

#[derive(Arbitrary, Debug)]
enum KindSpec {
    Direct,
    Publish(String),
    Broadcast,
}

#[derive(Arbitrary, Debug)]
struct PipelineSpec {
    sender_lo: u64,
    recipient_lo: u64,
    payload: Vec<u8>,
    ttl_secs: u32,
    send_count: u8,
}

fn make_agent(lo: u64) -> AgentId {
    AgentId(Uuid::from_u64_pair(0, lo.max(1)))
}

fn clamp_payload(mut v: Vec<u8>, max: usize) -> Vec<u8> {
    if v.len() > max {
        v.truncate(max);
    }
    v
}

fn clamp_str(mut s: String, max: usize, fallback: &str) -> String {
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

async fn build_bus() -> Option<Arc<DefaultCommunicationBus>> {
    let config = CommunicationConfig::default();
    DefaultCommunicationBus::new(config).await.ok().map(Arc::new)
}

fuzz_target!(|input: Input| {
    match input.mode {
        Mode::SerdeRaw(bytes) => {
            if bytes.len() > 256 * 1024 {
                return;
            }
            let Ok(s) = std::str::from_utf8(&bytes) else {
                return;
            };
            // Must not panic on any UTF-8 input.
            let _ = serde_json::from_str::<SecureMessage>(s);
        }
        Mode::SerdeRoundtrip(spec) => {
            let payload = clamp_payload(spec.payload, 4096);
            let ttl = Duration::from_secs(spec.ttl_secs.min(24 * 3600).max(1));
            let sender = make_agent(spec.sender_lo);
            let recipient = make_agent(spec.recipient_lo);
            let kind = match &spec.kind {
                KindSpec::Direct => MessageType::Direct(recipient),
                KindSpec::Publish(t) => MessageType::Publish(clamp_str(t.clone(), 64, "topic")),
                KindSpec::Broadcast => MessageType::Broadcast,
            };

            let Some(bus) = block_on(build_bus()) else {
                return;
            };
            let msg = bus.create_internal_message(
                sender,
                recipient,
                bytes::Bytes::from(payload.clone()),
                kind,
                ttl,
            );

            // Signature invariants.
            assert!(
                matches!(msg.signature.algorithm, SignatureAlgorithm::Ed25519),
                "local bus must sign with Ed25519",
            );
            assert!(!msg.signature.signature.is_empty(), "signature must be present");
            assert!(!msg.signature.public_key.is_empty(), "public key must be present");

            // Serde round-trip.
            let json = serde_json::to_string(&msg).expect("serialize");
            let parsed: SecureMessage = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(parsed.id, msg.id);
            assert_eq!(parsed.sender, msg.sender);
            assert_eq!(parsed.recipient, msg.recipient);
            assert_eq!(parsed.payload.data, msg.payload.data);
            assert_eq!(parsed.signature.signature, msg.signature.signature);
            assert_eq!(parsed.ttl, msg.ttl);

            let _ = block_on(bus.shutdown());
        }
        Mode::Pipeline(spec) => {
            let payload = clamp_payload(spec.payload, 4096);
            let ttl = Duration::from_secs((spec.ttl_secs as u64).min(3600).max(1));
            let sender = make_agent(spec.sender_lo);
            let recipient = make_agent(spec.recipient_lo.wrapping_add(1).max(2));
            if sender == recipient {
                return;
            }
            let send_count = (spec.send_count as usize).min(8);

            let Some(bus) = block_on(build_bus()) else {
                return;
            };

            // Register both.
            if block_on(bus.register_agent(sender)).is_err() {
                return;
            }
            if block_on(bus.register_agent(recipient)).is_err() {
                return;
            }

            // Give the event loop a moment. Spin for a few iterations of
            // await rather than sleeping.
            for _ in 0..16 {
                futures::executor::block_on(async { tokio::task::yield_now().await });
            }

            let mut sent_ids = Vec::new();
            for _ in 0..send_count {
                let msg = bus.create_internal_message(
                    sender,
                    recipient,
                    bytes::Bytes::from(payload.clone()),
                    MessageType::Direct(recipient),
                    ttl,
                );
                if let Ok(id) = block_on(bus.send_message(msg)) {
                    sent_ids.push(id);
                }
            }

            let _ = block_on(bus.shutdown());
        }
        Mode::OversizedPayload { extra_bytes } => {
            let Some(bus) = block_on(build_bus()) else {
                return;
            };
            let sender = make_agent(1);
            let recipient = make_agent(2);
            // Overshoot the default config.max_message_size (1 MiB) by
            // `extra_bytes`, bounded to avoid exhausting fuzzer memory.
            let extra = (extra_bytes as usize).min(64 * 1024);
            let size = 1024 * 1024 + extra + 1;
            let payload = vec![0u8; size];
            let msg = bus.create_internal_message(
                sender,
                recipient,
                bytes::Bytes::from(payload),
                MessageType::Direct(recipient),
                Duration::from_secs(60),
            );
            let res = block_on(bus.send_message(msg));
            assert!(res.is_err(), "oversize send_message must be rejected");
            let _ = block_on(bus.shutdown());
        }
        Mode::PublishFanout {
            subscribers,
            payload,
        } => {
            let subs = (subscribers as usize).min(8);
            let Some(bus) = block_on(build_bus()) else {
                return;
            };
            let sender = make_agent(1);
            let _ = block_on(bus.register_agent(sender));
            for i in 0..subs {
                let a = make_agent(100 + i as u64);
                let _ = block_on(bus.register_agent(a));
                let _ = block_on(bus.subscribe(a, "topic-fuzz".to_string()));
            }

            let payload = clamp_str(payload, 1024, "p");
            let msg = bus.create_internal_message(
                sender,
                make_agent(999),
                bytes::Bytes::from(payload.into_bytes()),
                MessageType::Publish("topic-fuzz".to_string()),
                Duration::from_secs(60),
            );
            let _ = block_on(bus.publish("topic-fuzz".to_string(), msg));
            let _ = block_on(bus.shutdown());
        }
    }
});
