# Session Types — Inter-Agent Protocol Conformance

> **Status: Experimental (developer preview).** The projection engine and the
> runtime monitor exist and are enforced through the communication gate, but
> there is **no DSL syntax** for declaring protocols yet and `symbi up` does not
> open sessions on its own — you open a session through the Rust API
> (`RuntimeBridge::open_session`). The surface will change. Track it under the
> `symbi-session` crate.
>
> The integration is **off by default** and gated behind the `session` cargo
> feature. Build with `--features session` to enable it (e.g.
> `cargo build -p symbi-shell --features session`); default builds do not compile
> the `symbi-session` crate or the runtime/repl-core session wiring.

Symbiont's per-message policy gate answers *"may this sender talk to this
recipient via this primitive?"* in isolation. **Session types** add the question
it cannot answer: *"does this **sequence** of messages follow an agreed
choreography?"* — ordering, branching, and causality across `ask` / `delegate` /
`send_to` / `parallel` / `race`.

This reproduces the ORGA reasoning loop's two-tier discipline one level up: a
global protocol is **projected** to a per-role finite-state machine, and those
FSMs run as **runtime monitors** that an LLM-driven message sequence cannot
leave. The audit becomes an offline-verifiable record that a multi-agent
workflow executed the approved choreography and nothing else.

## How it works

1. **Declare a global protocol** — the message sequence with branches and loops
   (today via the `symbi_session::Global` IR; a DSL/Scribble surface is deferred).
2. **Project** it to one local FSM per role. Unprojectable protocols are rejected
   at this step with an explanatory error.
3. **Open a session** with `RuntimeBridge::open_session(&protocol, role_binding, ttl)`.
   This establishes the per-role FSMs, attaches the monitor to the communication
   gate, and binds agents to roles.
4. **Send normally.** Under an active session, the inter-agent primitives
   **auto-derive** the protocol label from the FSM for the sender→recipient edge
   — your agent names only the *recipient*; the runtime supplies the legal
   *label*. The gate then validates the transition and advances the FSM.
5. **Out-of-protocol messages are denied** with a precise, recovery-oriented
   message (e.g. *"expected label 'task' to 'Validator'; you sent 'validate'"*)
   that is fed back to the agent so it re-plans — never a hard crash.
6. **Stalls fail closed.** A session past its deadline transitions to a defined
   `Aborted` terminal state.

## Why auto-derived labels

In practice the dominant failure when an LLM drives a protocol is *label
exactness* — the model writes a descriptive label (`"fix rejected work"`) instead
of the exact protocol token (`"fix"`). Because the runtime derives the label from
the projected FSM for the chosen recipient, the agent cannot get the token wrong.
At a genuine choice point (multiple legal labels to the same recipient), pass an
explicit `protocol_label` named argument on `ask` / `delegate`.

## Primitive mapping

| Primitive | Session-type construct |
|-----------|------------------------|
| `send_to` | output |
| `ask`     | output then input |
| `delegate`| sequential message exchange (true session delegation deferred) |
| `race`    | external choice, resolved by the first legal response |
| `parallel`| parallel composition (deferred) |

## Protocol transcript

Every transition (allowed and denied) is recorded to a tamper-evident, Ed25519
hash-chained **transcript** — the offline-verifiable proof that the workflow
executed the approved choreography and nothing else. Reach it via
`RuntimeBridge::session_transcript()` and verify it with `verify()`. (In-process
today; cross-node verification against a provisioned key comes with the
cross-instance work below.)

## Current limits

- No protocol-authoring syntax; protocols are built via the Rust IR.
- Higher-order delegation, `parallel` composition, and dynamic participants are
  out of scope for the current slice.
- **Cross-instance** propagation is not wired yet — the gate isn't consulted on
  the inbound path and there's no remote session-establishment handshake.
- One active session per runtime bridge.

See the [`symbi-session`](https://github.com/thirdkeyai/symbiont/tree/main/crates/symbi-session)
crate for the API.
