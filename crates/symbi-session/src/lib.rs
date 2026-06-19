//! # symbi-session
//!
//! A self-contained spike exploring multiparty session-type (MPST) **projection**
//! and a runtime FSM for monitoring. It is intentionally decoupled from the rest
//! of the workspace — no runtime, DSL, or transport integration — so the core
//! ideas can be evaluated in isolation.
//!
//! ## Pipeline
//!
//! 1. Describe a protocol as a [`global::Global`] IR (point-to-point messages,
//!    directed choice, recursion, end).
//! 2. [`project::project`] it onto each [`global::Role`] to obtain a per-role
//!    [`local::Local`] type, using [`project::merge`] to keep projection total
//!    over choices a role does not control.
//! 3. [`wellformed::check_well_formed`] verifies projectability, guarded
//!    recursion, and bound variables, collecting every error.
//! 4. [`fsm::Fsm::from_local`] compiles a local type into a finite-state machine
//!    whose [`fsm::Fsm::step`] a runtime monitor drives one [`fsm::Event`] at a
//!    time, reporting an [`fsm::IllegalTransition`] (with the expected events)
//!    on protocol violations.
//!
//! ## Worked examples
//!
//! See [`examples`] for request/response, a coordinator pipeline, a race choice,
//! and a retry loop.

pub mod examples;
pub mod fsm;
pub mod global;
pub mod local;
pub mod monitor;
pub mod project;
pub mod wellformed;

pub use fsm::{Event, Fsm, IllegalTransition, StateId};
pub use global::{Global, Label, RecVar, Role};
pub use local::Local;
pub use monitor::{SessionError, SessionId, SessionMonitor};
pub use project::{merge, project, ProjectError};
pub use wellformed::{check_well_formed, WfError};
