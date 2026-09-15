# Durable cron occurrences

Timer dispatch records a frozen occurrence, its pending history entry and the
schedule's next clock value in one SQLite transaction. A failed transaction
advances neither the clock nor execution. The occurrence ID derives from the job
ID and scheduled UTC instant. Concurrent schedulers and restart recovery converge
on the same protected scheduler claim before queue admission.

The frozen request includes agent source/configuration, input, schedule policy,
identity credential, session mode and delivery configuration. Each session ID
comes from the occurrence ID. Clock/counter changes do not change retry identity.
Jitter is deterministic for that occurrence and elapsed time is retained across
restart. A new future occurrence receives a different ID.

## Manual triggers

```bash
# Generate once and retain with the request.
INVOCATION_ID=$(cat /proc/sys/kernel/random/uuid)
curl -X POST "http://localhost:8080/api/v1/schedules/$JOB_ID/trigger" \
  -H "Authorization: Bearer $SYMBIONT_API_TOKEN" \
  -H "Idempotency-Key: $INVOCATION_ID"
```

The caller must have administrative authority. The UUID is bound to the verified
caller, job and frozen execution request. Another caller, job, policy or source
cannot reuse it. Responses use the existing [scheduler retry states](scheduler-idempotency.md):
`queued`, verified saved completion/failure, `in_progress`, `unresolved`, or
`conflict`. A missing, malformed or repeated header is refused before intent
creation. A manual trigger uses the job's persisted input.

A service or queue refusal is not proof that no intent was accepted: a prepared
intent may already be durable, or a partial claim may remain unresolved. Retain
and reuse the original UUID. A new UUID represents new work.

Trusted SDK callers use `trigger_identified`. `trigger_now` intentionally creates
a new ID and awaits actual completion and cleanup; calling it again is new work.
Install AgentPin and schedule policy guards using `new_with_guards`, before any
recovery or timer dispatch. Builder setters affect future checks but cannot
retroactively guard work started during construction.

The attached shell supports `/cron run <job-id> [invocation-id]`. It displays the
UUID and an exact retry/status command for both responses and transport errors.
Omitting the UUID starts new work; retain it for retries. `/cron list` displays
complete job IDs and `/cron history <job-id>` includes persisted audit references.

## Recovery and operator outcomes

| Persisted state | Recovery behavior |
| --- | --- |
| Prepared occurrence, no execution claim | Rechecks current execution guards and admits the frozen intent once. |
| Claim still owned by a live process | Leaves it in progress. |
| Verified saved execution, stale schedule history | Repairs history from the original result and audit; performs no new effects. |
| Claim abandoned without a verified result | Records `Unresolved`, disables the job and moves it to `DeadLetter`. |
| Started occurrence whose claim is missing | Records `Unresolved`; does not manufacture a replacement claim. |
| Legacy pending/running history without an occurrence identity | Preserves it as `Unresolved` and disables its job. |

History exposes the occurrence UUID as `run_id`. `admission_audit.run_id` identifies
the protected execution journal; `execution` contains the saved result, usage and
shared budget when available. `Pending` means accepted durable intent, `Running`
means admission began, and `Succeeded` requires completed execution. An admission
audit reference can exist before its journal contains a `Started` record; missing
or incomplete evidence cannot establish success.

Unresolved history prevents timers, manual new work and resume for that job.
Inspect the original signed journal using `symbi audit inspect` and verify external
effects before deciding on remediation. Changing job status or deleting a lease
is not reconciliation. Use [operator reconciliation](invocation-reconciliation.md)
to persist a signed assessment bound to the original evidence. History becomes
`Reconciled` with a separate receipt; the job remains paused for explicit resume.
The original occurrence cannot execute again. Keep the retained occurrence, claim
and journal evidence; automatic replay cannot resolve uncertainty.

Per-job/global cron caps count durable prepared/running occurrences. Recovery
rotates through bounded pages so live owners cannot indefinitely hide later
intents. Shared sandbox capacity remains an additional gate. Retained occurrence
storage is bounded to 4,096 records, 1 MiB per frozen request and 64 MiB of request
bytes. Exhaustion refuses new intents; deleting old identities to free space
would discard retry protection. Archival with durable deduplication is future work.

## Project ownership and migration

The default is now `<execution-project>/.symbiont/cron_jobs.db`. Both its parent
directory and database are private runtime authority (0700 directory, 0600 files
on Unix). Linked files and unsafe SQLite sidecars are refused. The `.symbiont`
path is excluded from governed per-operation filesystem grants.

Each store binds to one canonical execution project. A runtime from another
project refuses to use it. An explicit `CronSchedulerConfig.job_store_path` must
also be private and excluded from all worker mounts. Merely making a directory
0700 does not protect it from a worker granted that directory under the same UID.

Offline `symbi cron` commands use the same project store. `cron add` loads one
bounded regular source file from `agents/`, validates its selected policy and
sandbox, and persists the complete source. Linked or ambiguous sources are
refused. Offline resume also checks the retained unresolved-history barrier.

The former global `$XDG_DATA_HOME/symbi/cron_jobs.db` is left untouched. It is not
automatically imported or executed. To migrate, stop its old scheduler, preserve
the database and journals, inspect which project owns each definition, then
configure an appropriate protected store explicitly. The first explicit binding
claims that database for its selected project; later mismatches are refused.
Existing unfinished legacy history becomes unresolved during migration.

DSL source schedules loaded by `symbi up` use stable source-file/schedule-name
identities. Reopening an unchanged definition preserves its clock, disabled or
terminal state and history. Changed definitions are reported for explicit review;
the persisted definition remains authoritative. Renaming a source file or schedule
creates a new identity and does not retire the old job. Remove or pause superseded
jobs deliberately after inspecting their outcomes.

## Validation and scope

`scripts/test-cron-occurrences.py` uses the shipping HTTP service, a local scripted
provider and actual Docker workers. It checks useful outputs, concurrent manual
retries, timer identity, restart recovery, policy/caller conflicts, and a SIGKILL
after output publication. A second crash occurs during deterministic jitter after
intent persistence; the fixture advances elapsed jitter time before recovery.
A stale-history fixture verifies recovery from a retained signed completion.
Observers and audit keys remain outside worker file grants.

Focused tests also exercise transaction rollback, concurrent SQLite connections,
legacy migration, project ownership, immutable source registration, startup guards
and retained policy refusals. These tests do not establish exactly-once behavior
for arbitrary external services or an adaptive model escape rate. Other execution
routes, interrupted file-publication recovery and provider reservation recovery
still need work. Operator decisions are supported through the separate signed
reconciliation workflow.
