//! Integration tests: project each example onto all roles, build FSMs, and
//! replay the canonical happy-path trace through every role's FSM, asserting all
//! reach an accepting state. Also assert one illegal trace is rejected.

use std::collections::HashMap;

use symbi_session::examples;
use symbi_session::fsm::{Event, Fsm};
use symbi_session::global::Role;
use symbi_session::project::project;
use symbi_session::wellformed::check_well_formed;

/// A protocol-level message: `from -> to : label`. Each such message produces a
/// `Send` event for `from` and a `Recv` event for `to`.
struct Msg {
    from: &'static str,
    to: &'static str,
    label: &'static str,
}

fn msg(from: &'static str, to: &'static str, label: &'static str) -> Msg {
    Msg { from, to, label }
}

/// Build the per-role FSMs for a protocol.
fn fsms_for(g: &symbi_session::global::Global, roles: &[Role]) -> HashMap<Role, Fsm> {
    roles
        .iter()
        .map(|r| {
            let l = project(g, r).unwrap_or_else(|e| panic!("project onto {r} failed: {e}"));
            (r.clone(), Fsm::from_local(&l))
        })
        .collect()
}

/// Replay a sequence of protocol messages against the per-role FSMs, advancing
/// each involved role's state. Returns the final state per role.
fn replay(fsms: &HashMap<Role, Fsm>, trace: &[Msg]) -> HashMap<Role, symbi_session::fsm::StateId> {
    let mut states: HashMap<Role, _> = fsms.iter().map(|(r, f)| (r.clone(), f.start())).collect();
    for m in trace {
        // Sender steps a Send event.
        let sender = m.from.to_string();
        let s = states[&sender];
        let ns = fsms[&sender]
            .step(
                s,
                &Event::Send {
                    to: m.to.to_string(),
                    label: m.label.to_string(),
                },
            )
            .unwrap_or_else(|e| panic!("sender {} rejected {}: {e}", m.from, m.label));
        states.insert(sender, ns);

        // Receiver steps a Recv event.
        let receiver = m.to.to_string();
        let s = states[&receiver];
        let ns = fsms[&receiver]
            .step(
                s,
                &Event::Recv {
                    from: m.from.to_string(),
                    label: m.label.to_string(),
                },
            )
            .unwrap_or_else(|e| panic!("receiver {} rejected {}: {e}", m.to, m.label));
        states.insert(receiver, ns);
    }
    states
}

fn assert_all_accepting(fsms: &HashMap<Role, Fsm>, states: &HashMap<Role, usize>) {
    for (r, s) in states {
        assert!(
            fsms[r].is_accepting(*s),
            "role {r} did not reach an accepting state"
        );
    }
}

#[test]
fn request_response_happy_path() {
    let (g, roles) = examples::request_response();
    assert!(check_well_formed(&g, &roles).is_ok());
    let fsms = fsms_for(&g, &roles);
    let trace = [
        msg("Client", "Server", "req"),
        msg("Server", "Client", "resp"),
    ];
    let states = replay(&fsms, &trace);
    assert_all_accepting(&fsms, &states);
}

#[test]
fn coordinator_pipeline_happy_path() {
    let (g, roles) = examples::coordinator_pipeline();
    assert!(check_well_formed(&g, &roles).is_ok());
    let fsms = fsms_for(&g, &roles);
    let trace = [
        msg("Coordinator", "Validator", "task"),
        msg("Validator", "Coordinator", "ok"),
        msg("Coordinator", "Processor", "task"),
        msg("Processor", "Coordinator", "done"),
    ];
    let states = replay(&fsms, &trace);
    assert_all_accepting(&fsms, &states);
}

#[test]
fn race_choice_happy_path() {
    let (g, roles) = examples::race_choice();
    assert!(check_well_formed(&g, &roles).is_ok());
    let fsms = fsms_for(&g, &roles);
    // Coordinator picks the "fast" branch.
    let trace = [
        msg("Coordinator", "Worker", "fast"),
        msg("Worker", "Coordinator", "result"),
    ];
    let states = replay(&fsms, &trace);
    assert_all_accepting(&fsms, &states);
}

#[test]
fn retry_loop_happy_path() {
    let (g, roles) = examples::retry_loop();
    assert!(check_well_formed(&g, &roles).is_ok());
    let fsms = fsms_for(&g, &roles);
    // try, retry, try, ok.
    let trace = [
        msg("Coordinator", "Worker", "try"),
        msg("Worker", "Coordinator", "retry"),
        msg("Coordinator", "Worker", "try"),
        msg("Worker", "Coordinator", "ok"),
    ];
    let states = replay(&fsms, &trace);
    assert_all_accepting(&fsms, &states);
}

#[test]
fn illegal_trace_is_rejected() {
    let (g, roles) = examples::request_response();
    let fsms = fsms_for(&g, &roles);
    // Server tries to send before receiving the request.
    let server = "Server".to_string();
    let s0 = fsms[&server].start();
    let err = fsms[&server]
        .step(
            s0,
            &Event::Send {
                to: "Client".into(),
                label: "resp".into(),
            },
        )
        .unwrap_err();
    // The monitor should be able to say what it expected instead.
    assert!(!err.expected.is_empty());
    assert_eq!(
        err.expected[0],
        Event::Recv {
            from: "Client".into(),
            label: "req".into()
        }
    );
}
