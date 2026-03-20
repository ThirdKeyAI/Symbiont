//! ToolClad: Declarative tool interface contracts
//!
//! Loads `.clad.toml` manifests from the `tools/` directory and provides
//! an `ActionExecutor` that validates arguments, constructs commands,
//! executes tools, and wraps output in evidence envelopes.

pub mod executor;
pub mod manifest;
pub mod validator;

pub use executor::ToolCladExecutor;
pub use manifest::{load_manifest, load_manifests_from_dir, Manifest};
