//! Compilation of a [`Local`] type into a finite-state machine that a runtime
//! monitor can step one I/O event at a time.
//!
//! Each state corresponds to a point in the local type. `Send`/`Recv` produce a
//! single outgoing transition; `Select`/`Branch` produce one transition per
//! label; `Rec`/`Var` are resolved into a cyclic state graph by remembering the
//! state created for each recursion binder and looping `Var` back to it; `End`
//! is an accepting state with no transitions.

use crate::global::{Label, Role};
use crate::local::Local;

/// Index of a state within an [`Fsm`].
pub type StateId = usize;

/// A communication event observed by the monitor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// This role sends `label` to `to`.
    Send { to: Role, label: Label },
    /// This role receives `label` from `from`.
    Recv { from: Role, label: Label },
}

impl std::fmt::Display for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Event::Send { to, label } => write!(f, "send '{label}' to '{to}'"),
            Event::Recv { from, label } => write!(f, "recv '{label}' from '{from}'"),
        }
    }
}

/// Returned by [`Fsm::step`] when an event does not match any legal transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IllegalTransition {
    /// The events that would have been accepted from the current state.
    pub expected: Vec<Event>,
    /// The event that was actually offered.
    pub got: Event,
}

impl Event {
    fn direction_peer(&self) -> (&'static str, &str) {
        match self {
            Event::Send { to, .. } => ("send", to),
            Event::Recv { from, .. } => ("recv", from),
        }
    }

    fn label(&self) -> &str {
        match self {
            Event::Send { label, .. } | Event::Recv { label, .. } => label,
        }
    }
}

impl IllegalTransition {
    /// A precise, recovery-oriented message. When an expected event shares the
    /// same direction+peer as `got` but differs only in label, emphasize the
    /// label mismatch; otherwise report the structural mismatch and list options.
    pub fn diagnose(&self) -> String {
        let (gdir, gpeer) = self.got.direction_peer();
        if let Some(exp) = self.expected.iter().find(|e| {
            let (d, p) = e.direction_peer();
            d == gdir && p == gpeer
        }) {
            if exp.label() != self.got.label() {
                return format!(
                    "expected label '{}' to '{}', but you sent label '{}'",
                    exp.label(),
                    gpeer,
                    self.got.label()
                );
            }
        }
        if self.expected.is_empty() {
            return format!(
                "no further events allowed (session ended); you sent {}",
                self.got
            );
        }
        let opts = self
            .expected
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        format!("expected one of: {opts}; but you sent {}", self.got)
    }
}

impl std::fmt::Display for IllegalTransition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let expected = if self.expected.is_empty() {
            "no further events (session ended)".to_string()
        } else {
            self.expected
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        write!(
            f,
            "illegal transition: got {} but expected one of: {expected}",
            self.got
        )
    }
}

#[derive(Debug, Clone)]
struct State {
    /// Legal `(event, next state)` transitions out of this state.
    transitions: Vec<(Event, StateId)>,
    /// Whether reaching this state is a successful end of the session.
    accepting: bool,
}

/// A finite-state machine compiled from a local type.
#[derive(Debug, Clone)]
pub struct Fsm {
    states: Vec<State>,
    start: StateId,
}

impl Fsm {
    /// Compile a local type into an FSM.
    pub fn from_local(l: &Local) -> Fsm {
        let mut builder = Builder {
            states: Vec::new(),
            rec_env: Vec::new(),
        };
        let start = builder.build(l);
        Fsm {
            states: builder.states,
            start,
        }
    }

    /// The initial state.
    pub fn start(&self) -> StateId {
        self.start
    }

    /// Attempt the transition for `ev` from state `s`.
    pub fn step(&self, s: StateId, ev: &Event) -> Result<StateId, IllegalTransition> {
        for (e, next) in &self.states[s].transitions {
            if e == ev {
                return Ok(*next);
            }
        }
        Err(IllegalTransition {
            expected: self.expected(s),
            got: ev.clone(),
        })
    }

