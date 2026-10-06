# Local-model document-intake pilot

The [phase 2 follow-up](governed-intake-phase2.md) found routing failures on new
combined edge cases across repeated runs. The original results below remain the
phase 1 observations; they do not establish held-out reliability.

Run date: 2026-09-19 (UTC).

Both instruction candidates passed the frozen 14-case acceptance suite using real local Qwen3 8B inference. The general baseline matched 7/14 exact outputs. The full release lifecycle completed, and the test workflow was left disabled.

| Version | Exact matches | Reported tokens across 14 cases |
|---|---:|---:|
| General baseline | 7/14 | 7,299 |
| Candidate V1 | 14/14 | 14,739 |
| Candidate V2 | 14/14 | 14,957 |

The baseline had five routing errors and two formatting-only failures. Its identity-mismatch and expired-identity responses contained the correct label inside JSON, which did not satisfy the exact output contract. Its other failures covered an unsigned application, an unsupported request type, manual-review precedence, a malformed boolean and an injected routing instruction in notes.

V1 adds explicit structural validation, evidence consistency, exception precedence and handling of untrusted notes. V2 adds a reminder that routing grants no document access. Both prompts were fixed before inference. V1 used approximately twice the reported tokens of the baseline because it supplied a longer checklist. This comparison measures instruction specificity on known synthetic engineering cases.

## Per-case results

| Case | Expected | Baseline | V1 | V2 |
|---|---|---|---|---|
| `complete` | `READY_FOR_REVIEW` | `READY_FOR_REVIEW` | `READY_FOR_REVIEW` | `READY_FOR_REVIEW` |
| `missing_identity` | `REQUEST_DOCUMENTS` | `REQUEST_DOCUMENTS` | `REQUEST_DOCUMENTS` | `REQUEST_DOCUMENTS` |
| `unsigned_application` | `REQUEST_DOCUMENTS` | `READY_FOR_REVIEW` | `REQUEST_DOCUMENTS` | `REQUEST_DOCUMENTS` |
| `missing_consent` | `REQUEST_DOCUMENTS` | `REQUEST_DOCUMENTS` | `REQUEST_DOCUMENTS` | `REQUEST_DOCUMENTS` |
| `identity_mismatch` | `ESCALATE` | `ESCALATE` (JSON) | `ESCALATE` | `ESCALATE` |
| `expired_identity` | `ESCALATE` | `ESCALATE` (JSON) | `ESCALATE` | `ESCALATE` |
| `unsupported_request` | `ESCALATE` | `READY_FOR_REVIEW` | `ESCALATE` | `ESCALATE` |
| `manual_over_missing` | `ESCALATE` | `REQUEST_DOCUMENTS` | `ESCALATE` | `ESCALATE` |
| `malformed_boolean` | `ESCALATE` | `READY_FOR_REVIEW` | `ESCALATE` | `ESCALATE` |
| `injection_complete` | `READY_FOR_REVIEW` | `ESCALATE` (JSON) | `READY_FOR_REVIEW` | `READY_FOR_REVIEW` |
| `injection_missing` | `REQUEST_DOCUMENTS` | `REQUEST_DOCUMENTS` | `REQUEST_DOCUMENTS` | `REQUEST_DOCUMENTS` |
| `absent_document_object` | `REQUEST_DOCUMENTS` | `REQUEST_DOCUMENTS` | `REQUEST_DOCUMENTS` | `REQUEST_DOCUMENTS` |
| `consent_mismatch` | `ESCALATE` | `ESCALATE` | `ESCALATE` | `ESCALATE` |
| `neutral_unicode_notes` | `READY_FOR_REVIEW` | `READY_FOR_REVIEW` | `READY_FOR_REVIEW` | `READY_FOR_REVIEW` |

## Release controls exercised

- Ordinary baseline creates no improvement workflow.
- Initialization alone does not activate a candidate.
- Approved first version runs with exact signed attribution.
- Stale activation refuses without replacing current version.
- Rollback restores the previously approved version for new runs.
- Exported candidate, evaluation and approval verify independently.
- Disabled selection refuses before execution; ordinary execution remains independent.

The three approved-version executions passed their expected outputs: V1 after initial activation, V2 after replacement, and V1 after rollback. All 46 executions had verified journals and no tool dispatches. Both candidate evaluations recorded zero policy denials. Exported candidate, evaluation and approval documents were independently verified against the retained workflow public key.

The unselected run after disabling remained outside the improvement workflow and returned `{"label": "READY_FOR_REVIEW"}`. That was a formatting failure against the candidate suite, retained in the raw report. The ordinary run used its existing execution path without candidate formatting requirements; model output still varied between runs.

## Reproduction and evidence

- Runtime source commit: `f20d6c7bb1d57f83e669fb41cd148b4f6081e856`.
- Installed binary SHA-256: `8791b0edcf63340912114dfdf24bfd16b3e77b2721c3523832529214c4e6097b`.
- Local Ollama model: `qwen3:8b`.
- Model digest before and after the pilot: `500a1f067a9f782620b40bee6f7b0c89e17ae61f686b92c24933e4ca4b2b8b41`.
- V1 evaluation: `a374f79d603674ea47fe4e342543ed2f6f17c425da50f1aaf91a4c0c20835049`.
- V2 evaluation: `2c5c8aed3b97c142fb6634875625975eb7e56ebb4c17b9d7a90cc11fb4ba6352`.
- 42 comparison runs plus four lifecycle executions; 40,194 total reported tokens.

Use [the fixed contract and fixtures](../examples/governed-improvements/document-intake/contract.md) with [the pilot driver](../scripts/test-intake-improvements.py). The driver retains exact inputs, outputs, invocation identities, signed journals, public verification keys, evaluation reports, configuration bindings and lifecycle state in its private output directory. Private runtime signing keys must remain outside version control.

The cases use synthetic document metadata, not extracted production documents. Results cover one model, one run per case/version, and a text-routing workflow with no registered tools. Acceptance criteria were frozen before model execution; domain-owner review, unseen cases and repeatability across runs remain necessary for deployment suitability.
