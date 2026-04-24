<img src="https://raw.githubusercontent.com/ThirdKeyAI/Symbiont/main/logo-hz.png" alt="Symbi">

[中文简体](README.zh-cn.md) | [Español](README.es.md) | [Português](README.pt.md) | [日本語](README.ja.md) | [Deutsch](README.de.md)

[![Build](https://img.shields.io/github/actions/workflow/status/thirdkeyai/symbiont/docker-build.yml?branch=main)](https://github.com/thirdkeyai/symbiont/actions)
[![Crates.io](https://img.shields.io/crates/v/symbi)](https://crates.io/crates/symbi)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Docs](https://img.shields.io/badge/docs-online-brightgreen)](https://docs.symbiont.dev)

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
* **Sandboxing** for risky workloads — Docker isolation with resource limits
* **Audit trails** for what happened and why — cryptographically tamper-evident logs
* **Approval gates** for sensitive actions — human review before execution when policy requires it

Symbiont is built for that layer.

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

# Parse an agent DSL file
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest \
  dsl -f /workspace/agent.dsl
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

---

## Core capabilities

| Capability | What it does |
|-----------|-------------|
| **Policy engine** | Fine-grained [Cedar](https://www.cedarpolicy.com/) authorization for agent actions, tool calls, and resource access |
| **Tool verification** | [SchemaPin](https://github.com/ThirdKeyAI/SchemaPin) cryptographic verification of MCP tool schemas before execution |
| **Tool contracts** | [ToolClad](https://github.com/ThirdKeyAI/ToolClad) declarative contracts with argument validation, scope enforcement, and Cedar policy generation |
| **Agent identity** | [AgentPin](https://github.com/ThirdKeyAI/AgentPin) domain-anchored ES256 identity for agents and scheduled tasks |
| **Reasoning loop** | Typestate-enforced Observe-Reason-Gate-Act cycle with policy gates and circuit breakers |
| **Sandboxing** | Docker-based isolation with resource limits for untrusted workloads |
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
* **Sandbox boundaries** — Docker isolation for untrusted execution
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

---

## License

* **Community Edition** (Apache 2.0): Core runtime, DSL, policy engine, tool verification, sandboxing, agent memory, scheduling, MCP integration, RAG, audit logging, and all CLI/REPL tooling.
* **Enterprise Edition** (commercial): Advanced sandbox backends, compliance audit exports, AI-powered tool review, encrypted multi-agent collaboration, monitoring dashboards, and dedicated support.

Contact [ThirdKey](https://thirdkey.ai) for enterprise licensing.

---

<div align="right">
  <img src="https://raw.githubusercontent.com/ThirdKeyAI/Symbiont/main/symbi-trans.png" alt="Symbi Logo" width="120">
</div>
