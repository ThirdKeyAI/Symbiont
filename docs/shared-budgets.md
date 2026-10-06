# Shared budgets and worker admission

A reasoning run owns a token ledger. Authorized children receive scopes in that
same ledger; creating a child does not create another allowance. Each inference
request reserves its input allowance and maximum output against the child and
every ancestor atomically. Sibling requests cannot reserve the same remaining
tokens. The output limit sent to the provider shrinks to the available balance.
If no output allowance remains, inference stops with `MaxTokens` before a request.

The configured `max_total_tokens` still bounds each loop. A child can impose a
tighter limit, and the shared ancestor balance can reduce it further. Iteration
and depth limits remain in force. One root supports at most 4,096 accounting
scopes, bounding retained metadata even when a caller repeatedly delegates.

Confirmed usage settles a reservation and returns only its unused portion.
Provider errors, cancellation and missing usage retain the full reservation as
an uncertain charge. Inconsistent counters, output above the requested maximum,
or total usage above the reservation close the ancestor budgets and refuse
response-driven actions. No overflow can reopen a budget. A new root invocation
has a new allowance; this is not yet a persistent account-level spending quota.

`LoopResult.total_usage` includes this scope and its descendants. `LoopResult.budget`
also exposes the root identity, scope, limit, available tokens, outstanding
reservations, uncertain tokens and any exceeded accounting contract. Reported
usage and uncertainty remain distinct: an interrupted request is not reported
as known zero-cost inference. Required signed `BudgetUpdated` records precede
termination, after retained child cleanup. Serialized `LoopConfig` cannot supply
or recreate a live ledger; SDK callers pass `shared_budget` as runtime authority.

## Input accounting contract

`InferenceProvider::input_token_reservation` supplies the input allowance. The
default uses serialized UTF-8 request bytes plus 1,024 tokens of framing allowance.
The cloud adapter includes its provider request wrappers and tool schemas; the
SLM adapter includes its rendered prompt. An embedding provider can override the
method with an exact count or a bound appropriate to its actual model contract.

This conservative text estimate is not an exact tokenizer or a guarantee about
external billing. A provider that injects hidden input or ignores output limits
can violate its reservation; the runtime records reported excess and stops
further response-driven work, but cannot undo an external charge. Operators must
use a trustworthy accounting contract for strict spending requirements. Missing
usage consumes the reserved allowance instead of silently refunding it.

## Worker capacity

Docker, gVisor, Firecracker and governed Landlock launches reserve one worker slot plus their
configured memory and CPU allowance in the independent sandbox supervisor.
HTTP, scheduler, CLI and other governed worker routes using the same supervisor
state directory draw from this common pool, in addition to existing route caps.
Reservation and durable registration happen under one lock before worker creation.
Exhausted capacity returns `shared worker capacity exhausted`; admission has no
waiting queue. Inference needed to select a tool can already have occurred before
its worker is refused. This gate does not limit concurrent provider requests or
native HTTP tool calls that do not launch workers. Landlock MCP and low-level SDK
CLI launches require an externally managed delegated service and share this pool.
Their cgroups enforce CPU, memory and PID limits and retain capacity until whole
cgroup removal. See [native worker supervision](landlock-supervision.md).

The private state directory contains `admission.conf`, a JSON configuration read
when the supervisor starts. If absent, it is created with these defaults:

```json
{
  "max_workers": 16,
  "memory_bytes": 8589934592,
  "cpu_nanos": 8000000000
}
```

One CPU is 1,000,000,000 `cpu_nanos`; memory is in bytes. All values must be
positive and `max_workers` cannot exceed 512. The file must be owned by the
service user, mode 0600, and must not be a symbolic or hard link. Invalid or
unsafe configuration refuses service startup. Choose limits for the deployment,
leaving capacity for the host, runtime, Docker daemon and VMM overhead; these
defaults are not hardware discovery or a universal safe host size.

Set `SYMBIONT_SANDBOX_STATE_DIR` to the same absolute private directory in every
runtime process sharing an allocation. Backend profile `supervisor.state_dir`
settings must also match that path; a conflicting profile is refused. A file lock allows one supervisor owner per
directory. Local peer credentials require the same service user by default; the
[managed Firecracker host service](firecracker-host-service.md) authenticates its
configured unprivileged client UID and a root server UID. Separate
directories or users create separate pools, so operators must deliberately
partition capacity if they use them. The pool is not a multi-host quota.

Charges persist in worker lease records across supervisor restart. Cancellation,
creation with an unknown outcome and failed cleanup do not release capacity.
Only confirmed removal returns it. Old records without resource metadata block
new admission until reconciliation establishes cleanup. Inspect and resolve the
underlying cleanup failure; deleting an uncertain lease would discard the charge
without establishing that its worker stopped.

The supervisor checks Docker's actual memory and CPU configuration against the
reservation before returning a created worker to the caller. Zero or higher
limits refuse startup and trigger cleanup. The ordinary Firecracker supervisor
charges configured guest memory and vCPU count. The opt-in managed host service
also charges configured VMM overhead and provisions host cgroups and a jailer;
its privileged deployment validation remains required. Admission accounting
alone does not provision these OS controls.

Drain active work before changing `admission.conf` or replacing the supervisor,
then restart the service. Reducing limits below retained reservations blocks new
work until enough capacity is reconciled. The changed creation protocol requires
a matching runtime/helper build; an older running helper refuses the handshake
instead of accepting unaccounted work. Preserve its durable state during upgrade.

## Validation and remaining work

The focused ledger tests exercise concurrent siblings, nested accounting,
uncertain charges, integer overflow and exhausted allowances. The `shared_budget`
integration target exercises the real protected delegation path, verifies signed
root/child journals, confirms useful completed output and checks cancellation.

`scripts/test-worker-admission.py` exercises actual HTTP, scheduler and separate
CLI ToolClad workers against one pool using the shipping binary, a local scripted provider
and Docker. Supervisor lifecycle tests additionally cover failed cleanup, owner
restart and actual Docker limits exceeding a reservation. These fixtures verify
useful permitted work alongside refusal and protected evidence.

The root's signed reservation history now reconstructs family balances after a
runtime restart. Unsettled provider requests retain their full uncertain charge;
recovery does not resume execution or refund them. Provider-specific reconciliation
and persistent account quotas remain separate work. See
[inference budget recovery](provider-budget-recovery.md). Worker admission does not
replace OS CPU/memory enforcement.

The [operator Run Inspector](run-inspector.md) displays the latest signed token
snapshot with parent/child navigation. It labels recorded balances separately
from live usage. The separate [Worker capacity view](worker-capacity.md) reads
retained pool reservations and samples individual worker CPU/memory usage on
request. Neither inspection path executes or retries work.

Prepared Git and declared file snapshots additionally reserve shared host staging
capacity before copying. Live callers and durable worker references retain their
charges through cancellation and restart. See [staging capacity](staging-capacity.md)
for configuration, supported routes and the separate filesystem-quota limitation.
