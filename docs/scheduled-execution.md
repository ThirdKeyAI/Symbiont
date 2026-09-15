# Governed scheduled execution

Registering an agent stores its configuration without invoking inference or tools.
`AgentScheduler::schedule_invocation` accepts a JSON payload and returns a handle
with a unique run ID, cancellation and an eventual result. `execute_agent` waits
for that result. The older `schedule_agent` method still means queue admission.
Multiple runs of the same agent retain separate identities, results and resource
allocations.

The default execution service runs ORGA against the selected agent's DSL source.
It freezes the project root, selects and validates the agent's sandbox, checks
that its registered security tier agrees, intersects the container's CPU, memory
and time limits with the agent limits, and builds the governed ToolClad executor.
Cedar and mandatory exact approvals apply to individual tool calls. Production
execution has no fallback to host commands. Missing inference providers, invalid
agent selections and unavailable backends return failures.

Scheduled invocations require a protected, synchronously persisted signed journal
in `.symbiont/governed/`. Each invocation has its own file, while repeated runs
share the protected signing key. Completion includes the journal path and public
key. Verification requires a key retained through a trusted channel; a terminal
record is required for complete evidence. See `managed-cli-containment.md` for the
journal format and deployment assumptions.

Cancellation interrupts inference and tool dispatch and waits for worker cleanup.
Timeout includes time spent in the queue. Pending invocations can expire even
when all execution slots are occupied. An execution service that cannot confirm
cleanup returns failure. Normal completion releases the exact resource allocation;
completed jobs do not permanently occupy scheduler capacity. No-backend tool
executors return errors instead of fabricated success.

## Runtime API

Creating or loading an agent registers it. `POST /api/v1/agents/{id}/execute`
requires an `Idempotency-Key` UUID, accepts an optional `input` JSON value and
returns a new execution ID with status `queued`, or its existing durable state.
Workflow submissions use the same claim-before-queue path. See
[scheduler retry states](scheduler-idempotency.md). Agent status reflects running and terminal execution states, and its history records
the terminal status under the same invocation ID.
Updating registration changes future invocations; an already admitted invocation
retains its source snapshot.

Schedule creation resolves exactly one registered agent by name and accepts an
optional `input` value. Manual triggers and timer triggers use the same execution
path and concurrency controls. Manual triggers require an administrative token
and an `Idempotency-Key` UUID retained for retries. Durable run history starts as
`Pending`, becomes `Running` at protected admission, and reaches `Succeeded` only
after successful execution and cleanup. The history's `execution`
field contains the runtime run ID, output, error, duration and audit reference.
One-shot timer jobs are marked completed after execution, and failed one-shot jobs enter
the dead-letter state. SQLite migration preserves existing jobs and history and
supplies null input for older job definitions. Legacy unfinished runs become
`Unresolved` and disable their jobs. Occurrence intent and clock advancement
commit atomically; recovery resumes unstarted intents and reconciles completed
results without replay. Unresolved effects prevent resume or new execution of
that job. See [cron recovery](cron-recovery.md) for project-scoped storage, stable
DSL schedule identities and migration.

`symbi up` shares its runtime scheduler with cron. AgentPin verification and
schedule policy gates run before invocation; the runtime then independently
authorizes tool effects. Runtime AgentPin verification is installed before cron
recovery begins. SDK callers use `new_with_guards` to install startup guards; later
builder setters affect future checks and cannot retroactively protect startup.

## Current limits

The default scheduled execution service supports ORGA agents using the configured
cloud-compatible inference provider, including operator-configured local fixture
endpoints. Routed model selections, managed CLI scheduling, external-agent
transport remain explicit errors. Firecracker workers use the governed guest
transport; host-file capabilities are still limited. See [Firecracker setup](firecracker-setup.md). Trusted SDK
embedders can supply a `ScheduledAgentExecutor`.

CPU, memory and execution time are enforced at the selected container boundary;
disk/network bandwidth fields do not establish enforced rate limits. Docker
isolation retains the daemon, image, runtime and operator-configured mounts as
trusted components. The current tests establish the exercised Docker paths;
they do not establish Firecracker, gVisor or complete escape resistance.
