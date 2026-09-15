# Persistent invocation identities

Ordinary ORGA `symbi run` invocations now print an invocation ID before execution.
To retry a request, pass the same ID, source, input and loop settings:

```bash
symbi run reviewer --input '{"task":"review the change"}' \
  --invocation-id 72d6a833-b825-4b22-b50c-206337d77f7c
```

Choose a fresh UUID for a new intended invocation. Omitting the option generates
a new ID and therefore starts a new task; it is not a retry. Retain the ID when
automating calls so a lost response cannot cause the next attempt to silently
create another task.

IDs are scoped to the canonical project directory and the trusted `cli:orga`
entry point. The request identity binds the selected agent name and source,
literal input and loop configuration. Reusing an ID with a changed request is
refused. Separate projects or scopes have separate identity domains.

## Results and interrupted work

One process creates the durable claim before starting work. It keeps an exclusive
file lock while the invocation is active. Concurrent attempts using that ID exit
with code 2 and report that the invocation is already in progress. The signed
run start binds the invocation ID, scope and request hash; the claimed journal
refuses a second run start.

A finished invocation persists its small CLI result only after verifying its
protected run journal. Retrying returns that saved result and the original audit
reference without contacting a provider or starting a worker. The normal exit
status is preserved: 0 for loop completion, 1 for a recorded terminal failure.

If the process crashes, a result write fails, an effect is unknown, or a claim is
incomplete, the original identity remains claimed. Retrying exits with code 2
and requires reconciliation; it cannot obtain execution authority again. Even a
signed terminal `Completed` does not make an unknown tool effect cacheable.
On the initial attempt, bounded and escaped tool errors remain visible on stderr
alongside the unresolved status and audit reference.
Errors during setup after claiming can also leave an unresolved invocation with
an empty or incomplete journal. These cases are deliberately not retried
automatically. Use [crash inspection](crash-inspection.md) and backend evidence
to establish the original outcome. [Operator reconciliation](invocation-reconciliation.md)
can then persist a separate signed assessment. Repeating a reconciled CLI invocation
returns the receipt with exit 3; HTTP routes return HTTP 409 with `status: "reconciled"`.
Neither response repeats work or manufactures a missing runtime result.

The CLI stores only its displayed output, iteration count, total tokens and
termination reason. Its saved result does not reconstruct a live reasoning loop
or grant access to a previous worker. The audit is verified again before a
cached result is returned. A missing or corrupted original journal fails closed.

## Storage and limits

Claims live in the runtime-owned `.symbiont/invocations/` directory alongside
protected audit storage. The directory is mode 0700 and files are mode 0600.
Claim files refuse symbolic links, hard links, special files, unsafe ownership
and permissions. A store lock serializes creation and result writes across
processes; lock waits are bounded to two seconds.

The store currently permits 4,096 retained files (claims and reconciliation receipts)
and 64 MiB of total claim/result/receipt data,
with at most 2 MiB per file and 1 MiB per request identity or cached result.
Exhaustion refuses work or result delivery without deleting existing identities.
Capacity configuration and an archival policy that preserves duplicate detection
remain outstanding. Preserve claims with their journals. Deleting records,
restoring an older store snapshot or selecting a different project can discard
the history needed to detect a duplicate.

Private claim/result files rely on the runtime's filesystem boundary; the
separate journal supplies signed execution evidence. Workers receive neither
store nor signing-key access. SDK callers of `reasoning::invocation` supply a
trusted scope and must distinguish authenticated callers when using one project.

## HTTP submissions

HTTP Input submissions require an `Idempotency-Key` UUID. The first request binds
it to the authenticated caller, URI, JSON payload and trusted target. Completed
results survive server restarts; concurrent owners and unresolved outcomes return
HTTP 409 without another execution. The owner retains the claim through cleanup
and saving the result, including after disconnection. See
[HTTP retry states](http-input.md#retry-states) for authentication scope, credential
rotation and response details.

## Current scope

Persistent caller-retry handling applies to ordinary CLI ORGA runs and HTTP
Input submissions, runtime API registered-agent/workflow/manual schedule submissions,
and persisted timer occurrences. See [cron recovery](cron-recovery.md).
See [scheduler retry states](scheduler-idempotency.md). Managed CLI agents reject
`--invocation-id` explicitly. Managed sessions and other entry points still need their
corresponding identity integrations. Independently, reasoning loops and governed
tool sessions now stop further execution after unconfirmed tool or child outcomes;
see [stopping uncertain execution](crash-inspection.md#stopping-uncertain-execution).
Interrupted file-publication reconciliation and exactly-once guarantees for
arbitrary external services remain outside this implementation.

The focused unit tests cover concurrent owners, cached results, scope and
payload conflicts, lost outcomes, damaged records, unsafe paths and exhausted
storage. The shipping crash test adds retries during a real worker action,
retries after `SIGKILL` both during execution and after output publication,
result retrieval with provider/Docker access unavailable,
and changed-request refusal, alongside useful permitted work.
