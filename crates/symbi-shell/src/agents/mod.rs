//! Agent fleet ingestion for the shell: load TOML manifests into the runtime
//! registry so the orchestrator can delegate to them. `.symbi` agents carry
//! policy/sandbox constraints and are deferred (detected, not loaded).

pub mod loader;

// Re-exports for the orchestrator's fleet integration. The remaining loader
// types (specs, scanning, reports) are reachable as `loader::*` when a consumer
// needs them; only the two the orchestrator touches today are surfaced here.
pub use loader::{load_agents_into, AgentCard};
