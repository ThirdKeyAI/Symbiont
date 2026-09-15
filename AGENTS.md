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

## Local models

`symbi run` and `symbi up` accept any OpenAI-compatible endpoint, so a local
model works with no cloud key:

```bash
export OPENAI_API_KEY=ollama
export OPENAI_BASE_URL=http://localhost:11434/v1
export CHAT_MODEL=llama3.1
```

The SSRF guard is deliberately not applied to these operator-supplied base
URLs — they are configuration at the same trust level as the key beside them.
It stays on every attacker-influenced destination (ToolClad HTTP backends,
SchemaPin key discovery). Both the URL check and the SSRF-filtering DNS
resolver had to be lifted for this path; see
`net_guard::customise_operator_client`.

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

For validation rather than inspection, use `symbi dsl --check -f <file>`: one
line per file and an exit code, so it can gate CI. The bare form prints the
full parse tree, which is for debugging, not for checking.

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

An agent with `executor = "claude_code"` uses the project-selected Docker,
gVisor or Firecracker worker with a scratch workspace and private runtime channels.
VMs use `/tmp` and fixed tool/inference vsock capabilities. Their `--target` is an
absolute guest path; host source is not automatically transferred. Readable
rootfs contents are shared between the CLI and backend images. Source
mounts belong to ToolClad backends. The child has no direct source mount or
network access; provider credentials, policy, approval state and audit keys
remain outside it. Configure `[managed_cli.inference]` with an explicit endpoint,
model and credential variable, and provision an image with Claude Code and Python.
The reference reviewer also needs Git.

`allowed_tools` is an exact subset of registered ToolClad names. The runtime
disables built-in tools, automatic discovery and plugins. Both the session spawn
and individual actions require Cedar authorization in `policies/managed-cli/`
plus shared policies. Actions use the prepared-call dispatcher and mandatory
exact approvals. Ordinary and managed CLI runs can enable operator approvals with
`--approval-terminal` and an optional `--approval-timeout` in seconds. The relay
uses the controlling terminal, displays the complete escaped request and requires
its exact ID in the answer. Without a relay, approval-required manifests fail
closed. SDK sessions can attach the shared escalation queue. See
`docs/approval-lifecycle.md`.
Managed admission and broker calls enforce the supported inline effect policy
subset from the complete source. Allowlists must include `claude_code` for
admission. That name is reserved and is not exposed as a child tool. Managed
metadata `human_approval = true` requires an exact admission review; omission
keeps direct operator launches governed by source and Cedar policy. Executable
DSL bodies remain unsupported on this route.

Required signed journals live in private `.symbiont/governed/` storage. Pre-effect
records must sync before execution; outcomes and inference hashes are correlated.
Retain the printed public audit key through a trusted channel for verification.
A signed prefix without a terminal record is incomplete. Ordinary CLI, HTTP and
scheduled ORGA runs also require protected per-invocation storage. DSL `reason()` and
`tool_call()` defaults also use protected run journals. Direct DSL inference,
composition and pattern provider calls require per-call journals, with references
in `:audit`. Their hashes bind typed provider contracts, not arbitrary provider
internals or complete pattern lifetimes. Shell orchestrator and fleet turns also require protected per-turn journals;
fixed file operations use bounded snapshots and retained handles within configured
workspace ceilings. Writes require exact approval and bind prior content; arbitrary
commands use private container scratch without host mounts. Canonical fleet conversations retain source and supported per-agent
sandbox/timeout settings and supported inline allow/deny effect rules. Ordinary
CLI and registered HTTP/scheduler ORGA routes also enforce these rules before
approval on normalized calls. See `docs/inline-policies.md` for the bounded
subset; unsupported rules are refused. Executable canonical DSL bodies still
require a separate interpreter and are refused by fleet loading. Legacy REPL
registration rejects unsupported per-agent tier, sandbox, resource and execution
policy requirements; its supported builtins use the captured project boundary.
SDK loop builders require explicit journal configuration before inference.
See `docs/run-audit.md` and `docs/dsl-invocation-context.md`.

