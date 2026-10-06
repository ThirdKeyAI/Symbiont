# Recovering inference budget evidence

Reasoning loops persist a shared family's inference reservations in the root
run's protected journal. Each request records its input allowance, bounded output
allowance, provider/model identity, typed request hash and originating agent/run
before provider transport begins. The hash uses the actual reduced output limit;
it describes the typed provider contract, not an HTTP wire payload. Prompts and
credentials are not copied into these accounting records.

Children use the same root writer and record a **family accounting** backlink in
their own journals. Their immutable scope ancestry and limits are included in
each root reservation. Creating a child does not create an allowance. A shared
budget cannot move to another root journal or accept unaudited reservations after
attachment. Explicit SDK journals remain responsible for their storage contract;
an in-memory writer does not become durable merely by emitting these events.

The root history contains `BudgetOpened`, `BudgetReservationStarted` and
`BudgetReservationFinished` records. A `BudgetScopeLinked` record points a child
to that history. Required settlement records precede response-driven actions.
Failure to persist a reservation prevents the provider call; failure to persist
settlement discards the response and closes further allowance. Cancellation while
a required write is pending retains the reservation because the write may still
reach durable storage.

Timeout cleanup records unknown usage only for abandoned requests in the
terminating scope. Live requests and other family scopes remain open. A finish
append that was interrupted may still reach durable storage; timeout cleanup
does not append another finish for it. Failed required timeout settlement closes
the allowance and reports a run error. Neither case refunds uncertain usage.

## Inspecting an interrupted request

Run inspection reconstructs balances from authenticated reservation and settlement
records, even when a hard kill prevented the terminal `BudgetUpdated` snapshot:

```bash
symbi audit inspect /absolute/project/.symbiont/governed/AGENT.RUN.jsonl \
  --run-id RUN --public-key TRUSTED_PUBLIC_KEY
symbi invocation inspect --project /absolute/project --scope cli:orga --id INVOCATION
```

`recovered_budget` contains the root identity, all recorded scopes and their
balances, and individual reservations with start/finish sequences and outcomes.
An absent settlement is an uncertain charge for its full reserved amount. This
also applies to a request still running at the moment of inspection. Missing
usage retains that charge; inconsistent counters or usage above a reservation
close the ancestor allowances. Duplicate reservations/settlements, altered scope
ancestry or limits, and over-admission histories are rejected. Unknown history in
older journals is `null`, never an invented zero balance.

For example, a 100,000-token family with 20,000 recorded tokens and an interrupted
30,000-token reservation has 30,000 uncertain tokens and 50,000 remaining after
charges. The interrupted request is not counted as known zero-cost inference.
An interrupted write can leave an unauthenticated tail; only the verified prefix
contributes accounting evidence. Preserve the original bytes.

The administrative Run Inspector displays **Reconstructed family accounting**
separately from the latest recorded snapshot. Open **Inspect family accounting**
from a child to inspect the root's complete history. Neither the API nor the CLI
fetches provider billing data, changes balances, authorizes replay or resumes the
original execution. Existing invocation IDs continue to refuse unresolved retries.

Reconstruction is not a persistent account spending quota: a new operator
invocation still receives a new root allowance. Automatic execution resumption and
provider-specific reconciliation that safely replaces uncertain usage with verified
charges remain separate work. An operator's general invocation assessment does
not settle a provider reservation. This history covers reasoning-loop provider
calls; managed CLI broker traffic and direct calls outside a reasoning loop retain
their existing accounting contracts. See [shared budgets](shared-budgets.md) and
[invocation reconciliation](invocation-reconciliation.md).

## Validation

Focused tests cover concurrent siblings, cancellation during a journal write,
unknown/invalid usage, rejected ancestry changes, required writes before transport,
discarded responses after failed settlement and protected delegated accounting.
The shipping fixture uses a local synthetic provider, independently verifies
Ed25519 signatures, kills the CLI after provider acceptance, and checks recovery
and same-ID refusal in new processes:

```bash
python3 scripts/test-provider-budget-recovery.py --binary target/debug/symbi \
  --report /tmp/provider-budget-recovery.json
```

Add `--serve --api-port 18087` to retain the restarted API for browser inspection.
Build the UI and run `SYMBI_RUNTIME_URL=http://127.0.0.1:18087 npm run preview` from
`crates/symbi-a2ui`. The fixture prints a synthetic token and audit reference.
Write `finish` to its private `observer/control` file to stop it, or let it expire
after 15 minutes. No external inference service is contacted.
