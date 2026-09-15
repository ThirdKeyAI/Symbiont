# Containment branch: operator guide

This guide describes `fix/containment-boundary` relative to `main`, through the
runtime changes in `0b30552`. These are branch behaviors, not a claim about a
published release. Complete containment across all entry points remains unfinished.

## What changes for users

| Surface | Earlier behavior | Behavior on this branch |
|---|---|---|
| Ordinary CLI tools | Some command, parser, MCP and terminal paths could execute on the host. | Selected Docker/gVisor workers run commands, parsers, MCP and PTY tools. Missing images, invalid configuration and unavailable backends fail explicitly. |
| Managed CLI | The child could use host credentials, source access, native tools and plugin discovery. | The child gets scratch storage and private inference/tool channels. Registered tools provide allowed source access; credentials and decisions stay in the runtime. |
| Terminal approval | Ordinary and managed runs had no opt-in controlling-terminal approval workflow. | `--approval-terminal` shows a complete escaped request and requires `approve <request-id>`. |
| Shell Gate panel | Approval could be sent from the selected list row; busy turns obstructed review. | Ctrl+G works during a busy turn. Enter opens the complete request; only that reviewed snapshot can be approved or denied. |
| Chat approval | The request ID alone could authorize a pending action. | Review the complete request and copy its ID-plus-digest approval command. Slack requires authenticated callbacks in every environment. |
| Audit and failures | Several default entry points used buffered journals; some completion paths did not establish actual execution. | Covered runs require private signed storage, expose audit references and report execution/cleanup failures. A missing terminal record is incomplete evidence. |
| HTTP and scheduling | Active-agent routing and scheduler bookkeeping could obscure individual invocation outcomes. | HTTP reasoning calls run independently; registration does not execute an agent. Scheduled runs have distinct IDs and actual terminal results. |
| DSL/REPL | Async builtin identity and working-directory changes could affect execution context; declarations were unavailable to behavior execution. | Builtins retain the caller; default `reason()`/`tool_call()` use a frozen project root and protected journals. Successful declarations persist, behavior invocations snapshot their helpers, and the terminal client forwards commands and reports outcomes. Canonical fleet conversations retain selected source, sandbox and timeout settings; supported inline effect rules restrict normalized calls before approval; full canonical behavior and broader policy requirements remain open. |

## Ordinary CLI: prepare the execution environment

The usual `symbi run <agent> -i <input>` entry point remains. Tool execution now
needs a provisioned image with the declared executables and an available selected
backend. Images are not pulled during invocation. Docker is the default; gVisor
requires its configured Docker runtime. Explicit per-agent settings override the
project default. Ambiguous agent selection, conflicting settings and registered
HTTP security-tier mismatches are configuration errors.

Grant tool workers only the input/output mounts they need, and make permitted
output directories writable by the configured numeric user (default
`65534:65534`). Keep source/output data separate from the control project's
policies, agent definitions, tool manifests, credentials and audit storage.
Project-configured mounts cannot expose those protected control paths. Tool
arguments are validated and normalized before policy evaluation; values remain
literal arguments rather than shell syntax.

