# Outbound integration review qualification

The [retained workflow report](workflow.json) records **16 passing cases** through
the rebuilt CLI using synthetic credentials and local HTTP fixtures. It covers
the reconciled outbound feature, follow-up fixes and the further corrections
found during review. No external inference service or paid API was used.

The report records base revision `89f5e2dc7c22be38997c31febb0dbda19c97a89c`
plus the source digest of the reviewed working tree. That digest, rather than
the base revision alone, identifies the tested code. Binary, driver, verifier
and fixture configuration digests bind the other inputs. The controls catalogue
references this report; earlier reports remain historical records.

The workflow retains its original 13 cases covering reads, exact write approval,
denied writes, input and request limits, redirects, body reflection, explicit
file-store selection and failure, preflight refusal, and interrupted effects
with signed-journal inspection and replay refusal. Three additional cases prove:

- An ordinary encoded path parameter succeeds with explicit opt-in.
- A rendered parameterized dot segment is refused before reaching the service.
- Reflected response headers are rejected without contributing a body or header
  hash to the signed journal.

Focused validation also passed: 24 outbound tests, 10 HTTP transport tests,
3 serialization/IP regressions, 10 Landlock tests, 73 CLI tests and 3 local Vault
protocol tests. The transport tests include mixed-case reflected header names,
trimmed header-value whitespace, oversized/truncated responses, and legitimate
response evidence. Store tests cover concurrent credential resolution, secret-free
configuration digests, configured token/AppRole authentication, explicit TLS
settings and KVv2 version retrieval. A separate installed-CLI developer scaffold
check wrote and parsed `symbiont.toml` with unset credentials omitted.

Formatting, clippy for the affected CLI/runtime crates, six catalogue-checker
tests and all six documentation builds passed. The full workspace test suite was
not rerun. Vault fixtures validate the local protocol path, not a live Vault
deployment, Kubernetes authentication or an mTLS handshake.

This CLI workflow does not qualify server/scheduler routes, managed worker launch,
any sandbox boundary or a remote gateway. The separate Landlock test results do
not expand the workflow report's scope. The four associated controls remain
partially validated. Cancellation cannot undo an effect already accepted remotely.
Temporary journals and private signing material are not distributed; retained
journal digests and verifier outcomes are test records, not full transcripts.

Reproduce with Python 3.11+, OpenSSL with Ed25519 support, local loopback sockets,
a pseudo-terminal, and a CLI built from the source being assessed:

```sh
python3 scripts/test-outbound-capabilities.py \
  --binary /absolute/path/to/symbi \
  --report /absolute/new/workflow.json
python3 scripts/check-security-controls.py
python3 scripts/test-security-controls.py
```

The driver refuses to overwrite evidence. The catalogue checker validates
structure, source references, report integrity and claim scope; it does not rerun
the workflow or independently establish that recorded results are true.
