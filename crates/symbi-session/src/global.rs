//! Global protocol IR.
//!
//! A [`Global`] value describes a multiparty protocol from a bird's-eye view:
//! every message exchange between every pair of roles is recorded in one tree.
//! The supported fragment covers point-to-point messages, directed choice,
//! recursion, recursion variables, and termination — enough to express
//! request/response, pipelines, races, and retry loops.

/// A participant in the protocol. A bare `String` is used for the spike; a
/// newtype could replace it later without touching the algorithms.
pub type Role = String;

/// A message label (the "kind" of message, e.g. `req`, `ok`, `retry`).
pub type Label = String;

/// A recursion variable name (e.g. `Loop`).
pub type RecVar = String;

/// Global protocol type for the linear / branch / loop fragment.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Global {
    /// `from -> to : label ; cont` — a single point-to-point message.
    Message {
        from: Role,
        to: Role,
        label: Label,
        cont: Box<Global>,
    },
    /// `chooser -> to : { label_i . G_i }` — the chooser selects a branch and
    /// sends `label_i` to `to`, then the protocol continues as `G_i`.
    Choice {
        chooser: Role,
        to: Role,
        branches: Vec<(Label, Global)>,
    },
    /// `rec X . body` — bind recursion variable `X` over `body`.
    Rec { var: RecVar, body: Box<Global> },
    /// `X` — jump back to the enclosing `rec X`.
    Var(RecVar),
    /// `end` — successful termination.
    End,
}

impl Global {
    /// `from -> to : label ; cont`.
    pub fn msg(
        from: impl Into<Role>,
        to: impl Into<Role>,
        label: impl Into<Label>,
        cont: Global,
    ) -> Global {
        Global::Message {
            from: from.into(),
            to: to.into(),
            label: label.into(),
            cont: Box::new(cont),
        }
    }

    /// `chooser -> to : { label_i . G_i }`.
    pub fn choice(
        chooser: impl Into<Role>,
        to: impl Into<Role>,
        branches: Vec<(Label, Global)>,
    ) -> Global {
        Global::Choice {
            chooser: chooser.into(),
            to: to.into(),
            branches,
        }
    }

    /// `rec var . body`.
    pub fn rec(var: impl Into<RecVar>, body: Global) -> Global {
        Global::Rec {
            var: var.into(),
            body: Box::new(body),
        }
    }

    /// `var`.
    pub fn var(v: impl Into<RecVar>) -> Global {
        Global::Var(v.into())
    }

    /// `end`.
    pub fn end() -> Global {
        Global::End
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_build_expected_shape() {
        let g = Global::msg("A", "B", "hello", Global::end());
        assert_eq!(
            g,
            Global::Message {
                from: "A".to_string(),
                to: "B".to_string(),
                label: "hello".to_string(),
                cont: Box::new(Global::End),
            }
        );
    }

    #[test]
    fn serde_round_trips() {
        let g = Global::rec(
            "Loop",
            Global::msg(
                "A",
                "B",
                "ping",
                Global::choice(
                    "B",
                    "A",
                    vec![
                        ("ok".into(), Global::end()),
                        ("retry".into(), Global::var("Loop")),
                    ],
                ),
            ),
        );
        let json = serde_json::to_string(&g).unwrap();
        let back: Global = serde_json::from_str(&json).unwrap();
        assert_eq!(g, back);
    }
}
