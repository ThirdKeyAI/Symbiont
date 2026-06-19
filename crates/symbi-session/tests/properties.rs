//! Property tests.
//!
//! Two properties are checked:
//!
//! * **Projection totality** over a broad generated set of *well-formed* global
//!   protocols (linear messages, guarded recursion, and well-behaved choices):
//!   projection onto every role returns `Ok` and never panics.
//! * **Trace soundness** restricted to the *linear* fragment (no choice, no
//!   recursion): the canonical message sequence, replayed as Send/Recv events
//!   against the per-role FSMs, drives every role to an accepting state.
//!
//! Trace equivalence for the full branch+loop fragment is intentionally out of
//! scope for the spike; the linear restriction keeps the soundness property
//! tractable while still exercising projection + FSM compilation end to end.

use std::collections::HashMap;

use proptest::prelude::*;

use symbi_session::fsm::{Event, Fsm};
use symbi_session::global::{Global, Role};
use symbi_session::project::project;
use symbi_session::wellformed::check_well_formed;

/// Fixed small role pool used by every generated protocol.
const ROLES: &[&str] = &["A", "B", "C"];

fn role_vec() -> Vec<Role> {
    ROLES.iter().map(|s| s.to_string()).collect()
}

/// A protocol-level message used to drive FSMs in the soundness check.
#[derive(Clone, Debug)]
struct Msg {
    from: String,
    to: String,
    label: String,
}

// ---- Linear fragment generation -------------------------------------------

/// Generate a linear protocol (a chain of distinct-direction messages ending in
/// `end`) together with the flat list of messages it contains.
fn linear_strategy() -> impl Strategy<Value = (Global, Vec<Msg>)> {
    // 0..=6 messages, each with an ordered (from, to) pair of distinct roles and
    // a label drawn from a tiny set.
    let role_idx = 0usize..ROLES.len();
    let msg_strat =
        (role_idx.clone(), role_idx, 0u8..3).prop_filter_map("distinct from/to", |(a, b, l)| {
            if a == b {
                None
            } else {
                Some(Msg {
                    from: ROLES[a].to_string(),
                    to: ROLES[b].to_string(),
                    label: format!("m{l}"),
                })
            }
        });
    prop::collection::vec(msg_strat, 0..6).prop_map(|msgs| {
        let mut g = Global::End;
        for m in msgs.iter().rev() {
            g = Global::msg(m.from.clone(), m.to.clone(), m.label.clone(), g);
        }
        (g, msgs)
    })
}

// ---- Broader well-formed generation (for totality) ------------------------

/// Generate a structurally varied global protocol: linear messages, two-way
/// choices, and guarded loops, nested to bounded depth. Recursion is guarded by
/// construction and variables only appear under their binder, so the generated
/// protocols always satisfy the guardedness and bound-variable checks; they are
/// *not* guaranteed projectable, because a choice may leave an uninvolved role
/// with unmergeable branch behaviour (this is exactly the partiality the
/// totality property exercises).
fn global_strategy() -> impl Strategy<Value = Global> {
    // Leaf: end, or (if a binder is in scope we still just emit end here; the
    // recursive case wires guarded vars).
    let leaf = Just(Global::End);

    leaf.prop_recursive(4, 32, 3, |inner| {
        let roles = || prop::sample::select(ROLES);
        let label = (0u8..3).prop_map(|l| format!("m{l}"));

        // A single message followed by a sub-protocol.
        let message = (roles(), roles(), label.clone(), inner.clone()).prop_filter_map(
            "distinct from/to",
            |(a, b, lab, cont)| {
                if a == b {
                    None
                } else {
                    Some(Global::msg(a, b, lab, cont))
                }
            },
        );

        // A choice with two branches; chooser->target fixed across branches so
        // uninvolved roles see consistent structure.
        let choice = (roles(), roles(), inner.clone(), inner.clone()).prop_filter_map(
            "distinct chooser/target",
            |(a, b, c1, c2)| {
                if a == b {
                    None
                } else {
                    Some(Global::choice(
                        a,
                        b,
                        vec![("opt0".to_string(), c1), ("opt1".to_string(), c2)],
                    ))
                }
            },
        );

        // A guarded loop: rec X . from -> to : m ; { keep . X | stop . end }.
        let loop_proto =
            (roles(), roles(), inner).prop_filter_map("distinct loop roles", |(a, b, _cont)| {
                if a == b {
                    None
                } else {
                    Some(Global::rec(
                        "X",
                        Global::msg(
                            a,
                            b,
                            "tick",
                            Global::choice(
                                b,
                                a,
                                vec![
                                    ("stop".to_string(), Global::end()),
                                    ("keep".to_string(), Global::var("X")),
                                ],
                            ),
                        ),
                    ))
                }
            });

        prop_oneof![message, choice, loop_proto]
    })
}

// ---- Replay helper --------------------------------------------------------

fn replay_linear(g: &Global, roles: &[Role], msgs: &[Msg]) -> Result<(), String> {
    let fsms: HashMap<Role, Fsm> = roles
        .iter()
        .map(|r| {
            let l = project(g, r).map_err(|e| format!("project {r}: {e}"))?;
            Ok((r.clone(), Fsm::from_local(&l)))
        })
        .collect::<Result<_, String>>()?;

    let mut states: HashMap<Role, _> = fsms.iter().map(|(r, f)| (r.clone(), f.start())).collect();

    for m in msgs {
        let s = states[&m.from];
        let ns = fsms[&m.from]
            .step(
                s,
                &Event::Send {
                    to: m.to.clone(),
                    label: m.label.clone(),
                },
            )
            .map_err(|e| format!("sender {} step: {e}", m.from))?;
        states.insert(m.from.clone(), ns);

        let s = states[&m.to];
        let ns = fsms[&m.to]
            .step(
                s,
                &Event::Recv {
                    from: m.from.clone(),
                    label: m.label.clone(),
                },
            )
            .map_err(|e| format!("receiver {} step: {e}", m.to))?;
        states.insert(m.to.clone(), ns);
    }

    for (r, s) in &states {
        if !fsms[r].is_accepting(*s) {
            return Err(format!("role {r} not accepting at end"));
        }
    }
    Ok(())
}

proptest! {
    /// Projection is a *total function* on the broad generated set: for any
    /// generated global, projecting onto any role returns a `Result` without
    /// panicking (it may legitimately be an `Unmergeable` error). Additionally,
    /// whenever well-formedness holds, projection onto every role succeeds — the
    /// two notions are kept consistent.
    #[test]
    fn projection_is_total_and_agrees_with_well_formedness(g in global_strategy()) {
        let roles = role_vec();
        let wf = check_well_formed(&g, &roles).is_ok();
        for r in &roles {
            // No panics regardless of well-formedness.
            let projected = project(&g, r);
            if wf {
                prop_assert!(projected.is_ok(),
                    "well-formed protocol failed to project onto {r}: {g:?}");
            }
        }
    }

    /// Trace soundness on the linear fragment.
    #[test]
    fn linear_trace_drives_to_accepting((g, msgs) in linear_strategy()) {
        let roles = role_vec();
        prop_assert!(check_well_formed(&g, &roles).is_ok());
        prop_assert!(replay_linear(&g, &roles, &msgs).is_ok());
    }
}
