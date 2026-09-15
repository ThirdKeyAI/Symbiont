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
pub mod budget;
pub mod circuit_breaker;
pub mod context_manager;
pub mod delegation;
pub mod delegation_executor;
pub mod dispatch;
pub mod effect_journal;
pub mod executor;
pub mod governed;
pub mod governed_session;
#[cfg(unix)]
pub mod invocation;
pub mod knowledge_bridge;
pub mod knowledge_executor;
pub mod loop_types;
pub mod phases;
pub mod policy_bridge;
pub mod prepared;
#[cfg(unix)]
pub mod protected_journal;
pub mod reasoning_loop;
#[cfg(unix)]
pub mod recovery;
pub mod response_delivery;
pub mod response_run;
#[cfg(all(test, unix))]
mod retry_barrier_tests;
pub mod run_audit;
#[cfg(unix)]
pub mod run_view;
pub mod source_policy;
pub mod tool_executor_builder;

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

#[cfg(feature = "cedar")]
pub use cedar_gate::{CedarPolicy, CedarPolicyGate};
pub use conversation::{Conversation, ConversationMessage, MessageRole};
pub use governed::{governed_gate, GateOptions};
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
pub use tool_executor_builder::{build_agent_tool_executor, build_tool_executor};

// Advanced reasoning loop primitives (orga-adaptive)
#[cfg(feature = "orga-adaptive")]
pub mod pre_hydrate;
#[cfg(feature = "orga-adaptive")]
pub mod progress_tracker;
#[cfg(feature = "orga-adaptive")]
pub mod tool_profile;

#[cfg(feature = "orga-adaptive")]
pub use pre_hydrate::{HydratedContext, PreHydrationConfig, PreHydrationEngine};
#[cfg(feature = "orga-adaptive")]
pub use progress_tracker::{LimitAction, ProgressTracker, StepDecision, StepIterationConfig};
#[cfg(feature = "orga-adaptive")]
pub use tool_profile::ToolProfile;
