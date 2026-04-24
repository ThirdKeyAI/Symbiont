# Advanced Reasoning Primitives


Feature-gated runtime primitives that enhance the reasoning loop with tool curation, stuck-loop detection, deterministic context pre-fetch, and directory-scoped convention retrieval.



---

## Overview

The `orga-adaptive` feature gate adds four advanced capabilities to the reasoning loop:

| Primitive | Problem Solved | Module |
|-----------|---------------|--------|
| **Tool Profile** | LLM sees too many tools, wastes tokens on irrelevant ones | `tool_profile.rs` |
| **Progress Tracker** | Loops get stuck retrying the same failing step | `progress_tracker.rs` |
| **Pre-Hydration** | Cold-start context gap — agent must discover references itself | `pre_hydrate.rs` |
| **Scoped Conventions** | Convention retrieval is language-wide, not directory-specific | `knowledge_bridge.rs` |

### Enabling

```toml
# In your Cargo.toml
[dependencies]
symbi-runtime = { version = "1.11", features = ["orga-adaptive"] }
```

Or build from source:

```bash
cargo build --features orga-adaptive
cargo test --features orga-adaptive
```

All primitives are additive and backward-compatible — existing code compiles and runs identically without the feature gate.

---

## Tool Profile Filtering

Filters tool definitions before the LLM sees them. Reduces token waste and prevents the model from selecting irrelevant tools.

### Configuration

```rust
use symbi_runtime::reasoning::ToolProfile;

// Include only file-related tools
let profile = ToolProfile::include_only(&["file_*", "code_*"]);

// Exclude debug tools
let profile = ToolProfile::exclude_only(&["debug_*", "internal_*"]);

// Combined: include web tools, exclude experimental ones, cap at 10
let profile = ToolProfile {
    include: vec!["web_*".into(), "search_*".into()],
    exclude: vec!["web_experimental_*".into()],
    max_tools: Some(10),
    require_verified: false,
};
```

### Filtering Pipeline

The pipeline applies in order:

1. **Include** — If non-empty, only tools matching any include glob pass through
2. **Exclude** — Tools matching any exclude glob are removed
3. **Verified** — If `require_verified` is true, only tools with `[verified]` in their description pass
4. **Max cap** — Truncate to `max_tools` if set

### Glob Syntax

| Pattern | Matches |
|---------|---------|
| `web_*` | `web_search`, `web_fetch`, `web_scrape` |
| `tool_?` | `tool_a`, `tool_1` (single character) |
| `exact_name` | Only `exact_name` |

### Integration with LoopConfig

```rust
let config = LoopConfig {
    tool_profile: Some(ToolProfile::include_only(&["search_*", "file_*"])),
    ..Default::default()
};
```

The profile is applied automatically in `ReasoningLoopRunner::run()` after tool definitions are populated from the executor and knowledge bridge.

---

## Progress Tracker

Tracks per-step reattempt counts and detects stuck loops by comparing consecutive error outputs using normalized Levenshtein similarity.

### Configuration

```rust
use symbi_runtime::reasoning::{ProgressTracker, StepIterationConfig, LimitAction};

let config = StepIterationConfig {
    max_reattempts_per_step: 2,    // Stop after 2 failed attempts
    similarity_threshold: 0.85,    // Errors 85%+ similar = stuck
    on_limit_reached: LimitAction::SkipStep,
};

let mut tracker = ProgressTracker::new(config);
```

### Usage (Coordinator-Level)

The progress tracker is **not wired into the reasoning loop directly** — it is a higher-order concern for coordinators that orchestrate multi-step tasks.

```rust
// Begin tracking a step
tracker.begin_step("extract_data");

// After each attempt, record the error and check
let decision = tracker.record_and_check("extract_data", &error_output);

match decision {
    StepDecision::Continue => { /* retry */ }
    StepDecision::Stop { reason } => {
        // Emit LoopEvent::StepLimitReached and move on
        match tracker.limit_action() {
            LimitAction::SkipStep => { /* skip to next step */ }
            LimitAction::AbortTask => { /* abort entire task */ }
            LimitAction::Escalate => { /* hand off to human */ }
        }
    }
}
```

### Stuck Detection

The tracker computes normalized Levenshtein distance between consecutive error outputs. If similarity exceeds the threshold (default 85%), the step is considered stuck — even if the max reattempt count hasn't been reached.

This catches scenarios where the agent keeps hitting the same error with slightly different wording.

---

## Pre-Hydration Engine

Extracts references from the task input (URLs, file paths, GitHub issues/PRs) and resolves them in parallel before the reasoning loop starts. This eliminates cold-start latency where the agent would otherwise need to discover and fetch these references itself.

### Configuration

```rust
use symbi_runtime::reasoning::PreHydrationConfig;
use std::time::Duration;

let config = PreHydrationConfig {
    custom_patterns: vec![],
    resolution_tools: [
        ("url".into(), "web_fetch".into()),
        ("file".into(), "file_read".into()),
    ].into(),
    timeout: Duration::from_secs(15),
    max_references: 10,
    max_context_tokens: 4000,  // 1 token ~ 4 chars
};
```

