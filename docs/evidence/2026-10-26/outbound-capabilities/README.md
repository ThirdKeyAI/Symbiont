# Optional outbound workflow qualification

The [retained report](workflow.json) records **13 passing cases** on Linux using
the built CLI, synthetic credentials and two local HTTP fixtures. No external
provider or paid API was used. The new source was built on revision
`19f9ff11141ed28150771ee642618feedead23bd`; that base revision alone does not
contain this feature. The report additionally binds the changed source, tested
binary, fixture configuration, driver and independent verifier by SHA-256.

The workflow exercised:

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
journals and signing material are not distributed. Re-run the driver to generate
and verify a fresh fixture; the retained report is a test record, not an
independently verifiable transcript of every historical socket exchange.

Additional focused checks passed: nine outbound authority/configuration and
live-dispatch tests, seven HTTP transport tests, six prepared-authorization tests,
13 manifest regressions, seven protected-inference tests and six tests of the
catalogue checker's integrity and scope rules. CLI linting and runtime linting with no default features
and all targets passed with warnings denied. The CLI build and formatting checks
passed. These checks supplement, rather than expand, the report's declared route
and backend scope.

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

This report was produced for the initial outbound-capabilities source. Later
source changes are not covered by it; see the
[2026-10-27 report](../../2026-10-27/outbound-capabilities/README.md) for the
follow-up fixes. The catalogue checker reports whether a retained report's
source digest matches the current checkout.
