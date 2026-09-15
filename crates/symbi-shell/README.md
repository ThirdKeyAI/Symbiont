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

# Inside the shell, attach to a local runtime with its API token:
# /attach http://localhost:8080 --token <runtime-token>
```

### Key bindings

| Key | Action |
|-----|--------|
| `Ctrl+G` / `/gate` | Open Gate panel (held-action approvals) |
| `Enter` | Open the complete selected request in the Gate panel |
| `a` / `d` | Approve / deny the request opened for review |
| `↑/↓`, `Page Up/Down` | Scroll the complete request while reviewing |
| `Esc` | Return from review to queue, then close the Gate panel |
| `↑/↓` | Navigate lists |
| `Tab` | Switch panels |
| `Ctrl+D` | Quit |
| `Ctrl+C` | Cancel an active turn |

## Gate panel

Ctrl+G opens the Gate panel even during a busy turn. Select a pending action and
press Enter to review its complete escaped JSON; scroll through all arguments,
then press `a` or `d`. A list row alone cannot approve. Review follows the exact
request across queue reordering and is invalidated by changes, expiry, removal
or refresh failure.

The panel reports actual resolution outcomes; a timeout means the outcome is
unknown. A configured local escalation queue takes precedence, otherwise the
panel uses the attached runtime's authenticated `/api/v1/approvals` API. Changing
connections discards prior reviews and waits for pending resolution. See
[approval lifecycle](../../docs/approval-lifecycle.md) for limits and the
[branch guide](../../docs/containment-branch-guide.md) for remaining execution gaps.

## See Also

- [`symbi-runtime`](../runtime/README.md) — runtime that holds the escalation queue
- [Getting Started guide](../../docs/getting-started.md) — configuration reference including `[escalation]`
