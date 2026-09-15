# Interactive terminal execution

Session tools allocate a real PTY inside the selected Docker/gVisor container or
Firecracker guest.
The `toolclad-session` feature is enabled in the default shipping CLI. SDK builds
must enable it explicitly. Missing transports fail before execution; there is
no host PTY fallback. Hosted and development-host PTY transports remain
unavailable. Browser execution remains unavailable.

For Docker/gVisor, configure the image, mount ceilings, numeric user and resource bounds using the
[ToolClad command profile](toolclad-command-boundary.md). The image must contain
`/bin/sh`, `stty`, the declared executable and its dependencies. Startup is
literal argv parsed from the trusted manifest. Terminal echo is disabled before
startup; model arguments never select startup argv, mounts or environment.
Only `TERM=dumb` is supplied to the terminal by the runtime. The image and
operator configuration remain trusted. Network mode defaults to `none`.

Each `tool.command` accepts a `command` string. Its manifest `pattern` is a Rust
regular expression matching the entire string, not a template or substring.
Control characters, including newlines and terminal escape frames, are rejected.
Other declared arguments and scope constraints are validated before Cedar;
unknown arguments fail. Human approval flags require a bound approval receipt.
The same normalized command, manifest and selected boundary are frozen through
authorization and dispatch. A matching prompt means `prompt_observed`, with
`exit_code: null`; an interactive prompt does not establish a process exit code.

## Session file grants and finalization

Configured mounts are ceilings. A session without `[filesystem]` receives no
host mounts. A session can retain up to 32 bounded input snapshots and one staged
new output across its commands, using the [file broker](filesystem-grants.md):

```toml
[filesystem]
read = ["data/input.csv"]
create = ["data/report.csv"]
max_file_bytes = 8388608

[session.commands.finish]
pattern = "save-report"
description = "Save the report, close the session and publish its output"
finalize = true
```

Here `save-report` is an application command supplied by the operator's terminal
program. It must write `data/report.csv` and return the configured ready prompt.
The runtime then stops the worker, confirms removal, and atomically publishes
the output without overwriting an existing entry. The flag defaults to false;
declaring output requires at least one finalizing command. Such a command has
the same pattern, Cedar, scope and exact approval checks as other commands.
Approval context includes `session_finalizes` and the effective file grants.

Intermediate responses report `file_publication: "pending"` and empty
`created_files`. Finalization reports `execution_status: "session_finalized"`,
`session_closed: true`, `file_publication: "published"` and file receipts. With
no output grant, `file_publication` is `"none"`. These are runtime lifecycle and
publication acknowledgements; application-level correctness still depends on
the tool's result. A ready prompt alone does not certify a database transaction
or the contents of an exported file.

Files remain available to the whole session, including startup and background
processes. They are read-only snapshots and a private output inode, never the
containing host directory. Scratch files and in-memory application state persist
between commands. Each prepared call binds input hashes and the output parent
directory identity. Changing those files, grant paths or destination directory
during a live session refuses the next call; start a new run to use new grants.
Argument references use that command's validated arguments, so commands sharing
a session must resolve to the same file set.

Closing, cancelling or exhausting a session without finalization discards its
staged output and returns an explicit unpublished-output error. A reasoning run
clears its final answer and cannot report completion in that state. Finalization
must occur within the interaction and run limits. If a caller loses its response
after publication, the effect may already exist; retain the invocation identity
and audit evidence instead of repeating the action. Automatic reconciliation of
interrupted publication remains separate work.

An actor owns each live worker. Registry locks never cover I/O. Ownership
includes the trusted principal and run start, context/configuration binding,
manifest and contract digest. Successive iterations can reuse state; another
principal or run cannot address it using model-supplied IDs. Changes to trusted
context or the tool contract select a separate session. Commands execute in
sequence with a bounded admission queue.

Startup, interaction, idle, session lifetime, worker lifetime and run/grant
deadlines all apply. Each interaction may retain at most 4 MiB, and each
transcript at most 4 MiB; the profile also caps cumulative output, including idle
output and attachment stderr. There are at most 16 sessions per executor and
1,024 remembered closed runs until their original deadlines. Exhaustion fails
closed. Reaching an idle/lifetime/interaction bound cannot silently restart the
same session. Prompt matching checks the entire final nonempty line after ANSI
removal. An idle worker cannot extend its deadline by emitting output.

`ReasoningLoopRunner` owns persistent effects across the whole loop. Normal
completion and timeout await worker removal before returning and journaling
termination. A required cleanup failure changes termination to an error and
clears the final answer. Initialization failures also end the run with an error
because they may include a failed removal. Cancellation during creation awaits
its owner before acknowledging cleanup, and cannot start a cancelled terminal.
Dropping the run future signals cancellation, including
while inference is pending between tool calls. Container or VM removal terminates
descendants even if they started new process sessions. Cleanup on future drop
requires a live Tokio runtime. The [independent supervisor](docker-containment.md)
also owns workers after caller process loss. Recovery after supervisor/host loss
requires separately supervised durable state and a responsive Docker daemon;
these controls do not guarantee a hard deadline during daemon outages.

