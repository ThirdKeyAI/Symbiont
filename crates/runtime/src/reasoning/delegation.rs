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
    /// Parent accounting authority; child inference must charge this ledger.
    pub shared_budget: Option<super::budget::SharedBudget>,
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
            shared_budget: None,
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
    /// A child started but did not complete; retrying it may duplicate effects.
    Unconfirmed(String),
    /// A required parent or child audit could not be persisted.
    Audit(String),
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
            Self::Unconfirmed(reason) => write!(f, "delegation outcome is unconfirmed: {}", reason),
            Self::Audit(reason) => write!(f, "required delegation audit failed: {}", reason),
        }
    }
}

impl std::error::Error for DelegationError {}

/// Runs a delegated target agent and returns its final textual output.
#[async_trait]
pub trait DelegationExecutor: Send + Sync {
    /// Dispatch the exact governed grant. Custom SDK implementations retain
    /// ownership of their journal contract; protected production implementations
    /// require the supplied parent journal before starting a child.
    async fn delegate_authorized(
        &self,
        grant: &super::prepared::AuthorizedAction,
        ctx: DelegationContext,
        _journal: Option<&dyn super::loop_types::JournalWriter>,
    ) -> Result<String, DelegationError> {
        grant.check_live().map_err(DelegationError::Failed)?;
        let super::loop_types::ProposedAction::Delegate {
            target, message, ..
        } = grant.action()
        else {
            return Err(DelegationError::Failed(
                "expected a delegation grant".into(),
            ));
        };
        self.delegate(target, message, ctx).await
    }

    /// Cancel retained children when their parent future is dropped.
    fn cancel_run(&self, _run: &str) {}

    /// Await child cleanup and required terminal audit before parent termination.
    async fn close_run(&self, run: &str) -> Result<(), String> {
        self.cancel_run(run);
        Ok(())
    }

    /// Run `target` on `message`, honoring the recursion bounds in `ctx`.
    async fn delegate(
        &self,
        target: &str,
        message: &str,
        ctx: DelegationContext,
    ) -> Result<String, DelegationError>;
}

pub(super) struct DelegationRunGuard {
    executor: Option<std::sync::Arc<dyn DelegationExecutor>>,
    run: String,
}

impl DelegationRunGuard {
    pub(super) fn new(
        executor: Option<std::sync::Arc<dyn DelegationExecutor>>,
        state: &super::loop_types::LoopState,
    ) -> Self {
        Self {
            executor,
            run: super::prepared::execution_run_key(state),
        }
    }

    pub(super) async fn close(mut self) -> Result<(), String> {
        if let Some(executor) = self.executor.as_ref() {
            tokio::time::timeout(
                std::time::Duration::from_secs(22),
                executor.close_run(&self.run),
            )
            .await
            .map_err(|_| "delegated cleanup acknowledgement timed out".to_string())??;
        }
        self.executor = None;
        Ok(())
    }
}

impl Drop for DelegationRunGuard {
    fn drop(&mut self) {
        if let Some(executor) = &self.executor {
            executor.cancel_run(&self.run);
        }
    }
}
