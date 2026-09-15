# Protected run audit

Ordinary `symbi run`, HTTP reasoning requests and scheduled ORGA invocations require
private signed audit storage in the configured project's `.symbiont/governed/`
directory. The CLI prints its run ID, journal path and public key before inference.
Successful HTTP responses carry the same fields in `audit`. Initialization or
required append failures prevent further effects. CLI failures return a nonzero
exit code; HTTP failures use the configured error response.

The SDK `ReasoningLoopRunner::builder()` also requires explicit journal
configuration. Omitting `.journal(...)` returns a required-journal error at startup,
before inference or action dispatch. It no longer silently chooses an in-memory
buffer. Open a new journal for each invocation using
`reasoning::run_audit::open_run_journal(trusted_project, agent_id)`, retain its
public audit reference, pass the writer to `.journal(...)`, and run with that same
agent ID. See the [reasoning guide](reasoning-loop.md#minimal-example).

Explicit custom writers remain trusted embedding configuration. `BufferedJournal`
is available for controlled tests, evaluation and display; it does not provide
durability or protected evidence. The builder cannot infer a safe project directory
or turn an arbitrary caller-supplied writer into protected storage.

The shipping MCP `invoke_agent` route also requires protected per-invocation
storage. It binds the registered source, persists typed inference request/response
hashes, and authorizes the exact text response before releasing it. Its MCP result
includes an `audit` reference in `structuredContent`; cancellation retains terminal
audit ownership. It does not dispatch tools. See [MCP server](mcp-server.md) for
source restrictions, policy setup, file confinement and the shipping test.

Chat-platform responses started by `symbi up` use the same protected run through
actual delivery. The policy decision binds the formatted message, destination,
and actual HTTP method, canonical URL and JSON body without credentials;
`ResponseDeliveryStarted` persists before send, and `ResponseDeliveryFinished`
records the adapter receipt before completion. Negative or unconfirmed receipts
and required write failures return errors. A missing receipt after a send start
does not establish whether the platform accepted it. See
[chat-platform responses](chat-platform-responses.md).

Each HTTP request executes independently, including when another invocation of
its agent is active. Registered CPU, memory and execution-time limits tighten the
selected container profile; the registered execution deadline also bounds inference.
These per-invocation limits combine with shared worker admission: HTTP, scheduler
and other governed worker routes using one supervisor state directory reserve
from its common worker, CPU and memory pool. See [shared budgets](shared-budgets.md)
for configuration, scope and cleanup accounting.
A retained run owner handles cancellation, worker cleanup
and the terminal journal write if the request future is dropped. External-agent
transports and managed CLI execution remain unavailable on this HTTP route.

Coordinator Chat (`symbi up`, `/ws/chat`) also opens a protected journal for each
root turn, under the trusted project directory captured at startup. Before inference,
the server sends `AuditOpened` with `request_id` and `audit: { run_id, path, public_key }`.
The reference identifies storage; completion still requires a verified terminal record.
An unsafe or unavailable directory returns `AUDIT_UNAVAILABLE` without inference.
Required append failures and unsuccessful loop termination return an explicit
error or `INVOCATION_UNRESOLVED` without a successful final chat chunk. Live progress events are best effort and
are emitted only after their required journal writes succeed.

The socket reader remains active during inference. Disconnecting cancels the active
turn and retains the run owner until cleanup and terminal audit finish. Active
and queued messages retain durable claims; discarding queued input never makes
its ID executable again. Each connection allows one active turn and one queued message;
excess messages return `SESSION_BUSY`. Chat content is limited to 64 KiB, frames
and messages to 128 KiB, with a bounded conversation history. Oversized chat content
returns `MESSAGE_TOO_LARGE`; exhausted history returns `SESSION_LIMIT` and requires
a new session. A stalled socket writer also cancels the connection.

Clients retain a non-nil UUID in `ChatSend.id`; the admission receipt and all
events use it as `request_id`. `ChatInspect` performs read-only outcome lookup.
Cached final replies are marked `replayed` and do not reconstruct conversation
context. See [chat recovery](chat-recovery.md) for restart behavior and migration.

This fleet-wide chat surface requires an unscoped operator API key. Keys restricted
to selected agents receive HTTP 401 during the upgrade. The chat panel displays `AuditOpened` references during the turn and alongside
its final response or correlated error. Select **Inspect run** to verify the journal
and follow parent/child references. The runtime also logs them. See
[operator run inspection](run-inspector.md).

Delegated coordinator turns have separate protected journals under their stable
target principals, with a fresh invocation ID for each call. The parent must persist
`DelegationStarted` before child inference. It binds the exact call ID, approved
fingerprint, target, child audit reference and hashes of the system prompt and task.
The child's startup context and prepared-call bindings retain the same link plus
the parent principal and, when provided by its journal writer, the parent audit
reference. Verify both journals with the pinned project key and their
expected run IDs. Shipping coordinator targets now retain canonical source and
declaration hashes in the parent's prepared `delegation_target` and the child's
`agent_definition`. Child `source_policy` records identify the enforced inline
rules. Aliases share the declared principal. Prompt hashes alone do not prove
enforcement of arbitrary DSL rules. See [coordinator delegation](coordinator-delegation.md).

After a child has durably terminated, the parent records `DelegationFinished` with
the child outcome and output hash. Missing child storage, failed parent links and
required child append failures stop the parent run; none can become a successful
delegation result. Disconnect or timeout cancels retained child owners. The parent
waits for their cleanup and terminal writes before normal completion; a failed or
timed-out cleanup acknowledgement produces an error. Cancellation can leave a start
link without a finish record, so inspect the child journal's terminal outcome.
The existing chat panel does not stream child internal events or display child audit
references. Follow them from the parent's protected journal.

`SubLoopDelegationExecutor::new_protected` requires an authorized parent call and
journal. Its direct `delegate()` entry point and unaudited typestate dispatch refuse
execution. The SDK's existing constructor with an injected writer remains caller-owned.
Parent and child inference share a token ledger. A required `BudgetUpdated` record
precedes the terminal record and includes confirmed descendant usage, outstanding
reservations and uncertain charges. Parent terminal usage includes descendants,
including completed inference before cancellation. See [shared budgets](shared-budgets.md).

The focused shipping check uses the actual `symbi up` process, a local scripted
inference endpoint and an independently pinned temporary signing key. It verifies
a real runtime health response, durable startup before inference, storage refusal,
and a signed cancellation record after disconnect:

```bash
python3 scripts/test-coordinator-audit.py --binary target/debug/symbi --report /tmp/coordinator-audit.json
python3 scripts/test-coordinator-audit.py --delegate --binary target/debug/symbi --report /tmp/delegation-audit.json
```

Journals use a private directory (`0700`), exclusive files (`0600`) and a persistent
project signing key (`0600`). Workers cannot receive mounts that expose the protected
project paths. The key is separate from AgentPin's ES256 identity keys. Preserve the
public key through a trusted channel for independent verification.

Version-one signed payloads retain their existing format for legacy single-session
writers. Version-two payloads add `run_id`, covered by every Ed25519 signature.
`ProtectedJournal::verify` accepts both formats and checks their chain, sequence,
principal and internal run consistency. `verify_run` additionally requires the
expected invocation ID; legacy records cannot establish that binding. Renaming a
file or substituting another invocation's journal cannot satisfy this check.
Verify the exact embedded payload bytes; reformatting JSON before signature
verification can change the signed data.

A terminal record is necessary for complete execution evidence. Abrupt process
loss can leave an incomplete signed prefix; verification does not infer completion
from it. Normal completion indicates that the reasoning loop finished, and can
include a policy-refusal response. Individual tool outcomes remain in the journal.

Use `symbi audit inspect JOURNAL --run-id UUID --public-key HEX` for read-only
verification and classification of incomplete or unknown outcomes. The shared
audited dispatcher now writes per-tool `ToolDispatchStarted` and
`ToolDispatchFinished` checkpoints. Missing finishes and error results require
reconciliation; a terminal record does not resolve them. See
[crash inspection](crash-inspection.md) for exit codes, trusted key handling,
partial-record semantics and the outstanding persistent retry work.

Ordinary CLI runs print an invocation ID and accept `--invocation-id UUID` for
caller retries. Signed startup context binds its scope and request hash; repeated
requests return saved results or require reconciliation without another execution.
HTTP Input also requires caller-bound `Idempotency-Key` UUIDs and returns the
original audit on saved or unresolved matching requests. Runtime API agent,
workflow and manual schedule submissions also claim before queue admission.
Timer occurrences persist their identity and expose the protected audit through
schedule history; see [cron recovery](cron-recovery.md). Other entry points still
need their corresponding identity integration. See
[invocation idempotency](invocation-idempotency.md).

Governed ToolClad HTTP requests additionally emit `ToolEffect` records through a
channel bound to the current authorized call. `NetworkRequestStarted` must be
acknowledged before connecting. It carries the method, URL, request ID, byte
count and a hash of the built request, including resolved headers and body.
The enclosing record fixes the principal, iteration, execution run and approved
call fingerprint. Headers and bodies are hashed rather than copied into these
records; URLs remain visible as in the prepared call.

`NetworkRequestFinished` records the received HTTP status, body and header hashes,
body bytes and any incomplete outcome. For an incomplete response, the complete
body hash is absent; the byte count covers chunks received by the client. A
missing finish record means unconfirmed completion, not proof that no request
was sent. These hashes describe application data, not TLS packets or headers
added automatically by the HTTP library.

The dispatcher closes this channel when execution returns or is cancelled.
Retaining a worker handle cannot authorize later records. Missing or failed
network receipts make the enclosing dispatch unknown, even if its backend returns
success. After an unconfirmed outcome, the protected writer refuses new effect
starts while allowing receipts, cleanup and terminal records. Reasoning loops
stop with `UnconfirmedEffects` before another inference; managed tool sessions
refuse later calls. Policy refusals before dispatch remain recoverable. A failed
required write stops the dispatch. Each call permits at most 2,048 effect records of
32 KiB each; the shared journal has its own lower total storage budget where
configured. HTTP requests are bounded to 2 MiB of method, URL, declared headers
and body, with a 10 MiB response-body limit. Asynchronous transport obeys the
call deadline and is cancelled with the call.

Use `ReasoningLoopRunner` or `GovernedToolDispatcher` for governed HTTP actions.
The low-level typestate `dispatch_tools()` helper has no journal and cannot
execute a governed HTTP grant. Direct SDK executor calls and injected journal
implementations remain caller-owned. The browser backend is still unavailable;
the same exchange and journal channel are intended for its scoped request broker.

Managed CLI retains its protected session journals. DSL `reason()` and `tool_call()`
defaults also require protected run journals and return their public audit references;
see [DSL invocation context](dsl-invocation-context.md). Explicit SDK journal injection
remains caller-owned.

Direct DSL inference and registered-agent composition now require one journal per
provider call. `DirectInferenceRequested` binds the normalized typed conversation
and options, caller, operation and recipient definition before inference or any
communication enqueue. `DirectInferenceResponseReceived` binds the typed response
before response enqueue. `DirectInferenceFinished` and `Terminated` record the
outcome; required write failures return errors, even after the provider responded.
These typed records are distinct from the managed broker's HTTP wire records.
They do not attest to implementation details inside an injected provider.

The REPL's `:audit` command lists the latest 256 direct inference references, plus
the count omitted from its display. Existing builtin return types stay unchanged.
Background `send_to` acknowledges durable startup; inspect its terminal record for
completion. Caller cancellation keeps a journal owner alive to finish its writes.
Process loss or failed storage can still leave an incomplete prefix. A completed
leaf call does not establish completion of a surrounding pattern or an aggregate
token budget. See the route descriptions in this guide for the protected storage
and cancellation contracts; explicit SDK extensions remain caller-owned.

The shipping smoke test uses a local HTTP fixture and an independently pinned
ephemeral signing key. It checks pre-request durability, preserved response and
audit display, unsafe-storage refusal, tamper rejection, and incomplete evidence
after process termination:

```bash
python3 scripts/test-direct-inference.py --binary target/debug/repl-cli --report /tmp/direct-inference.json
```

Durable records do not
provide automatic replay or recovery, and do not establish complete containment.

Local shell orchestrator and fleet turns now use the same protected storage with
one journal per turn. `/audit` exposes the public references; response metadata
includes the run ID. Required durable writes precede the live display. Caller
cancellation retains the turn owner, and one-shot command cleanup acknowledgements
are awaited before the terminal record. Cleanup failures are recorded as failures.
Provider errors do not trigger automatic replay of the entire turn. See
[governed shell workspace](shell-containment.md) for setup and remaining limits.

Canonical fleet startup records include `execution_context.agent_definition` with
the exact declared name, source/declaration hashes, execution mode and settings.
The same runtime-owned metadata binds prepared policy and approval calls. Older
records omit this context and do not prove a canonical source binding. Raw source
stays in the operator registry; retain its exact bytes to verify the hashes.

Ordinary CLI, registered HTTP/scheduler and canonical fleet startup records also
include `execution_context.source_policy`. Approved calls retain the same source
and declaration hashes, policy names and `inline_effect_policy_v1` semantics in
`source_policy`, bound into their fingerprint. A denied inline rule is recorded
as a policy error before an approval request. These are the
[supported inline effect rules](inline-policies.md); the records do not prove
execution of arbitrary DSL behavior or broader policy requirements.
