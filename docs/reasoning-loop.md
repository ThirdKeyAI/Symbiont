---
layout: default
title: Reasoning Loop Guide
nav_order: 5
description: "Guide to the Symbiont agentic reasoning loop system"
---

# Reasoning Loop Guide
{: .no_toc }

---

Complete guide to the Symbiont agentic reasoning loop: a typestate-enforced Observe-Reason-Gate-Act (ORGA) cycle for autonomous agent behavior.
{: .fs-6 .fw-300 }

## Table of contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Overview

The reasoning loop is the core execution engine for autonomous agents in Symbiont. It drives a multi-turn conversation between an LLM, a policy gate, and external tools through a structured cycle:

1. **Observe** — Collect results from previous tool executions
2. **Reason** — LLM produces proposed actions (tool calls or text responses)
3. **Gate** — Policy engine evaluates each proposed action
4. **Act** — Approved actions are dispatched to tool executors

The loop continues until the LLM produces a final text response, hits iteration/token limits, or times out.

### Design Principles

- **Compile-time safety**: Invalid phase transitions are caught at compile time via Rust's type system
- **Opt-in complexity**: The loop works with just a provider and policy gate; knowledge bridge, Cedar policies, and human-in-the-loop are all optional
- **Backward compatible**: Adding new features (like the knowledge bridge) never breaks existing code
- **Observable**: Every phase emits journal events and tracing spans

---

## Quick Start

### Minimal Example

```rust
use std::sync::Arc;
use symbi_runtime::reasoning::circuit_breaker::CircuitBreakerRegistry;
use symbi_runtime::reasoning::context_manager::DefaultContextManager;
use symbi_runtime::reasoning::conversation::{Conversation, ConversationMessage};
use symbi_runtime::reasoning::executor::DefaultActionExecutor;
use symbi_runtime::reasoning::loop_types::{BufferedJournal, LoopConfig};
use symbi_runtime::reasoning::policy_bridge::DefaultPolicyGate;
use symbi_runtime::reasoning::reasoning_loop::ReasoningLoopRunner;
use symbi_runtime::types::AgentId;

// Set up the runner with default components
let runner = ReasoningLoopRunner {
    provider: Arc::new(my_inference_provider),
    policy_gate: Arc::new(DefaultPolicyGate::permissive()),
    executor: Arc::new(DefaultActionExecutor::default()),
    context_manager: Arc::new(DefaultContextManager::default()),
    circuit_breakers: Arc::new(CircuitBreakerRegistry::default()),
    journal: Arc::new(BufferedJournal::new(1000)),
    knowledge_bridge: None,
};

// Build a conversation
let mut conv = Conversation::with_system("You are a helpful assistant.");
conv.push(ConversationMessage::user("What is 6 * 7?"));

// Run the loop
let result = runner.run(AgentId::new(), conv, LoopConfig::default()).await;

println!("Output: {}", result.output);
println!("Iterations: {}", result.iterations);
println!("Tokens used: {}", result.total_usage.total_tokens);
```

### With Tool Definitions

```rust
use symbi_runtime::reasoning::inference::ToolDefinition;

let config = LoopConfig {
    max_iterations: 10,
    tool_definitions: vec![
        ToolDefinition {
            name: "web_search".into(),
            description: "Search the web for information".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                },
                "required": ["query"]
            }),
        },
    ],
    ..Default::default()
};

let result = runner.run(agent_id, conv, config).await;
```

---

## Phase System

### Typestate Pattern

The loop uses Rust's type system to enforce valid phase transitions at compile time. Each phase is a zero-sized type marker:

```rust
pub struct Reasoning;      // LLM produces proposed actions
pub struct PolicyCheck;    // Each action evaluated by the gate
pub struct ToolDispatching; // Approved actions executed
pub struct Observing;      // Results collected for next iteration
```

The `AgentLoop<Phase>` struct carries the loop state and can only call methods appropriate to its current phase. For example, `AgentLoop<Reasoning>` only exposes `produce_output()`, which consumes self and returns `AgentLoop<PolicyCheck>`.

This means the following mistakes are **compile errors**, not runtime bugs:
- Skipping the policy check
- Dispatching tools without reasoning first
- Observing results without dispatching

### Phase Flow

```
                    ┌─────────────────────────────────────────┐
                    │                                         │
                    ▼                                         │
    ┌──────────────────────┐                                  │
    │  AgentLoop<Reasoning>│                                  │
    │  produce_output()    │                                  │
    └──────────┬───────────┘                                  │
               │                                              │
               ▼                                              │
    ┌──────────────────────┐                                  │
    │ AgentLoop<PolicyCheck>│                                 │
    │  check_policy()      │                                  │
    └──────────┬───────────┘                                  │
               │                                              │
               ▼                                              │
    ┌────────────────────────────┐                            │
    │ AgentLoop<ToolDispatching> │                            │
    │  dispatch_tools()          │                            │
    └──────────┬─────────────────┘                            │
               │                                              │
               ▼                                              │
    ┌──────────────────────┐     Continue    ┌───────────┐    │
    │ AgentLoop<Observing> │───────────────▶│ Reasoning  │────┘
    │  observe_results()   │                └───────────┘
    └──────────┬───────────┘
               │ Complete
               ▼
         ┌───────────┐
         │ LoopResult │
         └───────────┘
```

