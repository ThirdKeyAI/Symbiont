//! Projection of a global protocol onto a single role, plus the merge operator
//! that makes projection of choices total when the projected role is not the
//! one driving the choice.
//!
//! # Projection rules (the implemented fragment)
//!
//! For role `r` and global `G`:
//!
//! * `Message { from, to, label, cont }`
//!   - `r == from` → `Send { to, label, cont: project(cont) }`
//!   - `r == to`   → `Recv { from, label, cont: project(cont) }`
//!   - otherwise   → `project(cont)` (the message is invisible to `r`)
//! * `Choice { chooser, to, branches }`
//!   - `r == chooser` → `Select { to, branches: project each }`
//!   - `r == to`      → `Branch { from: chooser, branches: project each }`
//!   - otherwise      → `merge` of every branch projection (see below)
//! * `Rec { var, body }` → `Rec { var, body: project(body) }`
//! * `Var(v)` → `Var(v)`
//! * `End` → `End`
//!
//! # Merge (full-merge, restricted)
//!
//! When `r` is uninvolved in a choice, each branch is projected and the results
//! must be merged into one local type, because `r` cannot observe which branch
//! was taken. The implemented merge rule is:
//!
//! * two structurally equal types merge to themselves;
//! * two `Branch { from, .. }` with the **same** `from` merge by taking the
//!   union of their label sets; labels present in both branches must have
//!   recursively mergeable continuations, labels present in only one are carried
//!   through unchanged (this is the "full merge");
//! * everything else is [`ProjectError::Unmergeable`].
//!
//! This is deliberately conservative: it accepts the standard well-behaved
//! protocols (where the uninvolved role behaves identically across branches, or
//! differs only by additional external-choice labels it could already receive)
//! and rejects the rest with an explanatory error rather than silently producing
//! an unsound local type.

use crate::global::{Global, Role};
use crate::local::Local;

/// Why projection (or the merge it performs) failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectError {
    /// Two branch projections of a choice the role does not drive could not be
    /// merged into a single deterministic local type.
    Unmergeable { role: Role, detail: String },
}

impl std::fmt::Display for ProjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectError::Unmergeable { role, detail } => {
                write!(
                    f,
                    "cannot project onto role '{role}': branches of a choice it does not control \
                     are not mergeable: {detail}"
                )
            }
        }
    }
}

impl std::error::Error for ProjectError {}

/// Project global protocol `g` onto `role`.
pub fn project(g: &Global, role: &Role) -> Result<Local, ProjectError> {
    match g {
        Global::Message {
            from,
            to,
            label,
            cont,
        } => {
            let cont = project(cont, role)?;
            if role == from {
                Ok(Local::Send {
                    to: to.clone(),
                    label: label.clone(),
                    cont: Box::new(cont),
                })
            } else if role == to {
                Ok(Local::Recv {
                    from: from.clone(),
                    label: label.clone(),
                    cont: Box::new(cont),
                })
            } else {
                // Role not involved in this message: it is invisible.
                Ok(cont)
            }
        }
        Global::Choice {
            chooser,
            to,
            branches,
        } => {
            if role == chooser {
                let mut out = Vec::with_capacity(branches.len());
                for (label, cont) in branches {
                    out.push((label.clone(), project(cont, role)?));
                }
                Ok(Local::Select {
                    to: to.clone(),
                    branches: out,
                })
            } else if role == to {
                let mut out = Vec::with_capacity(branches.len());
                for (label, cont) in branches {
                    out.push((label.clone(), project(cont, role)?));
                }
                Ok(Local::Branch {
                    from: chooser.clone(),
                    branches: out,
                })
            } else {
                // Uninvolved role: it cannot tell which branch was chosen, so
                // all branch projections must merge into one local type.
                let mut iter = branches.iter();
                let first = match iter.next() {
                    Some((_, g)) => project(g, role)?,
                    None => {
                        return Err(ProjectError::Unmergeable {
                            role: role.clone(),
                            detail: "choice has no branches".to_string(),
                        })
                    }
                };
                let mut acc = first;
                for (_, g) in iter {
                    let next = project(g, role)?;
                    acc = merge(acc, next, role)?;
                }
                Ok(acc)
            }
        }
        Global::Rec { var, body } => Ok(Local::Rec {
            var: var.clone(),
            body: Box::new(project(body, role)?),
        }),
        Global::Var(v) => Ok(Local::Var(v.clone())),
        Global::End => Ok(Local::End),
    }
}

/// Merge two local types produced by projecting different branches of a choice
/// the role does not control. See the module docs for the rule.
pub fn merge(a: Local, b: Local, role: &Role) -> Result<Local, ProjectError> {
    if a == b {
        return Ok(a);
    }
    match (a, b) {
        (
            Local::Branch {
                from: fa,
                branches: ba,
            },
            Local::Branch {
                from: fb,
                branches: bb,
            },
        ) => {
            if fa != fb {
                return Err(ProjectError::Unmergeable {
                    role: role.clone(),
                    detail: format!(
                        "external choices receive from different roles ('{fa}' vs '{fb}'); \
                         an uninvolved role cannot reconcile them"
                    ),
                });
            }
            let merged = merge_branch_sets(ba, bb, &fa, role)?;
            Ok(Local::Branch {
                from: fa,
                branches: merged,
            })
        }
        (a, b) => Err(ProjectError::Unmergeable {
            role: role.clone(),
            detail: format!(
                "incompatible local types across branches: {} vs {}",
                describe(&a),
                describe(&b)
            ),
        }),
    }
}

