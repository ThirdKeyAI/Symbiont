# DSL invocation identity

The evaluator passes the current `ExecutionContext.agent_id` to each async
reasoning, communication and pattern builtin. Each invocation clones the configured
builtin context and binds that caller locally. Nested user functions retain the
same principal, and overlapping calls do not share mutable sender identity.

Creating or declaring an agent produces a new agent instance. It does not enter
that agent's execution scope or change the creator's principal. Trusted callers
enter an agent scope through the execution API; ordinary expressions outside an
agent have no attributed agent in their evaluator context.

The receiving builtins use this identity for reasoning authorization and
communication policy checks. An unscoped caller retains the existing anonymous
builtin behavior; it does not inherit the last declared agent's privileges.
Direct SDK use of `ReasoningBuiltinContext` remains explicit caller configuration.

The REPL retains successful function and behavior declarations between inputs.
A module publishes its definitions and agents together only after every agent
passes declaration validation. A rejected module leaves existing definitions
unchanged. Each behavior invocation captures its entry point and helper functions
in one snapshot; later redefinitions affect subsequent invocations. Arguments,
local variables and caller identity remain private to the invocation.

Start the declared agent before calling `:agent execute <id> <behavior> [args]`.
The optional argument text is exposed as the behavior's `args` variable; this
command does not bind typed behavior input parameters. The interactive client
forwards agent and monitoring commands to the evaluator and displays its outputs
and errors. DSL `print()` writes diagnostics to stderr so stdout remains a framed
RPC response stream. The execution monitor records returned failures as completed
failed attempts; it is separate from the protected action journal.

`RuntimeBridge` captures a canonical project directory at construction. Each
builtin context keeps that same directory even if the process later changes its
working directory. SDK callers can select it with `with_project_root` before
sharing the bridge. A failed initial capture remains a configuration error.
Default `reason()` and `tool_call()` discover tools from this project's absolute
`tools/` path and open protected per-invocation journals before inference or effects.

Both builtins return an `audit` map containing `run_id`, `path` and `public_key`.
The principal is chosen once per invocation, including anonymous calls. Explicit
tool calls require a start record and a terminal record after worker cleanup;
denials, tool errors and cleanup failures have distinct terminal reasons. A failed
terminal write returns an error even if an effect already happened. Cancellation
can leave an incomplete signed prefix, which must not be treated as completion.
SDK callers can deliberately inject their own executor or journal; they then own
those implementations' guarantees and injected journals have no default audit map.
See [protected run audit](run-audit.md) for verification and key custody.

Direct `llm_call`, all provider calls inside `chain`, `debate`, `map_reduce` and
`director`, and registered-agent composition also require a journal per provider
call. Their existing string/list results are preserved. Use `:audit` to display
the latest 256 public references and a count of older references omitted from the
display; the older journal files remain available. SDK callers can read the same
display through `RuntimeBridge::audit_references()`.

Communication calls require a configured policy gate and retain the exact
registered recipient definition used during authorization. Replacing a name
cannot retarget an already authorized call. `delegate` requires a registered
agent. `send_to` returns null after durable acceptance, before background completion;
its journal records the later outcome. `race` waits for the first successful call,
and cancelling `parallel` or losing a race cancels the remaining provider futures.
A retained owner completes required journal writes after caller cancellation,
including cancellation during startup. Abrupt process loss can still leave an
incomplete signed prefix.

The audit binds canonical typed request and response hashes, the caller, and any
recipient snapshot. It does not attest to arbitrary provider-internal effects or
HTTP wire bytes. A configured communication bus must accept its request/response
enqueue; enqueue acceptance does not prove recipient consumption. The explicit
threaded conversation API does not copy conversation history into those queues.
Each call has a deadline, request/response size limits and a requested output token
cap. These are not aggregate budgets or a terminal record for an entire pattern.

The captured project supplies the default sandbox and tool limits. Registration
rejects explicit legacy `security.tier`, `security.sandbox`, resource fields and
execution-policy fields because this route cannot enforce them per agent. Empty
blocks and capability-only declarations retain their existing checks. A rejected
module publishes no agents or replacement helpers. Repeated constraint blocks
and repeated capability lists are rejected instead of overwriting requirements. Configure the project boundary
for supported legacy behavior, or use a supported canonical route when per-agent
settings are required; do not remove a required constraint merely to make a
registration succeed.

The legacy Tier1–Tier4 and strict/moderate/permissive values do not map to the
canonical Docker/gVisor/Firecracker settings. AgentPin domain identity and full
canonical per-agent source selection are separate integration work. Declaring or
starting an agent does not itself run a behavior. Local shell orchestrator and
fleet turns have protected per-turn audit; see [protected run audit](run-audit.md).
