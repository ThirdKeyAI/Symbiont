# Coordinator delegation

The `symbi up` WebSocket coordinator delegates to registered conversational
declarations. Startup reads `agents/*.symbi` and legacy `.dsl` files through a
pinned project directory, refusing symlinks, hardlinks and private paths.
Discovery permits at most 1,024 directory entries, 1 MiB per file, and 16 MiB
of source. Retained selections also have a 1,024-declaration and 16 MiB source
budget, counting a multi-agent file once for each selected declaration.

Each declared name selects that declaration. A filename alias is available for
a file containing one declaration. A multi-agent file requires an exact declared
name. Conflicting names invalidate all aliases of the affected declarations;
directory order never chooses a winner. Unsupported executable definitions,
alternate executors, malformed rules and unreadable sources are unavailable.
Only valid targets appear in the `delegate` tool description. Registry-wide
read or size failures prevent coordinator startup.

## Identity and policy

If `review.symbi` declares `agent reviewer`, both names resolve to one source
snapshot. The parent action is normalized to `Delegate { target: "reviewer" }`
before policy evaluation. Its Cedar action is `Action::"delegate::reviewer"`.
The child principal is `delegated_agent_id("reviewer")`; using the filename
does not create another principal or select another set of permissions.
Policies previously written against filename aliases must use the declared name.

The parent receives `context.invocation.resolved.delegation_target`, containing
the selected name, principal, full-source hash, selected-declaration hash,
policy names, timeout, declared sandbox tier and execution mode. The submitted
alias remains in `context.invocation.resolved.requested_name`. These values and
the exact delegated task are bound to the authorization fingerprint. Dispatch
requires the same immutable registered target. File replacement after startup
cannot change it; source updates require a restart.

The child prompt contains only the selected declaration. File-wide and selected
inline effect policies are enforced independently of Cedar. Sibling-agent rules
and declarations are excluded. Unsupported rules are refused rather than used
as prompt instructions. Responses and termination retain the inline policy
semantics described in [inline policies](inline-policies.md).

For example, this source allows the external gate to authorize health queries
while independently forbidding metrics queries:

```symbi
agent reviewer {
    with timeout = 10.seconds {}
    policy monitoring {
        deny: "query_metrics"
    }
}
```

The corresponding parent permission can inspect the canonical selection:

```cedar
permit(principal, action == Action::"delegate::reviewer", resource)
when { context.invocation.resolved.delegation_target.name == "reviewer" };
```

Child tool permissions remain separate. `agent_status` validates and normalizes
the requested UUID before authorization. Its resource is `Agent::"<uuid>"`,
with matching `invocation.arguments.agent_id` and
`invocation.resolved.runtime_resource.id`. Other monitoring tools retain their
existing action names. Extra tool arguments are refused.

## Execution, audit and operator experience

Children receive the coordinator's six in-process monitoring tools and no
ToolClad, MCP, command execution, knowledge retrieval or nested delegation.
This route does not launch an untrusted process; the declared sandbox tier is
recorded and does not provision a sandbox for these trusted runtime reads.
Running executable source requires the appropriate execution route.

The task is limited to 64 KiB. Each child request permits at most 4,096 output
tokens, and the child deadline is the minimum of its source timeout, 120 seconds
and the remaining parent authorization deadline. Iteration ceilings remain per loop. Parent and child inference reserve from one
shared token budget; child usage is deducted from the parent allowance and included
in parent results. Cancelled or unreported requests retain an uncertain charge.
See [shared inference budgets](shared-budgets.md) for provider accounting and limits.

The parent's required `DelegationStarted` record precedes child inference.
Child startup includes `agent_definition`, enforced `source_policy` and the
parent linkage. Required child failure and cancellation handling remain as
described in [protected run audit](run-audit.md).

The chat panel keeps its layout. Alias calls now show the declared identity in
policy and audit records; unavailable targets produce explicit tool errors.
A source policy denial can be reported in a normal chat response. A child
timeout is reported as a failed delegation, and required audit failures stop
the parent turn. Select **Inspect run** beside the response to view its recorded budget and
permissions, then **Inspect child** to verify a delegated journal. The inspector
labels parent-recorded outcomes separately. See [operator run inspection](run-inspector.md).

SDK callers using the legacy prompt-map `with_delegation` or
`SubLoopDelegationExecutor::new_protected` retain their own source-governance
responsibility. Shipping `symbi up` uses `with_registered_delegation`.

The local shipping fixture covers real WebSocket ingress, canonical identities,
source replacement, inline restrictions, rejected targets, child deadlines,
linked audit, storage failure and cancellation:

```bash
python3 scripts/test-coordinator-audit.py --delegate --binary target/debug/symbi --report /tmp/delegation-audit.json
```
