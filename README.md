<img src="logo-hz.png" alt="Symbi">

[中文简体](README.zh-cn.md) | [Español](README.es.md) | [Português](README.pt.md) | [日本語](README.ja.md) | [Deutsch](README.de.md)

[![Build](https://img.shields.io/github/actions/workflow/status/thirdkeyai/symbiont/docker-build.yml?branch=main)](https://github.com/thirdkeyai/symbiont/actions)
[![Crates.io](https://img.shields.io/crates/v/symbi)](https://crates.io/crates/symbi)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Docs](https://img.shields.io/badge/docs-online-brightgreen)](https://docs.symbiont.dev)

---

## 🚀 What is Symbiont?

**Symbi** is a **Rust-native, zero-trust agent framework** for building autonomous, policy-aware AI agents.
It fixes the biggest flaws in existing frameworks like LangChain and AutoGPT by focusing on:

* **Security-first**: cryptographic audit trails, enforced policies, and sandboxing.
* **Zero trust**: all inputs are treated as untrusted by default.
* **Enterprise-grade compliance**: designed for regulated industries (HIPAA, SOC2, finance).

Symbiont agents collaborate safely with humans, tools, and LLMs — without sacrificing security or performance.

---

## ⚡ Why Symbiont?

| Feature          | Symbiont                                 | LangChain      | AutoGPT   |
| ---------------- | ---------------------------------------- | -------------- | --------- |
| Language         | Rust (safety, performance)               | Python         | Python    |
| Security         | Zero-trust, cryptographic audit          | Minimal        | None      |
| Reasoning Loop   | Typestate-enforced ORGA with policy gate | Simple chains  | Ad-hoc    |
| Knowledge Bridge | Vector search + episodic memory          | RAG only       | None      |
| Policy Engine    | Built-in DSL + Cedar                     | Limited        | None      |
| Deployment       | REPL, Docker, HTTP API                   | Python scripts | CLI hacks |
| Audit Trails     | Cryptographic logs                       | No             | No        |

---

## 🏁 Quick Start

### Prerequisites

* Docker (recommended) or Rust 1.88+
* No external vector database required (LanceDB embedded; Qdrant optional for scaled deployments)

### Run with Pre-Built Containers

```bash
# Parse an agent DSL file
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest dsl parse /workspace/agent.dsl

# Run MCP Server
docker run --rm -p 8080:8080 ghcr.io/thirdkeyai/symbi:latest mcp

# Interactive development shell
docker run --rm -it -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest bash
```

### Build from Source

```bash
# Build dev environment
docker build -t symbi:latest .
docker run --rm -it -v $(pwd):/workspace symbi:latest bash

# Build unified binary
cargo build --release

# Run REPL
cargo run -- repl

# Parse DSL & run MCP
cargo run -- dsl parse my_agent.dsl
cargo run -- mcp --port 8080
```

---

## 🔧 Key Features

* ✅ **DSL Grammar** – Define agents declaratively with built-in security policies, `memory`, `webhook`, `schedule`, and `channel` blocks.
* ✅ **Agent Runtime** – Task scheduling, resource management, and lifecycle control.
* 🔄 **Agentic Reasoning Loop** – Typestate-enforced Observe-Reason-Gate-Act (ORGA) cycle with multi-turn conversation management, unified inference across cloud and local SLM providers, circuit breakers, and durable journal. Five implementation phases: core loop, policy integration, human-in-the-loop, multi-agent patterns, and observability.
* 🧠 **Knowledge-Reasoning Bridge** – Opt-in integration between the knowledge/context system and the reasoning loop. Injects relevant context before each reasoning step, exposes `recall_knowledge`/`store_knowledge` as LLM-callable tools, and persists learnings after loop completion.
* ⏰ **Cron Scheduling** – Persistent SQLite-backed cron engine with jitter, concurrency guards, dead-letter queues, and heartbeat pattern.
* 🧠 **Persistent Memory** – Markdown-backed agent memory with facts, procedures, learned patterns, daily logs, and retention-based compaction.
* 🪝 **Webhook Verification** – HMAC-SHA256 and JWT signature verification with GitHub, Stripe, and Slack presets.
* 🛡️ **Skill Scanning** – ClawHavoc scanner with 40 rules across 10 attack categories (reverse shells, credential harvesting, process injection, privilege escalation, network exfiltration, and more). 5-level severity model (Critical/High/Medium/Warning/Info) with executable whitelisting.
* 📈 **Metrics & Telemetry** – File and OTLP metric exporters with composite fan-out and background collection. OpenTelemetry distributed tracing spans for the reasoning loop.
* 🔒 **HTTP Security Hardening** – Loopback-only binding, CORS allow-lists, JWT EdDSA validation, health endpoint separation.
* 🔒 **Sandboxing** – Tier-1 Docker isolation for agent execution.
* 🔒 **SchemaPin Security** – Cryptographic verification of tools and schemas.
* 🔒 **AgentPin Identity** – Domain-anchored cryptographic identity for scheduled agents.
* 🔒 **Secrets Management** – HashiCorp Vault / OpenBao integration, AES-256-GCM encrypted storage.
* 🔑 **Per-Agent API Keys** – Argon2-hashed API key authentication with per-IP rate limiting.
* 🧠 **Context Compaction** – Automatic context window management with tiered compaction: LLM-driven summarization (Tier 1) and truncation (Tier 4). Multi-model token counting (OpenAI, Claude, Gemini, Llama, Mistral, and more).
* 📊 **RAG Engine** – Vector search (LanceDB embedded) with hybrid semantic + keyword retrieval. Optional Qdrant backend for scaled deployments.
* 🧩 **MCP Integration** – Native support for Model Context Protocol tools, plus Composio SSE integration for external tool access.
* 📡 **Optional HTTP API** – Feature-gated REST interface for external integration.
* 📋 **Delivery Routing** – Route scheduled agent output to webhooks, Slack, email, or custom channels.
* 📝 **AGENTS.md Support** – Bidirectional agent manifest generation and parsing for interoperability.
* ⚡ **Performance Verified** – Benchmarked claims: policy evaluation <1ms, ECDSA P-256 verification <5ms, 10k agent scheduling with <2% CPU overhead.

---

## 📦 Workspace Crates

| Crate | Description | Status |
|-------|-------------|--------|
| `symbi` | Unified CLI binary | Stable |
| `symbi-runtime` | Core agent runtime | Stable |
| `symbi-dsl` | DSL parser and evaluator | Stable |
| `symbi-channel-adapter` | Slack/Teams/Mattermost adapters | Stable |
| `repl-core` | REPL engine | Stable |
| `repl-proto` | JSON-RPC protocol | Stable |
| `repl-cli` | Interactive CLI + JSON-RPC server | Stable |
| `repl-lsp` | Language Server Protocol | Stable |
| `symbi-a2ui` | Admin dashboard (Lit/TypeScript) | Alpha |

---

## 📐 Symbiont DSL Example

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

## 🔒 Security Model

* **Zero Trust** – all agent inputs are untrusted by default.
* **Sandboxed Execution** – Docker-based containment for processes.
* **Audit Logging** – Cryptographically tamper-evident logs.
* **Secrets Control** – Vault/OpenBao backends, encrypted local storage, agent namespaces.

---

## 📚 Documentation

* [Getting Started](https://docs.symbiont.dev/getting-started)
* [DSL Guide](https://docs.symbiont.dev/dsl-guide)
* [Runtime Architecture](https://docs.symbiont.dev/runtime-architecture)
* [Reasoning Loop Guide](https://docs.symbiont.dev/reasoning-loop)
* [Security Model](https://docs.symbiont.dev/security-model)
* [API Reference](https://docs.symbiont.dev/api-reference)

---

## 🎯 Use Cases

* **Development & Automation**

  * Secure code generation & refactoring.
  * AI agent deployment with enforced policies.
  * Knowledge management with semantic search.

* **Enterprise & Regulated Industries**

  * Healthcare (HIPAA-compliant processing).
  * Finance (audit-ready workflows).
  * Government (classified context handling).
  * Legal (confidential document analysis).

---

## 📄 License

* **Community Edition**: MIT License
* **Enterprise Edition**: Commercial license required

Contact [ThirdKey](https://thirdkey.ai) for enterprise licensing.

---

*Symbiont enables secure collaboration between AI agents and humans through intelligent policy enforcement, cryptographic verification, and comprehensive audit trails.*


<div align="right">
  <img src="symbi-trans.png" alt="Symbi Logo" width="120">
</div>