`symbi audit inspect JOURNAL --run-id UUID --public-key HEX` verifies a stable
run snapshot and reports incomplete and unknown outcomes without replay. Audited
tool dispatch has durable start/result checkpoints; error results do not prove
absence of effects. Exit 2 requires reconciliation; exit 1 rejects invalid
evidence. Ordinary CLI ORGA runs now accept `--invocation-id UUID`: the same
request returns its saved result or refuses an unresolved retry without another
execution. Running loops stop before another model request after unconfirmed
tool or child outcomes; governed tool sessions refuse later calls after uncertainty.
HTTP Input also requires an `Idempotency-Key` UUID, bound to authenticated caller,
URI, payload and trusted target. It returns saved results or explicit HTTP 409
in-progress/unresolved/conflict states. Runtime API agent/workflow submissions
now also claim before queue admission.
Cron occurrences and manual triggers also retain durable identities. The default
cron store is project-scoped, and unresolved runs stop later occurrences. DSL
schedule loading preserves identity and terminal state across restart. Other
routes still need persistent identity integration. See `docs/cron-recovery.md`. See `docs/scheduler-idempotency.md`.
See `docs/invocation-idempotency.md` and
`docs/crash-inspection.md`. `symbi invocation inspect|reconcile` binds signed operator
assessments to immutable claim/journal snapshots without granting replay. Cron history
retains a separate Reconciled receipt and requires explicit resume for timers.
See `docs/invocation-reconciliation.md`.

The independent sandbox supervisor owns creation, deadline and descendant cleanup.
Governed workers using the same private supervisor state directory reserve from
one CPU, memory and worker pool. Configure `admission.conf` in that directory;
uncertain leases keep their charge until cleanup is confirmed. Route limits
remain additional bounds. See `docs/shared-budgets.md` for deployment and scope.
Git and declared command/MCP/terminal file snapshots also reserve shared disk
staging before copying. Caller locks and durable worker references retain charges
until private data cleanup. See `docs/staging-capacity.md`.
`--budget-tokens` bounds reserved inference output tokens; input traffic is bounded
by bytes and request count. It is not a total billing limit. `--plugin-dir` is
rejected. See `docs/managed-cli-containment.md` for configuration, evidence and
remaining deployment requirements.

## ToolClad Tools

Tools live in `tools/<name>.clad.toml` and are auto-discovered at startup by `symbi up`, the HTTP Input server, and `symbi tools`. The watcher (`crates/runtime/src/toolclad/watcher.rs`) hot-reloads on file changes — no restart needed.

The manifest carries everything: binary path, description, risk tier, human-approval flag, Cedar `resource`/`action` for policy evaluation, optional evidence-capture config. Cedar policies are auto-generated from manifest metadata via `crates/runtime/src/toolclad/cedar_gen.rs`. The ORGA Gate phase evaluates these before any tool invocation.

Argument types are validated in `crates/runtime/src/toolclad/validator.rs`. `agent_summary` is a best-effort defense-in-depth sanitizer for free text bound for a downstream prompt — **not** a load-bearing control. For a privileged downstream decision (routing, escalation, authorization), use typed `enum` args grounded in trusted context via Cedar, not free text: see `crates/runtime/src/toolclad/decision.rs` (`route_grounded`/`decide_route`), `tools/submit_triage.clad.toml`, and `examples/policies/triage_routing.cedar`. Mark decision-feeding args with `feeds_decision = true`; ToolClad manifest validation (`validate_toolclad`) flags free-text args that feed a privileged decision.

