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
  - `./policies/orchestrator.cedar` — the Cedar policy that governs ORCH's tools.
    **This file is required for ORCH's tools to run** (see §5); without it the
    tools fail closed (everything denied) and ORCH can only converse.

The repo root already contains both `agents/` and `policies/orchestrator.cedar`,
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
| `--yes` / `-y` | Pre-approve the orchestrator's artifact save/create prompts. |
| `--allow-shell` | Enable the `shell` tool (arbitrary command execution). **Off by default.** Even when enabled, every `shell` call still requires approval. |

On startup you should see notices like `Loaded N agent(s) from ./agents`, a
policy-load line (`Loaded N orchestrator policy rule(s)`), and — if you have
`.symbi` files in `./agents` — a line noting they are detected but deferred.

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

**`.symbi` agents are intentionally NOT loaded** in this release. They carry
policy/sandbox constraints that aren't enforced yet, so the shell reports them as
*deferred* rather than running them as plain personas. Dropping a `.symbi` file
in `./agents` should produce a "N .symbi agent(s) detected … not loaded" notice,
and the agent should **not** appear in `/agents list`.

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
The gate is a Cedar policy loaded from `policies/orchestrator.cedar`
(deny-by-default). If that file is missing or invalid, the gate falls back to
**fail-closed** — all tools denied, never allow-all. So:

- **Read-only tools** (allowed by the shipped policy, no approval needed):
  - `read_file {path}` — read a project file. Repo-rooted; absolute paths, `..`,
    and symlinks that escape the repo are rejected; large files truncated.
  - `search {query, path?}` — recursive substring search over text files; skips
    `target/`, `.git/`, `node_modules/`, binaries, and symlinked dirs that escape.
- **Mutating tools** (allowed by policy **and** require human approval):
  - `edit_file {path, content}` — create/overwrite a repo file (same path
    confinement as `read_file`; symlinked targets rejected).
  - `shell {command}` — run a command in the repo root. **Disabled unless you
    launched with `--allow-shell`** (when off it isn't advertised, the executor
    refuses it, and the policy doesn't permit it).

### Human-in-the-loop approval (the Gate panel)
When ORCH tries to use `edit_file` or `shell`, the call is **held** pending your
decision:

- Press **`Ctrl+G`** (or type `/gate`) to open the Gate panel.
- Use **↑/↓** to select a held action; **`a`** to approve, **`d`** to deny;
  **`Esc`** to close the panel.
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
| 3 | Drop a `*.symbi` file into `./agents`, `/agents reload` | "…detected … not loaded"; it does **not** appear in `/agents list` |
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
| 14 | Rename/remove `policies/orchestrator.cedar`, relaunch | Notice that tools fail closed; ORCH can converse but tools are denied |

## 7. Troubleshooting

- **"No inference provider configured"** — set `ANTHROPIC_API_KEY` /
  `OPENAI_API_KEY` / `OPENROUTER_API_KEY` and relaunch.
- **ORCH refuses every tool / delegation** — `policies/orchestrator.cedar` is
  missing or failed to load (check the startup notices). Run from the repo root.
- **`shell` "disabled"** — relaunch with `--allow-shell`.
- **Held actions never resolve** — open the Gate panel with `Ctrl+G` and approve/
  deny; otherwise they time out and deny after 120s.
- **`agents:0`** — no manifests in `./agents`; add a `.toml` manifest and
  `/agents reload`.

## 8. Out of scope (not in this preview)

- **`.symbi` agent execution** — detected but deferred (no policy/sandbox
  enforcement yet).
- **Per-agent tool grants** — `tools` in a manifest is recorded but not enforced
  (fleet agents don't call tools yet).
- **OS-level sandboxing** of tools (gVisor/Firecracker) — the gate enforces
  *policy*, not OS isolation. `shell`/`edit_file` run with the shell process's own
  privileges, mitigated by the policy gate, path confinement, and mandatory
  approval.
- Cross-instance / remote session propagation.

Please file findings with the exact steps, the startup notices shown, and whether
you launched with `--allow-shell` / `--yes`.
