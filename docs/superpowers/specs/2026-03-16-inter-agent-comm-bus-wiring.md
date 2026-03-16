# Wire Inter-Agent Builtins Through CommunicationBus

**Date:** 2026-03-16
**Status:** Approved
**Scope:** Tier 1 (same-process Docker) inter-agent communication governance

---

## Problem

The five inter-agent builtins (`ask`, `delegate`, `send_to`, `parallel`, `race`) bypass the CommunicationBus entirely. They call `InferenceProvider.complete()` directly, which means:

- No Cedar policy evaluation for inter-agent calls
- No audit trail for agent-to-agent communication
- No SecureMessage creation (no signing, no encryption)
- No delivery status tracking

The CommunicationBus infrastructure exists and is initialized in AgentRuntime, but nothing uses it.

## Solution

Insert a **CommunicationPolicyGate** and the **CommunicationBus** into the builtin execution path:

```
Builtin call (ask/delegate/send_to/parallel/race)
  → CommunicationPolicyGate.evaluate() — Cedar policy check
  → CommunicationBus.send_message() — creates SecureMessage, routes, audits
  → InferenceProvider.complete() — actual LLM call (unchanged)
  → CommunicationBus response tracking — delivery status, audit trail
```

The InferenceProvider call still happens — we're not replacing how agents think, just wrapping communication in governance.

## Components

### 1. CommunicationPolicyGate

**New file:** `crates/runtime/src/communication/policy_gate.rs`

Evaluates Cedar-style policies for inter-agent actions. Rules evaluated in priority order (highest first). First matching rule wins. Default is Allow (backward compatible).

```rust
pub struct CommunicationPolicyGate {
    rules: Vec<CommunicationPolicyRule>,
    default_allow: bool,
}

pub struct CommunicationPolicyRule {
    pub id: String,
    pub name: String,
    pub condition: CommunicationCondition,
    pub effect: CommunicationEffect,
    pub priority: u32,
}

pub enum CommunicationCondition {
    SenderIs(AgentId),
    RecipientIs(AgentId),
    MessageTypeIs(MessageType),
    TopicMatches(String),
    SenderHasCapability(String),
    Always,
    All(Vec<CommunicationCondition>),
    Any(Vec<CommunicationCondition>),
}

pub enum CommunicationEffect {
    Allow,
    Deny { reason: String },
}

pub struct CommunicationRequest {
    pub sender: AgentId,
    pub recipient: AgentId,
    pub message_type: MessageType,
    pub topic: Option<String>,
}
```

**Policy loading:** There are two sources for rules:

1. **Programmatic API** — `CommunicationPolicyGate::new()` accepts a `Vec<CommunicationPolicyRule>` built in Rust code. This is the primary interface used by the runtime when threading policies from config or Cedar definitions.

2. **DSL channel_policy_block** — The existing `extract_channel_definitions()` parses `ChannelPolicyRule { action: String, expression: String }` from DSL. These are raw `{action, expression}` pairs (e.g., `{action: "deny", expression: "sender != coordinator"}`). A `ChannelPolicyRule::into_communication_rule()` converter maps these to `CommunicationPolicyRule` using simple pattern matching on the expression string:
   - `"sender == X"` → `SenderIs(resolve_agent_id(X))`
   - `"recipient == X"` → `RecipientIs(resolve_agent_id(X))`
   - `"always"` or unrecognized → `Always`

   This is intentionally limited for v1 — complex expressions fall back to `Always` with the specified effect. Projects without channel policy blocks get default allow-all.

**Deny behavior:** Hard fail. Policy denial returns `CommunicationError::PolicyDenied { reason }`. This variant must be added to the `CommunicationError` enum in `crates/runtime/src/types/error.rs`. The ORGA loop handles it as a tool error — the agent sees "Policy denied: {reason}" and can reason about it.

### 2. Rewired Builtins

**Modify:** `crates/repl-core/src/dsl/agent_composition.rs` and `crates/repl-core/src/dsl/reasoning_builtins.rs`

Each builtin follows the same pattern. Using `ask` as canonical example:

**Current flow:**
```
builtin_ask(agent_name, message)
  → registry.get_agent(name)
  → inference_provider.complete(conversation)
  → return response string
```

**New flow:**
```
builtin_ask(agent_name, message)
  → build CommunicationRequest { sender, recipient, type: Request }
  → policy_gate.evaluate(request) — Err on deny
  → comm_bus.create_internal_message(sender, recipient, payload, Request, ttl)
  → comm_bus.send_message(secure_message)
  → inference_provider.complete(conversation) — unchanged
  → comm_bus.send_message(response_message) — log the response
  → return response string
```

