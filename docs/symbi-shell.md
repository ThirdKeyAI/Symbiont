# Symbi Shell — Interactive Agent Orchestration

> **Status: Beta.** `symbi shell` is usable day-to-day but the command surface, key bindings, and persistence formats can still change between minor releases. File issues at [thirdkeyai/symbiont](https://github.com/thirdkeyai/symbiont/issues) with the `shell` label.

`symbi shell` is a [ratatui](https://ratatui.rs)-based terminal UI for building, orchestrating, and operating Symbiont agents. It sits on top of the same runtime as `symbi up` and `symbi run`, but exposes it as an interactive session with conversational authoring, live orchestration, and remote attach.

## When to use the shell

| Use case | Command |
|----------|---------|
| Scaffold a project and iterate on agents, tools, and policies with LLM assistance | `symbi shell` |
| Run one agent to completion without an interactive loop | `symbi run <agent> -i <json>` |
| Start the full runtime for webhooks, cron, and chat adapters | `symbi up` |

The shell is the default entry point for authoring. The non-interactive commands are better inside CI, cron jobs, and deployment pipelines.

## Launching

```bash
symbi shell                    # start a fresh session
symbi shell --list-sessions    # show saved sessions and exit
symbi shell --resume <id>      # reopen a session by UUID
```

`--resume` accepts either a UUID or a snapshot name saved previously with `/snapshot`.

## Layout

The shell uses an inline viewport that shares the terminal with your existing scrollback. You'll see, top to bottom:

- **Project-structure sidebar** (toggleable) — file tree of the current project, highlighting agents, policies, and tools.
- **Trace timeline** — ORGA-phase-coloured cards for Observe, Reason, Gate, and Act, streaming in real time during LLM calls.
- **Agent card** — the currently selected agent's metadata, policies, and recent invocations.
- **Input line** — type `/command` or free-form prose. `@mention` pulls in paths and agents via fuzzy completion.

Syntax highlighting covers the Symbiont DSL, Cedar, and ToolClad manifests via tree-sitter grammars.

### Key bindings

| Binding | Action |
|---------|--------|
| `Enter` | Submit input (works even when the completion popup is visible) |
| `/` or `@` | Auto-open the completion popup |
| `↑` / `↓` | Navigate input history or popup entries |
| `Ctrl+R` | Reverse history search |
| `Tab` | Accept the highlighted completion |
| `Esc` | Close the popup / cancel an in-flight LLM call |
| `Ctrl+L` | Clear the visible output buffer |
| `Ctrl+D` | Exit the shell |

Under Zellij, the shell detects the multiplexer and prints an inline-viewport compatibility warning; use `--full-screen` if you want to run in an alternate-screen buffer instead.

## Command catalog

Commands are grouped by purpose. Every command accepts `help` / `--help` / `-h` to print a short usage blurb without dispatching to the orchestrator.

### Authoring

| Command | What it does |
|---------|-------------|
| `/init [profile\|description]` | Scaffold a Symbiont project. Known profile names (`minimal`, `assistant`, `dev-agent`, `multi-agent`) run a deterministic scaffold; any other string is treated as a free-form description the orchestrator uses to pick a profile. |
| `/spawn <description>` | Generate a DSL agent from prose. The result is validated against project constraints before being written to `agents/`. |
| `/policy <requirement>` | Generate a Cedar policy for the described requirement and validate it. |
| `/tool <description>` | Generate a ToolClad `.clad.toml` manifest and validate it. |
| `/behavior <description>` | Generate a reusable DSL behavior block and validate it. |

Authoring commands write to disk only after validation passes. Constraint violations are explained in the trace timeline with line-precise errors.

### Orchestration

| Command | Pattern |
|---------|---------|
| `/run <agent> [input]` | Start or re-run an agent. |
| `/ask <agent> <message>` | Send a message to an agent and wait for the reply. |
| `/send <agent> <message>` | Send a message without waiting for the reply. |
| `/chain <a,b,c> <input>` | Pipe the output of each agent into the next. |
| `/parallel <a,b,c> <input>` | Run agents in parallel with the same input; aggregate results. |
| `/race <a,b,c> <input>` | Run in parallel, first successful reply wins, rest are cancelled. |
| `/debate <a,b,c> <topic>` | Structured multi-agent debate on a topic. |
| `/exec <command>` | Execute a shell command inside the sandboxed dev agent. |

### Operations

| Command | What it does |
|---------|-------------|
| `/agents` | List active agents. |
| `/monitor [agent]` | Stream live status for the given agent (or all of them). |
| `/logs [agent]` | Show recent logs. |
| `/audit [filter]` | Show recent audit-trail entries; filter by agent, decision, or time range. |
| `/doctor` | Diagnose the local runtime environment. |
| `/memory <agent> [query]` | Query an agent's memory. |
| `/debug <agent>` | Inspect an agent's internal state. |
| `/pause`, `/resume-agent`, `/stop`, `/destroy` | Agent lifecycle controls. |

### Tools, skills, and verification

| Command | What it does |
|---------|-------------|
| `/tools [list\|add\|remove]` | Manage ToolClad tools available to agents. |
| `/skills [list\|install\|remove]` | Manage skills available to agents. |
| `/verify <artifact>` | Verify a signed artifact (tool manifest, skill) against its SchemaPin signature. |

### Scheduling

| Command | What it does |
|---------|-------------|
| `/cron list` | List scheduled agent jobs. |
| `/cron add` / `/cron remove` | Create or delete scheduled jobs. |
| `/cron history` | Show recent runs. |

`/cron` works both locally and over a remote attach (see below). See the [Scheduling guide](/scheduling) for the full cron engine.

### Channels

| Command | What it does |
|---------|-------------|
| `/channels` | List registered channel adapters (Slack, Teams, Mattermost). |
| `/connect <channel>` | Register a new channel adapter. |
| `/disconnect <channel>` | Remove an adapter. |

Channel management requires a remote attach when targeting a deployed runtime.

### Secrets

| Command | What it does |
|---------|-------------|
| `/secrets list\|set\|get\|remove` | Manage secrets in the runtime's encrypted local store. |

Secrets are encrypted at rest with `SYMBIONT_MASTER_KEY` and scoped per agent.

### Deployment (Beta)

> **Status: Beta.** The deploy stack is single-agent in the OSS edition. Multi-agent and managed deploys are on the roadmap.

| Command | Target |
|---------|--------|
| `/deploy local` | Docker with a hardened sandbox runner on the local Docker daemon. |
| `/deploy cloudrun` | Google Cloud Run — builds an image, pushes it, and deploys a service. |
| `/deploy aws` | AWS App Runner. |

`/deploy` reads the active agent and project config and produces a reproducible deployment artifact. For multi-agent topologies, deploy the coordinator and each worker separately and wire them with cross-instance messaging (see [Runtime Architecture](/runtime-architecture#cross-instance-agent-messaging)).

### Remote attach

| Command | What it does |
|---------|-------------|
| `/attach <url>` | Attach this shell to a remote runtime over HTTP. |
| `/detach` | Detach from the currently attached runtime. |

Once attached, `/cron`, `/channels`, `/agents`, `/audit`, and most operations commands act on the remote runtime instead of the local one. `/secrets` remains local — remote secrets stay in the remote runtime's store.

### Session management

| Command | What it does |
|---------|-------------|
| `/snapshot [name]` | Save the current session. |
| `/resume <snapshot>` | Restore a saved snapshot. |
| `/export <path>` | Export the conversation transcript to disk. |
| `/new` | Start a new session, discarding the current one. |
| `/compact [limit]` | Compact the conversation history to fit within a token budget. |
| `/context` | Show the current context window and token usage. |

Sessions are stored under `.symbi/sessions/<uuid>/`. The shell auto-triggers compaction when the context grows past the configured budget.

### Session controls

| Command | What it does |
|---------|-------------|
| `/model [name]` | Show or switch the active inference model. |
| `/cost` | Show token and API-cost totals for the session. |
| `/status` | Show runtime and session status. |
| `/dsl` | Toggle between DSL and orchestrator input modes — DSL mode evaluates in process. |
| `/clear` | Clear the visible output buffer (history is preserved). |
| `/quit` / `/exit` | Exit the shell. |
| `/help` | Show the command catalogue. |

## DSL mode

Press `/dsl` to switch the input line into DSL mode. In DSL mode the shell parses and evaluates input against the in-process DSL interpreter with tree-sitter-backed completion and errors, without routing through the orchestrator. Toggle back with `/dsl` again.

## Constraints and validation

Authoring commands enforce a local validation pipeline:

1. Generated artifacts are parsed against the Symbiont DSL grammar, Cedar, or ToolClad as appropriate.
2. A constraint loader checks the result against project-level constraints (e.g. forbidden capabilities, required policies).
3. Only after both steps succeed is the artifact written to disk.

The orchestrator LLM can see the constraint file's effects through validation errors but cannot modify the file itself — this is the same trust model used by the `symbi tools validate` pipeline.

## Beta caveats

The following parts of the shell are still under active development and may change without a deprecation window:

- `/branch` and `/copy` (session branching) are reserved commands and currently print a "planned for a future release" stub.
- `/deploy cloudrun` and `/deploy aws` are single-agent only.
- Snapshot format and `.symbi/sessions/` layout may change between minor releases; use `/export` if you need durable transcripts.
- Fuzzy-completion heuristics and the trace-timeline layout are tuned based on feedback and may shift.

If you need a stable surface today, prefer `symbi up`, `symbi run`, and the [HTTP API](/api-reference) — those are covered by the compatibility guarantees in `SECURITY.md`.

## See also

- [Getting Started](/getting-started) — installation and `symbi init`
- [DSL Guide](/dsl-guide) — agent definition language reference
- [ToolClad](/toolclad) — declarative tool contracts
- [Scheduling](/scheduling) — cron engine and delivery routing
- [Security Model](/security-model) — trust boundaries and policy enforcement
