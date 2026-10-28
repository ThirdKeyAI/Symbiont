# Optional outbound workflow qualification (follow-up fixes)

This historical qualification is superseded by the [review qualification](../outbound-review/README.md), which covers the reconciled feature and further fixes.

The [retained report](workflow.json) records **13 passing cases** on Linux using
the built CLI, synthetic credentials and two local HTTP fixtures. No external
provider or paid API was used. It was produced at revision
`ae88ca6f4ee21cccde0a02f0b86bd4a714f8a759`, which contains the outbound
capability follow-up fixes (bounded run registry, closed-run guard, credential-free
request hash, path-parameter and query-key checks, per-lease credential
resolution and session-bound assignment epochs). The report binds that source,
the tested binary, fixture configuration, driver and independent verifier by
SHA-256. It supersedes the [2026-10-26 report](../../2026-10-26/outbound-capabilities/README.md)
for the current source; the earlier report remains as a record of the initial
build.

The workflow exercised the same cases as the earlier report:

- Named reads across separate executions, with distinct assignment epochs.
- An exact terminal-approved write and refusal of a denied write.
- Typed-input rejection before any application request.
- A request allowance that permits one call and refuses the next.
- Refusal to follow a redirect or return upstream error details.
- Credential-reflection refusal before delivery to the CLI or model.
- Explicit file-store retrieval while a different environment value is present.
- Missing store credentials without an environment fallback or application call.
- Invalid profile versions and out-of-capability paths rejected before inference.
- Process interruption after the service receives a request, independent signed
  journal verification, incomplete-outcome inspection and refusal to replay the
  same invocation.

The report retains service receipts without authentication values, journal
digests, independent-verification outcomes and inspector exit codes. Temporary
journals and signing material are not distributed. The retained report is a test
record, not an independently verifiable transcript of every historical socket
exchange.

This report does **not** qualify HTTP server or scheduled entry points, a Vault
deployment, managed worker launch, browser execution, any sandbox boundary or a
remote gateway. Cancellation can stop further local admission and delivery; it
cannot undo a request already received by a remote service. The catalogue keeps
these coverage gaps explicit.

Reproduce from the repository root with Python 3.11+, OpenSSL with Ed25519 support
and a built CLI:

```sh
python3 scripts/test-outbound-capabilities.py \
  --binary /absolute/path/to/symbi \
  --report /absolute/new/workflow.json
python3 scripts/check-security-controls.py
python3 scripts/test-security-controls.py
```

Only the driver executes the workflow and records its results.
`check-security-controls.py` checks the catalogue's structure, source references,
report fields and report digests, and that validation claims stay within the
attached report's scope. It does not run any test or confirm that a report's
recorded results are true. `test-security-controls.py` tests the checker itself.

The workflow needs local loopback sockets and a pseudo-terminal for the approval
fixture. It refuses to overwrite a previous report. A different build or host
requires new evidence; its result must not be inferred from this one.
