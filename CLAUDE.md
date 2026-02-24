# Symbiont — Agent Instructions

Symbiont (Symbi) is a Rust-native, zero-trust agent framework for building autonomous, policy-aware AI agents. Part of the [ThirdKey](https://thirdkey.ai) trust stack: [SchemaPin](https://schemapin.org) → [AgentPin](https://agentpin.org) → **Symbiont**.

- **Docs**: https://docs.symbiont.dev
- **Repo**: https://github.com/ThirdKeyAI/Symbiont
- **Crate**: https://crates.io/crates/symbi

## Project Structure

```
crates/
├── dsl/              # Symbi DSL parser with Tree-sitter integration
├── runtime/          # Agent runtime (scheduling, routing, sandbox, AgentPin)
├── channel-adapter/  # Slack, Teams, Mattermost adapters
├── repl-core/        # Core REPL engine
├── repl-proto/       # JSON-RPC wire protocol types
├── repl-cli/         # Command-line REPL interface
├── repl-lsp/         # Language Server Protocol implementation
src/                  # Unified `symbi` CLI binary
```

## Build and Test

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace
cargo fmt --check
```

All four commands must pass before committing. Clippy must produce zero warnings.

## Code Style

- Rust edition 2021
- Run `cargo fmt` before committing
- Run `cargo clippy --workspace` and fix all warnings before committing
- Inline tests in source files using `#[cfg(test)] mod tests`
- ES256 (ECDSA P-256) only for AgentPin identity — reject all other algorithms

## Commit Guidelines

- Write concise commit messages focused on the "why"
- No mention of AI assistants or co-authoring in commit messages
- Use `date` command to determine the current date when adding dates to docs

## Security

- Zero-trust by default: all inputs are untrusted
- Cryptographic audit trails for agent actions
- Policy engine enforces runtime constraints via the Symbi DSL
- AgentPin integration for domain-anchored agent identity
- SchemaPin integration for tool schema verification
- Private keys (`*.private.pem`, `*.private.jwk.json`) must never be committed

## Docker

- Image: `ghcr.io/thirdkeyai/symbi:latest`
- Base: `rust:1.88-slim-bookworm` (builder), `debian:bookworm-slim` (runtime)
- The Dockerfile uses dependency caching with stub sources; cleanup globs must catch `libsymbi*` and `.fingerprint/symbi*`

## Releasing

See `.claude/RELEASE_RUNBOOK.md` for the full release process, including:

- How to determine which crates need version bumps
- Cross-crate version reference update checklist
- CI verification steps before tagging
- Docker build cache pitfalls
- crates.io publish order

## OSS Sync

Private repo is on Gitea. Public mirror is `github.com:ThirdKeyAI/Symbiont.git`.

```bash
bash scripts/sync_oss_to_github.sh --force
```

The script exits with code 1 during cleanup even on success — this is a known quirk.

## DSL Quick Reference

Agent definitions live in `agents/*.dsl`. Key block types:

```
metadata { version "1.0", author "team", description "What this agent does" }

with { sandbox docker, timeout 30.seconds }

schedule daily_report { cron: "0 9 * * *", timezone: "UTC", agent: "reporter" }

channel slack_support { platform: "slack", default_agent: "helper", channels: ["#support"] }

webhook github_events { path: "/hooks/github", provider: github, agent: "deployer" }

memory context_store { store markdown, path "data/agents", retention "90d" }
```

Parse DSL files with `symbi dsl -f agents/<name>.dsl`.

## MCP Server

Start with `symbi mcp` (stdio transport). Available tools:

- `invoke_agent` — Run a named agent with a prompt via LLM
- `list_agents` — List all agents in the `agents/` directory
- `parse_dsl` — Parse and validate DSL content (file or inline)
- `get_agent_dsl` — Get raw `.dsl` source for a specific agent
- `get_agents_md` — Read the project's AGENTS.md file
- `verify_schema` — Verify MCP tool schema via SchemaPin (ECDSA P-256)

## HTTP API

The runtime API runs on port 8080 (configurable via `--port`):

- `GET /api/v1/health` — Health check (no auth)
- `GET /api/v1/agents` — List agents
- `POST /api/v1/agents` — Create agent
- `POST /api/v1/agents/:id/execute` — Execute agent
- `GET /api/v1/schedules` — List cron schedules
- `POST /api/v1/schedules` — Create schedule
- `GET /api/v1/channels` — List channel adapters
- `POST /api/v1/workflows/execute` — Execute workflow
- `GET /api/v1/metrics` — Runtime metrics
- `GET /swagger-ui` — Interactive API docs

All endpoints except health require `Authorization: Bearer <token>`.

## Agent Capabilities

Agents defined in the Symbi DSL can:

- Invoke LLMs (OpenRouter, OpenAI, Anthropic) with policy-governed prompts
- Use skills (verified via SchemaPin cryptographic signatures)
- Run in sandboxed environments (Docker, gVisor, Firecracker, E2B)
- Operate on cron schedules with timezone support
- Connect to chat platforms (Slack, Teams, Mattermost) as channel adapters
- Receive webhooks (GitHub, Stripe, Slack, custom) with signature verification
- Maintain persistent memory stores with hybrid search (vector + keyword)
- Enforce runtime policies (allow, deny, require, audit)
- Produce cryptographic audit trails for all actions

## Trust Stack

Symbiont is part of the ThirdKey cryptographic trust chain:

1. **SchemaPin** — Tool schema verification. Ensures MCP tool schemas haven't been tampered with by verifying ECDSA P-256 signatures against publisher-hosted public keys.
2. **AgentPin** — Domain-anchored agent identity. Binds agent identities to DNS domains via `.well-known/agentpin.json`, enabling cross-runtime trust.
3. **Symbiont** — The agent runtime. Executes policy-aware agents with sandbox isolation, integrating SchemaPin for tool trust and AgentPin for agent identity.
