//! Well-formedness checks for global protocols.
//!
//! A global protocol is considered well-formed for the spike when:
//!
//! 1. **Projectable** — [`crate::project::project`] succeeds for every declared
//!    role.
//! 2. **Guarded recursion** — no `Rec { var, body }` can reach `Var(var)`
//!    without first crossing a `Message` or `Choice` (which rejects the
//!    diverging `rec X . X`).
//! 3. **Bound variables** — every `Var(v)` occurs inside an enclosing
//!    `Rec { var: v, .. }`.
//!
//! All discovered errors are collected and returned together.

use crate::global::{Global, RecVar, Role};
use crate::project::project;

/// A single well-formedness violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WfError {
    /// Projection onto `role` failed; `detail` is the projection error message.
    NotProjectable { role: Role, detail: String },
    /// `rec var` is unguarded: its body can reach `var` with no intervening
    /// communication.
    UnguardedRecursion { var: RecVar },
    /// `Var(var)` is not bound by any enclosing `rec var`.
    UnboundVariable { var: RecVar },
}

impl std::fmt::Display for WfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WfError::NotProjectable { role, detail } => {
                write!(f, "not projectable onto '{role}': {detail}")
            }
            WfError::UnguardedRecursion { var } => {
                write!(
                    f,
                    "unguarded recursion: 'rec {var}' can reach '{var}' with no \
                     intervening message"
                )
            }
            WfError::UnboundVariable { var } => {
                write!(f, "unbound recursion variable '{var}'")
            }
        }
    }
}

impl std::error::Error for WfError {}

/// Check all well-formedness conditions, collecting every error.
pub fn check_well_formed(g: &Global, roles: &[Role]) -> Result<(), Vec<WfError>> {
    let mut errors = Vec::new();

    // 1. Projectable onto every role.
    for r in roles {
        if let Err(e) = project(g, r) {
            errors.push(WfError::NotProjectable {
                role: r.clone(),
                detail: e.to_string(),
            });
        }
    }

    // 2 & 3. Bound variables and guarded recursion in one traversal.
    let mut bound: Vec<RecVar> = Vec::new();
    check_vars_and_guard(g, &mut bound, &mut errors);

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Walk the tree checking that every `Var` is bound, and that every `Rec` is
/// guarded.
fn check_vars_and_guard(g: &Global, bound: &mut Vec<RecVar>, errors: &mut Vec<WfError>) {
    match g {
        Global::Message { cont, .. } => check_vars_and_guard(cont, bound, errors),
        Global::Choice { branches, .. } => {
            for (_, b) in branches {
                check_vars_and_guard(b, bound, errors);
            }
        }
        Global::Rec { var, body } => {
            // Guardedness: from the body, can we reach `var` without crossing a
            // message/choice?
            if reaches_var_unguarded(body, var, &mut Vec::new()) {
                errors.push(WfError::UnguardedRecursion { var: var.clone() });
            }
            bound.push(var.clone());
            check_vars_and_guard(body, bound, errors);
            bound.pop();
        }
        Global::Var(v) => {
            if !bound.contains(v) {
                errors.push(WfError::UnboundVariable { var: v.clone() });
            }
        }
        Global::End => {}
    }
}

/// Return `true` if `target` is reachable from `g` without passing through any
/// `Message` or `Choice` (i.e. through `Rec`/`Var` only). `seen` guards against
/// looping on inner recursion variables that shadow nothing relevant.
fn reaches_var_unguarded(g: &Global, target: &RecVar, seen: &mut Vec<RecVar>) -> bool {
    match g {
        // Any communication guards the recursion.
        Global::Message { .. } | Global::Choice { .. } | Global::End => false,
        Global::Var(v) => v == target,
        Global::Rec { var, body } => {
            if var == target {
                // `target` is shadowed by an inner binder; the outer one is
                // guarded with respect to this subtree.
                return false;
            }
            if seen.contains(var) {
                return false;
            }
            seen.push(var.clone());
            let r = reaches_var_unguarded(body, target, seen);
            seen.pop();
            r
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roles(rs: &[&str]) -> Vec<Role> {
        rs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn well_formed_linear_protocol_passes() {
        let g = Global::msg("A", "B", "m", Global::end());
        assert!(check_well_formed(&g, &roles(&["A", "B"])).is_ok());
    }

    #[test]
    fn guarded_loop_passes() {
        let g = Global::rec("X", Global::msg("A", "B", "m", Global::var("X")));
        assert!(check_well_formed(&g, &roles(&["A", "B"])).is_ok());
    }

    #[test]
    fn unguarded_rec_is_rejected() {
        let g = Global::rec("X", Global::var("X"));
        let errs = check_well_formed(&g, &roles(&["A"])).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, WfError::UnguardedRecursion { var } if var == "X")));
    }

    #[test]
    fn unguarded_rec_through_inner_rec_is_rejected() {
        // rec X . rec Y . X  — no communication before reaching X.
        let g = Global::rec("X", Global::rec("Y", Global::var("X")));
        let errs = check_well_formed(&g, &roles(&["A"])).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, WfError::UnguardedRecursion { var } if var == "X")));
    }

    #[test]
    fn unbound_var_is_rejected() {
        let g = Global::msg("A", "B", "m", Global::var("Nope"));
        let errs = check_well_formed(&g, &roles(&["A", "B"])).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, WfError::UnboundVariable { var } if var == "Nope")));
    }

    #[test]
    fn collects_multiple_errors() {
        // Unbound var AND not projectable (uninvolved role over divergent choice).
        let g = Global::choice(
            "A",
            "B",
            vec![
                ("l".into(), Global::msg("C", "B", "x", Global::end())),
                ("r".into(), Global::msg("B", "C", "y", Global::var("Free"))),
            ],
        );
        let errs = check_well_formed(&g, &roles(&["A", "B", "C"])).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, WfError::UnboundVariable { .. })));
        assert!(errs
            .iter()
            .any(|e| matches!(e, WfError::NotProjectable { .. })));
    }
}
