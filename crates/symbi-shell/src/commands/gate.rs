use super::CommandResult;
use crate::app::App;

/// Open the Gate panel (held-action escalation queue) and trigger an
/// immediate poll. Works against an attached runtime OR the in-process
/// escalation queue (orchestrator HITL gate).
pub fn gate(app: &mut App, _args: &str) -> CommandResult {
    if app.remote.is_none() && app.escalation_queue.is_none() {
        return CommandResult::Error(
            "Not attached to a runtime and no local approval queue.".into(),
        );
    }
    app.gate_visible = true;
    app.gate_refresh();
    CommandResult::Handled
}
