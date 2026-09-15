# Persistent scheduler submissions

Runtime API execution and manual schedule endpoints require exactly one `Idempotency-Key`
header containing a UUID:

- `POST /api/v1/agents/{id}/execute` invokes an authorized registered agent.
- `POST /api/v1/workflows/execute` accepts raw source with administrative authority.
- `POST /api/v1/schedules/{id}/trigger` invokes a persisted schedule with administrative authority.

Generate the UUID once for a new task and retain it with the request. Resend the
same UUID and request when the admission response is lost or when checking the
result. Changing the UUID starts a different task and can repeat effects.
Authentication and per-agent authorization run before claim lookup.

```bash
# Generate once; retain this value for subsequent retries.
INVOCATION_ID=$(cat /proc/sys/kernel/random/uuid)
curl -X POST http://localhost:8080/api/v1/workflows/execute \
  -H "Authorization: Bearer $SYMBIONT_API_TOKEN" \
  -H "Idempotency-Key: $INVOCATION_ID" \
  -H "Content-Type: application/json" \
  -d '{"workflow_id":"agent report() { with sandbox = \"docker\" {} }","parameters":{}}'
```

The claim binds the verified credential and key scope, execution endpoint, JSON
input and complete selected agent configuration. All identified scheduler
submissions share one project-wide ID domain. Another authenticated caller cannot
use an existing ID to retrieve its result or start another execution. Credential,
key-scope, source, agent ID or configuration changes conflict under an old ID.
No token or secret is stored in the claim; its request identity is hashed.

Workflow submissions that omit `agent_id` derive the registration ID from the
invocation UUID, so a retry after restart selects the same identity. Supplying
`agent_id` retains the existing administrative create/replace behavior. A cached
workflow response does not replace a registration again. Registered-agent retries
require the original registration ID and configuration; if startup generated new
IDs, the old route is unavailable until that registration is restored. Submitting
the same invocation ID to a newly registered agent produces a conflict.

## Outcomes

| HTTP | `status` | Meaning |
| --- | --- | --- |
| 200 | `queued` | One new execution was admitted; includes its `execution_id`, `agent_id` and protected `audit`. |
| 200 | `completed` | Returns the verified saved completion, with `replayed: true`; no new execution. |
| 422 | `failed` | Returns a saved terminal failure whose effects are known; no new execution. |
| 409 | `in_progress` | A claim owner still holds queued, executing or finalizing work. |
| 409 | `unresolved` | No verified durable result exists. Inspect the original audit before deciding how to reconcile effects. |
| 409 | `reconciled` | Returns a separate signed operator assessment; the original ID cannot execute again. |
| 409 | `conflict` | The ID belongs to a different caller, endpoint, input or configuration. No original result or audit is exposed. |
| 400 | `invalid_invocation_id` / `invalid_request` | Invalid header or workflow source; no execution was admitted. |
| 403 | `forbidden` | A schedule identity or policy guard refused execution. |
| 404 | `not_found` | The selected registered agent or schedule is unavailable. |
| 503 | `unavailable` | The required execution service, queue or protected claim storage refused admission. A partially created claim can remain unresolved. |

Invocation responses echo `Idempotency-Key`, report `Idempotency-Replayed`, and
set `Cache-Control: no-store`. Saved results include output/error, elapsed time,
reported token usage, shared budget and the original audit reference. Poll using
the same authenticated submission; agent history remains a bounded in-memory
view and may be lost at restart.

The owner survives an unread or disconnected admission response. Cancellation
and scheduler deadlines still stop execution and wait for cleanup. The claim is
held until the final scheduler result is checked against its signed journal and
persisted. Failed setup, cancellation before journal startup, unconfirmed effects,
process death or result-storage failure leave the ID unresolved. A retry cannot
turn those states into a fresh queue entry. A successful output publication followed
by a crash therefore cannot silently cause another publication under that ID.

## SDK and remaining routes

On Unix, `AgentScheduler::schedule_identified_invocation` accepts a trusted
`InvocationIdentity` and returns a queued handle or the existing durable state.
The default governed executor supplies the pinned project root. Custom executors
must explicitly support protected admission; the default trait method refuses it.
The combined request identity and saved result have the existing invocation-store
size limits. See [invocation storage](invocation-idempotency.md).

Ordinary SDK `schedule_invocation` and `CronScheduler::trigger_now` calls still
mean new work. Retry-capable cron callers use `trigger_identified`. Timer
occurrences and their recovery now use durable identities; see
[cron recovery](cron-recovery.md). Other execution routes still need corresponding
identity integrations. This does not establish exactly-once semantics for
arbitrary external services or automatic reconciliation of unknown effects.

`scripts/test-scheduler-invocations.py` exercises both shipping API endpoints,
concurrent admission, unread responses, output publication followed by `SIGKILL`,
restart without a provider and changed caller/source refusals. Protected observers
verify signed journals and useful output outside the worker's file grants.

Operator-reconciled submissions return HTTP 409 with `status: "reconciled"`, the
original audit reference and a separate `resolution` receipt. The original ID
cannot enqueue work again. See [operator reconciliation](invocation-reconciliation.md)
for review, cron history repair and explicit resume.
