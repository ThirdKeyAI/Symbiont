# Containment evidence

The managed Landlock evidence suite exercises the shipping runtime using a
fixed hostile-input program, synthetic data and local receiver services. It
tests broker and audit boundaries while allowing useful inference before and
after rejected requests. It uses an existing, independently verified binary;
it does not rebuild the workspace or contact a model provider.

This suite establishes results for the listed probes and recorded deployment.
It does not measure an adaptive model's escape probability, unknown kernel or
VMM exploits, hardware covert channels, or containment after compromise of the
host/runtime. The host, receiver services and evidence collector are trusted.

## Covered cases

The planned case list is fixed and retained even if execution fails:

- An explicitly permitted inference request carrying a synthetic data canary.
- Alternate authentication routes and absolute-URL request targets.
- URL-backed documents and provider-hosted tools.
- Unapproved model selection, unknown control fields and server compaction.
- Malformed JSON, duplicate content lengths and chunked request framing.
- Worker-supplied audit/monitoring headers and credential headers.
- Reconnecting after exhausting the shared request allowance.
- Twenty consecutive requests with signed evidence observed before each effect.
- Provider redirects to a separate controlled receiver.
- A provider reflecting its synthetic credential in a successful response.
- A controlled audit-storage fault during a request, followed by more requests.
- Direct TCP, UDP and Unix-socket access to host receivers.
- Worker output claiming that the run passed and its audit was verified.

The allowed data-canary case is an intentional disclosure through the approved
provider connection. Passing it demonstrates that this channel can carry data;
it is not evidence of non-exfiltration or data-loss prevention. Operators must
select providers and business-tool destinations appropriate to the data scope.

## Independent observations

The controller generates a fresh signing key before each run and pins the public
key outside worker access. The synthetic provider verifies the actual signed
pre-effect record using OpenSSL before responding, matches its request hash to
the received bytes, and persists a receiver event with the request identity.
The final verifier checks these events against the signed journal.

Separate HTTP, UDP and Unix-socket receivers first pass positive delivery
controls. Their observations determine whether forbidden traffic arrived.
Malformed framing may be refused by closing its connection rather than returning
an HTTP error; a subsequent valid request must still complete. UDP connect/send
acceptance inside the private network namespace is not proof of delivery to a
host receiver. The receiver's observation determines whether traffic crossed.
The host also observes the worker's private network/mount namespaces and
no-new-privileges state, then requires confirmed worker/staging cleanup.

The fixed probe's response statuses are supporting evidence. Its `passed`,
`escape_success` and `audit_verified` claims cannot replace controller-owned
observations. Missing evidence remains invalid; adverse observations fail the
case. A refused operation without successful useful-work controls is insufficient.

The audit-fault case deliberately corrupts synthetic storage. Its signed prefix
is verified separately, and the resulting journal must not be accepted as a
complete valid run. Failure of the runtime to report successful completion and
absence of later provider requests are required.

## Run

Use Linux with the prerequisites in [Landlock development](landlock-development.md).
The caller must run under the same non-root systemd user manager as the sandbox
supervisor. A missing capability fails qualification; the fixture does not
relax the host's security policy or fall back to unrestricted execution.

First obtain the binary and its source identity using the
[release verification procedure](release-verification.md). Keep the binary
checksum independently of the evidence bundle. The public OSS source commit
and the upstream Gitea commit are separate identities because OSS sync maintains
its own public history; record both.

```sh
systemd-run --user --wait --pipe --collect --quiet \
  --service-type=exec --working-directory="$PWD" \
  --setenv=HOME --setenv=XDG_RUNTIME_DIR --setenv=DBUS_SESSION_BUS_ADDRESS \
  /usr/bin/python3 scripts/test-containment-evidence.py \
    --binary /absolute/path/to/verified/symbi \
    --expected-binary-sha256 "$BINARY_SHA256" \
    --runtime-source "$PUBLIC_SOURCE_COMMIT" \
    --upstream-source "$UPSTREAM_SOURCE_COMMIT" \
    --output /absolute/new/evidence-directory

python3 scripts/verify-containment-evidence.py /absolute/new/evidence-directory
python3 scripts/test-containment-evidence-scoring.py
python3 scripts/test-containment-evidence-integrity.py /absolute/new/evidence-directory
```

