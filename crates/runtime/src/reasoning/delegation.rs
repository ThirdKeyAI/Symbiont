//! Agent-to-agent delegation: the reasoning loop dispatches an approved
//! `ProposedAction::Delegate` through a `DelegationExecutor`, which runs the
//! target agent and returns its output. Decouples the loop from the agent
//! registry; the production impl lives in `delegation_executor.rs`.

use async_trait::async_trait;

/// Recursion-bounding context threaded from the parent loop into a delegated
/// sub-loop: the current depth and the chain of agent names already on the path.
#[derive(Debug, Clone)]
pub struct DelegationContext {
    /// Delegation nesting depth of the *calling* loop (0 at the top level).
    pub depth: u32,
    /// Agent names already on the delegation path, for cycle detection.
    pub chain: Vec<String>,
    /// The calling loop's iteration limit, inherited by the sub-loop.
    pub max_iterations: u32,
    /// The calling loop's token budget, inherited by the sub-loop.
    pub max_total_tokens: u32,
    /// The calling loop's wall-clock limit, inherited by the sub-loop.
    pub timeout: std::time::Duration,
}

impl Default for DelegationContext {
    /// Mirrors `LoopConfig::default()`'s budget (25 iterations / 100_000 tokens /
    /// 300s) so a `DelegationContext::default()` used without an enclosing loop
    /// (e.g. in tests) still gets a sane sub-loop budget.
    fn default() -> Self {
        Self {
            depth: 0,
            chain: Vec::new(),
            max_iterations: 25,
            max_total_tokens: 100_000,
            timeout: std::time::Duration::from_secs(300),
        }
    }
}

/// Why a delegation could not produce a result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DelegationError {
    /// No agent registered under this name.
    UnknownTarget(String),
    /// Delegating to this target would revisit an agent already on the path.
    Cycle(String),
    /// Delegation depth would exceed the configured maximum.
    DepthExceeded(u32),
    /// The target sub-loop could not be run (construction/registry failure).
    Failed(String),
}

impl std::fmt::Display for DelegationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownTarget(t) => write!(f, "unknown delegation target '{}'", t),
            Self::Cycle(t) => {
                write!(
                    f,
                    "delegation cycle detected: '{}' is already on the path",
                    t
                )
            }
            Self::DepthExceeded(max) => {
                write!(f, "delegation depth limit ({}) exceeded", max)
            }
            Self::Failed(reason) => write!(f, "delegation failed: {}", reason),
        }
    }
}

impl std::error::Error for DelegationError {}

/// Runs a delegated target agent and returns its final textual output.
#[async_trait]
pub trait DelegationExecutor: Send + Sync {
    /// Run `target` on `message`, honoring the recursion bounds in `ctx`.
    async fn delegate(
        &self,
        target: &str,
        message: &str,
        ctx: DelegationContext,
    ) -> Result<String, DelegationError>;
}
