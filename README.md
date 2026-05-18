<img src="https://raw.githubusercontent.com/ThirdKeyAI/Symbiont/main/logo-hz.png" alt="Symbi">

[中文简体](README.zh-cn.md) | [Español](README.es.md) | [Português](README.pt.md) | [日本語](README.ja.md) | [Deutsch](README.de.md)

[![Build](https://img.shields.io/github/actions/workflow/status/thirdkeyai/symbiont/docker-build.yml?branch=main)](https://github.com/thirdkeyai/symbiont/actions)
[![Crates.io](https://img.shields.io/crates/v/symbi)](https://crates.io/crates/symbi)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Docs](https://img.shields.io/badge/docs-online-brightgreen)](https://docs.symbiont.dev)

[![OATS Reference Implementation](https://img.shields.io/badge/OATS-Reference%20Implementation-1f6feb)](https://openagenttruststack.org)
[![DOI Typestate Loops](https://zenodo.org/badge/DOI/10.5281/zenodo.19896446.svg)](https://doi.org/10.5281/zenodo.19896446)
[![DOI ToolClad](https://zenodo.org/badge/DOI/10.5281/zenodo.19957596.svg)](https://doi.org/10.5281/zenodo.19957596)
[![DOI Empirical Eval](https://zenodo.org/badge/DOI/10.5281/zenodo.20043247.svg)](https://doi.org/10.5281/zenodo.20043247)

---

**Policy-governed agent runtime for production.**
*Same agent. Secure runtime.*

Symbiont is a Rust-native runtime for executing AI agents and tools under explicit policy, identity, and audit controls.

Most agent frameworks focus on orchestration. Symbiont focuses on what happens when agents need to run in real environments with real risk: untrusted tools, sensitive data, approval boundaries, audit requirements, and repeatable enforcement.

---

## Why Symbiont

AI agents are easy to demo and hard to trust.

Once an agent can call tools, access files, send messages, or invoke external services, you need more than prompts and glue code. You need:

* **Policy enforcement** for what an agent may do — built-in DSL and [Cedar](https://www.cedarpolicy.com/) authorization
* **Tool verification** so execution is not blind trust — [SchemaPin](https://github.com/ThirdKeyAI/SchemaPin) cryptographic verification of MCP tools
* **Tool contracts** for how tools execute — [ToolClad](https://github.com/ThirdKeyAI/ToolClad) declarative input validation, scope enforcement, and injection prevention
* **Agent identity** so you know who is acting — [AgentPin](https://github.com/ThirdKeyAI/AgentPin) domain-anchored ES256 identity
* **Sandboxing** for risky workloads — choose Docker, gVisor (`runsc`), or Firecracker microVM per agent
* **Audit trails** for what happened and why — cryptographically tamper-evident logs
* **Approval gates** for sensitive actions — human review before execution when policy requires it

Symbiont is built for that layer.

### Open Agent Trust Stack (OATS) — reference implementation

Symbiont is the **reference implementation of the [Open Agent Trust Stack (OATS)](https://openagenttruststack.org)** — an open specification (CC BY 4.0) for securing AI agent execution through structural enforcement rather than post-hoc interception ("define what is permitted and make everything else structurally inexpressible"). The OATS spec is grounded in Symbiont's production operational experience and Symbiont's design tracks the OATS layers directly:

| OATS Layer | Symbiont mapping |
|---|---|
| **Layer 1 — ORGA Loop** (typestate-enforced Observe-Reason-Gate-Act) | `crates/runtime/src/reasoning/` — typestate-enforced phases; policy gate is unskippable at compile time. See [Wanger 2026 / DOI 10.5281/zenodo.19896446](https://doi.org/10.5281/zenodo.19896446). |
| **Layer 2 — Tool Contracts** | [ToolClad](https://github.com/ThirdKeyAI/ToolClad) declarative `.clad.toml` manifests + the `agent_summary` typestate fence in `crates/runtime/src/toolclad/`. See [Wanger 2026 / DOI 10.5281/zenodo.19957596](https://doi.org/10.5281/zenodo.19957596). |
| **Layer 3 — Identity** | [SchemaPin](https://github.com/ThirdKeyAI/SchemaPin) for MCP tools + [AgentPin](https://github.com/ThirdKeyAI/AgentPin) ES256 domain-anchored agent identity. |
| **Layer 4 — Policy Engine** | Cedar policy gate (`crates/runtime/src/reasoning/cedar_gate.rs`) + `CommunicationPolicyGate` for inter-agent calls; both fail-closed by default since v1.14.0. |
| **Layer 5 — Audit Journal** | Hash-chained, Ed25519-signed `BufferedJournal` in the reasoning loop; encrypted model-I/O logs in `crates/runtime/src/logging.rs`. |

Symbiont conforms to **OATS Extended** (C1–C7 + E1–E8). The empirical comparison of structural-enforcement runtimes that informs the spec is [Wanger 2026 / DOI 10.5281/zenodo.20043247](https://doi.org/10.5281/zenodo.20043247).

---

## Quick start

### Prerequisites

* Docker (recommended) or Rust 1.82+

### Scaffold and run a project (Docker, ~60 seconds)

```bash
# 1. Create the project in the current directory.
#    Generates symbiont.toml, agents/, policies/, docker-compose.yml, and
#    a .env with a freshly generated SYMBIONT_MASTER_KEY.
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest \
  init --profile assistant --no-interact --dir /workspace

# 2. Start the runtime. Reads .env automatically.
docker compose up
```

That's it — Runtime API on `http://localhost:8080`, HTTP Input on `http://localhost:8081`.
Use `symbi init --catalog list` (or the Docker equivalent) to browse pre-built agents.

### Other Docker recipes

```bash
# Ad-hoc runtime without a project (ephemeral, no master key)
docker run --rm -p 8080:8080 -p 8081:8081 ghcr.io/thirdkeyai/symbi:latest up

# MCP server only
docker run --rm -p 8080:8080 ghcr.io/thirdkeyai/symbi:latest mcp

# Parse an agent definition (`.symbi`; legacy `.dsl` also accepted)
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest \
  dsl -f /workspace/agent.symbi
```

### Build from source

```bash
cargo build --release
./target/release/symbi --help

# Scaffold a project locally, then start the runtime
./target/release/symbi init --profile assistant --no-interact
./target/release/symbi up
```

> For production deployments, review `SECURITY.md` and the [deployment guide](https://docs.symbiont.dev/getting-started) before enabling untrusted tool execution.

---

## How it works

Symbiont separates agent intent from execution authority:

1. **Agents propose** actions through the reasoning loop (Observe-Reason-Gate-Act)
2. **The runtime evaluates** each action against policy, identity, and trust checks
3. **Policy decides** — allowed actions execute; denied actions are blocked or routed for approval
4. **Everything is logged** — tamper-evident audit trail for every decision

Model output is never treated as execution authority. The runtime controls what actually happens.

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
agent secure_analyst(input: DataSet) -> Result {
    policy access_control {
        allow: read(input) if input.verified == true
        deny: send_email without approval
        audit: all_operations
    }

    with memory = "persistent", requires = "approval" {
        result = analyze(input);
        return result;
    }
}
```

See the [DSL guide](https://docs.symbiont.dev/dsl-guide) for the full grammar including `metadata`, `schedule`, `webhook`, and `channel` blocks.

> **File extension:** Symbiont agent definitions use `.symbi` as their canonical extension (e.g. `agents/assistant.symbi`). The legacy `.dsl` extension continues to be parsed indefinitely for backward compatibility, but new projects scaffolded with `symbi init` and all examples in this repo use `.symbi`.

---

## Core capabilities

| Capability | What it does |
|-----------|-------------|
| **Policy engine** | Fine-grained [Cedar](https://www.cedarpolicy.com/) authorization for agent actions, tool calls, and resource access |
| **Tool verification** | [SchemaPin](https://github.com/ThirdKeyAI/SchemaPin) cryptographic verification of MCP tool schemas before execution |
| **Tool contracts** | [ToolClad](https://github.com/ThirdKeyAI/ToolClad) declarative contracts with argument validation, scope enforcement, and Cedar policy generation |
| **Agent identity** | [AgentPin](https://github.com/ThirdKeyAI/AgentPin) domain-anchored ES256 identity for agents and scheduled tasks |
| **Reasoning loop** | Typestate-enforced Observe-Reason-Gate-Act cycle with policy gates and circuit breakers |
| **Sandboxing** | Docker, gVisor (`runsc`), or Firecracker microVM — selectable per agent via the DSL `with { sandbox = ... }` block |
| **Audit logging** | Tamper-evident logs with structured records for every policy decision |
| **Secrets management** | Vault/OpenBao integration, AES-256-GCM encrypted storage, scoped per agent |
| **MCP integration** | Native Model Context Protocol support with governed tool access |

Additional capabilities: threat scanning for tool/skill content (40 rules, 10 attack categories), cron scheduling, persistent agent memory, hybrid RAG search (LanceDB/Qdrant), webhook verification, delivery routing, OTLP telemetry, HTTP security hardening, and governance plugins for [Claude Code](https://github.com/thirdkeyai/symbi-claude-code) and [Gemini CLI](https://github.com/thirdkeyai/symbi-gemini-cli). See the [full documentation](https://docs.symbiont.dev) for details.

Representative benchmarks are available in the [benchmark harness](crates/runtime/benches/performance_claims.rs) and [threshold tests](crates/runtime/tests/performance_claims.rs).

---

## Security model

Symbiont is designed around a simple principle: **model output should never be trusted as execution authority.**

Actions flow through runtime controls:

* **Zero trust** — all agent inputs are untrusted by default
* **Policy checks** — Cedar authorization before every tool call and resource access
* **Tool verification** — SchemaPin cryptographic verification of tool schemas
* **Sandbox boundaries** — pick the isolation level per agent: Docker (default), gVisor (`runsc` syscall filter), or Firecracker (microVM)
* **Operator approval** — human review gates for sensitive actions
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
| `symbi-shell` | Interactive TUI for authoring, orchestration, and remote attach (beta) |
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

If you are evaluating Symbiont for production, start with the security model and getting started docs.

---

## SDKs

Official client SDKs for integrating with the Symbiont runtime from your application:

| Language | Package | Repository |
|----------|---------|------------|
| **JavaScript/TypeScript** | [symbiont-sdk-js](https://www.npmjs.com/package/symbiont-sdk-js) | [GitHub](https://github.com/ThirdKeyAI/symbiont-sdk-js) |
| **Python** | [symbiont-sdk](https://pypi.org/project/symbiont-sdk/) | [GitHub](https://github.com/ThirdKeyAI/symbiont-sdk-python) |

> **Production recommendation:** The JS and Python SDKs are HTTP clients intended for application integration and prototyping. For production agent workloads, we recommend building directly on the **Rust implementation** to leverage Symbiont's full typestate-driven safety guarantees — capability authorization, policy enforcement, and lifecycle invariants enforced at compile time rather than runtime. Dynamic-language clients can only verify these properties after a request crosses the runtime boundary.

---

## License

* **Community Edition** (Apache 2.0): Core runtime, DSL, policy engine, tool verification, sandboxing, agent memory, scheduling, MCP integration, RAG, audit logging, and all CLI/REPL tooling.
* **Enterprise Edition** (commercial): Compliance audit exports, AI-powered tool review, encrypted multi-agent collaboration, monitoring dashboards, and dedicated support. (All three sandbox backends — Docker, gVisor, and Firecracker — are OSS.)

Contact [ThirdKey](https://thirdkey.ai) for enterprise licensing.

---

<div align="right">
  <img src="https://raw.githubusercontent.com/ThirdKeyAI/Symbiont/main/symbi-trans.png" alt="Symbi Logo" width="120">
</div>
