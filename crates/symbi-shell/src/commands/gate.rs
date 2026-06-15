use super::CommandResult;
use crate::app::App;

/// Open the Gate panel (held-action escalation queue) and trigger an
/// immediate poll. Requires an attached runtime.
pub fn gate(app: &mut App, _args: &str) -> CommandResult {
    if app.remote.is_none() {
        return CommandResult::Error("Not attached to a runtime. Use /attach first.".into());
    }
    app.gate_visible = true;
    app.gate_refresh();
    CommandResult::Handled
}
