//! Agentic Reasoning Loop
//!
//! Provides the core observe-reason-gate-act cycle for autonomous agents,
//! including multi-turn conversation management, unified inference across
//! cloud and SLM providers, schema-validated structured output, and
//! typestate-enforced phase transitions.

pub mod conversation;
pub mod inference;
pub mod output_schema;
pub mod providers;
pub mod schema_validation;

// Phase 2 modules
pub mod circuit_breaker;
pub mod context_manager;
pub mod executor;
pub mod knowledge_bridge;
pub mod knowledge_executor;
pub mod loop_types;
pub mod phases;
pub mod policy_bridge;
pub mod reasoning_loop;

// Phase 3 modules
pub mod human_critic;
pub mod pipeline_config;

// Phase 4 modules
pub mod agent_registry;
pub mod critic_audit;
pub mod saga;

// Phase 5 modules
#[cfg(feature = "cedar")]
pub mod cedar_gate;
pub mod journal;
pub mod metrics;
pub mod scheduler;
pub mod tracing_spans;

pub use conversation::{Conversation, ConversationMessage, MessageRole};
pub use inference::{
    InferenceOptions, InferenceProvider, InferenceResponse, ResponseFormat, ToolCallRequest,
    ToolDefinition, Usage,
};
pub use knowledge_bridge::{KnowledgeBridge, KnowledgeConfig};
pub use knowledge_executor::KnowledgeAwareExecutor;
pub use loop_types::{
    LoopConfig, LoopDecision, LoopEvent, LoopResult, LoopState, Observation, ProposedAction,
    RecoveryStrategy,
};
pub use output_schema::{OutputSchema, SchemaRegistry};
pub use phases::AgentPhase;
pub use policy_bridge::{ReasoningPolicyGate, ToolFilterPolicyGate};
pub use reasoning_loop::ReasoningLoopRunner;
pub use schema_validation::{SchemaValidationError, ValidationPipeline};
