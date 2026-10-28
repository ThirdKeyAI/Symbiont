# Current-source release assurance — 2026-10-25

This directory records the first combined qualification of the native Landlock
release gate, registered-route admission, Linux platform checks and a bounded
adaptive pilot. The frozen runtime source is commit
`495d2b77a83ae6781d4e698ba476c82a5846cc96`. The Ubuntu runs used the x86-64
candidate binary with SHA-256
`feddcf2f752882f0c037d33a4097ded24a8937831dc6b664e0339fec6619299b`.
The Debian platform run used the same source built on Debian Bookworm, with
SHA-256 `ffbf2e8b9246beeb7ad4d703131c09bf9d790f483c33f1f1e7d5c78cad4d4c52`.

| Check | Recorded result | Retained artifact |
|---|---|---|
| Publication containment gate | 35 of 35 cases passed: 19 broker, 11 lifecycle and five external-effect cases. The independent verifiers and deliberate artifact-tampering challenges also passed. | [Public evidence archive](current-source-containment-evidence.tar.gz), [qualification](ubuntu-qualification.json) |
| Registered execution admission | HTTP, scheduler and separate CLI processes shared one capacity ledger; conflicting profiles and over-capacity starts were refused before inference or worker creation; useful work resumed after capacity was released. | [Admission report](registered-route-admission.json) |
| Ubuntu platform diagnostic | Ubuntu 26.04.1, kernel `7.0.0-38-generic`, Landlock ABI 8, x86-64: restricted launch, delegated cleanup, private workspace and private-loopback transport passed after the disposable guest explicitly opted into unprivileged namespaces. | [Platform report](ubuntu-platform.json) |
| Debian platform diagnostic | Debian 13.7, kernel `6.12.113+deb13-amd64`, Landlock ABI 6, x86-64: the Debian-compatible build passed restricted launch, delegated cleanup, private workspace and private-loopback transport under the distribution's default namespace policy. | [Platform report](debian-platform.json), [build compatibility](build-compatibility.json) |
| Bounded adaptive pilot | 12 of 12 interface scenarios passed in three conditional rounds. The stop rule was not triggered and the independent verifier pinned the plan, manifest and binary hashes. | [Plan](adaptive-plan.json), [report](adaptive-report.json), [verification](adaptive-verification.json) |

The registered-route report covers the public API and scheduler path added in
this branch as well as a separate CLI process. It verifies combined admission
capacity rather than treating each entry route as an independent pool. Its
signed journals correlate refusals and cleanup without recording false cleanup
failures.

The host-built candidate required glibc 2.43 and was rejected by the Debian
loader before Symbiont started. Rebuilding the same frozen source in Debian
Bookworm produced a release-profile binary whose highest required glibc version
is 2.35; the Debian platform report qualifies that build. The release workflow
now pins its Linux x86-64 builder to Ubuntu 22.04 and rejects artifacts above the
2.35 ceiling so a moving hosted-runner alias cannot silently raise the runtime
requirement.

The adaptive plan was authored from the interface and threat model without
implementation or prior-result access. Existing fixed probes then mapped its
scenarios to executable tests. It uses synthetic services and data, makes no
paid provider calls and is bounded to 12 attempts. This is useful independent
scenario selection and response-driven sequencing; it is not an escape rate,
an unknown-vulnerability assessment or evidence against kernel compromise.

## Offline verification

From the repository root, first check every retained file:

```sh
cd docs/evidence/2026-10-25/release-assurance
sha256sum -c SHA256SUMS
mkdir /tmp/symbiont-current-containment
tar -xzf current-source-containment-evidence.tar.gz \
  -C /tmp/symbiont-current-containment
cd ../../../..
```

Then run the independent verifiers with Python 3.11+ and OpenSSL Ed25519
support:

```sh
python3 scripts/verify-containment-evidence.py \
  /tmp/symbiont-current-containment/containment/broker
python3 scripts/verify-containment-lifecycle.py \
  /tmp/symbiont-current-containment/containment/lifecycle
python3 scripts/verify-external-effect-recovery.py \
  /tmp/symbiont-current-containment/containment/external-effects
```

The archive contains public evidence only. Private signing fixtures, VM disks
and runtime binaries are excluded. The archive verifier proves internal
consistency against the retained manifest; a trusted release channel must still
pin the commit, binary and manifest hashes because an operator able to replace
all artifacts is outside this verifier's trust boundary.

The lab used disposable QEMU guests with no shared host mounts and no external
guest networking. Its controller reached each guest through a localhost SSH
forward. Native Landlock contained the worker inside the guest; the VM was an
additional test boundary, not a product requirement. Ubuntu's namespace setting
was changed only inside its disposable guest, and the report confirms it did not
change while the test ran.

This evidence qualifies only the recorded source, executable and environments.
It does not qualify ARM64, macOS, Windows, arbitrary third-party command-line
tools or installations with different namespace and service policies. A release
workflow must rebuild and rerun the gate for its exact publication commit and
binary; these local results cannot substitute for a hosted release check.