---

## Inference Providers

The `InferenceProvider` trait abstracts over LLM backends:

```rust
#[async_trait]
pub trait InferenceProvider: Send + Sync {
    async fn complete(
        &self,
        conversation: &Conversation,
        options: &InferenceOptions,
    ) -> Result<InferenceResponse, InferenceError>;

    fn provider_name(&self) -> &str;
    fn default_model(&self) -> &str;
    fn supports_native_tools(&self) -> bool;
    fn supports_structured_output(&self) -> bool;
}
```

### Cloud Provider (OpenRouter)

The `CloudInferenceProvider` connects to OpenRouter (or any OpenAI-compatible endpoint):

```bash
export OPENROUTER_API_KEY="sk-or-..."
export OPENROUTER_MODEL="google/gemini-2.0-flash-001"  # optional
```

```rust
use symbi_runtime::reasoning::providers::cloud::CloudInferenceProvider;

let provider = CloudInferenceProvider::from_env()
    .expect("OPENROUTER_API_KEY must be set");
```

---

## Policy Gate

Every proposed action passes through the policy gate before execution:

```rust
#[async_trait]
pub trait ReasoningPolicyGate: Send + Sync {
    async fn evaluate_action(
        &self,
        agent_id: &AgentId,
        action: &ProposedAction,
        state: &LoopState,
    ) -> LoopDecision;
}

pub enum LoopDecision {
    Allow,
    Deny { reason: String },
    Modify { modified_action: Box<ProposedAction>, reason: String },
}
```

### Built-in Gates

- **`DefaultPolicyGate::permissive()`** — Allows all actions (development/testing)
- **`DefaultPolicyGate::new()`** — Default policy rules
- **`OpaPolicyGateBridge`** — Bridges to the OPA-based policy engine
- **`CedarGate`** — Cedar policy language integration

### Policy Denial Feedback

When an action is denied, the denial reason is fed back to the LLM as a policy feedback observation, allowing it to adjust its approach on the next iteration.

---

## Action Execution

### ActionExecutor Trait

```rust
#[async_trait]
pub trait ActionExecutor: Send + Sync {
    async fn execute_actions(
        &self,
        actions: &[ProposedAction],
        config: &LoopConfig,
        circuit_breakers: &CircuitBreakerRegistry,
    ) -> Vec<Observation>;
}
```

### Built-in Executors

| Executor | Description |
|----------|-------------|
| `DefaultActionExecutor` | Parallel dispatch with per-tool timeouts |
| `EnforcedActionExecutor` | Delegates through `ToolInvocationEnforcer` → MCP pipeline |
| `KnowledgeAwareExecutor` | Intercepts knowledge tools, delegates rest to inner executor |

### Circuit Breakers

Each tool has an associated circuit breaker that tracks failures:

- **Closed** (normal): Tool calls proceed normally
- **Open** (tripped): Too many consecutive failures; calls rejected immediately
- **Half-open** (probing): Limited calls allowed to test recovery

```rust
let circuit_breakers = CircuitBreakerRegistry::new(CircuitBreakerConfig {
    failure_threshold: 3,
    recovery_timeout: Duration::from_secs(60),
    half_open_max_calls: 1,
});
```

---

## Knowledge-Reasoning Bridge

The `KnowledgeBridge` connects the agent's knowledge store (hierarchical memory, knowledge base, vector search) to the reasoning loop.

### Setup

```rust
use symbi_runtime::reasoning::knowledge_bridge::{KnowledgeBridge, KnowledgeConfig};

let bridge = Arc::new(KnowledgeBridge::new(
    context_manager.clone(),  // Arc<dyn context::ContextManager>
    KnowledgeConfig {
        max_context_items: 5,
        relevance_threshold: 0.3,
        auto_persist: true,
    },
));

let runner = ReasoningLoopRunner {
    // ... other fields ...
    knowledge_bridge: Some(bridge),
};
```

### How It Works

**Before each reasoning step:**
1. Search terms are extracted from recent user/tool messages
2. `query_context()` and `search_knowledge()` retrieve relevant items
3. Results are formatted and injected as a system message (replacing the previous injection)

**During tool dispatch:**
The `KnowledgeAwareExecutor` intercepts two special tools:

- **`recall_knowledge`** — Searches the knowledge base and returns formatted results
  ```json
  { "query": "capital of France", "limit": 5 }
  ```

