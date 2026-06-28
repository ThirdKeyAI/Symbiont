//! Agent fleet ingestion for the shell: load TOML manifests and `.symbi` DSL
//! agents into the runtime registry so the orchestrator can delegate to them.
//! `.symbi` agents are parsed and registered alongside TOML manifests, with
//! capabilities mapped to tools and a fail-closed sandbox-tier gate (only
//! `Permissive` or no sandbox declaration is honorable in-process).

pub mod loader;
pub mod symbi;

// Re-exports for the orchestrator's fleet integration. The remaining loader
// types (specs, scanning, reports) are reachable as `loader::*` when a consumer
// needs them; only the two the orchestrator touches today are surfaced here.
pub use loader::{load_agents_into, AgentCard};