See [command configuration](toolclad-command-boundary.md),
[selected agent settings](managed-cli-containment.md#selecting-an-agent-boundary),
[MCP setup](mcp-tools.md) and [interactive terminals](interactive-terminal-boundary.md).
Native HTTP is still a runtime broker operation, not a request issued inside the
command container. Production Docker/gVisor profiles reject bridge networking;
use network-disabled workers and governed HTTP tools. Explicit non-production
bridge networking grants unrestricted egress.

## Terminal approvals: review, then type the request ID

For an existing agent and configured tools/policies:

```sh
symbi run reviewer --approval-terminal --approval-timeout 120
```

1. The runtime holds an approval-required call and prints the full escaped JSON
   request on the operator's controlling terminal.
2. Review the tool, normalized arguments, identity, context and expiry.
3. Enter the displayed `approve <request-id>` exactly. Any other answer denies it.
4. Check the eventual tool/run result. An accepted approval still must pass
   policy, receipt and pre-effect audit checks before execution.

The timeout covers notification and waiting, defaults to 120 seconds, and accepts
1–3600 seconds. The run's earlier deadline still wins. Without the flag,
approval-required calls fail closed. A missing controlling terminal fails before
inference; piped stdin cannot approve. Requests above the 64 KiB display limit
are denied by this relay. Unattended jobs need a suitable attached approval
surface or a policy/tool design that does not require interactive approval.

## TUI: a separate complete review

In `symbi shell`, use this sequence while idle or while a turn is busy:

```text
Ctrl+G → ↑/↓ select → Enter review → ↑/↓ or Page Up/Down scroll → a approve / d deny
```

`/gate` also opens the panel when command input is available. Esc returns from
the review to the queue, then closes the panel. Full escaped JSON makes long
arguments and control characters inspectable; pressing `a` on a list row only
asks you to open a review first.

The review stays attached to its request ID when the queue reorders. If the
request changes, expires or disappears, open a fresh review. Malformed queue
responses and refresh failures disable review. The panel keeps resolution pending
until it receives the result and reports errors. A timeout means **outcome unknown**,
not a confirmed denial. Changing connections discards old reviews and waits for
any pending resolution.

A configured local queue takes precedence; otherwise the panel uses the attached
runtime's authenticated API. Local orchestrator and fleet turns now require protected
per-turn journals. Their fixed file tools use bounded snapshots and retained
handles within explicit workspace ceilings; commands use private Docker/gVisor
scratch space. `save_artifact` also requires exact approval,
including with `--yes`. See [shell workspace configuration](shell-containment.md).
Ordinary CLI, registered HTTP/scheduler runs and canonical fleet turns enforce
[supported inline effect rules](inline-policies.md). Other execution paths and
full canonical DSL behavior/policy execution remain in progress.

## Chat: use the command attached to the review

In a configured approval channel, an allowlisted operator can use:

```text
/symbi gate show <id>
/symbi gate approve <id> <review-digest>
/symbi gate deny <id>
```

Notifications and `show` return the same complete escaped request and decision
commands. Copy the approval command from that review. An ID alone, wrong digest,
extra arguments or expired request cannot approve it. Denial needs no prior
review. The 8 KiB message budget includes JSON and commands; an oversized request
requires another attached review surface, such as the Gate panel or authenticated
API. It cannot be approved in chat.

The digest binds the exact request and expiry, but does not prove a person read
it. The configured platform/channel sender allowlist remains the authority. Signed
resolution evidence includes platform, workspace, channel and sender identity.
Broader workspace/platform trust review remains open.

For Slack, configure a nonempty app signing secret before startup. Both callback
routes require a valid signature over the exact body and timestamp. The former
`SYMBIONT_SLACK_ALLOW_UNSIGNED` override no longer works. `gate` slash commands
route to approval control rather than agent invocation. See
[approval lifecycle](approval-lifecycle.md) and
[approval channel configuration](getting-started.md#human-in-the-loop-approvals).

## Managed CLI: source access moves to registered tools

The `executor = "claude_code"` path launches its child in the selected Docker,
gVisor or Firecracker worker. Containers use scratch `/workspace`; VMs use `/tmp`
and two private vsock broker capabilities. The child sees registered tools such as `read_file`,
`list_files`, `grep_files` and the Git inspection tools; it has no direct source
mount or external network connection. Each requested tool action passes through
runtime validation, policy, mandatory approval and signed audit.

Operators must provision the CLI/Python image, explicit backend source mounts,
Cedar policies, exact registered `allowed_tools`, and `[managed_cli.inference]`
provider/model/credential-variable settings. `--target` must map to a configured
backend mount for Docker/gVisor. With Firecracker, provision a matching CLI/Python
rootfs and give `--target` an absolute guest path (default: configured guest
working directory). Host repositories are not uploaded, VM scratch does not
persist, and readable rootfs files are shared between the CLI and backend images.
Host login state is not passed to the child. Native built-in tools,
automatic discovery and plugins are disabled; `--plugin-dir` is rejected.

`--approval-terminal` works here too. `--max-turns` bounds conversation length,
`--budget-timeout` bounds wall-clock lifetime, and `--budget-tokens` reserves
requested inference output tokens. It is not a total billing limit. Inference
responses, including SSE, are buffered until complete, so streamed output can
arrive later than before. Use the [managed CLI guide](managed-cli-containment.md)
for exact configuration and limits.

## Results, audit and automation

Ordinary CLI runs print a run ID, private journal path and public key before
inference. Successful HTTP reasoning responses and default DSL `reason()` and
`tool_call()` results expose `audit` with `run_id`, `path` and `public_key`.
Scheduled history includes invocation results and audit references; managed CLI
sessions retain their own signed journals.

Direct DSL inference, composition and pattern calls preserve their existing return
types. Their per-call public references appear in `:audit`, which shows the latest
256 entries and an omitted count. Missing required storage now prevents these
provider calls too. Communication needs a configured policy gate and keeps the
authorized recipient snapshot even when its registered name is replaced.
`delegate` requires registration; `send_to` acknowledges durable startup and records
completion later; `race` waits for the first success and cancels remaining calls.
The explicit threaded conversation API does not add history to communication queues.

Protected journals live under `.symbiont/governed/`. Missing or unsafe storage,
required write failure and failed cleanup prevent a durable successful run result.
A reasoning loop stops with `UnconfirmedEffects` after a dispatched tool error,
unconfirmed nested network outcome or started child that does not complete. It
does not ask the model for another action. Governed tool sessions also refuse
later call IDs. A policy refusal before dispatch remains recoverable; inspect
individual outcomes rather than treating `Completed` as proof that every proposed
action ran. A terminal write can fail after an effect has occurred:
an error is not rollback. Preserve the public key through a trusted channel and
verify the expected run and terminal record. See [run audit](run-audit.md).

The CLI now provides `symbi audit inspect JOURNAL --run-id UUID --public-key HEX`.
It reports complete versus incomplete evidence and tracked effects with recorded
versus unknown results. Exit 2 signals reconciliation; exit 1 rejects invalid
evidence. The inspection preserves original journals and performs no replay.
See [crash inspection](crash-inspection.md) for remaining retry limitations.

File publication now records a signed intent before the atomic rename. For an
interrupted invocation, `symbi invocation file-inspect|file-recover` can inspect
and finish the exact retained candidate or acknowledge its already published
inode. It refuses changed files and competing targets, preserves original
evidence and keeps the overall invocation unresolved. See
[file publication recovery](file-publication-recovery.md).

Ordinary CLI runs also print an invocation ID. Reuse it with `--invocation-id`
and the same request to retrieve a saved result or receive an explicit
in-progress/unresolved outcome without another execution. Omitting the option
starts a new invocation. HTTP Input requires a caller-bound `Idempotency-Key`
UUID and returns explicit saved, active, unresolved or conflicting outcomes.
Runtime API agent/workflow submissions also require the header and claim before
queue admission; see [scheduler retry states](scheduler-idempotency.md). Manual
schedule triggers require the same header. Timers persist their occurrence and
clock atomically; restart recovery resumes unstarted intents or repairs history
from a verified saved result. Unknown outcomes disable the schedule and block
resume. The default cron store is now project-scoped, and DSL schedules preserve
identity across restart. See [cron recovery and migration](cron-recovery.md).
The operator can now use `symbi invocation inspect|reconcile` to record a signed
assessment bound to exact claim/journal snapshots. The original ID stays closed
to execution. Cron history shows `Reconciled` with a separate receipt and preserves
the original error; the job remains paused until explicit resume. The shell
displays the operator assessment on retries. See
[operator reconciliation](invocation-reconciliation.md). Other entry points still
need their caller identity integration.

HTTP reasoning invocations run independently even if the same agent is already
active. Registered CPU, memory and time limits tighten each invocation's selected
profile and inference deadline. Governed workers also reserve from the shared
supervisor capacity pool; see [shared budgets](shared-budgets.md) for its scope,
configuration and cleanup-aware release.

The operations console now includes **Worker capacity** alongside **Run Inspector**.
Refresh it to see the selected supervisor pool's reservations, remaining capacity
and retained leases. Sample a worker for separately timestamped CPU/memory usage.
Missing service or measurement data produces an explicit unavailable state;
unknown charges never appear as zero. See [worker capacity](worker-capacity.md)
for units, authentication, backend scope and upgrade instructions.

Registering an agent stores configuration without executing it. API execution
returns a distinct queued invocation ID; manual and timer triggers share governed
execution. History becomes `succeeded` only after execution and cleanup succeed.
Inspect `execution` for output, error, duration and audit information. See
[scheduled execution](scheduled-execution.md) for supported agents and limits.

DSL builtin callers retain their identity through nested and overlapping calls;
creating an agent does not adopt its permissions. The bridge freezes its project
directory at construction. SDK applications can select it with `with_project_root`
before sharing the bridge. Later process-directory changes do not retarget tools
or journals. Explicitly injected SDK executors/journals remain caller-owned.
The interactive REPL now executes registered behaviors and nested helpers through
its terminal and RPC surfaces. Failed registrations leave existing definitions
intact; running behaviors keep their captured definitions. Canonical per-agent
source and sandbox selection use the separate canonical route. Legacy REPL
registration refuses per-agent security tiers, sandbox modes, resource limits and
execution policies it cannot enforce. A rejected module retains existing agents
and helpers. See
[DSL invocation context](dsl-invocation-context.md).

## Architecture and remaining boundaries

The branch retains ORGA, ToolClad, Cedar, SchemaPin and AgentPin. It adds an
immutable prepared call binding the validated arguments, principal, contract,
selected sandbox, context and deadline through approval, required audit and
single-use dispatch. MCP discovery, signature verification and invocation share
one contained session. An independent sandbox supervisor records durable leases
before creation and owns worker cleanup after runtime loss.

These controls still rely on trusted host configuration, images, the container
runtime and a responsive Docker daemon. Review [prepared calls](prepared-calls.md),
[Docker ownership and recovery](docker-containment.md) and the
[governed broker](governed-tool-broker.md) for the precise contracts.

The SDK loop builder requires an explicit journal and refuses omission before
inference. Direct DSL inference, coordinator/delegated turns, chat-platform
delivery and shell turns have protected audit as described in
[run audit](run-audit.md). Raw-source workflow submission requires an admin key;
its parameters reach actual execution, and its response reports a queued invocation
ID that can be matched to terminal history.

Firecracker oneshot commands and parsers, MCP stdio, PTY sessions and managed CLI
workers use the [guest transport](firecracker-setup.md). The opt-in
[managed host service](firecracker-host-service.md) adds jailer provisioning,
dedicated identities, host cgroups and independent watchdog cleanup. Its privileged
deployment and outage E2E remains pending. Declared VM command/MCP/PTY files
use bounded byte transfer with verified new-file publication. Git queries use a
separate sealed snapshot stream. External Git metadata grants and staging across
the managed service identity remain integration work. Public browser execution remains unavailable.
Automatic replay of uncertain effects is outside these contracts. The legacy approval-relay package is
separate from the shipping exact-call approval queue.

Docker/gVisor/Firecracker ToolClad command and MCP calls use [per-operation file grants](filesystem-grants.md):
configured host roots are ceilings, declared inputs become read-only snapshots and a
new output is published after successful cleanup. Their parsers receive no host
mounts. Standalone MCP discovery also receives no host mounts. PTYs retain
explicit file snapshots and private scratch state across commands; a finalizing
command closes the worker before publishing output. Shell and fleet fixed file
tools use a checked broker; their general commands receive no host mounts.
Remaining runtime/VM file capabilities still need review. Production executable workers have no network; native
HTTP goes through the bounded runtime broker with destination/DNS checks and no
automatic redirects. Operator-supplied inference endpoints, images, project
configuration and injected SDK implementations remain trusted.

Containment evaluation must retain protected observers, actual effects, normalized
payloads, build/policy provenance and positive task-completion checks. Deterministic
matched laboratory trials establish their individual scenarios; they do not supply
a model escape rate. Independent held-out discovery is additional validation.
Missing backends and incomplete journals are never successful execution evidence.