Docker/gVisor/Firecracker command and MCP tools declare individual input and new output paths in
`[filesystem]`; configured mounts are access ceilings. Missing declarations give
these workers no host mounts, and custom parsers inherit no host mounts. See
`docs/filesystem-grants.md` for limits and migration.
The bundled read/list/search tools use an explicit `[source]` runtime broker with
bounded results and no worker source mounts. The bundled Git operations use
private repository snapshots in a supervised worker, with fixed configuration
and no network. See `docs/source-queries.md` and `docs/git-source-queries.md`;
Firecracker read/list/search uses explicit read-only `source_roots` through the
same broker; those roots are never mounted or automatically copied to the guest.
Declared VM command/MCP/PTY files use bounded protocol-5 byte transfer and separate
`output_roots` new-file ceilings. Git uses a separate bounded tree stream sealed
read-only before execution; no original source directory enters the VM. External
Git metadata grants and cross-identity root-managed staging remain outstanding.
Standalone MCP discovery and SDK calls also omit host mounts; SDK integrations
with explicit files use `verified_invoke_with_files`. Schema verification and
the actual invocation share one worker, and output publication requires success
and confirmed cleanup.
Persistent PTYs also receive explicit file grants. A command with `finalize = true`
closes the worker and publishes its new output after confirmed cleanup. Other
commands retain private state and report pending publication. Output declarations
require a finalizer; closing without it returns an unpublished-output error.
Changed file grants require a new run. See `docs/interactive-terminal-boundary.md`.

**Adding a new tool does not require Rust code.** Drop a `.clad.toml` in `tools/`, the runtime picks it up.

**MCP backend (`mcp-client` feature).** A manifest can carry an `[mcp]` block (`server`, `tool`, optional `field_map`) to route the tool to an upstream MCP server over stdio instead of a local binary. Servers are declared in `mcp-config.toml` (per-project, then `~/.symbiont/`). Invocation is SchemaPin-verified fail-closed by default (TOFU key pinning; a post-pin key swap is rejected); `ToolCladExecutor::with_mcp_verification(false)` opts out for local dev. This is how `symbi run` and the DSL `reason()`/`tool_call()` builtins execute real tools — see `docs/mcp-tools.md`.

## Agent Delegation (chat coordinator)

The `symbi up` chat coordinator advertises `delegate` for valid conversational
sources loaded through bounded, confined startup reads. Declared names and
unambiguous filename aliases resolve to one source snapshot and declared policy
principal. Conflicting identities, linked files, unsupported executable sources
and unsupported policies are unavailable. Parent authorization binds the selected
source hashes; child actions enforce the selected inline policy independently of
Cedar. See `docs/coordinator-delegation.md` for selection and migration details.

Bounds and current limits, all worth knowing before relying on it:

- Depth is capped (`max_delegation_depth`, default 3) with cycle detection; both
  guards reject before the target runs.
- Failures are explicit: unknown target, cycle, depth exceeded, policy denial, or
  construction failure each produce a recoverable error observation before the
  target starts. A started child that does not reach `Completed` stops the parent
  with `UnconfirmedEffects`; required parent/child audit failures also terminate it.
- The sub-agent is offered the coordinator's read-only monitoring tools, via
  `CoordinatorExecutor`'s `ActionExecutor::tool_definitions` impl. It is not
  offered `delegate` (no target registry of its own), so nested delegation is not
  reachable from a sub-loop today even though the depth guard allows it. It gets
  **no knowledge bridge**, so no retrieval.
- The sub-loop runs under an id derived from the declared target's name
  (`delegated_agent_id`), so its policy decisions and journal entries are
  attributable and a Cedar policy can name the principal.
- Parent and child inference reserve from one shared token ledger. Child usage
  reduces the remaining parent allowance and is included in parent results.
  Missing usage and cancelled requests retain uncertain charges; a required
  `BudgetUpdated` record precedes termination. Child output requests are capped
  at 4,096 tokens; source timeout and 120 seconds tighten the remaining parent
  authorization deadline. See `docs/shared-budgets.md` for input reservation limits.
- Each child has a protected per-invocation journal linked from the parent's
  required `DelegationStarted` record before child inference. Startup context
  binds the parent principal, call fingerprint and prompt/task hashes. Completed
  children produce a parent `DelegationFinished` record. The chat panel's Inspect run action opens the verified operator view, with
  separate child inspection and parent backlinks. See `docs/run-inspector.md`.
  Parent cancellation cancels retained child owners and awaits their cleanup and
  terminal audit before normal completion. See `docs/run-audit.md`.
