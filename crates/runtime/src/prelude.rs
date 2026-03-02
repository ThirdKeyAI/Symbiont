//! Prelude for standalone agent development.
//!
//! Import everything a standalone agent needs with a single line:
//!
//! ```ignore
//! use symbi_runtime::prelude::*;
//! ```

// Core reasoning types
pub use crate::reasoning::{
    Conversation, ConversationMessage, LoopConfig, LoopDecision, LoopResult, LoopState,
    MessageRole, Observation, ProposedAction, ReasoningLoopRunner,
};

// Traits
pub use crate::reasoning::executor::ActionExecutor;
pub use crate::reasoning::inference::{InferenceProvider, ToolDefinition};
pub use crate::reasoning::policy_bridge::{ReasoningPolicyGate, ToolFilterPolicyGate};

// Default implementations
pub use crate::reasoning::circuit_breaker::CircuitBreakerRegistry;
pub use crate::reasoning::context_manager::DefaultContextManager;
pub use crate::reasoning::loop_types::BufferedJournal;
pub use crate::reasoning::policy_bridge::DefaultPolicyGate;

// Identity
pub use crate::types::AgentId;

// Cloud inference (feature-gated)
#[cfg(feature = "cloud-llm")]
pub use crate::reasoning::providers::cloud::CloudInferenceProvider;

#[cfg(test)]
mod tests {
    #[test]
    fn test_prelude_imports_compile() {
        use super::*;

        let _config = LoopConfig::default();
        let _conv = Conversation::new();
        let _agent_id = AgentId::new();
        let _obs = Observation::tool_result("test", "result");
        let _decision = LoopDecision::Allow;
    }
}
