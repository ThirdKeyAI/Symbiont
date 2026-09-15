# Operator run inspection

The public A2UI console includes a **Run Inspector**. Open it from the sidebar
and paste a retained audit reference, or select **Inspect run** beside a
coordinator response. References remain available while a chat turn is running
and alongside errors carrying the same request identity. Closing the inspector
returns to the conversation without disconnecting or clearing its messages.

Schedule history exposes separate **Inspect admission** and **Inspect execution**
buttons when those references are available. Operator assessments remain separate
from the original errors and results. Missing history sources produce an explicit
partial-history warning. Older agent history entries containing only an execution
ID cannot supply a journal link; paste a reference retained from the original
response or runtime log instead.

The inspector verifies a fresh snapshot when opened or refreshed. It displays:

- Journal identity, signature-chain verification, snapshot hash and last record.
- Parent and child references. A parent-recorded child outcome is labelled as such;
  opening the child verifies its own journal. Older children without a parent audit
  reference retain their parent context but cannot provide a backlink.
- The latest recorded shared token budget: usage, reservations, uncertain charges
  and available allowance. Parent usage includes descendants. Availability also
  respects ancestor balances. Snapshots are not live balances.
- Reconstructed family accounting from the root's signed inference reservations
  and settlements. Unsettled requests retain their full uncertain charge, including
  after a hard kill without a final snapshot. Children link to the root through
  **Inspect family accounting**. See [budget recovery](provider-budget-recovery.md).
- Per-action allow/deny decisions, contract identities and recorded effective file,
  network, delegation and command-boundary grants. Raw arguments, executable argv,
  prompts and tool results are not included in the permission summary.
- Tracked effects and explicit incomplete or unknown outcomes. No tracked effects
  does not establish that no work happened: a process can die before recording a
  result or supported effect checkpoint.
- A separately verified invocation resolution when available. A signed operator
  assessment preserves the original incomplete/unknown outcome and does not grant
  permission to replay the action.

CPU, memory and execution-time values in grants are configured ceilings. The view
does not report measured CPU/memory usage or live occupancy of the shared
supervisor admission pool. Missing budget/permission records are shown as missing,
never as zero usage or unrestricted access.
Use the separate [Worker capacity](worker-capacity.md) sidebar page for current
pool reservations and individually sampled worker CPU/memory. Those live
observations do not replace this view's signed historical evidence.

## Verification and access

The console calls this authenticated, administrative endpoint:

```text
GET /api/v1/audit/runs/{agent_uuid}/{run_uuid}?public_key={64_hex_characters}
```

The runtime derives the journal filename from typed IDs under the trusted project
captured at startup. It does not open a caller-supplied path. Linked references
must stay in that project and use the same key. Storage must be runtime-owned,
with no directory links or untrusted writers; the journal directory and file must
be private. Invalid signatures, mismatched identities and unsafe storage are
refused. Successful and handler-generated error responses use `Cache-Control:
no-store`. Scoped API keys receive 403; malformed keys receive 400; unavailable or
invalid evidence receives 422. An unsupported provider also returns unavailable.

The supplied public key is a trust input. Retain it independently through the
operator's trusted channel. A key obtained alongside the journal does not provide
independent evidence of that journal's origin. The view verifies signed records,
not the truth of an external service's state or an operator's assessment.

The endpoint permits two concurrent verification workers per runtime process and
refuses additional inspections while busy. It bounds source journals to 16 MiB,
responses to 2 MiB, and child links, permission decisions and tracked effects to
256 each. Oversized evidence requires offline inspection; it is not silently
truncated. A stable signed prefix may have an unverified final fragment, whose
byte count is displayed separately. Use [crash inspection](crash-inspection.md)
and [invocation reconciliation](invocation-reconciliation.md) for offline review.
The inspector has no execute, retry or reconcile action.

## Local UI and validation

Build the UI in `crates/symbi-a2ui` with `npm ci --ignore-scripts` and
`npm run build`. Development and preview servers default to a runtime on
`http://localhost:8080`. Set `SYMBI_RUNTIME_URL` to a trusted runtime address when
using another port. Serve the built `dist` with the deployment's runtime proxy
for production.

The focused shipping fixture uses local scripted inference, a real delegated chat
turn and an owned service process killed during a pending request. It validates
parent/child links, shared usage, authentication, wrong-key refusal, read-only
inspection, an incomplete journal after restart, scheduled history links and an
operator assessment of an interrupted schedule:

```bash
python3 scripts/test-run-inspector.py --binary target/debug/symbi --report /tmp/run-inspector.json
```

For a browser session, add `--serve --api-port 18085`, then run
`SYMBI_RUNTIME_URL=http://127.0.0.1:18085 npm run preview` from `crates/symbi-a2ui`.
The fixture prints its synthetic token and expires after 15 minutes. Stop it with
Ctrl-C when finished. The default run terminates after its checks.
