<img src="https://raw.githubusercontent.com/ThirdKeyAI/Symbiont/main/logo-hz.png" alt="Symbi">

[中文简体](README.zh-cn.md) | [Español](README.es.md) | [Português](README.pt.md) | [日本語](README.ja.md) | [Deutsch](README.de.md)

[![Build](https://img.shields.io/github/actions/workflow/status/thirdkeyai/symbiont/docker-build.yml?branch=main)](https://github.com/thirdkeyai/symbiont/actions)
[![Crates.io](https://img.shields.io/crates/v/symbi)](https://crates.io/crates/symbi)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Docs](https://img.shields.io/badge/docs-online-brightgreen)](https://docs.symbiont.dev)

---

**Policy-governed agent runtime for production.**

Symbiont is a Rust-native runtime for executing AI agents, tools, and workflows under explicit policy, identity, and audit controls.

Most agent frameworks focus on orchestration. Symbiont focuses on what happens when agents need to run in real environments with real risk: untrusted tools, sensitive data, approval boundaries, audit requirements, and repeatable enforcement.

---

## Why Symbiont

AI agents are easy to demo and hard to trust.

Once an agent can call tools, access files, send messages, or invoke external services, you need more than prompts and glue code. You need:

* **Policy enforcement** for what an agent may do — built-in DSL and [Cedar](https://www.cedarpolicy.com/) authorization
* **Tool verification** so execution is not blind trust — [SchemaPin](https://github.com/ThirdKeyAI/SchemaPin) cryptographic verification of MCP tools
* **Agent identity** so you know who is acting — [AgentPin](https://github.com/ThirdKeyAI/AgentPin) domain-anchored ES256 identity
* **Sandboxing** for risky workloads — Docker isolation with resource limits
* **Audit trails** for what happened and why — cryptographically tamper-evident logs
* **Review workflows** for actions that require approval — human-in-the-loop gates in the reasoning loop

Symbiont is built for that layer.

---

## Quick start

### Prerequisites

* Docker (recommended) or Rust 1.82+
* No external vector database required (LanceDB embedded; Qdrant optional for scaled deployments)

### Run with Docker

```bash
# Start the runtime (API on :8080, HTTP input on :8081)
docker run --rm -p 8080:8080 -p 8081:8081 ghcr.io/thirdkeyai/symbi:latest up

# Run MCP server only
docker run --rm -p 8080:8080 ghcr.io/thirdkeyai/symbi:latest mcp

# Parse an agent DSL file
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest dsl parse /workspace/agent.dsl
```

### Build from source

```bash
cargo build --release
./target/release/symbi --help

# Run the runtime
cargo run -- up

# Interactive REPL
cargo run -- repl
```

> For production deployments, review `SECURITY.md` and the [deployment guide](https://docs.symbiont.dev/getting-started) before enabling untrusted tool execution.

---

## How it works

Symbiont separates agent intent from execution authority:

1. **Agents propose** actions through the ORGA reasoning loop (Observe-Reason-Gate-Act)
2. **The runtime evaluates** each action against policy, identity, and trust checks
3. **Policy decides** — allowed actions execute; denied actions are blocked or routed for approval
4. **Everything is logged** — tamper-evident audit trail for every decision

This means model output is never treated as execution authority. The runtime controls what actually happens.

### Example: untrusted tool blocked by policy

An agent attempts to call an unverified MCP tool. The runtime:

1. Checks SchemaPin verification status — tool signature is missing or invalid
2. Evaluates Cedar policy — `forbid(action == Action::"tool_call") when { !resource.verified }`
3. Blocks execution and logs the denial with full context
4. Optionally routes to an operator for manual approval

No code change required. The policy governs execution.

---

## DSL example

```symbiont
metadata {
    version = "1.0.0"
    author = "Your Name"
    description = "Data analysis agent"
}

agent analyze_data(input: DataSet) -> Result {
    capabilities = ["data_analysis", "visualization"]

    policy data_privacy {
        allow: read(input) if input.anonymized == true
        deny: store(input) if input.contains_pii == true
        audit: all_operations
    }

    with memory = "persistent", requires = "approval" {
        if (llm_check_safety(input)) {
            result = analyze(input);
            return result;
        } else {
            return reject("Safety check failed");
        }
    }
}
```

---

## Core capabilities

| Capability | What it does |
|-----------|-------------|
| **Cedar policy engine** | Fine-grained authorization for agent actions, tool calls, and resource access |
| **SchemaPin verification** | Cryptographic verification of MCP tool schemas before execution |
| **AgentPin identity** | Domain-anchored ES256 identity for agents and scheduled tasks |
| **ORGA reasoning loop** | Typestate-enforced Observe-Reason-Gate-Act cycle with policy gates and circuit breakers |
| **Sandboxing** | Docker-based isolation with resource limits for untrusted workloads |
| **Audit logging** | Tamper-evident logs with structured records for every policy decision |
| **ClawHavoc scanning** | 40 rules across 10 attack categories for skill/tool content analysis |
| **Secrets management** | Vault/OpenBao integration, AES-256-GCM encrypted storage, scoped per agent |
| **Cron scheduling** | SQLite-backed scheduler with jitter, concurrency guards, and dead-letter queues |
| **Persistent memory** | Markdown-backed agent memory with fact extraction, procedures, and compaction |
| **RAG engine** | Hybrid semantic + keyword search via LanceDB (embedded) or Qdrant (scaled) |
| **MCP integration** | Native Model Context Protocol support with governed tool access |
| **Webhook verification** | HMAC-SHA256 and JWT verification with GitHub, Stripe, and Slack presets |
| **Delivery routing** | Route agent output to webhooks, Slack, email, or custom channels |
| **Metrics & telemetry** | OTLP export with OpenTelemetry tracing spans for the reasoning loop |
| **HTTP security** | Loopback-only binding, CORS allow-lists, JWT EdDSA validation, per-agent API keys |
| **AI assistant plugins** | Governance plugins for [Claude Code](https://github.com/thirdkeyai/symbi-claude-code) and [Gemini CLI](https://github.com/thirdkeyai/symbi-gemini-cli) |

Performance: policy evaluation <1ms, ECDSA P-256 verification <5ms, 10k agent scheduling with <2% CPU overhead. See [benchmarks](crates/runtime/benches/performance_claims.rs) and [threshold tests](crates/runtime/tests/performance_claims.rs).

---

## Security model

Symbiont is designed around a simple principle: **model output should never be trusted as execution authority.**

Actions flow through runtime controls:

* **Zero trust** — all agent inputs are untrusted by default
* **Policy checks** — Cedar authorization before every tool call and resource access
* **Tool verification** — SchemaPin cryptographic verification of tool schemas
* **Sandbox boundaries** — Docker isolation for untrusted execution
* **Operator approval** — human-in-the-loop gates for sensitive actions
* **Secrets control** — Vault/OpenBao backends, encrypted local storage, agent namespaces
* **Audit logging** — cryptographically tamper-evident records of every decision

If you are executing untrusted code or risky tools, do not rely on a weak local execution model as your only boundary. See [`SECURITY.md`](SECURITY.md) and the [security model docs](https://docs.symbiont.dev/security-model).

---

## Workspace

| Crate | Description |
|-------|-------------|
| `symbi` | Unified CLI binary |
| `symbi-runtime` | Core agent runtime and execution engine |
| `symbi-dsl` | DSL parser and evaluator |
| `symbi-channel-adapter` | Slack/Teams/Mattermost adapters |
| `repl-core` / `repl-proto` / `repl-cli` | Interactive REPL and JSON-RPC server |
| `repl-lsp` | Language Server Protocol support |
| `symbi-a2ui` | Admin dashboard (Lit/TypeScript, alpha) |

Governance plugins: [`symbi-claude-code`](https://github.com/thirdkeyai/symbi-claude-code) | [`symbi-gemini-cli`](https://github.com/thirdkeyai/symbi-gemini-cli)

---

## Documentation

* [Getting Started](https://docs.symbiont.dev/getting-started)
* [Security Model](https://docs.symbiont.dev/security-model)
* [Runtime Architecture](https://docs.symbiont.dev/runtime-architecture)
* [Reasoning Loop Guide](https://docs.symbiont.dev/reasoning-loop)
* [DSL Guide](https://docs.symbiont.dev/dsl-guide)
* [API Reference](https://docs.symbiont.dev/api-reference)
* [Advanced Reasoning Primitives](https://docs.symbiont.dev/orga-adaptive)

If you are evaluating Symbiont for production, start with the security model and getting started docs.

---

## License

* **Community Edition** (Apache 2.0): Core runtime, DSL, ORGA reasoning loop, Cedar policy engine, SchemaPin/AgentPin verification, Docker sandboxing, persistent memory, cron scheduling, MCP integration, RAG (LanceDB), audit logging, webhook verification, ClawHavoc skill scanning, and all CLI/REPL tooling.
* **Enterprise Edition** (commercial license): Multi-tier sandboxing (gVisor, Firecracker, E2B), cryptographic audit trails with compliance exports (HIPAA, SOX, PCI-DSS), AI-powered tool review and threat detection, encrypted multi-agent collaboration, real-time monitoring dashboards, and dedicated support. See [`enterprise/README.md`](enterprise/README.md) for details.

Contact [ThirdKey](https://thirdkey.ai) for enterprise licensing.

---

*Same agent. Secure runtime.*

<div align="right">
  <img src="https://raw.githubusercontent.com/ThirdKeyAI/Symbiont/main/symbi-trans.png" alt="Symbi Logo" width="120">
</div>
