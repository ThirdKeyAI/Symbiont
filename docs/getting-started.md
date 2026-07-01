# Getting Started

This guide will walk you through setting up Symbi and creating your first AI agent.

▶ **Watch the get-started walkthrough:**

[![Symbiont — get started](https://img.youtube.com/vi/RPyKpqKz5ik/hqdefault.jpg)](https://www.youtube.com/watch?v=RPyKpqKz5ik)

---

## Prerequisites

What you need depends on how you install and run Symbi.

### To run the pre-built binary

The [pre-built binaries](#option-1-pre-built-binaries-quick-start) are already compiled — you do **not** need Rust, protobuf, or Git to install or run them. Install with Homebrew, the install script (`curl`), or a manual download from GitHub Releases.

- **Docker** is only needed at *runtime* if you execute agents under the default sandbox tier (`tier1`, Docker-backed). It is **not** needed to install Symbi or to run `symbi init`, `symbi dsl`, or `symbi --version`.

### To build from source

Required only if you install via `cargo install` or build the repository yourself:

- **Rust 1.86+**
- **protobuf-compiler** (`apt install protobuf-compiler` on Ubuntu, `brew install protobuf` on macOS)
- **Git** (to clone the repository)

### Optional

- **[symbi-claude-code](https://github.com/thirdkeyai/symbi-claude-code)** (Claude Code governance plugin)
- **[symbi-gemini-cli](https://github.com/thirdkeyai/symbi-gemini-cli)** (Gemini CLI governance extension)

> **Note:** Vector search is built in. Symbi ships with [LanceDB](https://lancedb.com/) as an embedded vector database -- no external service required.

---

## Installation

### Option 1: Pre-Built Binaries (Quick Start)

> **Note:** Pre-built binaries are tested but considered less reliable than cargo install or Docker.

**macOS (Homebrew):**
```bash
brew tap thirdkeyai/tap
brew install symbi
```

**macOS / Linux (install script):**
```bash
curl -fsSL https://symbiont.dev/install.sh | bash
```

**Manual download:**
Download from [GitHub Releases](https://github.com/thirdkeyai/symbiont/releases) and add to your PATH.

### Option 2: Docker (Recommended)

The fastest way to get a working runtime is to let the container scaffold the project for you:

```bash
# 1. Scaffold symbiont.toml, agents/, policies/, docker-compose.yml, and
#    a .env with a freshly generated SYMBIONT_MASTER_KEY.
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest \
  init --profile assistant --no-interact --dir /workspace

# 2. Start the runtime. Reads .env automatically.
docker compose up
```

Runtime API is now on `http://localhost:8080` and HTTP Input on `http://localhost:8081`.

If you'd rather work from a clone (to build the image yourself or run tests):

```bash
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont

# Build the unified symbi container
docker build -t symbi:latest .

# Run the development environment
docker run --rm -it -v $(pwd):/workspace symbi:latest bash
```

### Option 3: Local Installation

For local development:

```bash
# Clone the repository
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont

# Install Rust dependencies and build
cargo build --release

# Run tests to verify installation
cargo test
```

### Verify Installation

Test that everything is working correctly.

**Installed binary or Docker:**

```bash
symbi --version
symbi dsl --help
symbi mcp --help

# ...or via the published Docker image
docker run --rm ghcr.io/thirdkeyai/symbi:latest --version
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest dsl --help
```

**From a source clone:**

```bash
# Test the DSL parser and the runtime
cargo test -p symbi-dsl
cargo test -p symbi-runtime

# Run example agents
cargo run -p symbi-runtime --example basic_agent
cargo run -p symbi-runtime --example full_system

# Exercise the unified symbi CLI
cargo run -- dsl --help
cargo run -- mcp --help
```

---

## Project Initialization

The fastest way to start a new Symbiont project is `symbi init`:

```bash
symbi init
```

This launches an interactive wizard that guides you through:
- **Profile selection**: `minimal`, `assistant`, `dev-agent`, or `multi-agent`
- **SchemaPin mode**: `tofu` (Trust-On-First-Use), `strict`, or `disabled`
- **Sandbox tier**: `tier0` (none, dev only), `tier1` (Docker), `tier2` (gVisor / `runsc`), or `tier3` (Firecracker microVM)

### What `init` produces

Every run writes:

| File | Purpose |
|------|---------|
| `symbiont.toml` | Runtime and policy configuration |
| `policies/default.cedar` | Deny-by-default Cedar policy |
| `agents/*.symbi` | Profile-specific agent definitions (legacy `.dsl` is also recognized; except `minimal`) |
| `AGENTS.md` | Auto-generated index of declared agents |
| `.symbiont/audit/` | Tamper-evident audit log directory |
| `.gitignore` | Appended with Symbiont-specific entries, including `.env` |
| `.env` | `SYMBIONT_MASTER_KEY` generated from `/dev/urandom` (0600 perms) |
| `.env.example` | Safe-to-commit template showing required env vars |
| `docker-compose.yml` | Ready-to-run compose file with correct volume mounts and env wiring |

Pass `--no-docker-compose` to skip the compose file, and `--dir <PATH>` to write into a directory other than the current one (essential inside a Docker container — see below).

### Non-interactive mode

For CI/CD or scripted setups:

```bash
symbi init --profile assistant --schemapin tofu --sandbox tier1 --no-interact
```

### Running `init` inside Docker

Because the image's WORKDIR is `/var/lib/symbi`, use `--dir` to write into your mounted volume:

```bash
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest \
  init --profile assistant --no-interact --dir /workspace
```

That populates the host's current directory with the full project tree.

### Profiles

| Profile | What it creates |
|---------|----------------|
| `minimal` | `symbiont.toml` + default Cedar policy |
| `assistant` | + single governed assistant agent |
| `dev-agent` | + CliExecutor agent with safety policies |
| `multi-agent` | + coordinator/worker agents with inter-agent policies |

### Importing from the catalog

Import pre-built agents alongside any profile:

```bash
symbi init --profile minimal --no-interact
symbi init --catalog assistant,dev
```

List available catalog agents:

```bash
symbi init --catalog list
```

After initialization, validate and run:

```bash
symbi dsl -f agents/assistant.symbi   # validate your agent
symbi run assistant -i '{"query": "hello"}'  # test a single agent
symbi up                             # start the runtime locally
docker compose up                    # ...or start it in Docker (reads .env)
```

### Running a single agent

Use `symbi run` to execute one agent without starting the full runtime server:

```bash
symbi run <agent-name-or-file> --input <json>
```

The command resolves agent names by searching: direct path, then `agents/` directory. It sets up cloud inference from environment variables (`OPENROUTER_API_KEY`, `OPENAI_API_KEY`, or `ANTHROPIC_API_KEY`), runs the ORGA reasoning loop, and exits.

```bash
symbi run assistant -i 'Summarize this document'
symbi run agents/recon.symbi -i '{"target": "10.0.1.5"}' --max-iterations 5
```

### Starting from a template (`symbi new`)

`symbi init` scaffolds a generic project; `symbi new` scaffolds a project around one of several task-shaped templates. Useful when you know the kind of agent you need before you know the agents you need.

```bash
symbi new --list                     # show available templates
symbi new <template> <project-name>  # create a new project from a template
```

Shipped templates:

| Template | What you get |
|----------|--------------|
| `webhook-min` | Minimal webhook-driven agent — HTTP Input config + a handler DSL |
| `webscraper-agent` | Scraping agent with Cedar access policies and a ToolClad scraper tool |
| `slm-first` | Router + SLM allow-list + confidence fallback pattern |
| `rag-lite` | Qdrant-backed ingestion scripts plus a search agent |

`symbi new` and `symbi init` are complementary: `new` gives you a task-specific starting point, `init` (+ `--catalog`) gives you a governance-specific one. You can also combine — scaffold with `new`, then `symbi init --catalog ...` to pull in additional pre-built agents from the catalogue.

---

## Your First Agent

Let's create a simple data analysis agent to understand the basics of Symbi.

### 1. Create Agent Definition

Create a new file `my_agent.symbi`:

```rust
metadata {
    version = "1.0.0"
    author = "your-name"
    description = "My first Symbi agent"
}

agent greet_user(name: String) -> String {
    capabilities = ["greeting", "text_processing"]
    
    policy safe_greeting {
        allow: read(name) if name.length <= 100
        deny: store(name) if name.contains_sensitive_data
        audit: all_operations with signature
    }
    
    with memory = "ephemeral", privacy = "low" {
        if (validate_name(name)) {
            greeting = format_greeting(name);
            audit_log("greeting_generated", greeting.metadata);
            return greeting;
        } else {
            return "Hello, anonymous user!";
        }
    }
}
```

### 2. Run the Agent

First, validate the agent definition:

```bash
symbi dsl -f my_agent.symbi
```

Then run it. `symbi run` executes the agent through the ORGA reasoning loop,
which needs a cloud LLM key — export one of `OPENROUTER_API_KEY`,
`OPENAI_API_KEY`, or `ANTHROPIC_API_KEY` first:

```bash
export OPENROUTER_API_KEY=sk-...            # or OPENAI_API_KEY / ANTHROPIC_API_KEY
symbi run my_agent.symbi -i '{"name": "World"}'
```

---

## Understanding the DSL

The Symbi DSL has several key components:

### Metadata Block

```rust
metadata {
    version = "1.0.0"
    author = "developer"
    description = "Agent description"
}
```

Provides essential information about your agent for documentation and runtime management.

### Agent Definition

```rust
agent agent_name(parameter: Type) -> ReturnType {
    capabilities = ["capability1", "capability2"]
    // agent implementation
}
```

Defines the agent's interface, capabilities, and behavior.

### Policy Definitions

```rust
policy policy_name {
    allow: action_list if condition
    deny: action_list if condition
    audit: operation_type with audit_method
}
```

Declarative security policies that are enforced at runtime.

### Execution Context

```rust
with memory = "persistent", privacy = "high" {
    // agent implementation
}
```

Specifies runtime configuration for memory management and privacy requirements.

---

## Next Steps

### Explore Examples

The repository includes several example agents:

```bash
# Basic agent example
cd crates/runtime && cargo run --example basic_agent

# Full system demonstration
cd crates/runtime && cargo run --example full_system

# Context and memory example
cd crates/runtime && cargo run --example context_example

# RAG-powered agent
cd crates/runtime && cargo run --example rag_example
```

### Enable Advanced Features

#### HTTP API (Optional)

```bash
# Enable the HTTP API feature
cd crates/runtime && cargo build --features http-api

# Run with API endpoints
cd crates/runtime && cargo run --features http-api --example full_system
```

**Key API Endpoints:**
- `GET /api/v1/health` - Health check and system status
- `GET /api/v1/agents` - List all active agents with real-time execution status
- `GET /api/v1/agents/{id}/status` - Get detailed agent execution metrics
- `POST /api/v1/workflows/execute` - Execute workflows

**New Agent Management Features:**
- Real-time process monitoring and health checks
- Graceful shutdown capabilities for running agents
- Comprehensive execution metrics and resource usage tracking
- Support for different execution modes (ephemeral, persistent, scheduled, event-driven)

#### Cloud LLM Inference

Connect to cloud LLM providers via OpenRouter:

```bash
# Enable cloud inference
cargo build --features cloud-llm

# Set API key and model
export OPENROUTER_API_KEY="sk-or-..."
export OPENROUTER_MODEL="google/gemini-2.0-flash-001"  # optional
```

#### AWS Bedrock Provider

Use AWS Bedrock-hosted models via the Converse API:

```bash
# Enable Bedrock provider
cargo build --features bedrock

# For reasoning-loop inference, also enable cloud-llm
cargo build --features bedrock,cloud-llm

# Set Bedrock model and region
export BEDROCK_MODEL_ID="anthropic.claude-3-5-sonnet-20241022-v2:0"
export AWS_REGION="us-east-1"

# Credentials from standard AWS chain (env, profile, or role)
export AWS_ACCESS_KEY_ID="<your-key>"
export AWS_SECRET_ACCESS_KEY="<your-secret>"
```

The Bedrock provider integrates with `LlmClient` and `CloudInferenceProvider` for agent reasoning. It uses the Converse API (non-streaming) with tool-use support for tool-capable models. Credentials follow the standard AWS credential chain: environment variables, shared credentials file, or IAM role.

For the reasoning loop's `symbi run` / `symbi up` with Bedrock, always build with both `bedrock` and `cloud-llm` features.

#### Standalone Agent Mode

One-liner for cloud-native agents with LLM inference:

```bash
cargo build --features standalone-agent
# Enables: cloud-llm
```

> **Note:** Composio MCP and SymbiBot integration were removed in this version due to security concerns — see SECURITY_AUDIT.md C3 for context.

#### Advanced Reasoning Primitives

Enable tool curation, stuck-loop detection, context pre-fetch, and scoped conventions:

```bash
cargo build --features orga-adaptive
```

See the [orga-adaptive guide](/orga-adaptive) for full documentation.

#### Cedar Policy Engine

Formal authorization with the Cedar policy language. **Default-on since v1.14.x**: published `symbi` binaries (crates.io, Docker, GitHub Release tarballs) include Cedar, and `symbi up` / `symbi run` auto-wire `CedarPolicyGate` from `policies/*.cedar` files at startup; if none are present the runtime falls through to the fail-closed `DefaultPolicyGate`. To build without Cedar (e.g. when you intend to wire `OpaPolicyGateBridge` or a custom `ReasoningPolicyGate` instead), use:

```bash
cargo build --no-default-features --features "keychain,vector-lancedb"  # drop cedar
```

#### Vector Database (Built-in)

Symbi includes LanceDB as a zero-config embedded vector database. Semantic search and RAG work out of the box -- no separate service to start:

```bash
# Run agent with RAG capabilities (vector search just works)
cd crates/runtime && cargo run --example rag_example

# Test context management with advanced search
cd crates/runtime && cargo run --example context_example
```

> **Minimal build:** LanceDB is included by default but can be excluded for lighter binaries: `cargo build --no-default-features`. The runtime gracefully falls back to a no-op vector backend.
>
> **Scaled deployments:** Qdrant is available as an optional backend. Build with `--features vector-qdrant` and set `SYMBIONT_VECTOR_BACKEND=qdrant`.

**Context Management Features:**
- **Multi-Modal Search**: Keyword, temporal, similarity, and hybrid search modes
- **Importance Calculation**: Sophisticated scoring algorithm considering access patterns, recency, and user feedback
- **Access Control**: Policy engine integration with agent-scoped access controls
- **Automatic Archiving**: Retention policies with compressed storage and cleanup
- **Knowledge Sharing**: Secure cross-agent knowledge sharing with trust scores

#### Feature Flags Reference

| Feature | Description | Default |
|---------|-------------|---------|
| `keychain` | OS keychain integration for secrets | Yes |
| `vector-lancedb` | LanceDB embedded vector backend | Yes |
| `vector-qdrant` | Qdrant distributed vector backend | No |
| `embedding-models` | Local embedding models via Candle | No |
| `http-api` | REST API with Swagger UI | No |
| `http-input` | Webhook server with JWT auth | No |
| `bedrock` | AWS Bedrock LLM provider (implies `http-input`) | No |
| `cloud-llm` | Cloud LLM inference (OpenRouter / OpenAI / Anthropic) | No |
| `standalone-agent` | Cloud LLM meta-feature | No |
| `cedar` | Cedar policy engine — auto-wires from `policies/*.cedar` at startup | **Yes** |
| `orga-adaptive` | Advanced reasoning primitives | No |
| `cron` | Persistent cron scheduling | **Yes** |
| `cli-executor` | Governed AI CLI subprocesses — Mode B | **Yes** |
| `native-sandbox` | Native process sandboxing | No |
| `metrics` | OpenTelemetry metrics/tracing | No |
| `session` | Experimental multiparty session-type protocol monitor | No |
| `toolclad-session` | Persistent ToolClad tool sessions (PTY-backed) | No |
| `interactive` | Interactive prompts for `symbi init` (dialoguer) | Default |
| `minimal` | Minimal build for faster CI (no optional backends) | No |
| `full` | All optional runtime, vector, and policy features | No |

```bash
# Build with specific features
cargo build --features "cloud-llm,orga-adaptive,cedar"

# Build with everything
cargo build --features full
```

---

## AI Assistant Plugins

Symbiont provides first-party governance plugins for popular AI coding assistants with three progressive protection tiers:

1. **Awareness** (default) — advisory logging of all state-modifying tool calls
2. **Protection** — blocking hook enforces a local deny list (`.symbiont/local-policy.toml`)
3. **Governance** — Cedar policy evaluation when `symbi` is on PATH

The deny list config is tool-agnostic — the same `.symbiont/local-policy.toml` works with both plugins:

```toml
[deny]
paths = [".env", ".ssh/", ".aws/"]
commands = ["rm -rf", "git push --force"]
branches = ["main", "master", "production"]
```

### Claude Code

```bash
# Install from marketplace
/plugin marketplace add https://github.com/thirdkeyai/symbi-claude-code

# Available skills: /symbi-init, /symbi-policy, /symbi-verify, /symbi-audit, /symbi-dsl
```

See [symbi-claude-code](https://github.com/thirdkeyai/symbi-claude-code) for details.

#### Mode B: governed Claude Code subprocess

Beyond the in-editor hooks, Symbiont can run Claude Code as a *governed
subprocess* — the "Mode B" (ORGA-managed) path. An agent whose metadata declares
`executor = "claude_code"` runs by spawning Claude Code under the runtime's
`CliExecutor` instead of the LLM reasoning loop. The bundled `code_reviewer`
agent is the reference example:

```bash
# Review a working tree with a governed Claude Code subprocess
symbi run code_reviewer --target /path/to/repo

# Bounds: --max-turns is the primary (cooperative) limit; --budget-timeout is a
# hard wall-clock backstop (graceful SIGTERM -> SIGKILL).
symbi run code_reviewer --target . --max-turns 12 --budget-timeout 15m
```

On each run Symbiont:

- evaluates the spawn through the policy **Gate** (fail-closed — allow it via a
  Cedar policy, or `SYMBI_INSECURE_ALLOW_ALL=1` for local development);
- sets the env handshake (`SYMBIONT_MANAGED=true`, `SYMBIONT_SESSION_ID`,
  `SYMBIONT_BUDGET_TOKENS`, `SYMBIONT_BUDGET_TIMEOUT`, `CLAUDE_PROJECT_DIR`) so the
  symbi-claude-code plugin **defers** its hooks to the outer Gate;
- loads the plugin via `--plugin-dir` and wires the stdio `symbi mcp` back-channel
  via `--mcp-config --strict-mcp-config`;
- runs Claude Code headless (`--print --output-format json --permission-mode dontAsk`).

| Variable / flag | Purpose | Default |
|---|---|---|
| `SYMBIONT_CLAUDE_PLUGIN_DIR` | Path to the symbi-claude-code plugin | autodetect sibling repo |
| `--plugin-dir` | Override the plugin path for one run | — |
| `--target` | Working directory to operate on | current dir |
| `--max-turns` | Primary cooperative bound (agentic turns) | 12 |
| `--budget-timeout` | Wall-clock backstop, e.g. `15m` / `900s` | 15m |
| `--budget-tokens` | Token budget hint passed to the subprocess (awareness) | 100000 |

> **Auth:** the subprocess uses Claude Code's own authentication — a logged-in
> session (`claude /login`) or `ANTHROPIC_API_KEY`. The `cli-executor` feature is
> on by default.

### Gemini CLI

```bash
# Install extension
gemini extensions install https://github.com/thirdkeyai/symbi-gemini-cli
```

The Gemini CLI extension provides additional defense-in-depth via `excludeTools` manifest blocking and native `policies/*.toml` enforcement at the platform level.

See [symbi-gemini-cli](https://github.com/thirdkeyai/symbi-gemini-cli) for details.

---

## Configuration

### Environment Variables

Set up your environment for optimal performance:

```bash
# Required: 32-byte hex key used to encrypt persistent state.
# Generate with: openssl rand -hex 32
# `symbi init` writes one into .env automatically, and `symbi` auto-loads
# .env from the working directory — so in a scaffolded project you don't
# need to export it by hand. The line below is only for ad-hoc shells.
export SYMBIONT_MASTER_KEY="..."

# Basic configuration
export SYMBI_LOG_LEVEL=info
export SYMBI_RUNTIME_MODE=development

# Vector search works out of the box with the built-in LanceDB backend.
# To use Qdrant instead (optional, enable the `vector-qdrant` feature):
# export SYMBIONT_VECTOR_BACKEND=qdrant
# export QDRANT_URL=http://localhost:6333

# MCP integration (optional)
export MCP_SERVER_URLS="http://localhost:8080"
```

#### Security-related Environment Variables (post-v1.13.0 audit)

| Variable | Default | Effect |
|---|---|---|
| `SYMBI_INSECURE_ALLOW_ALL` | unset | When set to `1`, `symbi up` / `symbi run` use the permissive policy gate (every tool call and delegation is allowed). Equivalent to the `--insecure-allow-all` flag. Loud stderr banner is printed. **For local development only.** Without this, the reasoning loop is fail-closed and rejects tool calls and delegations until an explicit policy backend is wired in. |
| `SYMBI_REJECT_LEGACY_API_KEYS` | unset | When set to `1`, the API-key validator short-circuits the deprecated O(n) Argon2 scan for unprefixed keys. Use this immediately after re-issuing every key in `keyid.secret` format. The legacy path will be removed in the next minor release regardless. |
| `SYMBI_UNSAFE_NATIVE_SANDBOX` | unset | Required (in addition to `SYMBI_ENV=production`-not-set) to construct the `native` sandbox runner. The `native-sandbox` Cargo feature also fails to compile in release builds. The native runner provides zero isolation and is intended only for local debugging. |
| `SYMBI_TRUSTED_PROXIES` | unset | CIDR allowlist for trusted reverse proxies; `X-Forwarded-For` is only honored from these addresses. |
| `SYMBIONT_ESCALATION_TIMEOUT` | `120` | Seconds a held action waits for human approval before failing closed (deny). |
| `SYMBIONT_REQUIRE_APPROVAL_TOOLS` | unset | Comma-separated tool names that require human approval before execution (e.g. `http_post,delete_file`). |

The following environment variables were **removed**:

- `SYMBIONT_ALLOW_NO_JWT_AUDIENCE` — the JWT verifier now always requires `aud`. (Removed in the post-v1.13.0 audit; was an unsafe escape hatch.)
- `COMPOSIO_API_KEY`, `COMPOSIO_MCP_URL` — the Composio MCP integration was removed entirely. See `SECURITY_AUDIT.md` C3.

### Tool Contracts (ToolClad)

Define governed tool contracts in the `tools/` directory:

```bash
symbi tools init my_tool          # create a starter manifest
symbi tools validate              # validate all manifests
symbi tools test my_tool --arg target=10.0.1.5   # dry-run with args
```

See the [ToolClad guide](/toolclad) for the full manifest format, execution modes, and scope enforcement.

### Runtime Configuration

Create a `symbi.toml` configuration file:

```toml
[runtime]
max_agents = 1000
memory_limit_mb = 512
execution_timeout_seconds = 300

[security]
default_sandbox_tier = "docker"
audit_enabled = true
policy_enforcement = "strict"

[vector_db]
enabled = true
backend = "lancedb"              # default; also supports "qdrant"
collection_name = "symbi_knowledge"
# url = "http://localhost:6333"  # only needed when backend = "qdrant"
```

### Human-in-the-loop approvals

When a tool listed in `SYMBIONT_REQUIRE_APPROVAL_TOOLS` (or a policy that escalates)
holds an agent action, the runtime queues it and waits (up to the escalation
timeout, fail-closed). Operators resolve held actions in real time via:

- **REST:** `GET /api/v1/approvals`, `POST /api/v1/approvals/{id}/approve`, `.../deny`
- **The `symbi-shell` Gate panel:** open with `/gate` or `Ctrl+G`; `↑/↓` select, `a` approve, `d` deny.
- **Chat:** reply `/symbi gate approve <id>` or `/symbi gate deny <id> [reason]` in a configured approval channel (allowlisted senders only).

Configure the timeout and chat approval channels in `symbiont.toml`:

```toml
[escalation]
timeout_seconds = 120

[[escalation.approval_channels]]
platform   = "slack"
channel_id = "C0APPROVERS"
approvers  = ["U0ALICE", "U0BOB"]   # allowlisted sender ids; empty = nobody may approve via chat
```

---

## Common Issues

### Docker Issues

**Problem**: Docker build fails with permission errors
```bash
# Solution: Ensure Docker daemon is running and user has permissions
sudo systemctl start docker
sudo usermod -aG docker $USER
```

**Problem**: Container exits immediately
```bash
# Solution: Check Docker logs
docker logs <container_id>
```

### Rust Build Issues

**Problem**: Cargo build fails with dependency errors
```bash
# Solution: Update Rust and clean build cache
rustup update
cargo clean
cargo build
```

**Problem**: Missing system dependencies
```bash
# Ubuntu/Debian
sudo apt-get update
sudo apt-get install build-essential pkg-config libssl-dev

# macOS
brew install pkg-config openssl
```

### Runtime Issues

**Problem**: Agent fails to start
```bash
# Check agent definition syntax
symbi dsl -f your_agent.symbi

# Enable debug logging
RUST_LOG=debug symbi run your_agent.symbi -i '{}'
```

---

## Getting Help

### Documentation

- **[DSL Guide](/dsl-guide)** - Complete DSL reference
- **[Runtime Architecture](/runtime-architecture)** - System architecture details
- **[Security Model](/security-model)** - Security and policy documentation

### Community Support

- **Issues**: [GitHub Issues](https://github.com/thirdkeyai/symbiont/issues)
- **Discussions**: [GitHub Discussions](https://github.com/thirdkeyai/symbiont/discussions)
- **Documentation**: [Complete API Reference](https://docs.symbiont.dev/api-reference)

### Debug Mode

For troubleshooting, enable verbose logging:

```bash
# Enable debug logging
export RUST_LOG=symbi=debug

# Run with detailed output
cd crates/runtime && cargo run --example basic_agent 2>&1 | tee debug.log
```

---

## What's Next?

Now that you have Symbi running, explore these advanced topics:

1. **[DSL Guide](/dsl-guide)** - Learn advanced DSL features
2. **[Reasoning Loop Guide](/reasoning-loop)** - Understand the ORGA cycle
3. **[Advanced Reasoning (orga-adaptive)](/orga-adaptive)** - Tool curation, stuck-loop detection, pre-hydration
4. **[Runtime Architecture](/runtime-architecture)** - Understand the system internals
5. **[Security Model](/security-model)** - Implement security policies
6. **[Contributing](/contributing)** - Contribute to the project

Ready to build something amazing? Start with our [example projects](https://github.com/thirdkeyai/symbiont/tree/main/crates/runtime/examples) or dive into the [complete specification](/dsl-specification).