**Per-builtin specifics:**

| Builtin | MessageType | TTL | Notes |
|---------|------------|-----|-------|
| `ask` | `Request(id)` / `Response(id)` | 30s default | Synchronous, waits for response |
| `delegate` | `Request(id)` / `Response(id)` | timeout arg or 60s | Creates separate conversation context |
| `send_to` | `Direct(recipient_id)` | 30s | Fire-and-forget, no response message logged. `comm_bus` Arc cloned into spawned task. |
| `parallel` | `Request(id)` / `Response(id)` | 30s per task | Policy checked per-task sequentially before spawning; any deny fails the batch |
| `race` | `Request(id)` / `Response(id)` | 30s per task | Policy checked per-task sequentially before spawning; any deny fails the batch |

### 3. Context Plumbing

**Modify:** `ReasoningBuiltinContext` (in `crates/repl-core/src/dsl/reasoning_builtins.rs`) and agent composition execution context.

Add three new fields:

```rust
pub sender_agent_id: Option<AgentId>,
pub comm_bus: Option<Arc<dyn CommunicationBus + Send + Sync>>,
pub comm_policy: Option<Arc<CommunicationPolicyGate>>,
```

`sender_agent_id` is required so builtins know who the calling agent is when building `CommunicationRequest`. It is set by the runtime when constructing the context for an agent's execution.

All fields are `Option` because existing tests and standalone REPL usage may not have a full runtime. When `None`, builtins behave exactly as today (no policy check, no message tracking). This keeps backward compatibility.

**Agent name-to-AgentId resolution:** Builtins receive agent names as strings from DSL code, but `CommunicationBus` and `CommunicationRequest` use `AgentId` (UUID). The `AgentRegistry` (already in the context) resolves names to agents. Add a helper:

```rust
fn resolve_agent_id(name: &str, registry: &AgentRegistry) -> Result<AgentId> {
    registry.get_agent(name)
        .map(|agent| agent.id)
        .ok_or_else(|| anyhow::anyhow!("Unknown agent: {}", name))
}
```

This is called at the start of each builtin before building the `CommunicationRequest`.

**For `parallel` and `race`:** Policy checks run sequentially before spawning concurrent tasks. If any policy check fails, the entire batch fails immediately (no tasks spawned). The `Arc<dyn CommunicationBus>` is cloned into each spawned task for message tracking.

### 4. Audit Integration

**Modify:** `crates/runtime/src/communication/mod.rs`

Every inter-agent message logged with: sender, recipient, message type, timestamp, policy decision. Uses existing `SecureMessage` fields — no new types needed. Audit entries written to the same journal the ORGA loop uses.

## Files Changed

| File | Change |
|------|--------|
| `crates/runtime/src/communication/policy_gate.rs` | New — CommunicationPolicyGate, rules, evaluation, ChannelPolicyRule converter |
| `crates/runtime/src/communication/mod.rs` | Add `pub mod policy_gate;`, audit logging |
| `crates/runtime/src/types/error.rs` | Add `PolicyDenied { reason }` variant to `CommunicationError` |
| `crates/repl-core/src/dsl/agent_composition.rs` | Rewire `ask`, `send_to`, `parallel`, `race` |
| `crates/repl-core/src/dsl/reasoning_builtins.rs` | Rewire `delegate`, extend `ReasoningBuiltinContext` |
| `crates/repl-core/src/dsl/mod.rs` or context types | Add sender_agent_id + comm_bus + comm_policy to builtin contexts |
| `crates/runtime/src/lib.rs` | Thread CommunicationBus + PolicyGate + sender ID to REPL context |

## Testing

**Unit tests:**
- `CommunicationPolicyGate`: rule matching, priority ordering, default allow, deny with reason
- Each rewired builtin: policy allow path, policy deny path, `None` context fallback

**Integration tests:**
- Two-agent `ask`: policy check → message creation → inference → response tracking
- Policy deny: agent A delegates to agent B, rule blocks it, error surfaces correctly
- Backward compat: builtins with `comm_bus: None` behave identically to today

## Out of Scope

- DSL grammar changes (no new `delegate` keyword)
- Cross-tier message routing (Tier 2/3)
- Soft deny / advisory mode
- Remote catalog fetch for policies
