# symbi-shell

Interactive TUI shell for the Symbi agent orchestration platform. Provides a full-featured terminal interface for managing agents, sessions, secrets, deployments, and real-time approvals.

## Features

- **Agent orchestration** — start, stop, and monitor agents from a live TUI dashboard
- **Session management** — create, switch, and persist named sessions
- **Secrets store** — encrypted local vault for credentials used by agents
- **Deploy panel** — push agent definitions and policies to a remote runtime
- **Channels** — connect to and configure channel adapters (Slack, Teams, Mattermost)
- **Gate panel (`/gate` or `Ctrl+G`)** — review and approve/deny held agent actions in real time
- **Scheduling** — browse and manage scheduled agent runs
- **Tool registry** — inspect registered ToolClad contracts
- **Syntax-highlighted authoring** — edit `.symbi` agent definitions with inline validation

## Usage

```bash
# Launch the interactive shell
symbi-shell

# Connect to a remote runtime
symbi-shell --endpoint http://localhost:8080
```

### Key bindings

| Key | Action |
|-----|--------|
| `Ctrl+G` / `/gate` | Open Gate panel (held-action approvals) |
| `a` | Approve selected held action (Gate panel) |
| `d` | Deny selected held action (Gate panel) |
| `↑/↓` | Navigate lists |
| `Tab` | Switch panels |
| `q` / `Ctrl+C` | Quit |

## Gate panel

The Gate panel lets operators respond to held agent actions without leaving the shell. When the runtime blocks an action pending human approval (see `SYMBIONT_REQUIRE_APPROVAL_TOOLS` and the `[escalation]` config block), the pending item appears in the Gate panel. Select it, review the action details, and press `a` to approve or `d` to deny. Approvals are sent to the runtime's `/api/v1/approvals` REST endpoint.

## See Also

- [`symbi-runtime`](../runtime/README.md) — runtime that holds the escalation queue
- [Getting Started guide](../../docs/getting-started.md) — configuration reference including `[escalation]`
