# Agent Orchestration in `symbi-shell` — Tester's Guide

> **Status: Experimental (developer preview).** This guide covers the governed
> agent-orchestration features in `symbi-shell`: loading an agent fleet, talking
> to the orchestrator, addressing agents directly, and the orchestrator's
> governed tools (read / edit / shell) with human-in-the-loop approval. APIs and
> commands may change.

`symbi-shell` is an interactive TUI where you talk to an **orchestrator (ORCH)**
in natural language. ORCH can answer directly, **delegate** sub-tasks to a fleet
of agents you've loaded, and use **governed tools** — every delegation and every
tool call is checked by a policy gate and (for mutating tools) held for your
approval.

---

## 1. Prerequisites

- **An inference provider key** (ORCH needs an LLM). Set one of:
  - `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, or `OPENROUTER_API_KEY`.
  - Without a key the shell still starts, but the orchestrator is disabled (you'll
    see a notice); fleet loading and `/`-commands still work.
- **A working directory containing:**
  - `./agents/` — agent manifests (see §2). The repo ships an example
    `agents/researcher.toml`.
  - `./policies/shell/orchestrator.cedar` — the Cedar policy that governs ORCH's tools.
    **This file is required for ORCH's tools to run** (see §5); without it the
    tools fail closed (everything denied) and ORCH can only converse.

The repo root already contains both `agents/` and `policies/shell/orchestrator.cedar`,
so running from the repo root is the easiest way to test.

## 2. Building and launching

```bash
# from the symbiont repo root
cargo build -p symbi-shell
cargo run  -p symbi-shell            # or: ./target/debug/symbi-shell
# equivalently, via the umbrella binary (args forwarded verbatim):
#   symbi shell
```

Flags (append to either form, e.g. `symbi shell --allow-shell`):

| Flag | Effect |
|------|--------|
| `--yes` / `-y` | Skip conversational artifact confirmation; exact runtime approval remains required. |
| `--allow-shell` | Enable the `shell` tool inside the selected sandbox. **Off by default.** Even when enabled, every `shell` call still requires approval. |

On startup you should see notices like `Loaded N agent(s) from ./agents`, a
policy-load line (`Loaded N orchestrator policy rule(s)`), and — if you have
`.symbi` files in `./agents` — a line confirming they are parsed and loaded
(or refused with a diagnostic for unsupported executable requirements).

The footer shows: model, `agents:N` (loaded fleet size), token count, and the
current addressee (`→ ORCH`).

---

## 3. The agent fleet

Agents are **TOML manifests** in `./agents`. Example (`agents/researcher.toml`):

```toml
name = "researcher"
description = "Finds and summarizes sources on a topic"
system_prompt = """
You are a careful research assistant. Given a topic, return a concise,
well-structured summary with the key points. Do not fabricate sources.
"""
tools = []        # optional; recorded, not yet enforced
```

Commands:

| Command | What it does |
|---------|--------------|
| `/agents list` | Show the loaded fleet (name — description). |
| `/agents load <dir>` | Load additional manifests from a directory. |
| `/agents reload` | Re-scan `./agents`. |

### 3.1. `.symbi` agent loading

`.symbi` agent files in `./agents` are now **parsed and registered** as fleet agents
alongside TOML manifests using the full Symbiont DSL grammar (the same parser that
`symbi run` uses). They are addressable with `@name` and run through the same
governed loop (policy gate → executor → human approval).

**Capability mapping:** An agent's `security.capabilities` declare which tools it can use:

- `read` → tools `read_file`, `search`
- `write` → tool `edit_file`
- `execute` → tool `shell`
- Unknown capabilities grant nothing
- `delegate` stays orchestrator-only (never delegated to fleet agents)

**Sandbox enforcement:** Fixed file operations use bounded, checked capabilities
within the selected `/workspace` access ceilings. General commands run in a
Docker/gVisor worker with private scratch space and no host mounts. Canonical conversational agents
retain their source bytes and exact declared name; supported sandbox and timeout
settings override project defaults. Source hashes bind startup audit, Cedar,
approval and dispatch. See [supported declarations](shell-containment.md).
Supported [inline effect policies](inline-policies.md) restrict normalized calls
before approval. Unsupported behavior statements, policy expressions and executor
requirements are refused. Ambiguous names grant no authority. Reloading removes invalid or
missing definitions and clears cached runners.

**Deferred items:** interpreting canonical behavior and broader policy requirements.
Prompt-only composition cannot enforce canonical contracts and refuses those
recipients; use `@name` for supported canonical ORGA conversations.

## 4. Talking to agents

### Via the orchestrator (default)
Type a request in natural language. ORCH decides whether to answer directly or
**delegate** to a fleet agent:

```
> research the Raft consensus protocol and summarize the key ideas
```

Expect ORCH to call its `delegate` tool, route the task to `researcher`, and fold
the reply into its answer. The delegation appears in the transcript as a tool
call.

### Direct addressing
- **`@<name> <message>`** — a direct, multi-turn conversation with one agent
  (its own thread, separate from ORCH). Example: `@researcher what are the trade-offs?`
- **`/agent use <name>`** — focus the prompt on one agent; subsequent plain
  messages go to it. `/agent clear` (or `/agent use orchestrator`) returns to ORCH.
  `/agent status` reports the current addressee.
- The footer updates to `→ @<name>` in focus mode.
- `/agent clear <name>` clears that agent's conversation thread.

Direct messages are **governed exactly like orchestrator delegation** — they pass
through the communication policy gate and are audited.

## 5. Governed tools (the security model)

ORCH's tools run inside its reasoning loop **only if the policy gate allows them**.
The gate is a Cedar policy loaded from `policies/shell/orchestrator.cedar`
(deny-by-default). If that file is missing or invalid, the gate falls back to
**fail-closed** — all tools denied, never allow-all.

The `shell/` subdirectory is load-bearing. Policies sitting flat in
`policies/*.cedar` are loaded by **every** surface's gate — `symbi run`, the
`symbi up` chat coordinator, the HTTP input server. A `permit` for `edit_file`
written for this shell would reach all of them. Files under
`policies/<surface>/` are read only by the surface they name, so keep
surface-specific grants there and reserve the flat directory for rules that
should genuinely apply everywhere. `symbi init` scaffolds no shell policy at
all; you write this file yourself when you want ORCH's tools to run.

The legacy flat path `policies/orchestrator.cedar` is still honoured if the
scoped one is absent, so existing projects keep working — but it is loaded by
every surface, and moving it into `policies/shell/` is the fix.

So:

- **Read-only tools** (Cedar permission required): `read_file {path}` reads a
  snapshot of one authorized workspace file; `search {query, path?}` performs bounded substring
  search. Paths are relative to `/workspace`; symlinks, multiple hard links and
  special files are refused by the file broker.
- **Mutating tools** (Cedar permission and exact approval required):
  `edit_file {path, content}` writes one authorized file and checks any prior
  content against its prepared snapshot; `save_artifact` validates
  and writes an artifact within a writable ceiling. `shell {command}` executes
  inside the selected worker and is available only with `--allow-shell`.

The host project root is not implicitly mounted. Saved artifacts remain data until
an operator installs them into trusted project configuration. See
[governed shell workspace](shell-containment.md) for permissions, limits and audit.

### Human-in-the-loop approval (the Gate panel)
When ORCH tries to use `edit_file`, `save_artifact` or `shell`, the call is **held** pending your
decision:

- Press **`Ctrl+G`** (or type `/gate`) to open the Gate panel.
- Use **↑/↓** to select a held action, press **Enter** to review its complete
  request, then **`a`** to approve or **`d`** to deny; **`Esc`** closes the panel.
- If you don't decide within the timeout (120s), the action **fails closed**
  (denied).

This local Gate panel drives the orchestrator's **in-process** approval queue —
no separate runtime needs to be attached.

---

## 6. What to test (checklist)

| # | Step | Expected |
|---|------|----------|
| 1 | Launch from repo root with a provider key | Welcome message; `agents:N`>0; policy-load notice; `→ ORCH` in footer |
| 2 | `/agents list` | Lists `researcher` (and any other manifests) |
| 3 | Drop a `*.symbi` file into `./agents`, `/agents reload` | Agent is parsed and registered; it **appears** in `/agents list` (or is refused with a diagnostic for unsupported executable requirements) |
| 4 | Ask ORCH something that needs an agent | ORCH delegates (tool call visible) and returns a folded answer |
| 5 | `@researcher <question>` | Direct reply rendered as that agent; ask a follow-up — it remembers context |
| 6 | `/agent use researcher`, then plain messages, then `/agent clear` | Footer shows `→ @researcher`, plain text routes to it, then back to `→ ORCH` |
| 7 | `@nosuchagent hi` | Recovery error listing the loaded fleet (no crash) |
| 8 | Ask ORCH to read a file (e.g. "show me README.md") | `read_file` runs and returns content (no approval prompt) |
| 9 | Ask ORCH to read `/etc/passwd` or `../something` | Denied — path rejected |
| 10 | Ask ORCH to edit/create a file | Action is **held**; `Ctrl+G` shows it; approve → file written; deny → not written |
| 11 | Don't decide on a held action for 120s | It auto-denies (fail-closed) |
| 12 | Without `--allow-shell`, ask ORCH to run a command | `shell` unavailable / denied |
| 13 | Relaunch with `--allow-shell`, ask ORCH to run e.g. `ls` | Action held for approval; approve → command output returned |
| 14 | Rename/remove `policies/shell/orchestrator.cedar`, relaunch | Notice that tools fail closed; ORCH can converse but tools are denied |

## 7. Troubleshooting

- **"No inference provider configured"** — set `ANTHROPIC_API_KEY` /
  `OPENAI_API_KEY` / `OPENROUTER_API_KEY` and relaunch.
- **ORCH refuses every tool / delegation** — `policies/shell/orchestrator.cedar` is
  missing or failed to load (check the startup notices). Run from the repo root.
- **`shell` "disabled"** — relaunch with `--allow-shell`.
- **Held actions never resolve** — open the Gate panel with `Ctrl+G` and approve/
  deny; otherwise they time out and deny after 120s.
- **`agents:0`** — no manifests in `./agents`; add a `.toml` manifest and
  `/agents reload`.

## 7. Fleet agent tool execution

Fleet agents now execute their manifest `tools` through the same governed reasoning
loop as the orchestrator. A tool is callable only if it is in the agent's manifest
`tools` **and** permitted by `policies/shell/orchestrator.cedar`:

- **Effective tools** = manifest `tools` ∩ `orchestrator.cedar` allowlist.
- **`delegate`** remains orchestrator-only; `shell` still requires `--allow-shell`.
- **No `orchestrator.cedar`** → all tools denied (you'll see a one-time hint on
  first use).
- **Tool-less agents** stay conversational with the no-fabrication guard (they
  cannot execute tools/commands/network and keep an explicit system-prompt
  constraint).

Test by loading an agent with a `tools` manifest list, querying it, and
confirming it either runs the tool (if policy permits) or denies it with a clear
reason. When tools are denied solely because `policies/shell/orchestrator.cedar` is
missing, you'll see a diagnostic hint on startup or first call attempt.

Deferred items (for future releases): per-agent Cedar principals, `.symbi`
agent execution, agent-initiated delegation.

## 8. Out of scope (not in this preview)

- **`.symbi` behavior-step execution** — agents load and are addressable, but
  DSL behavior steps are not yet executed; agents respond conversationally.
- **Agent-initiated delegation** — agents cannot yet delegate to other agents
  directly; the orchestrator can.
- **Deployment and distributed limits** — Docker/gVisor and Firecracker command
  effects use the selected isolated boundary. Workers sharing one supervisor
  directory share its CPU, memory and worker pool; see
  [shared budgets](shared-budgets.md). Host process hardening and distributed
  quotas remain separate deployment work.
- Cross-instance / remote session propagation.

Please file findings with the exact steps, the startup notices shown, and whether
you launched with `--allow-shell` / `--yes`.
