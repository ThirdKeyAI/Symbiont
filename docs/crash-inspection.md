# Inspecting interrupted runs

Inspect a saved run with the invocation ID and public audit key retained through
a trusted channel:

```bash
symbi audit inspect /path/to/run.jsonl --run-id UUID --public-key HEX
```

The command emits JSON. Exit code **0** means the journal is complete and its
tracked operations have results; **2** means the authenticated evidence needs
reconciliation; **1** means verification or inspection failed. A successful
inspection is not proof that an external service performed an operation exactly
once. Review `terminal_reason` for the run's result: a valid terminal error is
still a complete journal.

The report verifies Ed25519 signatures, the hash chain, sequence, principal and
expected invocation ID. It reads regular files within the existing 64 MiB journal
and 1 MiB record limits, refuses symbolic links, and rejects files that change
during verification. Stop or copy an active run before relying on its snapshot.
Inspection tracks at most 32,768 operations; exceeding the limit fails explicitly.

A final fragment without a newline is reported as `unverified_tail_bytes` and
always requires reconciliation. That fragment is **not authenticated** and may
reflect interruption or tampering. Every complete record must verify; malformed,
invalidly signed or out-of-sequence complete records fail inspection. The command
does not truncate, repair or append to the original file. Existing `verify` and
`verify_run` library calls remain strict and reject incomplete final records.

## Dispatch and effect evidence

The shared audited dispatcher writes `ToolDispatchStarted` for each authorized
tool before invoking the executor. Its runtime-generated dispatch ID binds the
run, call ID, prepared fingerprint and tool name. Failure to persist a start
prevents execution of the batch. After dispatch, a `ToolDispatchFinished` record
binds a hash of the correlated observation before returning it to the caller.
Cancelled, missing or invalid correlations become unconfirmed error results.
A finish-write failure stops result delivery and leaves the persisted start
unresolved. Explicit SDK calls without a journal do not gain these guarantees.

`effects` classifies each tracked operation as:

- `result_recorded`: the runtime recorded a non-error tool result, complete
  response, confirmed delivery or successful delegation result.
- `unknown`: no finish was recorded, or the returned result was an error or
  unconfirmed response. An error cannot establish that no side effect occurred.

These rows also cover existing ToolClad network exchanges, managed inference,
direct provider calls, parent/child delegation links and response delivery.
Nested network uncertainty is recorded even if the backend claims success. The
runtime turns that observation into an error with `effect_outcome = "unknown"`. Delegation rows contain the child audit reference; inspect
that journal separately with its expected ID. Inspection does not recursively
read paths from journal records.

`journal_complete` requires a signed start and terminal record without an
unverified tail. `requires_reconciliation` additionally reflects unknown effects
and approved tools lacking dispatch lifecycle records. Older journals or calls
that stopped between approval and dispatch can have these gaps. A missing start
is not treated as proof that a previously approved tool never ran.

## Stopping uncertain execution

After an audited tool dispatch, errors, missing results and invalid correlations
are treated as unknown effects. A missing or incomplete nested network receipt
also makes the enclosing call unknown. Backend-supplied outcome metadata cannot
override this runtime decision.

The reasoning loop records the observations and terminates with
`UnconfirmedEffects` before another model request. It skips pending delegations
in that batch. A child that started but did not complete likewise stops its
parent, because repeating the child could repeat earlier effects. Unknown target,
cycle/depth rejection and policy or preparation refusals before dispatch remain
recoverable; they have not started that action.

A governed managed-tool session cancels after an unknown result and refuses later
calls, including calls with new IDs. Required protected journals also refuse new
effect-start records after an unconfirmed outcome. Receipts for work already in
flight, cleanup and terminal evidence remain recordable. SDK applications using
custom executors or journal writers retain responsibility for their contracts.

This is not batch rollback. Work already dispatched concurrently may have taken
effect before an error becomes known; its results and cleanup must still be
accounted for. A fresh invocation ID intentionally creates a new task and does
not prove that repeating its requested operation is safe.

## Recovery limits

This command provides authenticated evidence for operator reconciliation. It
does not resume runs, refund provider reservations, repeat tools or discard
worker leases. A quiet incomplete journal may belong to a live process.

Ordinary CLI, HTTP Input and runtime API agent/workflow runs have [persistent invocation identities](invocation-idempotency.md)
that return saved results or refuse unresolved retries under the same ID. Other
route integrations and recovery of interrupted file publication remain outstanding.
[Operator reconciliation](invocation-reconciliation.md) now records signed decisions
without modifying the original journal or granting another execution. A fresh invocation still has a new
identity and allowance. Establish an uncertain external effect's state through an
authoritative backend receipt or another operation-specific check before choosing
to repeat it. Resuming an interrupted reasoning loop remains unsupported.

`scripts/test-crash-inspection.py` uses the shipping CLI, a local scripted
provider, a real Docker worker and an independently retained audit key. It checks
useful output, kills the runtime while an observed worker action is in progress,
verifies independent worker cleanup and explicit unknown outcomes, and checks
partial, forged and substituted journal evidence without changing originals.
An error after a real worker write must stop further inference and same-ID retries.