### Built-in Patterns

| Pattern | Type | Example Matches |
|---------|------|----------------|
| URLs | `url` | `https://example.com/api`, `http://localhost:3000` |
| File paths | `file` | `./src/main.rs`, `~/config.toml` |
| Issues | `issue` | `#42`, `#100` |
| Pull requests | `pr` | `PR #55`, `pr #12` |

### Custom Patterns

```rust
use symbi_runtime::reasoning::pre_hydrate::ReferencePattern;

let config = PreHydrationConfig {
    custom_patterns: vec![
        ReferencePattern {
            ref_type: "jira".into(),
            pattern: r"[A-Z]+-\d+".into(),  // PROJ-123
        },
    ],
    ..Default::default()
};
```

### Resolution Flow

1. **Extract** — Regex patterns scan the task input, deduplicating matches
2. **Resolve** — Each reference is resolved via the configured tool (e.g., `web_fetch` for URLs)
3. **Budget** — Results are pruned to fit within `max_context_tokens`
4. **Inject** — Formatted as a `[PRE_HYDRATED_CONTEXT]` system message (separate from the knowledge bridge's `[KNOWLEDGE_CONTEXT]` slot)

### Integration with LoopConfig

```rust
let config = LoopConfig {
    pre_hydration: Some(PreHydrationConfig {
        resolution_tools: [("url".into(), "web_fetch".into())].into(),
        ..Default::default()
    }),
    ..Default::default()
};
```

Pre-hydration runs automatically at the start of `run_inner()` before the main reasoning loop begins. A `LoopEvent::PreHydrationComplete` journal event is emitted with extraction and resolution statistics.

---

## Directory-Scoped Conventions

Extends the `recall_knowledge` tool with `directory` and `scope` parameters for retrieving coding conventions scoped to a specific directory.

### How It Works

When called with `scope: "conventions"` and a `directory`, the knowledge bridge:

1. Searches for conventions matching the directory path
2. Walks up parent directories (e.g., `src/api/` → `src/` → project root)
3. Falls back to language-level conventions
4. Deduplicates by content across all levels
5. Truncates to the requested limit

### LLM Tool Call

```json
{
  "name": "recall_knowledge",
  "arguments": {
    "query": "rust",
    "directory": "src/api/handlers",
    "scope": "conventions"
  }
}
```

### Backward Compatibility

The `directory` and `scope` parameters are optional. Without them, `recall_knowledge` behaves identically to the standard version — a plain knowledge search with `query` and `limit`.

---

## LoopConfig Fields

When the `orga-adaptive` feature is enabled, `LoopConfig` gains three optional fields:

```rust
pub struct LoopConfig {
    // ... existing fields ...

    /// Tool profile for filtering tools visible to the LLM.
    pub tool_profile: Option<ToolProfile>,
    /// Per-step iteration limits for stuck loop detection.
    pub step_iteration: Option<StepIterationConfig>,
    /// Pre-hydration configuration for deterministic context pre-fetch.
    pub pre_hydration: Option<PreHydrationConfig>,
}
```

All default to `None` and are serialized with `#[serde(default, skip_serializing_if = "Option::is_none")]` for backward compatibility.

## Journal Events

Two new `LoopEvent` variants are available:

```rust
pub enum LoopEvent {
    // ... existing variants ...

    /// A step hit its reattempt limit (emitted by coordinators).
    StepLimitReached {
        step_id: String,
        attempts: u32,
        reason: String,
    },
    /// Pre-hydration phase completed.
    PreHydrationComplete {
        references_found: usize,
        references_resolved: usize,
        references_failed: usize,
        total_tokens: usize,
    },
}
```

---

## Testing

```bash
# Without feature (no regressions)
cargo clippy --workspace -j2
cargo test --workspace -j2

# With feature
cargo clippy --workspace -j2 --features orga-adaptive
cargo test --workspace -j2 --features orga-adaptive
```

All tests are inline `#[cfg(test)]` modules — no external test fixtures needed.

---

## Module Map

| Module | Public Types | Description |
|--------|-------------|-------------|
| `tool_profile` | `ToolProfile` | Glob-based tool filtering with verified flag and max cap |
| `progress_tracker` | `ProgressTracker`, `StepIterationConfig`, `StepDecision`, `LimitAction` | Per-step iteration tracking with Levenshtein stuck detection |
| `pre_hydrate` | `PreHydrationEngine`, `PreHydrationConfig`, `HydratedContext` | Reference extraction, parallel resolution, token budget pruning |
| `knowledge_bridge` | (extended) | `retrieve_scoped_conventions()`, extended `recall_knowledge` tool |

---

## Next Steps

- **[Reasoning Loop Guide](reasoning-loop.md)** — Core ORGA cycle documentation
- **[Runtime Architecture](runtime-architecture.md)** — Full system architecture overview
- **[API Reference](api-reference.md)** — Complete API documentation
