# Phase 2: held-out routing and governed actions

Routing run: 2026-09-23 (UTC). Action validation: 2026-10-03 (UTC).

The frozen routing candidate failed all three held-out evaluations. Its result
matched an ordinary agent given the same domain instructions: 32/36 exact answers
for each, compared with 15/36 for the general baseline. Every failed evaluation
was refused approval. The routing workflow was never promoted and was disabled
after the experiment.

This supports the improvement lifecycle as a release control, but does not
establish that wrapping instructions in that lifecycle improves model accuracy.
The phase 1 perfect score did not generalize to the new combined edge cases.

## Repeated comparison

The original V1 proposal and general baseline were unchanged. Twelve new inputs
were evaluated three times, rotating the order of three arms. The third arm
placed the complete V1 instructions in ordinary agent-source comments, providing
an information-matched control. Prompt packaging still differs between that
control and an explicitly selected candidate.

| Arm | Exact answers | Formatting-only failures | Wrong or ambiguous routing | Reported tokens | Median seconds | p95 seconds |
|---|---:|---:|---:|---:|---:|---:|
| General baseline | 15/36 | 0 | 21 | 18,532 | 3.072 | 7.638 |
| Information-matched ordinary control | 32/36 | 0 | 4 | 36,995 | 3.941 | 16.219 |
| Frozen governed candidate | 32/36 | 0 | 4 | 45,633 | 5.203 | 25.111 |

Latency includes the first CLI execution, inference and durable state, excluding
offline audit verification and cached receipt reads. p95 uses nearest rank. One
retained warm-up is excluded from this table. The candidate used about 23% more
reported tokens than the information-matched control in this sample. This is
observed workload cost, not a measurement isolating lifecycle overhead.

The candidate scored 10/12, 11/12 and 11/12. Each repetition independently needed
12/12 exact answers, no policy denials and at most 8,000 reported tokens per case.
No thresholds or instructions were changed after seeing results.

Two cases explain its four failures:

- An identity document marked `present: false` also had unused expired-status and
  mismatched-subject fields. The contract treats that document as absent. The
  candidate incorrectly escalated it in two repetitions.
- A missing application coincided with an unsigned consent bearing a different
  subject identifier. The identifier conflict must take precedence and escalate.
  The candidate requested documents in two repetitions. The information-matched
  control made this error in all three repetitions.

One incorrect candidate response also included a long explanation despite the
exact-label contract. Every failed response had a routing error or ambiguity;
output-format normalization alone would not repair these results. Formatting
classification is diagnostic only; the mandatory evaluator compares raw answers.

All 108 scored executions and the warm-up completed with verified journals and
no tool dispatches. Candidate evaluations recorded no policy denials. Both
ordinary controls stayed outside the improvement lifecycle.

## Governed action validation

The separate receipt-action workflow completed all eight E2E checks using the
1.21.0 CLI, real local `qwen3:8b` inference and Landlock workers. This candidate
has its own one-case acceptance suite and approval; it is independent of the
rejected routing candidate above.

| Scenario | Tool dispatches | Receipt | Verified outcome |
|---|---:|---|---|
| Acceptance trial | 1 | Created | Exact `RECORDED`; accepted signed trial |
| Active approved version | 1 | Created | Exact approval bound to the prepared call |
| Terminal denial | 0 | Absent | Exact `DENIED`; no automatic retry |
| Approval relay omitted | 0 | Absent | Signed refusal: required approval relay unavailable |
| Runtime killed before approval | 0 | Absent | Incomplete journal; operator resolution records no tool effect |
| Runtime killed after publication | 1 | Retained | Incomplete journal; operator resolution records completed publication |
| Explicit new invocation after recovery | 1 | Created | Fresh exact approval and complete journal |

All seven journals were verified. The two interrupted invocation IDs refused
execution before reconciliation and returned their retained operator resolutions
afterward, without another inference or tool dispatch. Reconciliation preserved
the original journals. Ordinary completed invocation retries returned saved
results without another provider request.

The observer found exactly four expected receipts, zero retained worker or
staging reservations, and no proxy errors. The workflow was disabled and its
private supervisor pool removed at completion. Thirteen inference requests
reached the local proxy; the post-publication crash request was held before
forwarding, so twelve requests reached Ollama. Model identity and frozen fixture
hashes remained unchanged.

An earlier interrupted attempt and a later harness failure remain recorded as
failures. The harness failure expected a stale refusal word despite the CLI
correctly returning exit 2 and requiring reconciliation. The completed run checks
the actual refusal, exit code, signed evidence and absence of repeated effects;
no acceptance threshold was relaxed.

These results validate this synthetic action lifecycle on one Linux host. The
terminal operator is simulated, the tool writes one benign local file, and the
publication crash happens after the write was confirmed. This does not establish
independent human review, arbitrary remote-transaction recovery or correctness of
regulated business decisions.

Action-test binary SHA-256:
`a01ca2da7a398b7c65d307b769890ced985a1f46e5e13451c6a738b02ccfb726`.
See [release validation](release-1.21-validation.md) for the accompanying checks.

## Reproduction and limits

Use the [frozen phase 2 protocol](../examples/governed-improvements/document-intake-phase2/contract.md),
[comparison driver](../scripts/test-intake-phase2.py), and
[action driver](../scripts/test-intake-actions.py). Each driver retains a private
project, exact inputs and outputs, configuration hashes, signed journals,
verification results and CLI logs. Output includes private signing keys and must
remain outside version control.

Routing-run provenance:

- Runtime source: `f20d6c7bb1d57f83e669fb41cd148b4f6081e856`.
- Installed binary SHA-256: `8791b0edcf63340912114dfdf24bfd16b3e77b2721c3523832529214c4e6097b`.
- Local model: `qwen3:8b`.
- Model digest: `500a1f067a9f782620b40bee6f7b0c89e17ae61f686b92c24933e4ca4b2b8b41`.

These are synthetic engineering holdouts, not independently supplied domain-owner
cases or production documents. Repetitions on one model are correlated and do not
establish statistical reliability, regulatory suitability or cross-model results.
No production configuration or existing agent was changed. The routing run reused its recorded runtime binary. The action validation used
the separate 1.21.0 build identified above.

## Proposed next step

Keep this routing candidate unapproved against the new suite. First obtain
independently owned domain cases and propose deterministic checks for structural
validation and exception precedence, leaving language interpretation and proposed
instruction changes subject to the same governed lifecycle. Then evaluate that
design against a fresh holdout. These changes are proposals, not implemented by
this validation phase. Enterprise integration would not correct the routing
failures demonstrated here.
