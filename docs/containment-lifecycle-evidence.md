# Concurrency, fault and approval evidence

On 2026-10-25, all 11 lifecycle cases passed against the published Linux x86-64
2.0.1 binary. They comprised 16 distinct governed invocations, five retry checks
and 28 requests to a local synthetic provider. Thirteen focused Rust tests also
passed against the release source: four shared-ledger tests, six prepared-call tests and three protected
parent/child budget integration tests. No runtime code changed for this work.

## Results

| Area | Exercised behavior | Observed result |
|---|---|---|
| Concurrent isolation | Two native workers with distinct input values, separate file grants and signing keys | Each produced its own expected calculation. Reads and writes to sibling inputs/keys and adjacent files, supervisor-channel access and signalling the controller were denied. Protected file hashes stayed unchanged, parent secrets were absent, and the controller observed separate mount namespaces. |
| Shared admission | Two active workers fill a pool limited to two workers, 1 GiB and two CPUs; a third requests admission | The third created no worker or output. After cleanup, a fresh invocation succeeded. |
| Audit failure before dispatch | Limit the runtime's file size at the current signed journal length before the first inference response | No tool dispatch, worker or output; the invocation required reconciliation. |
| Audit failure during an action | Apply the same write limit after signed dispatch and worker observation, before publication | The worker stopped and its output was not published. Retry did no work and required reconciliation. |
| Audit failure after publication | Apply the write limit after the approved output exists, before the final inference response | The published output remained, the journal stayed incomplete, and retry required reconciliation without repeating work. |
| Provider disconnection | Close the final inference connection after confirmed file publication | The runtime recorded a terminal error. Retry returned that recorded failure without another request or file effect. |
| Supervisor loss | Kill the supervisor while the worker is running, then restart using retained state | No output was published; the original invocation required reconciliation. Fresh work succeeded after cleanup. |
| Manifest substitution | Replace the tool manifest while its approval is pending | The captured original command executed; the replacement did not. |
| Input substitution | Replace source bytes while their approval is pending | Execution used the approved input snapshot and produced the original calculation. |
| Output substitution | Insert a symlink at the approved output path while approval is pending | Publication failed; the symlink and protected target remained unchanged. |
| Argument and run replay | Present an earlier approval ID for a different output and for a different pending run | The original approved effect occurred once. The substituted output and other run produced no effects. |

Some cases contain multiple invocations or checks. These counts should remain
separate from the original broker suite and must not be treated as independent
samples of an escape rate.

The Rust checks establish a different layer of evidence. Eight simultaneous
sibling ledger users cannot reserve more than the parent balance. Confirmed
child usage is deducted from that balance; cancellation and missing usage retain
uncertain charges. The protected delegation tests exercise real reasoning loops,
parent/child signed journals, cancellation and a child timeout while parent and
sibling reservations remain live. They use an in-process synthetic provider;
they are not additional shipping CLI or HTTP route qualification runs.

## How evidence is checked

The [recorded results](evidence/2026-10-25/README.md) retain the complete evidence
bundle, checksums and offline verification results in the repository.

Each invocation has a controller-provisioned signing key whose public half is
pinned before startup. The local provider verifies the signed journal and
observes a budget reservation before responding. These ordinary CLI reservations
hash a typed provider contract; the separately recorded HTTP request hash must
not be described as the same signed wire-byte hash.

The host records terminal decisions, real worker identities, shared capacity,
published files and retries. The offline verifier checks signatures and chains,
run identity, approval receipts against displayed calls, and actual output bytes
against signed publication hashes. Missing cases, substituted output bytes,
substituted approval fingerprints and a signed prefix missing its terminal
record were rejected even after the artifact manifests were recomputed. Python
optimization mode is refused.

The deliberate audit-write faults are different: their valid signed prefixes
are retained as incomplete evidence. They must accompany failed execution and
refused replay. A recorded terminal provider error is instead a known failed
outcome that can be returned on retry. Neither outcome grants permission to
repeat an effect.

The write-failure injection sets `RLIMIT_FSIZE` on the runtime process to the
current journal length and ignores `SIGXFSZ`, causing later extending writes to
fail. It affects other regular files opened by that process too. It is a bounded
write-failure test, not a simulation of every disk-full, filesystem or power-loss
failure. Substitutions are made by the trusted test controller to exercise stale
approval binding; the worker is not granted access to configuration or approval
state.

## Reproduce

Use the same native Linux prerequisites and verified-binary procedure as the
[broker evidence suite](containment-evidence.md). Run under the non-root systemd
user manager that owns the supervisor:

```sh
systemd-run --user --wait --pipe --collect --quiet \
  --service-type=exec --working-directory="$PWD" \
  /usr/bin/python3 scripts/test-containment-lifecycle.py \
    --binary /absolute/path/to/verified/symbi \
    --expected-binary-sha256 "$BINARY_SHA256" \
    --output /absolute/new/evidence-directory

python3 scripts/verify-containment-lifecycle.py /absolute/new/evidence-directory
python3 scripts/test-containment-lifecycle-integrity.py /absolute/new/evidence-directory
```

`--case NAME` selects a focused subset. The verifier reports whether the complete
case list was present; the artifact-integrity challenges require the full suite.
Private fixture signing keys stay outside the evidence directory and are removed
after the service stops. The terminal approver is a synthetic local test operator,
not evidence of independent human review.

Run the focused Rust checks from the release source checkout:

```sh
features=http-api,http-input,cloud-llm,mcp-client,cedar,vector-lancedb,vector-qdrant
cargo test --locked -p symbi-runtime --features "$features" --lib reasoning::budget::tests -- --test-threads=1
cargo test --locked -p symbi-runtime --features "$features" --lib reasoning::prepared::tests -- --test-threads=1
cargo test --locked -p symbi-runtime --features "$features" --test shared_budget -- --test-threads=1
```

Recorded identities:

- Public runtime source: `926f4a291b35b1a71f8f2013ba0fb8a0808139bd`.
- Upstream and focused Rust-test source: `19f9ff11141ed28150771ee642618feedead23bd`.
- Binary SHA-256: `8c4c385d83965c3af965340ae50482400337ae596bbc232f4beb9026ec915ad7`.
- Final lifecycle manifest SHA-256: `9f4b2092c03bc2f6af7d3a570e0999d5d2c89ed1afa943e3462dcc581e42d56b`.

The E2E laboratory used Ubuntu 26.04, kernel `7.0.0-38-generic`, Landlock ABI 8,
and a disposable VM with CPU emulation and blocked external networking. The guest
permitted unprivileged namespaces; the original host security policy was
unchanged. The VM was the laboratory boundary; the workers used native Landlock.
The VM and its private keys and disks were removed after collecting the evidence.

Earlier runs are retained separately. Fixture corrections addressed observing
the trusted launcher before restrictions were applied, restarting a helper that
expected a new directory, distinguishing a cached terminal failure from an
unresolved invocation, and retaining the correct protected-file hashes in the
report. Concurrency probes were strengthened to use distinct inputs and attempt
both reads and writes. These changes are not runtime fixes or additional passing
trials.

These fixed probes do not establish adaptive attacker resistance, unknown-exploit
resistance, cross-worker covert-channel absence, arbitrary remote-transaction
exactly-once behavior, or containment after host compromise. The host collector
remains trusted. Keep the manifest hash separately: a recomputable manifest alone
cannot authenticate evidence against an operator replacing the entire bundle.
