# Policy denial — the gate, in isolation

Runs offline. No API key, no model, no network.

The same Cedar gate the runtime wires into the live reasoning loop, evaluated
directly so you can see a decision without starting an agent.

## Run it

```bash
cd examples/policy-denial
./run.sh
```

```json
{"decision":"deny","reason":"deny policies matched: deny_1","tool":"list_agents","policies_dir":"."}
{"decision":"allow","reason":"allow policies matched: deny_0","tool":"system_health","policies_dir":"."}
```

`deny.cedar` forbids one tool and permits another. Nothing about the agent
changes between the two calls — only policy decides.

## Try it yourself

Delete the `permit` line and re-run. `system_health` flips to `deny`: the gate
is **fail-closed**, so a tool with no matching `permit` is refused rather than
allowed by default. That is why a fresh `symbi init` project denies every tool
call until you write a policy — the empty state is the safe state.

Add a `forbid` for a tool that also has a `permit` and the `forbid` wins.
Cedar denies on conflict, so a prohibition cannot be overridden by adding
another allow rule somewhere else.

## Where this runs for real

`symbi run` and `symbi up` load `policies/*.cedar` at startup and evaluate
every proposed tool call, response, and delegation through this gate before
dispatch. Surface-scoped policies in `policies/<surface>/` apply to one entry
point only — see [security-model](../../docs/security-model.md).
