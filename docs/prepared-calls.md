# Prepared calls and exact-call approval

The ORGA policy phase and shared explicit-tool dispatcher prepare each invocation before evaluating it. ToolClad
applies argument validation, defaults, normalization, custom type resolution,
and scope checks at that point. Unknown tools and unexpected arguments fail
before effects. Custom scope aliases preserve their scope requirement, and
cyclic aliases fail instead of recursing indefinitely.

A prepared call contains the normalized action, contract identity, and resolved
effect description. ToolClad freezes its manifest, rendered argv, MCP registry,
and verification mode for dispatch. Generated argv values participate in the
call fingerprint. The actual HTTP URL and mapped MCP arguments are available
for policy evaluation. Credentials embedded in HTTP URLs are refused; header
and body secret placeholders remain references in the contract.

Policy approval produces an `AuthorizedAction` which cannot be cloned or
reconstructed from JSON. Dispatch consumes it once and checks the original
principal, session start, iteration, trusted context, configuration, and
expiry. ToolClad additionally checks its executor instance identity. Replacing
or reconfiguring the executor cannot transfer an existing grant to another
backend. A policy modification is a new proposal: it must be prepared and
explicitly allowed again. Modifications cannot change the originating call ID or action kind. Duplicate or empty call IDs are rejected before dispatch.

## Cedar requests

For a manifest containing:

```toml
[tool.cedar]
resource = "Tool::Fetch"
action = "execute"
```

The runtime uses `Tool::Fetch::Action::"execute"` and a `Tool::Fetch` resource
identified by the invoked tool name. This matches ToolClad's generated Cedar
policies. Resource attributes include `tool_name`, `version`, `contract_hash`,
and `call_fingerprint`. `context.invocation` contains normalized arguments,
contract identity, the resolved effect description, and the fingerprint.
For example, a policy may restrict `context.invocation.resolved.http_url`.
ToolClad's advertised arguments are strings; integer validation can normalize
`"999"` to `"5"` when an explicit clamp requests that behavior.

Calls without manifest Cedar metadata retain their conventional
`Action::"tool_call::<name>"` action and default resource. Invalid trusted
context fails policy evaluation; it does not fall back to an empty context.

## Approval

`human_approval = true` is mandatory independently of an Allow policy result.
The configured `EscalationGate` obtains an approval receipt for the exact
prepared call. A missing relay, denial, expired receipt, changed arguments,
changed identity/session/context, or replayed queue response prevents execution.
Approval never overrides a Cedar denial. Policy modifications require approval
for the newly prepared call.

The runtime derives `has_human_approval` and `approved_fingerprint` from the
receipt for each policy evaluation. Supplying a reusable approval boolean in
trusted context cannot waive the requirement. Legacy direct ToolClad/MCP,
PTY, and browser methods refuse approval-required calls because they do not
carry an authorization receipt.

## Audit and coverage

The pre-dispatch `PolicyEvaluated` event records the approved normalized calls,
resolved effects, contract fingerprints, approval receipts, authorization
bindings, and denied proposals with reasons. A required journal write failure
stops dispatch. `ToolBatchCompleted` records correlated outcomes, with the call
fingerprint and normalized arguments. Missing or ambiguous backend result
correlations fail explicitly. Ordinary/managed CLI, HTTP, scheduled ORGA and
default DSL `reason()`/`tool_call()`
runs now require protected signed storage. Other default reasoning surfaces and
explicit SDK injection retain their own journal contracts. See [run audit](run-audit.md).

Regressions drive the real ORGA loop, generated Cedar policies, an in-memory
operator approval queue, and synthetic file effects. They verify normalization
agreement between gate/audit/execution, independent approval and policy denial,
replay rejection, modification reauthorization, receipt binding/expiry,
replacement-executor rejection, and scope aliases.

The ORGA path and its knowledge wrapper preserve prepared calls. The DSL
`tool_call()` builtin and adaptive context pre-fetch now use the same
preparation and authorization implementation, mandatory audit checkpoint,
and bounded dispatch. DSL contexts can supply their executor, journal, and
trusted run configuration. Pre-fetch correlates results by call ID and treats
retrieved content as user-level data rather than a system instruction. Its
budget truncation preserves UTF-8 boundaries.

Managed CLI action brokerage, selected Docker/gVisor command/MCP/PTY execution
and protected journals on covered entry points are implemented. Remaining work
includes canonical DSL/REPL source and sandbox binding, other effect/audit paths,
full egress controls and the complete shipping-entry-point E2E matrix. See the
[branch guide](containment-branch-guide.md) for the current coverage. These
call-binding controls do not establish whole-system containment.
