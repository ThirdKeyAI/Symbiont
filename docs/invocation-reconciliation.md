# Operator reconciliation

An interrupted invocation keeps its original ID claimed. Once an operator has
established its effects and cleanup, `symbi invocation reconcile` records that
assessment without running the original task again. The receipt is distinct from
a saved runtime result: an operator reporting completion does not turn an
incomplete journal into a completed journal.

## Inspect and review

Run these commands as the runtime service user, against the same project directory
used for admission. Ordinary CLI runs use scope `cli:orga`; scheduled and runtime
API submissions use `scheduler:v1`; HTTP Input uses `http:input:v1`; coordinator
chat uses `ws:chat:v1`. For other
routes, retain the scope from the
original claim or signed `Started.execution_context.invocation` record.

```bash
symbi invocation inspect --project /srv/project --scope scheduler:v1 \
  --id 72d6a833-b825-4b22-b50c-206337d77f7c
```

Inspection reports the request identity, original audit reference, claim and
journal hashes, recovery analysis, and a `snapshot_hash` for the complete review.
Exit 2 means unresolved; exit 0 means recorded or reconciled. An active owner,
invalid evidence or unsafe file produces exit 1. It does not contact a provider
or launch a worker. Preserve the original public audit key through a trusted
channel and compare it with the displayed reference.

Inspect the protected journal and each relevant external system before preparing
the review. Confirm that workers have stopped and pending requests have settled.
An unlocked claim, missing finish record, timeout or error cannot establish those
facts. For example, a published file can exist even though its invocation never
recorded completion; an external API may have accepted a request whose response
was lost. Retain the backend receipts or observations used for the decision.

Create a JSON review using the exact snapshot hash and real evidence digests:

```json
{
  "snapshot_hash": "sha256:<hash from inspection>",
  "outcome": "completed",
  "rationale": "Verified the published report and confirmed worker cleanup.",
  "evidence": [
    {
      "reference": "/srv/operator-evidence/report-and-cleanup.json",
      "sha256": "<64 hexadecimal characters>"
    }
  ],
  "effects_stopped": true
}
```

`outcome` is the operator's assessment: `completed`, `failed`, or `no_effects`.
`no_effects` requires affirmative evidence; silence is insufficient. The runtime
requires a rationale, 1–32 evidence references with SHA-256 digests, and the
explicit stopped/settled-effects assertion. It stores references without fetching
them or checking external facts. Keep referenced evidence available separately.
The review file is bounded to 128 KiB and must be a regular, non-symlink file.

## Persist the decision

```bash
symbi invocation reconcile --project /srv/project --scope scheduler:v1 \
  --id 72d6a833-b825-4b22-b50c-206337d77f7c --review reviewed-outcome.json
```

The command refuses live owners, stale snapshots, changed signing keys, and
already recorded runtime results. Repeating the same review returns the original
receipt; attempting a different decision is refused. The receipt records the
service UID and timestamp. The UID identifies the local account, not necessarily
the individual operator when accounts are shared.

Original claim and journal bytes remain intact, including an unfinished final
fragment. A separate `.resolved.json` file in `.symbiont/invocations/` contains
the signed receipt. Its signature covers the serialized payload prefixed by
`symbi-invocation-resolution:v1` and a newline, using the original protected audit
key. Publication uses a synced temporary file, non-replacing atomic publication
and directory sync. Receipts count toward the existing store file/byte limits.

Subsequent lookup verifies the signature and hashes against the original evidence.
Changing or losing the evidence fails closed. A complete malformed or forged
journal record cannot be reconciled by this command. A complete claim and audit
reference are required; an empty startup journal is reported explicitly. Missing
claim identities, lost audit keys and damaged complete records require separate
incident investigation. Preserve all evidence and duplicate-detection records.

## Retry and cron behavior

Reconciliation never grants another execution under the original ID:

| Surface | Result of retrying a reconciled invocation |
|---|---|
| `symbi run --invocation-id` | JSON operator receipt and exit 3; no saved CLI output is invented. |
| HTTP Input and runtime API | HTTP 409 with `status: "reconciled"` and the receipt; no execution. |
| Coordinator Chat | Original audit reference and `INVOCATION_RECONCILED`; no execution. Inspect the signed assessment through the invocation CLI or run view. |
| Attached shell `/cron run` | Displays the operator assessment and original audit reference. |
| Cron history | `Reconciled`, original error/evidence and a separate `resolution` object. |

For scheduler scope, the CLI also updates matching history in the existing
project `.symbiont/cron_jobs.db`. Use `--cron-store DATABASE` for a custom store;
it must belong to the same execution project. The scheduler's bounded recovery
scan also applies published receipts, including after a crash between receipt
publication and the history transaction. Repeating the review repairs that same
interruption without publishing another decision.

The job remains paused after reconciliation. Inspect all its unresolved
occurrences, then use `symbi cron resume JOB_UUID` or the corresponding operator
UI/API action to permit future timer work. A new manual invocation also represents
an explicit request for new work. Any remaining unresolved history still blocks
execution. The original occurrence, its error and its receipt remain available;
the original ID cannot be reused for a new task.

## Scope and validation

This is an operator authority API, not a model-callable tool. Its signature proves
which runtime key endorsed the assessment and which evidence bytes were reviewed;
it does not independently prove the supplied external facts. It does not refund
token reservations, discard worker leases, restore partial files, resume a lost
reasoning loop, or provide exactly-once behavior for arbitrary services.

For broker-created files with a signed publication intent, a separate
[file recovery command](file-publication-recovery.md) can finish the exact
pending publication before this invocation-wide assessment. File recovery keeps
the original invocation unresolved and does not repeat its tool. Complete it
before recording a final assessment; a resolved invocation cannot authorize a
new file recovery.

The focused `invocation_reconciliation` integration tests cover locked owners,
stale snapshots, partial claim/journal tails, forged receipts, immutable evidence,
missing claims, empty startup journals, cached outcomes and cron history.
`scripts/test-cron-occurrences.py --reconcile` exercises actual Docker effects,
`SIGKILL`, independent signature verification, persisted operator decisions,
same-ID refusal, repair after a failed cron-history transaction, and useful new
work after explicit resume. `scripts/test-http-invocations.py --reconcile` covers
HTTP Input receipts and CLI receipt retrieval with no provider configured. The optional shell
binary checks terminal presentation. The fixture provider is scripted.
