# Getting Started

This guide will walk you through setting up Symbi and creating your first AI agent.



---

## Prerequisites

Before getting started with Symbi, ensure you have the following installed:

### Required Dependencies

- **Docker** (for containerized development)
- **Rust 1.82+** (if building locally)
- **protobuf-compiler** (required for building — `apt install protobuf-compiler` on Ubuntu, `brew install protobuf` on macOS)
- **Git** (for cloning the repository)

### Optional Dependencies

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
curl -fsSL https://raw.githubusercontent.com/thirdkeyai/symbiont/main/scripts/install.sh | bash
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

Test that everything is working correctly:

```bash
# Test the DSL parser
cd crates/dsl && cargo run && cargo test

# Test the runtime system
cd ../runtime && cargo test

# Run example agents
cargo run --example basic_agent
cargo run --example full_system

# Test the unified symbi CLI
cd ../.. && cargo run -- dsl --help
cargo run -- mcp --help

# Test with Docker container
docker run --rm symbi:latest --version
docker run --rm -v $(pwd):/workspace symbi:latest dsl parse --help
docker run --rm symbi:latest mcp --help
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
- **Sandbox tier**: `tier0` (none), `tier1` (Docker), or `tier2` (gVisor)

### What `init` produces

Every run writes:

| File | Purpose |
|------|---------|
| `symbiont.toml` | Runtime and policy configuration |
| `policies/default.cedar` | Deny-by-default Cedar policy |
| `agents/*.dsl` | Profile-specific agent definitions (except `minimal`) |
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
symbi dsl -f agents/assistant.dsl   # validate your agent
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
symbi run agents/recon.dsl -i '{"target": "10.0.1.5"}' --max-iterations 5
```

---

## Your First Agent

Let's create a simple data analysis agent to understand the basics of Symbi.

### 1. Create Agent Definition

Create a new file `my_agent.dsl`:

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

```bash
# Parse and validate the agent definition
cargo run -- dsl parse my_agent.dsl

# Run the agent in the runtime
cd crates/runtime && cargo run --example basic_agent -- --agent ../../my_agent.dsl
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

#### Standalone Agent Mode

One-liner for cloud-native agents with LLM inference and Composio tool access:

```bash
cargo build --features standalone-agent
# Enables: cloud-llm + composio
```

#### Advanced Reasoning Primitives

Enable tool curation, stuck-loop detection, context pre-fetch, and scoped conventions:

```bash
cargo build --features orga-adaptive
```

See the [orga-adaptive guide](/orga-adaptive) for full documentation.

#### Cedar Policy Engine

Formal authorization with the Cedar policy language:

```bash
cargo build --features cedar
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
| `cloud-llm` | Cloud LLM inference (OpenRouter) | No |
| `composio` | Composio MCP tool integration | No |
| `standalone-agent` | Cloud LLM + Composio combo | No |
| `cedar` | Cedar policy engine | No |
| `orga-adaptive` | Advanced reasoning primitives | No |
| `cron` | Persistent cron scheduling | No |
| `native-sandbox` | Native process sandboxing | No |
| `metrics` | OpenTelemetry metrics/tracing | No |
| `interactive` | Interactive prompts for `symbi init` (dialoguer) | Default |
| `full` | All features except enterprise | No |

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
# `symbi init` writes one into .env automatically.
export SYMBIONT_MASTER_KEY="..."

# Basic configuration
export SYMBI_LOG_LEVEL=info
export SYMBI_RUNTIME_MODE=development

# Vector search works out of the box with the built-in LanceDB backend.
# To use Qdrant instead (optional, enterprise):
# export SYMBIONT_VECTOR_BACKEND=qdrant
# export QDRANT_URL=http://localhost:6333

# MCP integration (optional)
export MCP_SERVER_URLS="http://localhost:8080"
```

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
cargo run -- dsl parse your_agent.dsl

# Enable debug logging
RUST_LOG=debug cd crates/runtime && cargo run --example basic_agent
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

Ready to build something amazing? Start with our [example projects](https://github.com/thirdkeyai/symbiont/tree/main/crates/runtime/examples) or dive into the [complete specification](/specification).