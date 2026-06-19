# symbi-session

> **Status: Experimental.** Multiparty session-type projection + a runtime
> protocol monitor for inter-agent choreography. The projection engine and the
> monitor are wired into the runtime communication gate, but there is **no DSL
> syntax** for declaring protocols and `symbi up` does not yet open sessions on
> its own — sessions are opened via the Rust API (`RuntimeBridge::open_session`).
> APIs may change between minor releases.
>
> The runtime + repl-core integration is **off by default**, gated behind the
> `session` cargo feature. Enable it with `--features session` (e.g.
> `cargo build -p symbi-shell --features session`); default builds do not compile
> this crate or its wiring.

`symbi-session` adds **protocol-conformance monitoring** to Symbiont's inter-agent
messaging. Symbiont's `CommunicationPolicyGate` already answers *"may this sender
talk to this recipient via this primitive?"* per message. This crate answers the
question it cannot: *"does this **sequence** of messages follow an agreed
choreography?"* — ordering, branching, and causality — using multiparty session
types (MPST) as runtime monitors.

It mirrors the ORGA reasoning loop's two-tier discipline one level up: a global
protocol is **projected** to a per-role finite-state machine at build time, and
those FSMs run as **runtime monitors** that the LLM-driven message sequence
cannot leave.

## What it provides

- **Global protocol IR** (`global::Global`) — `Message`, `Choice`, `Rec`/`Var`,
  `End` for the linear / branch / loop fragment.
- **Projection** (`project::project`) — global type → per-role `local::Local`,
  including the branch-merge that makes projection total (returns a typed
  `ProjectError` rather than panicking when a protocol is unprojectable).
- **Well-formedness** (`wellformed::check_well_formed`) — projectability for all
  roles, guarded recursion, bound variables.
- **FSM** (`fsm::Fsm`) — compiles a local type to a state machine with
  `step`/`expected`/`is_accepting` and an `IllegalTransition::diagnose()` that
  pinpoints a wrong **label** vs a wrong **target** (recovery-oriented messages).
- **`SessionMonitor`** (`monitor::SessionMonitor`) — keyed by `SessionId`, holds
  the per-role FSM state; `establish` / `observe` (advances sender + receiver
  atomically) / `is_complete` / `abort` / `is_aborted` / `legal_next` /
  `legal_labels_to`.

## How it's wired into the runtime

- `symbi-runtime` owns a `SessionRegistry` (`session::SessionRegistry`) that
  opens sessions (mint id + project + establish), tracks lifecycle
  (Running / Complete / Aborted) and a deadline, and exposes the shared
  `Arc<SessionMonitor>`.
- `CommunicationPolicyGate::with_session_monitor` consults the monitor: after the
  usual per-message authorization, a tagged message is checked against the
  projected FSM (`check_session`), denying illegal transitions with the precise
  `diagnose()` message — fed back through the policy-feedback path so the loop
  re-plans.
- In the DSL layer (`repl-core`), when a session is active the inter-agent
  primitives **auto-derive** the protocol label from `legal_labels_to` for the
  sender→recipient edge: the agent names only the *recipient*, the runtime
  supplies the legal *label*. This eliminates the most common LLM failure mode
  (writing a descriptive label instead of the exact protocol token). Ambiguous
  edges (multiple legal labels to one recipient) take an explicit
  `protocol_label` named argument on `ask` / `delegate`.

## Minimal usage (Rust API)

```rust
use std::time::Duration;
use symbi_session::examples::coordinator_pipeline;        // a sample protocol
use symbi_runtime::session::RoleBinding;
use symbi_runtime::types::AgentId;
// (bridge is a repl_core::runtime_bridge::RuntimeBridge)

let (protocol, _roles) = coordinator_pipeline();
let (coord, validator, processor) = (AgentId::new(), AgentId::new(), AgentId::new());
let binding = RoleBinding::new()
    .bind(coord, "Coordinator")
    .bind(validator, "Validator")
    .bind(processor, "Processor");
let _session_id = bridge.open_session(&protocol, binding, Duration::from_secs(120))?;
// Subsequent inter-agent messages are now tagged + conformance-checked, with
// labels auto-derived from the protocol.
```

## Mapping to Symbiont primitives

| Primitive | Session-type construct |
|-----------|------------------------|
| `send_to` | output |
| `ask`     | output then input (a two-step micro-protocol) |
| `delegate`| modeled as a sequential message exchange in v1a (true session-passing is deferred) |
| `race`    | external choice resolved by the first legal response |
| `parallel`| parallel composition (deferred) |

## Not yet (deferred)

- DSL syntax (`protocol { … }` or Scribble) for declaring protocols — today the
  global type is built via the Rust IR / `examples`.
- Higher-order delegation (passing a session to a third agent), `parallel`
  composition, and dynamic / parameterised participants.
- **Cross-instance** session propagation over `RemoteCommunicationBus` (the gate
  is not consulted on the inbound path, and there is no remote
  session-establishment handshake yet) — and, with it, cross-node transcript
  verification against a provisioned (non-ephemeral) key.
- A single bridge currently tracks one active session at a time.

## Available: protocol transcript (in-process)

Every session transition (allowed and denied) is recorded to a tamper-evident,
Ed25519 hash-chained **transcript** — the offline-verifiable proof that a
multi-agent workflow executed the approved choreography. Reach it via
`RuntimeBridge::session_transcript()` (an `Arc<Mutex<SessionTranscript>>`) and
check it with `verify()`. Fields are length-prefixed before hashing so delimiter
injection cannot forge an entry. Verification is currently against the
transcript's own in-process key; cross-node verification (provisioned key) is
part of the deferred cross-instance work above.

## Design notes

See the design + viability analysis under `docs/superpowers/specs/` and
`docs/superpowers/plans/` (`2026-06-15-symbi-session-*`). The crate is
dependency-light on purpose so the JS/Python SDKs can mirror the lattice/FSM.
