---
title: Inline Effect Policies
description: Supported canonical DSL policy rules for normalized ORGA effects
---

# Inline Effect Policies

Canonical fleet conversations, coordinator child delegations, ordinary and managed `symbi run`, and registered
HTTP ORGA runs enforce the supported inline policy subset described here. Native
scheduler invocations, including cron, manual triggers and the agent execution API,
also compile the selected source before inference and enforce these rules on
prepared calls. Unsupported policies fail the invocation before provider calls.
These restrictions
are independent of the external Cedar gate: both must permit an effect, and a
manifest's mandatory approval still requires an exact, unexpired receipt.

```symbi
policy shared {
    deny: ["shell", "delegate::untrusted"]
}

agent writer() {
    capabilities = ["write"]
    with sandbox = "docker", timeout = 20.seconds {}
    policy files {
        allow: "edit_file" if invocation.arguments.path == "result.txt"
        deny: "edit_file" if invocation.arguments.content == ""
    }
}
```

For shell `edit_file`, `./result.txt` is normalized to `result.txt` before the
rules run. This example permits that path only; the content remains literal.
It does not grant a tool or workspace mount. The operator must configure a
sandbox, explicit data mount, external Cedar permission and approval relay.
See [shell containment](shell-containment.md).

## Rules and conditions

Only `allow:` and `deny:` rules are supported. The selector is a literal tool
name, a nonempty array of literal tool names, or a boolean. `true` matches all
effects; `false` matches none. Delegations use `delegate::<target>` as their
effect name. Tool names are exact, case-sensitive names, without wildcard or
capability expansion. Responses and termination are exempt from inline effect
rules so the run can report a refusal; the external gate still evaluates them.

Each selected policy block is a separate restriction. A matching deny wins. A
block containing allow rules requires a matching allow. A block with only deny
rules leaves other effects for the external gate to decide. All file-wide and
selected-agent blocks apply; sibling-agent blocks do not. Duplicate names in
the selected scope and empty blocks are errors.

An optional `if` condition supports:

- Boolean, JSON string and signed 64-bit integer literals; underscores in integers.
- Dot access to `principal`, `invocation` and `context` as described below.
- `==`, `!=`, integer `<`, `<=`, `>`, `>=`, and scalar `in` a flat literal array.
- Boolean `!`, `not`, `&&`, `||`, and parentheses, with normal precedence and
  short-circuit evaluation.

| Value | Authority |
|---|---|
| `principal` | Actual runtime agent UUID, as a string. |
| `invocation.arguments` | Validated, normalized arguments used for dispatch. |
| `invocation.contract` | Frozen tool name, digest, action/resource identity and approval requirement. |
| `invocation.resolved` | Executor-produced effect details, such as command, selected boundary and scope. |
| `invocation.source_policy` | Retained source and selected declaration hashes, policy names and semantics version. |
| `context` | Runtime-owned trusted context for the current run. |

Missing fields, nulls, mismatched scalar types and non-boolean conditions deny
the effect. A skipped short-circuit branch is not evaluated. An argument named
`principal` or `context` cannot replace the runtime values. Arguments are still
untrusted data: an argument claiming consent, verified identity or a signature
does not establish that property. `context.has_human_approval` and
`context.approved_fingerprint` are unavailable to inline conditions. Receipt
validation remains a separate runtime check.

Function calls, indexing, arithmetic, executable requirements, `require:` and
`audit:` rules are unsupported and rejected before inference on these routes.
This includes rules inside conditions that would be false. A `require` record
claiming rate limits or PII detection is not an implemented control. Existing
examples using the broader DSL need an interpreter for those requirements.

Compilation is bounded by the source's 1 MiB limit, 128 selected rules, 256 tokens
per expression and 24 levels of expression nesting. No policy expression
executes code, reads a file, performs network access or asks the model to decide.

## Approval, audit and replacement

Inline rules run after preparation and before requesting approval. They are
checked again for every externally modified action and before an authorization
grant is issued. A permissive external gate cannot waive an inline restriction.
Unsupported definitions do not start a provider call; a supported rule that
denies an effect returns an ordinary policy error to the loop.

The source snapshot survives file edits and reloads during a pending run. Its
hashes are part of the call fingerprint, approval details and signed pre-effect
record. Startup audit context contains `source_policy`; approved call evidence
contains the same metadata. Hashes use SHA-256 over the canonical JSON string
representation of the original source and selected declaration, respectively.
Keep the matching source revision with the audit evidence. See
[run audit](run-audit.md).

For the scheduler, `crates/runtime/tests/scheduler_execution.rs` checks real
allowed/denied Docker effects and signed source bindings, and verifies that an
unsupported policy cannot call the provider. The shipping API/cron regression
reuses the escape harness's scheduler fixture:

```bash
python3 scripts/test-scheduler-policies.py --binary target/debug/symbi \
  --harness-scripts /path/to/symbiont-karpathy-loop/evals/escape/scripts \
  --report /tmp/scheduler-policies.json
```

It verifies file-wide and selected-agent denies against permissive Cedar,
normalization before an allow condition, sibling-policy isolation, useful worker
output, signed source/declaration hashes and cleanup. Fixtures use local scripted
inference and a cached image. This does not implement executable DSL statements
or broader `require:`/`audit:` rules.

In the shell, a policy-denied write never appears as a held approval. A permitted
write still opens the normal Gate review, including its source binding. Existing
`--yes` behavior does not approve effects.

## Managed worker admission

Managed CLI sessions retain the complete source, including file-wide policies.
The fixed `claude_code` action admits the worker. An inline allowlist must include
that name as well as each permitted broker tool; it does not grant the worker a
`claude_code` tool. The name is reserved and cannot be registered as a managed
ToolClad backend. For example:

```symbi
policy managed_scope {
    allow: ["claude_code", "read_file"]
    deny: "read_file" if invocation.arguments.path != "src/main.rs"
}
```

Admission arguments contain the fixed task request, argv, explicit environment,
stdin strategy, bounds, inference configuration and selected tool definitions.
`sandbox` describes the actual scratch worker boundary; `tool_sandbox` describes
the backend boundary with its source mounts. Broker calls carry their own
normalized backend contracts. Both paths use the same actual session principal
and source metadata. Worker success records bounded output hashes and sizes.

Set managed agent metadata `human_approval = true` to require exact admission
review. The default is false for a direct operator CLI invocation; external Cedar
may still require approval. Individual ToolClad approval requirements remain
independent. Refused admission starts no worker or provider request and ends with
a failed signed journal. Source edits during review apply to later runs.

## Scope and SDK use

`dsl::ExecutionPolicy::parse(source, selector)` compiles the selected restrictions.
Wrap the fully composed executor with
`reasoning::source_policy::SourcePolicyExecutor::new(inner, policy)`, then use a
governed runner or dispatcher. Place this wrapper outside knowledge/delegation
tool wrappers. Direct unapproved execution is refused. Source-bound grants
cannot be dispatched through a different source wrapper instance.

Canonical fleet loading retains this compiled policy with the declaration and
continues to refuse executable DSL bodies, parameters, return types and other
unsupported requirements. Ordinary CLI and registered HTTP execution remain
ORGA conversations; this change does not implement their DSL behavior bodies.
The separate legacy REPL language, legacy SDK prompt-map delegation and other SDK
entry points are not covered merely because these routes are wired. Broader policy constructs and route migration remain
open work.
