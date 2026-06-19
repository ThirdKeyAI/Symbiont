//! Local (per-role) protocol types.
//!
//! A [`Local`] value is the projection of a [`crate::global::Global`] onto a
//! single role. It only mentions the messages that role sends or receives, plus
//! the internal/external choices it drives or observes.

use crate::global::{Label, RecVar, Role};

/// Local protocol type — the view of one role.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Local {
    /// Send `label` to `to`, then continue as `cont`.
    Send {
        to: Role,
        label: Label,
        cont: Box<Local>,
    },
    /// Receive `label` from `from`, then continue as `cont`.
    Recv {
        from: Role,
        label: Label,
        cont: Box<Local>,
    },
    /// Internal choice: this role selects which `label` to send to `to`.
    Select {
        to: Role,
        branches: Vec<(Label, Local)>,
    },
    /// External choice: this role receives one of the `label`s from `from`.
    Branch {
        from: Role,
        branches: Vec<(Label, Local)>,
    },
    /// `rec X . body`.
    Rec { var: RecVar, body: Box<Local> },
    /// `X`.
    Var(RecVar),
    /// `end`.
    End,
}

impl Local {
    /// `send to : label ; cont`.
    pub fn send(to: impl Into<Role>, label: impl Into<Label>, cont: Local) -> Local {
        Local::Send {
            to: to.into(),
            label: label.into(),
            cont: Box::new(cont),
        }
    }

    /// `recv from : label ; cont`.
    pub fn recv(from: impl Into<Role>, label: impl Into<Label>, cont: Local) -> Local {
        Local::Recv {
            from: from.into(),
            label: label.into(),
            cont: Box::new(cont),
        }
    }

    /// `select to { label_i . L_i }`.
    pub fn select(to: impl Into<Role>, branches: Vec<(Label, Local)>) -> Local {
        Local::Select {
            to: to.into(),
            branches,
        }
    }

    /// `branch from { label_i . L_i }`.
    pub fn branch(from: impl Into<Role>, branches: Vec<(Label, Local)>) -> Local {
        Local::Branch {
            from: from.into(),
            branches,
        }
    }

    /// `rec var . body`.
    pub fn rec(var: impl Into<RecVar>, body: Local) -> Local {
        Local::Rec {
            var: var.into(),
            body: Box::new(body),
        }
    }

    /// `var`.
    pub fn var(v: impl Into<RecVar>) -> Local {
        Local::Var(v.into())
    }

    /// `end`.
    pub fn end() -> Local {
        Local::End
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_build_expected_shape() {
        let l = Local::send("B", "hello", Local::end());
        assert_eq!(
            l,
            Local::Send {
                to: "B".to_string(),
                label: "hello".to_string(),
                cont: Box::new(Local::End),
            }
        );
    }
}
