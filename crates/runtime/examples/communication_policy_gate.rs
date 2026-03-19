//! CommunicationPolicyGate Example
//!
//! Demonstrates how to configure and evaluate inter-agent communication policies.
//! The policy gate controls which agents can communicate, using priority-based
//! rule evaluation with Cedar-style allow/deny effects.
//!
//! Run with: cargo run -j2 --example communication_policy_gate

use symbi_runtime::communication::policy_gate::{
    CommunicationCondition, CommunicationEffect, CommunicationPolicyGate, CommunicationPolicyRule,
    CommunicationRequest,
};
use symbi_runtime::types::{AgentId, MessageType, RequestId};

fn main() {
    println!("=== CommunicationPolicyGate Example ===\n");

    // Create some agent identities
    let coordinator = AgentId::new();
    let worker_a = AgentId::new();
    let worker_b = AgentId::new();
    let untrusted = AgentId::new();

    println!("Agents:");
    println!("  coordinator: {}", coordinator);
    println!("  worker_a:    {}", worker_a);
    println!("  worker_b:    {}", worker_b);
    println!("  untrusted:   {}", untrusted);
    println!();

    // --- Example 1: Permissive gate (default) ---
    println!("--- Example 1: Permissive Gate (no rules) ---");
    let permissive = CommunicationPolicyGate::permissive();

    let request = CommunicationRequest {
        sender: worker_a,
        recipient: coordinator,
        message_type: MessageType::Request(RequestId::new()),
        topic: None,
    };
    match permissive.evaluate(&request) {
        Ok(()) => println!("  worker_a → coordinator: ALLOWED (no rules, default allow)"),
        Err(e) => println!("  worker_a → coordinator: DENIED — {}", e),
    }
    println!();

    // --- Example 2: Block a specific agent ---
    println!("--- Example 2: Block Untrusted Agent ---");
    let gate = CommunicationPolicyGate::new(vec![CommunicationPolicyRule {
        id: "block-untrusted".into(),
        name: "block-untrusted".into(),
        condition: CommunicationCondition::SenderIs(untrusted),
        effect: CommunicationEffect::Deny {
            reason: "Agent is not trusted".into(),
        },
        priority: 10,
    }]);

    // Untrusted agent tries to talk — blocked
    let blocked = CommunicationRequest {
        sender: untrusted,
        recipient: coordinator,
        message_type: MessageType::Request(RequestId::new()),
        topic: None,
    };
    match gate.evaluate(&blocked) {
        Ok(()) => println!("  untrusted → coordinator: ALLOWED"),
        Err(e) => println!("  untrusted → coordinator: DENIED — {}", e),
    }

    // Worker A can still talk — no matching deny rule
    let allowed = CommunicationRequest {
        sender: worker_a,
        recipient: coordinator,
        message_type: MessageType::Request(RequestId::new()),
        topic: None,
    };
    match gate.evaluate(&allowed) {
        Ok(()) => println!("  worker_a → coordinator: ALLOWED (no matching rule, default allow)"),
        Err(e) => println!("  worker_a → coordinator: DENIED — {}", e),
    }
    println!();

    // --- Example 3: Workers cannot delegate to each other ---
    println!("--- Example 3: Prevent Worker-to-Worker Delegation ---");
    let gate = CommunicationPolicyGate::new(vec![
        // High priority: block worker-to-worker
        CommunicationPolicyRule {
            id: "no-worker-lateral".into(),
            name: "no-worker-lateral".into(),
            condition: CommunicationCondition::All(vec![
                CommunicationCondition::Any(vec![
                    CommunicationCondition::SenderIs(worker_a),
                    CommunicationCondition::SenderIs(worker_b),
                ]),
                CommunicationCondition::Any(vec![
                    CommunicationCondition::RecipientIs(worker_a),
                    CommunicationCondition::RecipientIs(worker_b),
                ]),
            ]),
            effect: CommunicationEffect::Deny {
                reason: "Workers cannot communicate directly — use the coordinator".into(),
            },
            priority: 20,
        },
        // Low priority: allow everything else
        CommunicationPolicyRule {
            id: "default-allow".into(),
            name: "default-allow".into(),
            condition: CommunicationCondition::Always,
            effect: CommunicationEffect::Allow,
            priority: 1,
        },
    ]);

    // Worker A → Worker B: blocked
    let lateral = CommunicationRequest {
        sender: worker_a,
        recipient: worker_b,
        message_type: MessageType::Request(RequestId::new()),
        topic: None,
    };
    match gate.evaluate(&lateral) {
        Ok(()) => println!("  worker_a → worker_b: ALLOWED"),
        Err(e) => println!("  worker_a → worker_b: DENIED — {}", e),
    }

    // Worker A → Coordinator: allowed
    let to_coord = CommunicationRequest {
        sender: worker_a,
        recipient: coordinator,
        message_type: MessageType::Request(RequestId::new()),
        topic: None,
    };
    match gate.evaluate(&to_coord) {
        Ok(()) => println!("  worker_a → coordinator: ALLOWED"),
        Err(e) => println!("  worker_a → coordinator: DENIED — {}", e),
    }

    // Coordinator → Worker A: allowed
    let from_coord = CommunicationRequest {
        sender: coordinator,
        recipient: worker_a,
        message_type: MessageType::Request(RequestId::new()),
        topic: None,
    };
    match gate.evaluate(&from_coord) {
        Ok(()) => println!("  coordinator → worker_a: ALLOWED"),
        Err(e) => println!("  coordinator → worker_a: DENIED — {}", e),
    }
    println!();

    // --- Example 4: Deny-by-default (whitelist mode) ---
    println!("--- Example 4: Deny-by-Default (Whitelist) ---");
    let gate = CommunicationPolicyGate::deny_by_default(vec![
        // Only the coordinator can initiate communication
        CommunicationPolicyRule {
            id: "coordinator-only".into(),
            name: "coordinator-only".into(),
            condition: CommunicationCondition::SenderIs(coordinator),
            effect: CommunicationEffect::Allow,
            priority: 10,
        },
    ]);

    let coord_sends = CommunicationRequest {
        sender: coordinator,
        recipient: worker_a,
        message_type: MessageType::Request(RequestId::new()),
        topic: None,
    };
    match gate.evaluate(&coord_sends) {
        Ok(()) => println!("  coordinator → worker_a: ALLOWED (whitelisted sender)"),
        Err(e) => println!("  coordinator → worker_a: DENIED — {}", e),
    }

    let worker_sends = CommunicationRequest {
        sender: worker_a,
        recipient: coordinator,
        message_type: MessageType::Request(RequestId::new()),
        topic: None,
    };
    match gate.evaluate(&worker_sends) {
        Ok(()) => println!("  worker_a → coordinator: ALLOWED"),
        Err(e) => println!("  worker_a → coordinator: DENIED — {}", e),
    }

    println!("\n=== Done ===");
}
