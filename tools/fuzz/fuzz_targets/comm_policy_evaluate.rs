#![no_main]

//! Fuzz target for `CommunicationPolicyGate::evaluate`.
//!
//! Builds arbitrary rule sets with nested `All`/`Any` conditions and
//! evaluates arbitrary requests against them. Asserts:
//! - Evaluation never panics, even with deeply nested conditions.
//! - Deny-by-default: when no rules match, the default policy applies.
//! - Permissive: empty rules + `permissive()` always returns Ok.
//! - Rule priority: higher-priority matching rule wins over lower.

use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use symbi_runtime::communication::policy_gate::{
    CommunicationCondition, CommunicationEffect, CommunicationPolicyGate, CommunicationPolicyRule,
    CommunicationRequest,
};
use symbi_runtime::types::{AgentId, MessageType, RequestId};
use uuid::Uuid;

#[derive(Arbitrary, Debug)]
struct Input {
    rules: Vec<RuleSpec>,
    requests: Vec<RequestSpec>,
    default_allow: bool,
    mode: Mode,
}

#[derive(Arbitrary, Debug)]
enum Mode {
    /// Evaluate every request against the built gate.
    EvaluateAll,
    /// Confirm permissive shortcut.
    PermissiveShortcut,
    /// Confirm deny-by-default: empty rules + default-deny must deny.
    DenyByDefaultShortcut,
    /// Priority: if a high-priority allow rule matches "Always" and a
    /// low-priority deny rule also matches "Always", allow must win.
    PriorityCheck,
}

#[derive(Arbitrary, Debug)]
struct RuleSpec {
    id: String,
    name: String,
    condition: CondSpec,
    effect: EffectSpec,
    priority: u32,
}

#[derive(Arbitrary, Debug)]
enum CondSpec {
    Always,
    SenderIsIdx(u8),
    RecipientIsIdx(u8),
    All(Vec<CondSpec>),
    Any(Vec<CondSpec>),
}

#[derive(Arbitrary, Debug)]
enum EffectSpec {
    Allow,
    Deny(String),
}

#[derive(Arbitrary, Debug)]
struct RequestSpec {
    sender_idx: u8,
    recipient_idx: u8,
    kind: MsgKindSpec,
    topic: Option<String>,
}

#[derive(Arbitrary, Debug)]
enum MsgKindSpec {
    Direct,
    Publish(String),
    Subscribe(String),
    Broadcast,
    Request,
    Response,
}

const MAX_DEPTH: usize = 8;
const AGENT_POOL_SIZE: u8 = 16;

fn agent_for(idx: u8) -> AgentId {
    AgentId(Uuid::from_u64_pair(0, (idx % AGENT_POOL_SIZE) as u64))
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

fn cond_from(spec: &CondSpec, depth: usize) -> CommunicationCondition {
    if depth >= MAX_DEPTH {
        return CommunicationCondition::Always;
    }
    match spec {
        CondSpec::Always => CommunicationCondition::Always,
        CondSpec::SenderIsIdx(i) => CommunicationCondition::SenderIs(agent_for(*i)),
        CondSpec::RecipientIsIdx(i) => CommunicationCondition::RecipientIs(agent_for(*i)),
        CondSpec::All(children) => CommunicationCondition::All(
            children
                .iter()
                .take(8)
                .map(|c| cond_from(c, depth + 1))
                .collect(),
        ),
        CondSpec::Any(children) => CommunicationCondition::Any(
            children
                .iter()
                .take(8)
                .map(|c| cond_from(c, depth + 1))
                .collect(),
        ),
    }
}

fn rule_from(spec: &RuleSpec) -> CommunicationPolicyRule {
    let id = clamp(spec.id.clone(), 64, "rule");
    let name = clamp(spec.name.clone(), 64, "rule-name");
    let effect = match &spec.effect {
        EffectSpec::Allow => CommunicationEffect::Allow,
        EffectSpec::Deny(r) => CommunicationEffect::Deny {
            reason: clamp(r.clone(), 128, "policy denied"),
        },
    };
    CommunicationPolicyRule {
        id,
        name,
        condition: cond_from(&spec.condition, 0),
        effect,
        priority: spec.priority,
    }
}

fn request_from(spec: &RequestSpec) -> CommunicationRequest {
    let recipient = agent_for(spec.recipient_idx);
    let message_type = match &spec.kind {
        MsgKindSpec::Direct => MessageType::Direct(recipient),
        MsgKindSpec::Publish(t) => MessageType::Publish(clamp(t.clone(), 64, "t")),
        MsgKindSpec::Subscribe(t) => MessageType::Subscribe(clamp(t.clone(), 64, "t")),
        MsgKindSpec::Broadcast => MessageType::Broadcast,
        MsgKindSpec::Request => MessageType::Request(RequestId::new()),
        MsgKindSpec::Response => MessageType::Response(RequestId::new()),
    };
    CommunicationRequest {
        sender: agent_for(spec.sender_idx),
        recipient,
        message_type,
        topic: spec.topic.as_ref().map(|t| clamp(t.clone(), 64, "t")),
    }
}

fuzz_target!(|input: Input| {
    let rules: Vec<_> = input.rules.iter().take(32).map(rule_from).collect();

    match input.mode {
        Mode::EvaluateAll => {
            let gate = if input.default_allow {
                // Note: `new()` is deny-by-default; we can't directly flip
                // the flag for a rule-carrying gate without the builder, so
                // use `deny_by_default` for the fuzz contract and vary rules.
                CommunicationPolicyGate::deny_by_default(rules)
            } else {
                CommunicationPolicyGate::new(rules)
            };
            for req_spec in input.requests.iter().take(16) {
                let req = request_from(req_spec);
                // Must never panic.
                let _ = gate.evaluate(&req);
            }
        }
        Mode::PermissiveShortcut => {
            let gate = CommunicationPolicyGate::permissive();
            for req_spec in input.requests.iter().take(16) {
                let req = request_from(req_spec);
                assert!(
                    gate.evaluate(&req).is_ok(),
                    "permissive gate must always allow"
                );
            }
        }
        Mode::DenyByDefaultShortcut => {
            let gate = CommunicationPolicyGate::new(Vec::new());
            for req_spec in input.requests.iter().take(16) {
                let req = request_from(req_spec);
                assert!(
                    gate.evaluate(&req).is_err(),
                    "deny-by-default gate with no rules must deny"
                );
            }
        }
        Mode::PriorityCheck => {
            // Craft two rules: low-priority Deny-Always, high-priority Allow-Always.
            // Any request must return Ok because the higher-priority Allow wins.
            let allow_high = CommunicationPolicyRule {
                id: "allow-high".to_string(),
                name: "allow-high".to_string(),
                condition: CommunicationCondition::Always,
                effect: CommunicationEffect::Allow,
                priority: 100,
            };
            let deny_low = CommunicationPolicyRule {
                id: "deny-low".to_string(),
                name: "deny-low".to_string(),
                condition: CommunicationCondition::Always,
                effect: CommunicationEffect::Deny {
                    reason: "low".into(),
                },
                priority: 1,
            };
            let gate = CommunicationPolicyGate::new(vec![deny_low, allow_high]);
            for req_spec in input.requests.iter().take(8) {
                let req = request_from(req_spec);
                assert!(
                    gate.evaluate(&req).is_ok(),
                    "higher-priority Allow must win over lower-priority Deny"
                );
            }
        }
    }
});