- **`store_knowledge`** — Stores a new fact as a subject-predicate-object triple
  ```json
  { "subject": "Earth", "predicate": "has", "object": "one moon", "confidence": 0.95 }
  ```

All other tool calls are delegated to the inner executor unchanged.

**After loop completion:**
If `auto_persist` is enabled, the bridge extracts assistant responses and stores them as working memory for future conversations.

### Backward Compatibility

Setting `knowledge_bridge: None` makes the runner behave identically to before — no context injection, no knowledge tools, no persistence.

---

## Conversation Management

### Conversation Type

`Conversation` manages an ordered sequence of messages with serialization to both OpenAI and Anthropic API formats:

```rust
let mut conv = Conversation::with_system("You are a helpful assistant.");
conv.push(ConversationMessage::user("Hello"));
conv.push(ConversationMessage::assistant("Hi there!"));

// Serialize for API calls
let openai_msgs = conv.to_openai_messages();
let (system, anthropic_msgs) = conv.to_anthropic_messages();
```

### Token Budget Enforcement

The in-loop `ContextManager` (not to be confused with the knowledge `ContextManager`) manages the conversation token budget:

- **Sliding Window**: Remove oldest messages first
- **Observation Masking**: Hide verbose tool results
- **Anchored Summary**: Keep system message + N recent messages

---

## Durable Journal

Every phase transition emits a `JournalEntry` to the configured `JournalWriter`:

```rust
pub struct JournalEntry {
    pub sequence: u64,
    pub timestamp: DateTime<Utc>,
    pub agent_id: AgentId,
    pub iteration: u32,
    pub event: LoopEvent,
}

pub enum LoopEvent {
    Started { agent_id, config },
    ReasoningComplete { iteration, actions, usage },
    PolicyEvaluated { iteration, action_count, denied_count },
    ToolsDispatched { iteration, tool_count, duration },
    ObservationsCollected { iteration, observation_count },
    Terminated { reason, iterations, total_usage, duration },
    RecoveryTriggered { iteration, tool_name, strategy, error },
}
```

The default `BufferedJournal` stores entries in memory. Production deployments can implement `JournalWriter` for persistent storage.

---

## Configuration

### LoopConfig

```rust
pub struct LoopConfig {
    pub max_iterations: u32,        // Default: 25
    pub max_total_tokens: u32,      // Default: 100,000
    pub timeout: Duration,          // Default: 5 minutes
    pub default_recovery: RecoveryStrategy,
    pub tool_timeout: Duration,     // Default: 30 seconds
    pub max_concurrent_tools: usize, // Default: 10
    pub context_token_budget: usize, // Default: 8,000
    pub tool_definitions: Vec<ToolDefinition>,
}
```

### Recovery Strategies

When tool execution fails, the loop can apply different recovery strategies:

| Strategy | Description |
|----------|-------------|
| `Retry` | Retry with exponential backoff |
| `Fallback` | Try alternative tools |
| `CachedResult` | Use a cached result if fresh enough |
| `LlmRecovery` | Ask the LLM to find an alternative approach |
| `Escalate` | Route to a human operator queue |
| `DeadLetter` | Give up and log the failure |

---

## Testing

### Unit Tests (No API Key Required)

```bash
cargo test -j2 -p symbi-runtime --lib -- reasoning::knowledge
```

### Integration Tests with Mock Provider

```bash
cargo test -j2 -p symbi-runtime --test knowledge_reasoning_tests
```

### Live Tests with Real LLM

```bash
OPENROUTER_API_KEY="sk-or-..." OPENROUTER_MODEL="google/gemini-2.0-flash-001" \
  cargo test -j2 -p symbi-runtime --features http-input --test reasoning_live_tests -- --nocapture
```

---

## Implementation Phases

The reasoning loop was built in five phases, each adding capabilities:

| Phase | Focus | Key Components |
|-------|-------|----------------|
| **1** | Core loop | `conversation`, `inference`, `phases`, `reasoning_loop` |
| **2** | Resilience | `circuit_breaker`, `executor`, `context_manager`, `policy_bridge` |
| **3** | DSL integration | `human_critic`, `pipeline_config`, REPL builtins |
| **4** | Multi-agent | `agent_registry`, `critic_audit`, `saga` |
| **5** | Observability | `cedar_gate`, `journal`, `metrics`, `scheduler`, `tracing_spans` |
| **Bridge** | Knowledge | `knowledge_bridge`, `knowledge_executor` |

---

## Next Steps

- **[Runtime Architecture](runtime-architecture.md)** — Full system architecture overview
- **[Security Model](security-model.md)** — Policy enforcement and audit trails
- **[DSL Guide](dsl-guide.md)** — Agent definition language
- **[API Reference](api-reference.md)** — Complete API documentation
