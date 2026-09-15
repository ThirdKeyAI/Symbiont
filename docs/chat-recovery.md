# Durable coordinator messages

Coordinator Chat retains each admitted message in the project's protected
invocation store. The client supplies a non-nil UUID in `ChatSend.id` and keeps
that ID for retries. All replies use that same UUID as `request_id`.

The claim binds the ID to the authenticated caller and exact message content in
scope `ws:chat:v1`. Changing credentials or content cannot create another
execution under the same ID. A new UUID is an explicit request for new work,
even when its text repeats a previous message. The identity refers to the first
accepted message in its original conversation context.

## Admission and recovery

The server reserves queue space, persists the claim and opens its audit before
returning `AuditOpened`. That receipt means admission, not completion: a queued
message can have an empty journal until its reasoning loop starts. Each socket
allows one active message and one queued message. A full queue refuses new
admission; lookup of an existing ID remains available.

The queue and execution owner retain the exclusive claim through cancellation,
worker cleanup, terminal audit and result persistence. Only a verified result
with no unresolved effects is cached. The final `ChatChunk` is sent after that
result is durable. It includes `replayed: false` for fresh work or `true` for a
saved reply. Returning a saved reply invokes no provider or tool and adds no
duplicate messages to the live conversation.

A disconnect cancels active work and drops queued work without making either ID
available for another execution. A hard kill can leave an incomplete journal.
After restart, unfinished IDs remain unresolved, including admitted queued
messages whose execution never began. An absent result never proves absent
effects. The operator can inspect and reconcile them using the existing CLI:

```bash
symbi invocation inspect --project /srv/project --scope ws:chat:v1 --id MESSAGE_UUID
symbi invocation reconcile --project /srv/project --scope ws:chat:v1 \
  --id MESSAGE_UUID --review reviewed-outcome.json
```

Reconciliation records an assessment without replay. See
[operator reconciliation](invocation-reconciliation.md) for evidence requirements.
Queued messages with empty startup journals are inspected through the invocation
CLI; a run-only view cannot invent a signed `Started` record.

## Protocol

Authenticate the `/ws/chat` connection using an administrative API credential.
Scoped keys cannot use the fleet coordinator. Retain the UUID and content before
transport, then send:

```json
{"type":"ChatSend","id":"59e1e64e-4d39-440c-a5b1-577c7172b7d3","content":"Check runtime health"}
```

Send `ChatInspect` with the same `id` and `content` to look up the outcome without
creating a claim or starting work. `ChatSend` with the same fields returns the
existing outcome if admitted, or attempts admission if no claim exists.

| Reply | Meaning |
|---|---|
| `AuditOpened` | Original protected audit reference; not a success assertion. |
| Final `ChatChunk` with `replayed: true` | Saved completed response; no work repeated. |
| `INVOCATION_IN_PROGRESS` | An owner retains the active or queued claim. |
| `INVOCATION_UNRESOLVED` | Original effects or completion need inspection and reconciliation. |
| `INVOCATION_RECONCILED` | An operator assessment exists; the ID remains closed to execution. |
| `INVOCATION_CONFLICT` | Caller or message differs from the original claim. |
| `INVOCATION_NOT_FOUND` | Read-only lookup found no claim and submitted no work. |
| `SESSION_BUSY` | Queue capacity refused this submission before new admission. |
| `INVALID_INVOCATION_ID` | Supply a non-nil UUID; older arbitrary string IDs are refused. |
| `AUDIT_UNAVAILABLE` / `LOOP_ERROR` | Storage or execution failed; do not infer that effects are absent. |

Errors carry the message UUID when valid. Existing outcomes with an available
audit reference send `AuditOpened` before the result or error. Independently
retain the public audit key. Concurrent clients must correlate events by ID;
an admission receipt for a queued message can precede another message's result.

## Console behavior and limits

The console retains one pending UUID and its message text in browser session
storage before sending. It survives a reconnect or reload in that tab and does
not trigger automatic resubmission. **Check outcome** uses read-only lookup.
Only after a missing admission does **Retry same request** attempt admission
under the retained ID. Dismissing the notice changes no runtime evidence.

Conversation history remains local to a connection. Reconnecting starts a fresh
conversation; retrieving a saved response does not reconstruct previous context.
The console labels saved responses and prevents an accidental new submission
while a retained request still needs attention. Closing the tab or clearing
browser storage can lose the client copy, so retain IDs and audit references
separately for longer-term recovery. Message text stays subject to the 64 KiB
limit; protected claims/results retain the existing store size and count limits.

The trusted SDK convenience method `handle_chat` starts new work. Use
`handle_chat_with_id` with a retained UUID for retries; SDK calls share the
project's SDK caller identity. Durable chat admission requires the supported Unix
storage contract. Unsupported platforms refuse execution.

Focused WebSocket tests cover caller/content conflicts, cached replies, bounded
queued claims, storage refusal and cancellation. `scripts/test-chat-invocations.py`
uses the shipping binary and local synthetic inference to exercise real runtime
tool dispatch, process death, restart lookup, queued-message reconciliation and
useful new work. It does not contact an external model service.