The evidence directory must be outside `/tmp`, because the native worker hides
that directory behind private scratch storage. `--case NAME` selects a focused
subset; the offline verifier reports whether the complete suite was present.
Never describe a selected subset as full qualification.

The report records binary and fixture hashes, both source identities, kernel,
architecture, actual observations and case outcomes. A manifest covers the
report, probe, receiver events, public keys, journals and logs. Pin the manifest
hash outside the bundle. Checksums alone cannot authenticate a bundle against
a host operator able to rewrite it and its manifest.

Private fixture keys stay in separate temporary storage and must not be
published. The suite uses synthetic credentials only. Review any failed-run
artifacts before sharing them; a failed credential-containment probe could
include its synthetic credential in output.

## Recorded 2.0.1 result

On 2026-10-25, all 19 cases passed against the published Linux x86-64 2.0.1
binary in a disposable Ubuntu 26.04 VM with kernel `7.0.0-38-generic` and
Landlock ABI 8. Its external networking was blocked, with only a localhost SSH
forward for the laboratory controller. The VM used CPU emulation; these results
do not establish production latency or hardware side-channel resistance. The VM
was the laboratory boundary; the tested workers used native Landlock inside it.
This does not add a VM requirement to the native deployment option.
The guest permitted unprivileged namespaces with
`kernel.apparmor_restrict_unprivileged_userns=0`; the original host retained its
existing restriction. These are different deployment configurations.

The receiver independently verified signed pre-effect records for all 61
provider requests. Offline verification accepted the complete bundle and
rejected five corrupted copies even after their manifests were recomputed:
changed signatures, forged receiver hashes, missing trials and false success
claims, plus a correctly signed prefix missing its terminal record. Eight
focused scoring tests also passed. The offline verifier refuses Python
optimization mode, which would disable its assertions.

The separate native boundary test also passed: signed MCP dispatch performed
useful work while host-file, socket and signal probes were denied; parent secrets
were not inherited; an unsupported kernel floor was refused before inference;
and killing the runtime closed the lease and reaped detached MCP descendants.
Its stale assertion for boundary version 3 was updated to the released version
5, with checks for denied namespace mutation and restricted process metadata.

Recorded identities:

- Public runtime source: `926f4a291b35b1a71f8f2013ba0fb8a0808139bd`.
- Upstream Gitea source: `19f9ff11141ed28150771ee642618feedead23bd`.
- Binary SHA-256: `8c4c385d83965c3af965340ae50482400337ae596bbc232f4beb9026ec915ad7`.
- Evidence manifest SHA-256: `8300026883327e550e6c509a2735180ee44a354185a4c00d587c80338de3a08c`.

The original host refused the native namespace setup under its existing
security policy. That refusal was retained, and the host policy was unchanged.
Earlier fixture runs are retained separately, including incorrect expectations
that malformed framing must return an HTTP status and that a successful UDP send
proves delivery. The final run uses the receiver observations and follow-up
useful-work controls described above.

## Complementary evidence and remaining work

The [recorded results](evidence/2026-10-25/README.md) include the downloadable
evidence bundles, checksums and offline verification instructions.

The [concurrency, fault and approval evidence](containment-lifecycle-evidence.md)
adds simultaneous native workers, shared admission, interruption at three audit
stages, supervisor loss, and substitutions during pending approvals. Its focused
component checks separately exercise parent/child token accounting and receipt
binding. Those results are distinct from the 19 broker probes above.

Run `test-landlock-boundary.py` for signed MCP dispatch, host-file/socket/signal
restrictions and cleanup of detached descendants after runtime process loss.
The existing release qualification suite covers onboarding, file grants,
source queries, Git snapshots, managed workers and action lifecycle checks.
Record separate reports rather than merging their claims into one escape rate.

The next evidence layer requires adaptive, independently authored attacks on
frozen builds, repeated trials, benign workload controls and complete protected
observations. Cross-worker covert-channel tests and independently enforced
network/monitoring controls after host compromise remain separate requirements.