    /// Whether `s` is an accepting (end) state.
    pub fn is_accepting(&self, s: StateId) -> bool {
        self.states[s].accepting
    }

    /// The legal events from state `s`.
    pub fn expected(&self, s: StateId) -> Vec<Event> {
        self.states[s]
            .transitions
            .iter()
            .map(|(e, _)| e.clone())
            .collect()
    }
}

/// Construction-time helper carrying the state arena and the binder→state map.
struct Builder {
    states: Vec<State>,
    /// Stack of `(recursion var, state id)` bindings currently in scope.
    rec_env: Vec<(String, StateId)>,
}

impl Builder {
    fn fresh(&mut self, accepting: bool) -> StateId {
        let id = self.states.len();
        self.states.push(State {
            transitions: Vec::new(),
            accepting,
        });
        id
    }

    /// Build states for `l` and return the entry state id.
    fn build(&mut self, l: &Local) -> StateId {
        match l {
            Local::End => self.fresh(true),
            Local::Var(v) => {
                // Loop back to the binder's state. An unbound var (should not
                // happen for well-formed input) becomes a dead accepting state.
                match self.rec_env.iter().rev().find(|(name, _)| name == v) {
                    Some((_, sid)) => *sid,
                    None => self.fresh(true),
                }
            }
            Local::Rec { var, body } => {
                // Reserve a state for the binder so `Var` can loop to it, then
                // wire the binder to behave exactly like the body's entry.
                let entry = self.fresh(false);
                self.rec_env.push((var.clone(), entry));
                let body_entry = self.build(body);
                self.rec_env.pop();
                // Make the binder state an alias of the body entry by copying
                // its transitions and acceptance.
                let (transitions, accepting) = {
                    let s = &self.states[body_entry];
                    (s.transitions.clone(), s.accepting)
                };
                self.states[entry].transitions = transitions;
                self.states[entry].accepting = accepting;
                entry
            }
            Local::Send { to, label, cont } => {
                let next = self.build(cont);
                let s = self.fresh(false);
                self.states[s].transitions.push((
                    Event::Send {
                        to: to.clone(),
                        label: label.clone(),
                    },
                    next,
                ));
                s
            }
            Local::Recv { from, label, cont } => {
                let next = self.build(cont);
                let s = self.fresh(false);
                self.states[s].transitions.push((
                    Event::Recv {
                        from: from.clone(),
                        label: label.clone(),
                    },
                    next,
                ));
                s
            }
            Local::Select { to, branches } => {
                let s = self.fresh(false);
                for (label, cont) in branches {
                    let next = self.build(cont);
                    self.states[s].transitions.push((
                        Event::Send {
                            to: to.clone(),
                            label: label.clone(),
                        },
                        next,
                    ));
                }
                s
            }
            Local::Branch { from, branches } => {
                let s = self.fresh(false);
                for (label, cont) in branches {
                    let next = self.build(cont);
                    self.states[s].transitions.push((
                        Event::Recv {
                            from: from.clone(),
                            label: label.clone(),
                        },
                        next,
                    ));
                }
                s
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_trace_accepts() {
        // send m to B ; end
        let l = Local::send("B", "m", Local::end());
        let fsm = Fsm::from_local(&l);
        let s0 = fsm.start();
        assert!(!fsm.is_accepting(s0));
        let s1 = fsm
            .step(
                s0,
                &Event::Send {
                    to: "B".into(),
                    label: "m".into(),
                },
            )
            .unwrap();
        assert!(fsm.is_accepting(s1));
    }

    #[test]
    fn illegal_event_reports_expected() {
        let l = Local::send("B", "m", Local::end());
        let fsm = Fsm::from_local(&l);
        let s0 = fsm.start();
        let err = fsm
            .step(
                s0,
                &Event::Send {
                    to: "B".into(),
                    label: "wrong".into(),
                },
            )
            .unwrap_err();
        assert_eq!(
            err.expected,
            vec![Event::Send {
                to: "B".into(),
                label: "m".into()
            }]
        );
    }

    #[test]
    fn branch_offers_both_labels() {
        let l = Local::branch(
            "A",
            vec![("l".into(), Local::end()), ("r".into(), Local::end())],
        );
        let fsm = Fsm::from_local(&l);
        let s0 = fsm.start();
        let mut got = fsm.expected(s0);
        got.sort_by_key(|e| match e {
            Event::Recv { label, .. } => label.clone(),
            _ => String::new(),
        });
        assert_eq!(got.len(), 2);
        // Either label should step.
        assert!(fsm
            .step(
                s0,
                &Event::Recv {
                    from: "A".into(),
                    label: "r".into()
                }
            )
            .is_ok());
    }

    #[test]
    fn recursion_loops_back() {
        // rec X . recv ping from A ; X  — should accept ping repeatedly and
        // never reach an accepting state (it's an infinite protocol).
        let l = Local::rec("X", Local::recv("A", "ping", Local::var("X")));
        let fsm = Fsm::from_local(&l);
        let mut s = fsm.start();
        for _ in 0..5 {
            s = fsm
                .step(
                    s,
                    &Event::Recv {
                        from: "A".into(),
                        label: "ping".into(),
                    },
                )
                .unwrap();
        }
        // We have returned to a non-accepting recurring state.
        assert!(!fsm.is_accepting(s));
        assert_eq!(
            fsm.expected(s),
            vec![Event::Recv {
                from: "A".into(),
                label: "ping".into()
            }]
        );
    }

    #[test]
    fn diagnose_emphasizes_label_when_target_matches() {
        let it = IllegalTransition {
            expected: vec![Event::Send {
                to: "Remediator".into(),
                label: "fix".into(),
            }],
            got: Event::Send {
                to: "Remediator".into(),
                label: "fix rejected work".into(),
            },
        };
        let msg = it.diagnose();
        assert!(msg.contains("label 'fix'"), "got: {msg}");
        assert!(msg.contains("Remediator"));
        assert!(msg.contains("fix rejected work"));
    }

    #[test]
    fn diagnose_reports_target_when_peer_differs() {
        let it = IllegalTransition {
            expected: vec![Event::Send {
                to: "Remediator".into(),
                label: "fix".into(),
            }],
            got: Event::Send {
                to: "Processor".into(),
                label: "fix".into(),
            },
        };
        let msg = it.diagnose();
        assert!(msg.contains("Remediator"), "got: {msg}");
        assert!(msg.contains("Processor"));
    }

    #[test]
    fn diagnose_lists_options_when_no_close_match() {
        let it = IllegalTransition {
            expected: vec![Event::Recv {
                from: "Client".into(),
                label: "req".into(),
            }],
            got: Event::Send {
                to: "X".into(),
                label: "y".into(),
            },
        };
        let msg = it.diagnose();
        assert!(msg.contains("expected"), "got: {msg}");
        assert!(msg.contains("req"));
    }

    #[test]
    fn retry_loop_can_exit() {
        // rec X . recv try from W ; branch from W { ok . end, retry . X }
        let l = Local::rec(
            "X",
            Local::recv(
                "W",
                "try",
                Local::branch(
                    "W",
                    vec![
                        ("ok".into(), Local::end()),
                        ("retry".into(), Local::var("X")),
                    ],
                ),
            ),
        );
        let fsm = Fsm::from_local(&l);
        let mut s = fsm.start();
        // try, retry, try, ok.
        s = fsm
            .step(
                s,
                &Event::Recv {
                    from: "W".into(),
                    label: "try".into(),
                },
            )
            .unwrap();
        s = fsm
            .step(
                s,
                &Event::Recv {
                    from: "W".into(),
                    label: "retry".into(),
                },
            )
            .unwrap();
        s = fsm
            .step(
                s,
                &Event::Recv {
                    from: "W".into(),
                    label: "try".into(),
                },
            )
            .unwrap();
        s = fsm
            .step(
                s,
                &Event::Recv {
                    from: "W".into(),
                    label: "ok".into(),
                },
            )
            .unwrap();
        assert!(fsm.is_accepting(s));
    }
}
