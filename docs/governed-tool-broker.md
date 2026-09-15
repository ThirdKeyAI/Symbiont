# Governed tool broker API

`reasoning::governed_session::GovernedToolSession` accepts tool calls from an
untrusted client while keeping the principal, trusted context, tool contracts,
policy gate, approvals, deadline and journal in the runtime. The shipping
`symbi run` managed CLI route uses this API with a required signed journal and
protected inference; its configuration and limits are described in [managed CLI containment](managed-cli-containment.md).

The trusted caller supplies an `ActionExecutor`, `ReasoningPolicyGate`,
`JournalWriter`, `LoopState` and bounded `LoopConfig` to `start`. Each
`BrokerToolCall` contains only `call_id`, `name` and object-valued `arguments`.
Extra identity, approval, policy or sandbox fields are rejected. The advertised
tools must exactly match the executor registry or an explicit subset of it.
Registry changes invalidate the session.

Calls are serialized through `GovernedToolDispatcher`: prepare and normalize,
obtain mandatory exact approval, evaluate policy, append the required
pre-effect record, dispatch the authorized call, and record its outcome.
Every identity is single-use within the session, including denied calls.
Denials consume the attempt budget. Invalid requests, policy denials and tool
errors do not grant permission to other calls. An interrupted dispatch or
journal failure cancels the session; later requests cannot resume it.

`close` stops new work, waits for backend cleanup and appends a terminal record.
Expiration, interruption and cleanup failure cannot report successful session
completion. Callers must await `close` and propagate its error. Dropping a
session signals cancellation but cannot synchronously guarantee a final journal
write. The supplied writer determines durability, signing and storage
protection. This API requires journal writes but does not make an in-memory
writer durable or introduce a protected signed default.

## Private MCP transport

On Unix, with `cli-executor`, `cli_executor::broker::McpToolBroker` owns a bounded
MCP listener and a static Python stdio bridge. Its parent directory must be
private and owned by the runtime user. The runtime mounts only the channel
subdirectory read-only into the CLI worker. The socket conveys access to that
specific governed session; no request can select another principal or policy.
Host administrators and other processes with the runtime user's privileges
remain trusted.

`child_boundary` derives the worker profile from the operator-selected Docker
or gVisor configuration, preserves its resource and isolation settings, replaces
source mounts with the private channel, disables direct network access, and
uses a scratch `/workspace`. Tool backends retain their separately authorized
profiles and mounts. Unsupported tiers fail without host fallback. The image
must contain Python 3 and the chosen CLI. gVisor socket compatibility requires
deployment validation; it is not established by Docker tests.

`mcp_config` returns a stdio server configuration for the packaged bridge. The
protocol exposes initialization, ping, tool listing and tool calls. MCP request
identities are namespaced by the server-created connection identity; replaying
a tool request identity on that connection fails. A new connection requires
new authorization and approval for every action. Reconnection does not reset
the session budget. This narrow implementation does not offer resources,
sampling, subscriptions or client-supplied policy configuration.

The transport limits frames to 1 MiB, total session I/O to 64 MiB, concurrent
connections to eight, accepted connections to 64, and frames per connection to
1,024. Session calls are capped at 1,000 and lifetime at one day. Reads, writes,
dispatch and cleanup have deadlines. Journal entries are limited to 1 MiB with
a 64 MiB session allowance, including reserved capacity for a terminal record.
Oversized post-effect output causes failure; it does not roll back the effect.

The owner must keep the broker alive during CLI execution, then close it when
the CLI exits or fails. Dropping the broker signals a cleanup task that stops
connections and closes the governed session. A transport disconnect alone is
not a revocation of an already dispatched call; revocation is controlled by
the session owner and its deadline. Runtime process loss relies on the
independent sandbox supervisor for container cleanup.

## Validation and remaining integration

`crates/runtime/tests/governed_broker.rs` exercises actual ToolClad normalization,
Cedar decisions, operator approval, replay rejection, required journal failure,
budget/deadline handling and owner cancellation. Its explicitly ignored Docker
tests run a contained CLI fixture through the packaged MCP bridge, create an
approved effect through a separate contained backend, and check that the CLI
cannot access the backend's source mount or a synthetic host file directly.
They also cover stalled notification, denial, expiry, cancellation, exact chat
review approval and signed operator identity. Run them explicitly with a matching
sandbox supervisor; an ordinary workspace test run skips them. Chat decisions
use local fixtures, not external platform delivery.

`start_with_inference` additionally owns a fixed-destination inference listener.
The trusted launcher validates its configuration before journaling the spawn,
reads the explicitly named credential, and supplies `ProtectedInference`. The
worker can change message content but cannot change the provider, model,
credential, configured beta headers or output reservation limit. Both channels
share bounded connection ownership and the same cancellation lifecycle.

`crates/runtime/tests/protected_services.rs` covers actual socket transport,
credential/header isolation, canonical request forwarding, output reservation,
redirect and remote-source denial, interrupted requests, storage failure,
concurrent durable writes and signature verification. `ProtectedJournal` is a
Unix per-run writer with a persistent protected key. The managed launcher uses
it; other callers must still choose and secure their own journal.

These tests use synthetic credentials, local scripted inference and controlled
effects. They do not establish whole-system containment, all vendor versions,
gVisor socket compatibility, default audit migration for other routes or a
complete protected escape-evaluation lab.
