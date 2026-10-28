# Recorded containment results — 2026-10-25

These bundles retain the signed journals, controller observations, verification
results and reproduction tools for the published Linux x86-64 Symbiont 2.0.1
binary. Each archive includes its own file inventory and earlier fixture runs
under `history/`; only its `evidence/` directory is the final qualification.

| Suite | Result | Details | Bundle |
|---|---|---|---|
| Broker containment | 19 E2E cases passed; five artifact-tampering challenges rejected | [Report](../../containment-evidence.md), [summary](broker-summary.json), [verification](broker-verification.json) | [Download](symbiont-2.0.1-containment-evidence.tar.gz) |
| Concurrent workers, interrupted actions and approvals | 11 E2E cases and 13 focused Rust tests passed; four artifact-tampering challenges rejected | [Report](../../containment-lifecycle-evidence.md), [summary](lifecycle-summary.json), [verification](lifecycle-verification.json) | [Download](symbiont-2.0.1-lifecycle-evidence.tar.gz) |

The lifecycle run contains 16 distinct governed invocations, five retry checks
and 28 requests to a synthetic local provider. Its focused Rust tests separately
cover parent/child accounting and prepared-call binding. These counts describe
different layers of evidence and must not be combined into an escape rate.

## Verify the retained results

From this directory, verify the checksums and extract the bundles:

```sh
sha256sum -c SHA256SUMS
tar -xzf symbiont-2.0.1-containment-evidence.tar.gz
tar -xzf symbiont-2.0.1-lifecycle-evidence.tar.gz
```

With Python 3.11+ and OpenSSL supporting Ed25519, verify each extracted bundle:

```sh
python3 symbiont-2.0.1-containment-evidence/reproduce/scripts/verify-containment-evidence.py symbiont-2.0.1-containment-evidence/evidence
python3 symbiont-2.0.1-lifecycle-evidence/reproduce/scripts/verify-containment-lifecycle.py symbiont-2.0.1-lifecycle-evidence/evidence
```

These commands check existing artifacts without starting workers or inference.
The archive READMEs include the corresponding tamper-check commands. Both
archives were checked against their inventories, then their offline verifiers
and tamper checks were rerun before this commit; see
[archive validation](archive-validation.json), [broker integrity](broker-integrity.json)
and [lifecycle integrity](lifecycle-integrity.json).

The binary SHA-256 is
`8c4c385d83965c3af965340ae50482400337ae596bbc232f4beb9026ec915ad7`.
Fixture commits are `35493482dc11c408b856c800f2d7f57b259615ef` for the broker suite
and `274949dc1cb7d8ef3c04276a65da126a761b1b7b` for the lifecycle suite. Their reports
pin the final evidence manifest hashes. Retain the signed commit identities
through a trusted channel: recomputable checksums alone cannot authenticate
evidence against an operator replacing every artifact.

Private keys, VM disks and runtime binaries are excluded. The disposable test
VMs were removed; native Landlock contained the workers, and the original host
security policy remained unchanged. Results apply to these fixed probes and the
recorded deployment. They do not establish adaptive attacker resistance,
unknown-exploit resistance or containment after host compromise.
