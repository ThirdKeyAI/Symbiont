//! Example protocols used by the tests and as documentation of the supported
//! fragment. Each constructor returns the global protocol together with the list
//! of roles that participate.

use crate::global::{Global, Role};

fn roles(rs: &[&str]) -> Vec<Role> {
    rs.iter().map(|s| s.to_string()).collect()
}

/// Two-party request/response (models a synchronous `ask`):
///
/// `Client -> Server : req ; Server -> Client : resp ; end`
pub fn request_response() -> (Global, Vec<Role>) {
    let g = Global::msg(
        "Client",
        "Server",
        "req",
        Global::msg("Server", "Client", "resp", Global::end()),
    );
    (g, roles(&["Client", "Server"]))
}

/// Three-party pipeline (models a `delegate` chain):
///
/// `Coordinator -> Validator : task ; Validator -> Coordinator : ok ;
///  Coordinator -> Processor : task ; Processor -> Coordinator : done ; end`
pub fn coordinator_pipeline() -> (Global, Vec<Role>) {
    let g = Global::msg(
        "Coordinator",
        "Validator",
        "task",
        Global::msg(
            "Validator",
            "Coordinator",
            "ok",
            Global::msg(
                "Coordinator",
                "Processor",
                "task",
                Global::msg("Processor", "Coordinator", "done", Global::end()),
            ),
        ),
    );
    (g, roles(&["Coordinator", "Validator", "Processor"]))
}

/// Choice toward a single worker (models a `race` resolution / external choice):
///
/// `Coordinator -> Worker : { fast . Worker -> Coordinator : result ; end,
///                            slow . Worker -> Coordinator : result ; end }`
pub fn race_choice() -> (Global, Vec<Role>) {
    let g = Global::choice(
        "Coordinator",
        "Worker",
        vec![
            (
                "fast".into(),
                Global::msg("Worker", "Coordinator", "result", Global::end()),
            ),
            (
                "slow".into(),
                Global::msg("Worker", "Coordinator", "result", Global::end()),
            ),
        ],
    );
    (g, roles(&["Coordinator", "Worker"]))
}

/// Retry loop (models a `loop`):
///
/// `rec Loop . Coordinator -> Worker : try ;
///   Worker -> Coordinator : { ok . end, retry . Loop }`
pub fn retry_loop() -> (Global, Vec<Role>) {
    let g = Global::rec(
        "Loop",
        Global::msg(
            "Coordinator",
            "Worker",
            "try",
            Global::choice(
                "Worker",
                "Coordinator",
                vec![
                    ("ok".into(), Global::end()),
                    ("retry".into(), Global::var("Loop")),
                ],
            ),
        ),
    );
    (g, roles(&["Coordinator", "Worker"]))
}

/// All example protocols, paired with a stable name for table-driven tests.
pub fn all() -> Vec<(&'static str, Global, Vec<Role>)> {
    let mut v = Vec::new();
    let (g, r) = request_response();
    v.push(("request_response", g, r));
    let (g, r) = coordinator_pipeline();
    v.push(("coordinator_pipeline", g, r));
    let (g, r) = race_choice();
    v.push(("race_choice", g, r));
    let (g, r) = retry_loop();
    v.push(("retry_loop", g, r));
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wellformed::check_well_formed;

    #[test]
    fn all_examples_are_well_formed() {
        for (name, g, rs) in all() {
            assert!(
                check_well_formed(&g, &rs).is_ok(),
                "example '{name}' should be well-formed"
            );
        }
    }
}