## Firecracker terminals

Select `sandbox.tier = "firecracker"` and provision the matching kernel/rootfs
using [Firecracker setup](firecracker-setup.md). The trusted startup executable
and dependencies must exist inside the guest. Session manifests, command
patterns, Cedar rules, exact approvals and prompt matching use the same interface.
The runtime supplies `TERM=dumb` and the guest's minimal PATH/HOME/LANG defaults.
No host mounts or network interface are exposed; writable scratch remains in
`/tmp` for the session's lifetime.

Protocol version 3 allocates a private devpts terminal inside the guest. The
workload runs as UID/GID 65534, becomes its session leader, and receives the slave
as its controlling foreground terminal on stdin/stdout/stderr. Echo is disabled.
Initial noncanonical input avoids the kernel's canonical-line truncation; the
application may configure its own terminal settings. The initial terminal size
is 80 columns by 24 rows; runtime resizing is not exposed. Terminal stdout/stderr
are merged, and normal terminal output processing may produce CRLF. Ordinary
stdio mode continues to preserve raw bytes and separate stdout/stderr.

The independent VM owner keeps startup and whole-session deadlines separate.
Cancellation during startup awaits removal; dropping the temporary startup
sender after success does not close a healthy session. Finishing or dropping the
session closes the whole VM, including processes in new sessions. PTYs have no
independent write-half close: closing SDK terminal stdin ends the workload,
rather than injecting a guessed terminal control character. Keep stdin open
between commands and close the lifetime guard when done.

`FirecrackerRunner::spawn_terminal` exposes the same bounded stream/guard handles
as stdio, with separate startup and lifetime budgets and an explicit startup
cancellation receiver. Await `guard.finish()` for the transport outcome and
cleanup acknowledgement. `guard.finish_cleanup()` reports removal separately:
a guest exit, output failure or expired VM lifetime can fail the operation even
when the supervisor confirms that removal succeeded. Run cleanup retains that
confirmation, so a run deadline remains a timeout; missing or failed removal
acknowledgement still fails the run.
Rebuild rootfs images from older guest protocols before using these interfaces.

## SDK and DSL lifecycle

Use `SessionExecutor::execute_session_command_async` inside Tokio. For declared
output, invoke its finalizing command before awaiting `cleanup_async`; cleanup
alone does not publish it. The old synchronous method returns an explicit async-required
error after validating input. A direct SDK executor owns one bounded run; create
a new executor for a new run. `cleanup` signals shutdown synchronously, while
`cleanup_async` also waits for acknowledgement. Direct SDK methods cannot satisfy
mandatory approval or bypass scoped-call requirements.

SDK users of `GovernedToolDispatcher` should retain one `ExecutionRunGuard` for
their shared `LoopState` and await its `close` at the end. Dropping it signals
cancellation. `reason()` uses the loop's automatic ownership. An explicit DSL
`tool_call()` is a standalone run and awaits cleanup before returning; use
`reason()` or the governed SDK lifecycle for persistent multi-command sessions.

## Verification

`crates/runtime/tests/pty_sandbox.rs` contains real Docker PTY tests for state,
concurrent principals, Cedar and approval denial, control frames, host canaries,
ambient credentials, denied networking, bounds and cancellation. The DSL builtin
has a real PTY cleanup test. These tests are ignored by ordinary workspace runs
and must be executed explicitly with the fixture image and documented synthetic
environment. The escape harness's `verify_runtime_pty.py` builds the shipping CLI
and checks scripted multi-call execution, external effects and worker removal.
`firecracker_sandbox.rs` also exercises actual guest terminal ownership,
persistent effects, long input, output limits and cancellation with the static
`pty_fixture` example installed in the image.
Unavailable-backend cases prove refusal only. These suites do not establish
containment of all other runtime surfaces or prove a gVisor deployment.

`scripts/test-pty-filesystem.py` runs the shipping CLI with a real terminal,
scripted inference and protected signed journals. It checks useful state reuse,
exact mounts, finalization, missing finalization, changed grants, policy refusal,
timeouts and existing or linked paths. The SDK suite also covers destination
directory replacement and interaction exhaustion. Firecracker host-file grants
remain unavailable; its guest-only terminal lifecycle is unchanged.

Firecracker file sessions use the same declarations and finalizer. The guest
retains an exact output inode and exports it only after descendants stop; the
host verifies the bytes and confirms VM removal before signed publication.
A dropped guard or failed finalization leaves the output unpublished. See
[VM file transfer](filesystem-grants.md#firecracker-file-transfer) for root
configuration, protocol upgrade and current host-service limits.
