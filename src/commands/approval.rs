//! Shared opt-in approval configuration for ordinary and managed CLI runs.

use std::{sync::Arc, time::Duration};
use symbi_runtime::escalation::{terminal_approval_queue, EscalationGateConfig, EscalationQueue};

pub(super) async fn from_matches(
    matches: &clap::ArgMatches,
) -> Result<Option<(Arc<EscalationQueue>, EscalationGateConfig)>, String> {
    if !matches.get_flag("approval-terminal") {
        return Ok(None);
    }
    let timeout = matches
        .get_one::<u64>("approval-timeout")
        .copied()
        .unwrap_or(120);
    Ok(Some((
        terminal_approval_queue().await?,
        EscalationGateConfig {
            require_approval_tools: Vec::new(),
            timeout: Duration::from_secs(timeout),
        },
    )))
}