- The chat surface cannot run ToolClad/MCP tools: `build_tool_executor` is wired
  into `symbi run` and the DSL builtins, not the coordinator, so a delegated agent
  cannot reach them either.
- Conversion of a `delegate` tool call into a delegation only happens when the
  runner holds a delegation handle. Runners that implement their own `delegate`
  tool (symbi-shell) keep receiving it as a plain tool call.

`delegate` names three different mechanisms across the tree — see the table in
SKILL.md before assuming which guarantees apply.

## Chat-platform responses

Slack, Teams and Mattermost responses in `symbi up` resolve one registered
conversational source and use protected inference, response policy and delivery
audit. Cedar sees the final formatted message and destination at
`context.invocation.resolved.response_delivery`. The existing shared/coordinator
policy surface remains in use. The exact authorized message is sent once;
negative receipts or required audit failures cannot report completion. Runtime
logs identify the audit reference. This text-response route cannot dispatch tools.
The prepared contract also includes the actual HTTP method, canonical URL and
JSON body. Platform HTTP clients refuse redirects and bound response bytes and
request duration. Teams callbacks require signature verification and an exact
match between the signed service URL and the activity destination; development
verification bypasses are refused. SDK adapters must implement `prepare_response`
for governed delivery. Operator network/proxy configuration remains trusted.
See `docs/chat-platform-responses.md` for source selection, bounds and the local
shipping E2E driver.

## MCP Server

Start with `symbi mcp` (stdio transport). Available tools:

- `invoke_agent` — Request a policy-checked text response from a registered
  conversational agent, with required protected audit and cancellation ownership
- `list_agents` — List all agents in the `agents/` directory
- `parse_dsl` — Parse and validate DSL content (file or inline)
- `get_agent_dsl` — Get raw agent definition source (`.symbi` or legacy `.dsl`) for a specific agent
- `get_agents_md` — Read the project's AGENTS.md file
- `verify_schema` — Verify MCP tool schema via SchemaPin (ECDSA P-256)

MCP invocation refuses unknown/ambiguous sources, unsupported executable definitions
and unadvertised tools. `system_prompt` is caller guidance, not runtime authority.
Policies come from shared files and `policies/mcp-server/`; malformed configured
policies deny responses as well as tools. Results expose protected audit references
through `structuredContent`. Project file tools use bounded reads from a pinned
directory, excluding links and private paths. See `docs/mcp-server.md`.

## HTTP API

The runtime API runs on port 8080 (configurable via `--port`):

- `GET /api/v1/health` — Health check (no auth)
- `GET /api/v1/agents` — List agents
- `POST /api/v1/agents` — Create agent
- `POST /api/v1/agents/:id/execute` — Execute agent
- `GET /api/v1/schedules` — List cron schedules
- `POST /api/v1/schedules` — Create schedule
- `GET /api/v1/channels` — List channel adapters
- `POST /api/v1/workflows/execute` — Submit raw workflow source (admin only)
- `GET /api/v1/audit/runs/:agent_id/:run_id?public_key=HEX` — Verified operator run view (admin only)
- `GET /api/v1/sandbox/capacity` — Retained shared worker reservations (admin only)
- `GET /api/v1/sandbox/workers/:lease/usage` — Separately sampled worker CPU/memory (admin only)
- `GET /api/v1/metrics` — Runtime metrics
- `GET /swagger-ui` — Interactive API docs

All endpoints except health require `Authorization: Bearer <token>`.

Workflow submissions can replace a registration and always require administrative
authority. Scoped keys invoke registered source through `/agents/:id/execute`.
Both execution endpoints require an `Idempotency-Key` UUID retained for retries.
Workflow parameters become invocation input; a fresh response reports `queued` and
the actual `execution_id`. Match that ID in agent history for the terminal status.
See `crates/runtime/API_REFERENCE.md` for source selection and migration details.

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
