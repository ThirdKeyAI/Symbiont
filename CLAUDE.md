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
- Agent files use `.symbi` (canonical) — `.dsl` is supported indefinitely for backward compatibility. Use `dsl::is_symbi_file` / `dsl::strip_symbi_extension` for file discovery instead of inlining extension checks. New scaffolding emits `.symbi` only.

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

Agent definitions live in `agents/*.symbi` (legacy `.dsl` is also recognized for backward compatibility). Key block types:

```
metadata { version "1.0", author "team", description "What this agent does" }

with { sandbox docker, timeout 30.seconds }

schedule daily_report { cron: "0 9 * * *", timezone: "UTC", agent: "reporter" }

channel slack_support { platform: "slack", default_agent: "helper", channels: ["#support"] }

webhook github_events { path: "/hooks/github", provider: github, agent: "deployer" }

memory context_store { store markdown, path "data/agents", retention "90d" }
```

Parse agent definitions with `symbi dsl -f agents/<name>.symbi`. (The `symbi dsl` subcommand name is intentionally preserved — it's a stable CLI surface, even though the file extension flipped.)

## Sandbox Tiers (all OSS)

The tiers form a monotonically increasing host-isolation ladder:

| Tier  | Backend              | Selection                          | Prerequisites |
|-------|----------------------|------------------------------------|---------------|
| tier0 | None (dev only)      | `with { sandbox = "none" }` / SYMBIONT_ALLOW_UNISOLATED=1 | — |
| tier1 | Docker               | default                            | `docker` daemon |
| tier2 | gVisor (`runsc`)     | `with { sandbox = "gvisor" }`      | `runsc` registered as Docker runtime |
| tier3 | Firecracker microVM  | `with { sandbox = "firecracker" }` | `firecracker` binary + operator-supplied vmlinux + rootfs.ext4 |

All three host-isolation tiers ship in the OSS runtime — no "Enterprise" gating on gVisor or Firecracker. Per-agent tier comes from the DSL `with { sandbox = "..." }` block; project default lives in `[sandbox] tier = "..."` in `symbiont.toml`.

For Tier 3 setup (kernel + rootfs prep, in-VM init contract, hardening checklist), see `docs/firecracker-setup.md`. Scaffold a tier3 project with:

```bash
symbi init --profile assistant --sandbox tier3 \
  --firecracker-kernel /path/to/vmlinux \
  --firecracker-rootfs /path/to/rootfs.ext4
```

`symbi init` validates both paths exist before writing `symbiont.toml`. `symbi doctor` reports whether `runsc` and `firecracker` binaries are reachable.

### Hosted execution: E2B (not a tier)

E2B is a separate hosted-cloud backend, **not** a peer of Tier 1/2/3. Code runs on E2B's infrastructure via their HTTPS API, so it carries no on-host isolation guarantees. Maps to `SecurityTier::Hosted`, which sorts below `Tier1` — policies requiring host isolation (`tier >= Tier1`) will reject it.

| Backend | Selection | Prerequisites | Use cases |
|---------|-----------|---------------|-----------|
| E2B (hosted) | `with { sandbox = "e2b" }` (DSL only — no `--sandbox` flag) | `E2B_API_KEY` env var | Quick-start demos, evaluation without setting up a sandbox host. **Not for production workloads with privacy or compliance requirements.** |

## Managed CLI agents (Mode B)

An agent whose metadata declares `executor = "claude_code"` is run by spawning a
governed Claude Code subprocess via `crates/runtime/src/cli_executor` (the
`cli-executor` feature, on by default) instead of the ORGA reasoning loop. The
reference agent is `agents/code_reviewer.symbi`; the path lives in
`src/commands/managed_cli.rs`.

`symbi run code_reviewer --target <dir>`:
- passes the spawn through the policy Gate (fail-closed; allow via Cedar or
  `SYMBI_INSECURE_ALLOW_ALL=1`);
- injects the env handshake `SYMBIONT_MANAGED=true`, `SYMBIONT_SESSION_ID`,
  `SYMBIONT_BUDGET_TOKENS`, `SYMBIONT_BUDGET_TIMEOUT`, `CLAUDE_PROJECT_DIR` (the
  symbi-claude-code plugin defers its hooks to the outer Gate on `SYMBIONT_MANAGED`);
- loads the plugin via `--plugin-dir` (resolve order: `--plugin-dir` flag,
  `SYMBIONT_CLAUDE_PLUGIN_DIR`, then sibling-repo autodetect) and wires the stdio
  `symbi mcp` back-channel via `--mcp-config --strict-mcp-config`;
- bounds the run with `--max-turns` (primary, cooperative) and `--budget-timeout`
  (hard wall-clock backstop; `CliExecutor` kills with graceful SIGTERM → SIGKILL).

Do **not** pass `--bare` to the spawned `claude` — it skips reading `~/.claude`
(credentials included) and breaks subscription auth.

## ToolClad Tools

Tools live in `tools/<name>.clad.toml` and are auto-discovered at startup by `symbi up`, the HTTP Input server, and `symbi tools`. The watcher (`crates/runtime/src/toolclad/watcher.rs`) hot-reloads on file changes — no restart needed.

The manifest carries everything: binary path, description, risk tier, human-approval flag, Cedar `resource`/`action` for policy evaluation, optional evidence-capture config. Cedar policies are auto-generated from manifest metadata via `crates/runtime/src/toolclad/cedar_gen.rs`. The ORGA Gate phase evaluates these before any tool invocation.

Argument types are validated in `crates/runtime/src/toolclad/validator.rs`. `agent_summary` is a best-effort defense-in-depth sanitizer for free text bound for a downstream prompt — **not** a load-bearing control. For a privileged downstream decision (routing, escalation, authorization), use typed `enum` args grounded in trusted context via Cedar, not free text: see `crates/runtime/src/toolclad/decision.rs` (`route_grounded`/`decide_route`), `tools/submit_triage.clad.toml`, and `examples/policies/triage_routing.cedar`. Mark decision-feeding args with `feeds_decision = true`; ToolClad manifest validation (`validate_toolclad`) flags free-text args that feed a privileged decision.

**Adding a new tool does not require Rust code.** Drop a `.clad.toml` in `tools/`, the runtime picks it up.

## MCP Server

Start with `symbi mcp` (stdio transport). Available tools:

- `invoke_agent` — Run a named agent with a prompt via LLM
- `list_agents` — List all agents in the `agents/` directory
- `parse_dsl` — Parse and validate DSL content (file or inline)
- `get_agent_dsl` — Get raw agent definition source (`.symbi` or legacy `.dsl`) for a specific agent
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
- Run in sandboxed environments — choose Tier 1 (Docker), Tier 2 (gVisor), or Tier 3 (Firecracker) per agent (all OSS host-isolation tiers); E2B is a separate hosted-cloud backend opt-in via the DSL
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
