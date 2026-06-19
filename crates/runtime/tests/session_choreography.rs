//! End-to-end viability probe: drive the `coordinator_pipeline` choreography
//! through the REAL [`CommunicationPolicyGate`] with a [`SessionMonitor`]
//! enforcing message ordering.
//!
//! The gate is configured to allow all messages by policy (`default_allow`), so
//! that the ONLY governance over message order comes from the session monitor.
//! A conforming sequence is accepted and completes the session; an out-of-order
//! sequence is rejected with an explanatory error.
#![cfg(feature = "session")]

use std::collections::HashMap;
use std::sync::Arc;

use symbi_runtime::communication::policy_gate::{CommunicationPolicyGate, CommunicationRequest};
use symbi_runtime::types::{AgentId, MessageType, RequestId};
use symbi_session::examples::coordinator_pipeline;
use symbi_session::monitor::{SessionId, SessionMonitor};

/// Build a request carrying session metadata for one choreography message.
fn req(sender: AgentId, recipient: AgentId, sid: &SessionId, label: &str) -> CommunicationRequest {
    CommunicationRequest {
        sender,
        recipient,
        message_type: MessageType::Request(RequestId::new()),
        topic: None,
        session_id: Some(sid.to_string()),
        protocol_label: Some(label.to_string()),
    }
}

/// Bind the three concrete agents to the protocol roles.
fn assignment(
    coordinator: AgentId,
    validator: AgentId,
    processor: AgentId,
) -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert(coordinator.to_string(), "Coordinator".to_string());
    m.insert(validator.to_string(), "Validator".to_string());
    m.insert(processor.to_string(), "Processor".to_string());
    m
}

#[test]
fn conforming_run_is_allowed_and_completes() {
    let (global, _roles) = coordinator_pipeline();

    let coordinator = AgentId::new();
    let validator = AgentId::new();
    let processor = AgentId::new();

    let monitor = Arc::new(SessionMonitor::new());
    let sid = SessionId("choreo-conforming".to_string());
    monitor
        .establish(
            sid.clone(),
            &global,
            assignment(coordinator, validator, processor),
        )
        .expect("session established");

    // Gate allows everything by policy; only the session monitor governs order.
    let gate = CommunicationPolicyGate::permissive().with_session_monitor(monitor.clone());

    // 1. Coordinator -> Validator : task
    assert!(gate
        .evaluate(&req(coordinator, validator, &sid, "task"))
        .is_ok());
    // 2. Validator -> Coordinator : ok
    assert!(gate
        .evaluate(&req(validator, coordinator, &sid, "ok"))
        .is_ok());
    // 3. Coordinator -> Processor : task
    assert!(gate
        .evaluate(&req(coordinator, processor, &sid, "task"))
        .is_ok());
    // 4. Processor -> Coordinator : done
    assert!(gate
        .evaluate(&req(processor, coordinator, &sid, "done"))
        .is_ok());

    assert!(
        monitor.is_complete(&sid),
        "session should be complete after the conforming sequence"
    );
}

#[test]
fn out_of_order_run_is_rejected_with_clear_message() {
    let (global, _roles) = coordinator_pipeline();

    let coordinator = AgentId::new();
    let validator = AgentId::new();
    let processor = AgentId::new();

    let monitor = Arc::new(SessionMonitor::new());
    let sid = SessionId("choreo-out-of-order".to_string());
    monitor
        .establish(
            sid.clone(),
            &global,
            assignment(coordinator, validator, processor),
        )
        .expect("session established");

    let gate = CommunicationPolicyGate::permissive().with_session_monitor(monitor.clone());

    // Coordinator -> Processor : task is illegal as the first message; the
    // protocol must begin with Coordinator -> Validator : task.
    let result = gate.evaluate(&req(coordinator, processor, &sid, "task"));
    assert!(result.is_err(), "out-of-order message must be rejected");

    let msg = result.unwrap_err().to_string().to_lowercase();
    assert!(
        msg.contains("session"),
        "rejection should mention 'session': {msg}"
    );
    assert!(
        msg.contains("illegal"),
        "rejection should mention 'illegal': {msg}"
    );
    assert!(
        msg.contains("expected"),
        "rejection should mention 'expected': {msg}"
    );

    assert!(
        !monitor.is_complete(&sid),
        "session must not be complete after a rejected message"
    );
}

// ── Task 7 additions ──────────────────────────────────────────────────────────

#[test]
fn branch_routing_allows_a_legal_choice_via_registry() {
    use std::time::Duration;
    use symbi_runtime::session::{RoleBinding, SessionRegistry};
    use symbi_session::examples::race_choice;

    // race_choice: Coordinator -> Worker : { fast . Worker -> Coordinator : result,
    //                                        slow . Worker -> Coordinator : result }
    let (g, _roles) = race_choice();
    let reg = SessionRegistry::new();
    let coord = AgentId::new();
    let worker = AgentId::new();

    let rb = RoleBinding::new()
        .bind(coord, "Coordinator")
        .bind(worker, "Worker");
    let sid = reg.open(&g, rb, Duration::from_secs(60)).unwrap();

    let gate = CommunicationPolicyGate::permissive().with_session_monitor(reg.monitor());

    // A legal choice label ("fast") from Coordinator to Worker must be allowed.
    let good = CommunicationRequest {
        sender: coord,
        recipient: worker,
        message_type: MessageType::Request(RequestId::new()),
        topic: None,
        session_id: Some(sid.to_string()),
        protocol_label: Some("fast".into()),
    };
    assert!(
        gate.evaluate(&good).is_ok(),
        "legal label 'fast' must be allowed"
    );

    // An illegal label for the same step is rejected with a session-related error.
    let bad = CommunicationRequest {
        sender: coord,
        recipient: worker,
        message_type: MessageType::Request(RequestId::new()),
        topic: None,
        session_id: Some(sid.to_string()),
        protocol_label: Some("totally-not-a-protocol-label".into()),
    };
    let err = gate.evaluate(&bad).unwrap_err();
    assert!(
        format!("{err}").contains("session"),
        "rejection must mention 'session': {err}"
    );
}

#[test]
fn deadline_abort_then_observe_denied() {
    use std::time::Duration;
    use symbi_runtime::session::{RoleBinding, SessionRegistry, SessionStatus};
    use symbi_session::examples::coordinator_pipeline;

    let (g, _r) = coordinator_pipeline();
    let reg = SessionRegistry::new();
    let (c, v, p) = (AgentId::new(), AgentId::new(), AgentId::new());

    let rb = RoleBinding::new()
        .bind(c, "Coordinator")
        .bind(v, "Validator")
        .bind(p, "Processor");
    let sid = reg.open(&g, rb, Duration::from_millis(0)).unwrap();

    // Expire the session immediately.
    assert!(
        reg.abort_expired().contains(&sid),
        "session with zero TTL must be in the aborted list"
    );
    assert_eq!(reg.status(&sid), Some(SessionStatus::Aborted));

    // Any subsequent observe on the monitor must return Aborted.
    let err = reg
        .monitor()
        .observe(&sid, &c.to_string(), &v.to_string(), "task")
        .unwrap_err();
    assert!(
        matches!(err, symbi_session::monitor::SessionError::Aborted(_)),
        "expected Aborted, got: {err}"
    );
}
