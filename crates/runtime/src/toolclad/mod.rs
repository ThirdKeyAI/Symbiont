//! ToolClad: Declarative tool interface contracts
//!
//! Loads `.clad.toml` manifests from the `tools/` directory and provides
//! an `ActionExecutor` that validates arguments, constructs commands,
//! executes tools, and wraps output in evidence envelopes.

pub mod cedar_gen;
pub mod executor;
pub mod manifest;
pub mod scope;
pub mod template_vars;
pub mod validator;
pub mod watcher;

pub use executor::ToolCladExecutor;
pub use manifest::{load_custom_types, load_manifest, load_manifests_from_dir, Manifest};
pub use scope::Scope;
