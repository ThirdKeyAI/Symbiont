# Linux containment qualification

Landlock support depends on kernel features, executable compatibility and the
deployment's namespace and service policy. The published Linux x86-64 Symbiont
2.0.1 baseline has SHA-256
`8c4c385d83965c3af965340ae50482400337ae596bbc232f4beb9026ec915ad7`.
Current-source checks also used commit
`495d2b77a83ae6781d4e698ba476c82a5846cc96` with two release-profile builds:
the Ubuntu candidate with SHA-256
`feddcf2f752882f0c037d33a4097ded24a8937831dc6b664e0339fec6619299b`
and the Debian-compatible candidate with SHA-256
`ffbf2e8b9246beeb7ad4d703131c09bf9d790f483c33f1f1e7d5c78cad4d4c52`.
Results qualify only the binary named in each row.

| Recorded environment | Landlock ABI | Namespace policy | Result |
|---|---|---|---|
| Debian 13.7, kernel `6.12.113+deb13-amd64`, systemd 257; published 2.0.1 baseline | 6 | Distribution default permits unprivileged namespaces | Real restricted launch, private workspace and managed transport passed; all 35 broker/lifecycle/external-effect cases passed. |
| Same Debian image; current source built on Debian Bookworm, maximum required glibc 2.35 | 6 | Distribution default permits unprivileged namespaces | Platform diagnostic passed restricted launch, delegated cleanup, private workspace and managed transport. |
| Ubuntu 26.04.1, kernel `7.0.0-38-generic`, systemd 259; published 2.0.1 baseline | 8 | Default AppArmor restriction enabled | Native workspace setup refused with guidance; no host fallback. This is a passing refusal test, **not** a usable-platform qualification. |
| Same Ubuntu image, separate opted-in lab configuration; current-source Ubuntu candidate | 8 | Namespace restriction disabled inside the disposable guest | Real startup and cleanup passed; all 35 release-gate cases, registered-route admission and the bounded adaptive pilot passed. |

The Debian result exercises the advertised ABI floor. The Ubuntu comparison
demonstrates why detecting a newer ABI is insufficient: `symbi doctor` must test
the actual private workspace and transport. Services ran under a non-root user
with a systemd user manager and cgroup v2 delegation. Cloud images needed a local
test user and enabled user-manager lingering; that service setup is distinct
from namespace security policy.

The first current-source binary was built on a host with glibc 2.43 and could not
start on Debian 13 because its loader requires that newer ABI. Rebuilding the
same frozen source in Debian Bookworm produced a binary whose highest referenced
glibc version is 2.35, and that binary passed the Debian platform diagnostic.
The Linux x86-64 release job is therefore pinned to Ubuntu 22.04 and rejects an
artifact requiring a glibc version newer than 2.35. This makes distribution
compatibility an enforced publication property instead of an assumption about
the current hosted runner image.

## Scope and retained evidence

The labs used disposable QEMU guests with CPU emulation, no shared host mounts
and blocked external networking; only a localhost SSH forward was available for
the controller. The workers themselves used native Landlock. The VM was an
additional laboratory boundary, not a product requirement. Ubuntu's namespace
adjustment was confined to that guest and was recorded separately from its
default-policy result. No original host security policy was changed.

Official image downloads were checked against their publishers' HTTPS checksum
files. Detached image-checksum signatures were not verified. Image identities,
platform reports, evidence inventories and reproduction tools are retained in
[release assurance evidence](evidence/2026-10-25/release-assurance/README.md).

These results cover x86-64 and the recorded configurations. They do not establish
ARM64 execution, arbitrary desktop distribution compatibility, resistance to
kernel exploits or containment after host compromise. A distribution policy
refusal should be resolved through the operator's approved deployment policy;
the test guest's broad namespace switch is not an installer action.

## Reproduce the platform diagnostic

Run as the non-root service user under the systemd user manager, with Python
3.11+ and the verified executable. Supply the expected ABI from the selected
test image. The diagnostic does not alter kernel or distribution policy:

```sh
python3 scripts/test-landlock-platform.py \
  --binary /absolute/path/to/verified/symbi \
  --expected-binary-sha256 "$BINARY_SHA256" \
  --expected-abi 6 --expect usable --report /absolute/new/platform.json
```

For an image expected to deny native namespaces, use
`--expect namespace-refused`. Its report must say `qualified: false`, retain the
specific failure and confirm cleanup. A pass in this mode proves the refusal
behavior only. For execution evidence, follow the
[broker](containment-evidence.md), [lifecycle](containment-lifecycle-evidence.md)
and [external-effect](external-effect-evidence.md) procedures or run the combined
release driver in a suitably prepared disposable lab.
