# External effects and lost acknowledgements

Five end-to-end cases on the published Linux x86-64 2.0.1 binary confirm that a
lost acknowledgement does not authorize another execution under the same
invocation ID. A synthetic HTTP service records an accepted order before each
fault. It does **not** deduplicate requests: a second delivery would create a
second observable effect.

| Case | Observed runtime behavior |
|---|---|
| Complete acknowledgement | Saved success is returned on retry; no repeated effect or inference. |
| Connection closes after acceptance | Invocation requires reconciliation; same-ID retry is refused. |
| Response ends partway through its body | Invocation requires reconciliation; same-ID retry is refused. |
| Receiver accepts, then exceeds the tool timeout | Invocation requires reconciliation; same-ID retry is refused. |
| Runtime is terminated after acceptance | Invocation requires reconciliation; same-ID retry is refused. |

Each case produced one accepted order and one exact approval. Each fault case
refused a stale operator review, accepted a review bound to the current snapshot
and receiver evidence, and returned the signed receipt on a subsequent retry.
The original journal did not change. No additional model request occurred after
an uncertain result. The operator and model provider were synthetic fixtures.

## Evidence and independent checks

The receiver checks the signed pre-request checkpoint before recording an
acceptance. Its observation binds the invocation ID, request ID, HTTP method,
destination, explicit headers, body and journal-prefix hash. The request identity
uses the runtime's application-level request format; it is not a packet capture
or a hash of transport-added headers.

The offline verifier checks the complete inventory, pinned public key, journal
signatures, exact approval, provider observations and signed reconciliation
receipt. Five integrity challenges alter the case set, receiver record, approval,
receipt payload or receipt signature, then recompute the outer inventory. All
must still be rejected. Assertions disabled with `python -O` are also refused.

Recorded artifacts and reproduction files are retained under
[release assurance evidence](evidence/2026-10-25/release-assurance/README.md).

## Reproduce

Use a disposable Linux lab with the Landlock prerequisites, Python 3.11+ and
OpenSSL supporting Ed25519. Assign `198.18.0.1/32` to its loopback interface before
running the driver as a non-root user under its systemd user manager. The fixture
binds that benchmarking address before offering any tool and has no real external
business endpoint or model service. Keep the lab disconnected from external
networks. Obtain and verify the release binary separately.

```sh
python3 scripts/test-external-effect-recovery.py \
  --binary /absolute/path/to/verified/symbi \
  --expected-binary-sha256 "$BINARY_SHA256" --output /absolute/new/effects
python3 scripts/verify-external-effect-recovery.py /absolute/new/effects
python3 scripts/test-external-effect-integrity.py /absolute/new/effects
```

This establishes behavior for five controlled faults on the recorded route and
binary. It does not establish arbitrary-service exactly-once execution, prevent
duplicate work submitted with new IDs, authenticate an operator's external
claims, or assess adaptive attacks. Application-specific reconciliation and
destination-side idempotency remain necessary where the business process needs
them. The trusted collector and retained commit identity remain part of the
evidence trust boundary.