/// Union two external-choice branch sets. Shared labels must have recursively
/// mergeable continuations; labels unique to one side are carried through.
fn merge_branch_sets(
    ba: Vec<(String, Local)>,
    bb: Vec<(String, Local)>,
    from: &Role,
    role: &Role,
) -> Result<Vec<(String, Local)>, ProjectError> {
    let mut out: Vec<(String, Local)> = ba;
    for (label, lb) in bb {
        if let Some(slot) = out.iter_mut().find(|(l, _)| *l == label) {
            // Shared label: continuations must merge.
            let la = std::mem::replace(&mut slot.1, Local::End);
            slot.1 = merge(la, lb, role).map_err(|e| match e {
                ProjectError::Unmergeable { detail, .. } => ProjectError::Unmergeable {
                    role: role.clone(),
                    detail: format!(
                        "label '{label}' received from '{from}' has incompatible \
                         continuations across branches: {detail}"
                    ),
                },
            })?;
        } else {
            out.push((label, lb));
        }
    }
    Ok(out)
}

/// A short human-readable tag for a local type, used in error messages. Where
/// it is cheap to do so, the peer role and label are named so the error pinpoints
/// the divergence.
fn describe(l: &Local) -> String {
    match l {
        Local::Send { to, label, .. } => format!("a send of '{label}' to '{to}'"),
        Local::Recv { from, label, .. } => format!("a receive of '{label}' from '{from}'"),
        Local::Select { to, .. } => format!("an internal choice (select) toward '{to}'"),
        Local::Branch { from, .. } => format!("an external choice (branch) from '{from}'"),
        Local::Rec { .. } => "a recursion".to_string(),
        Local::Var(v) => format!("a recursion variable '{v}'"),
        Local::End => "end".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role(s: &str) -> Role {
        s.to_string()
    }

    #[test]
    fn projects_message_onto_sender_and_receiver() {
        let g = Global::msg("A", "B", "m", Global::end());
        assert_eq!(
            project(&g, &role("A")).unwrap(),
            Local::send("B", "m", Local::end())
        );
        assert_eq!(
            project(&g, &role("B")).unwrap(),
            Local::recv("A", "m", Local::end())
        );
    }

    #[test]
    fn uninvolved_role_skips_message() {
        let g = Global::msg("A", "B", "m", Global::msg("A", "C", "n", Global::end()));
        // C only sees the second message.
        assert_eq!(
            project(&g, &role("C")).unwrap(),
            Local::recv("A", "n", Local::end())
        );
    }

    #[test]
    fn projects_choice_onto_chooser_and_target() {
        let g = Global::choice(
            "A",
            "B",
            vec![("l".into(), Global::end()), ("r".into(), Global::end())],
        );
        assert_eq!(
            project(&g, &role("A")).unwrap(),
            Local::select(
                "B",
                vec![("l".into(), Local::end()), ("r".into(), Local::end())]
            )
        );
        assert_eq!(
            project(&g, &role("B")).unwrap(),
            Local::branch(
                "A",
                vec![("l".into(), Local::end()), ("r".into(), Local::end())]
            )
        );
    }

    #[test]
    fn merge_succeeds_when_uninvolved_role_behaves_identically() {
        // A chooses toward B; in both branches C receives "x" from B.
        let g = Global::choice(
            "A",
            "B",
            vec![
                ("l".into(), Global::msg("B", "C", "x", Global::end())),
                ("r".into(), Global::msg("B", "C", "x", Global::end())),
            ],
        );
        let c = project(&g, &role("C")).unwrap();
        assert_eq!(c, Local::recv("B", "x", Local::end()));
    }

    #[test]
    fn merge_unions_distinct_labels_from_same_sender() {
        // A chooses; B forwards a different label to C per branch. C receives
        // from B in both cases, so the projections are external choices with the
        // same `from` and merge into a union.
        let g = Global::choice(
            "A",
            "B",
            vec![
                (
                    "l".into(),
                    Global::choice("B", "C", vec![("x".into(), Global::end())]),
                ),
                (
                    "r".into(),
                    Global::choice("B", "C", vec![("y".into(), Global::end())]),
                ),
            ],
        );
        let c = project(&g, &role("C")).unwrap();
        match c {
            Local::Branch { from, branches } => {
                assert_eq!(from, "B");
                let labels: Vec<_> = branches.iter().map(|(l, _)| l.clone()).collect();
                assert!(labels.contains(&"x".to_string()));
                assert!(labels.contains(&"y".to_string()));
                assert_eq!(labels.len(), 2);
            }
            other => panic!("expected Branch, got {other:?}"),
        }
    }

    #[test]
    fn merge_fails_when_uninvolved_role_must_act_differently() {
        // A chooses; in one branch C sends, in the other C receives. C cannot
        // know which, so projection must fail.
        let g = Global::choice(
            "A",
            "B",
            vec![
                ("l".into(), Global::msg("C", "B", "x", Global::end())),
                ("r".into(), Global::msg("B", "C", "y", Global::end())),
            ],
        );
        let err = project(&g, &role("C")).unwrap_err();
        match err {
            ProjectError::Unmergeable { role, .. } => assert_eq!(role, "C"),
        }
    }

    #[test]
    fn merge_fails_for_different_senders() {
        let g = Global::choice(
            "A",
            "B",
            vec![
                ("l".into(), Global::msg("B", "C", "x", Global::end())),
                ("r".into(), Global::msg("D", "C", "x", Global::end())),
            ],
        );
        let err = project(&g, &role("C")).unwrap_err();
        assert!(matches!(err, ProjectError::Unmergeable { .. }));
        // The message should explain the differing senders.
        let msg = err.to_string();
        assert!(msg.contains('B') && msg.contains('D'));
    }
}
