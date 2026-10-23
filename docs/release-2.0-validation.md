# 2.0 release preparation

Validation date: 2026-10-10. This branch prepares the main workspace crates at
2.0.0 because strict ToolClad contracts and gated Rust execution APIs break
compatibility. It does not tag or publish a release. Independently versioned,
unchanged helper crates keep their versions. Governed improvements and the other
optional capabilities remain opt-in.

## Invocation lock lifetime

A deterministic regression retains a duplicated descriptor while the invocation
owner drops or completes. Before this change, inspection still refused the
invocation as owned: closing one descriptor does not release a `flock` while
another descriptor retains the same open file description. Fork inheritance can
create this condition until the child closes or executes, including with
close-on-exec descriptors.

Invocation, store and inspection guards now explicitly unlock when their owning
scope exits. A PID check prevents a forked child dropping an inherited guard from
unlocking its live parent's claim. Completion consumes the owner in the blocking
call before its result becomes visible to the async caller. Durable invocation
records still prevent replay; releasing a lock does not grant a new execution or
establish that an external effect completed.

The deterministic owner regression failed before the fix and passes afterward.
All five invocation unit tests also passed in 20 focused repetitions (100 test
executions) while the workspace was building, including the new store and
inspection guard regression.
Forty focused runs of the older reconciliation integration suite did not reproduce
the intermittent failure described in [1.21 validation](release-1.21-validation.md).
The reproduced descriptor case therefore establishes a concrete lifetime defect,
not the sole cause of that historical load-dependent failure.

## Rust verification

Workspace build, formatting and all-target Clippy passed, with Clippy warnings
denied. Runtime Clippy also passed with every feature enabled and with default
features disabled. The explicit `mcp-client,toolclad-direct` integration target
passed both real stdio fixture checks, including fail-closed missing-server
handling; shipping binaries still exclude the direct fixture feature.

All 75 workspace unit and integration test targets passed: 2,407 tests passed
and 67 retained their explicit ignore markers. The initial run stopped at the
RAG end-to-end test's 500 ms timing assertion, measuring 787 ms while all-feature
Clippy was compiling. After builds finished, the full eleven-test RAG suite passed
serially without changing its threshold or implementation. The other 26 unrun
targets then passed using the exact Cargo-produced executables, package working
directories and serial test execution. Previously passing targets were not
repeated. The ordinary ignored external-service cases are not represented as
passed; actual Linux isolation is covered separately below.

Workspace documentation tests also passed: four passed and three retained their
explicit ignore markers.

## Local Linux E2E

The complete release driver passed all six groups on Linux 7.0.0-38-generic,
with Landlock ABI 8 and systemd 259:

- 13 onboarding checks, including concurrent automatic service startup,
  installation, configured executables, refusals and generated-project execution.
- 11 filesystem cases covering declared input/output grants, substitution,
  descendants, size limits and denied access.
- Nine source-broker cases covering useful reads, literal search, listing,
  traversal, symbolic/hard links, missing ceilings, denied content and scratch
  worker isolation.
- Five Git cases covering log, diff, snapshot, capacity refusal and crash cleanup.
- Two managed CLI cases through private adapters and supervised workers.
- Eight action checks covering exact approval, denial, missing relay,
  interruption, signed reconciliation without replay and new approved recovery.

The CLI reported `symbi 2.0.0`; its SHA-256 was
`9b20f7f6e06a794ce5bd7051610dcf7dca2385391b2fa48114aacb0c71cead10`, unchanged
throughout qualification. This was a debug build from the development tree, with
its dirty status explicitly recorded. No remaining workers or leases were found.
The first complete attempt rejected an old source-test interpreter manifest;
migrating that fixture to explicit argv and interpreter opt-in made its nine
cases and the complete rerun pass. Runtime restrictions were preserved.

Workspace build and Clippy across all targets passed with warnings denied.
Every shipped tool manifest and the replacement example in the migration guide
passed the built CLI's validation command. Reports, journals, private fixtures
and logs are retained outside the repository.

## Release qualification

### Operational improvement follow-up

The optional operational integration was validated with focused checks after the
baseline qualification above. Twelve improvement and administrative API tests,
seven invocation tests, 32 HTTP tests and 20 Console approval tests passed. One
existing HTTP test retains its ignore marker. Forty-seven UI/client tests,
TypeScript checking, lint and the production UI build passed. Workspace all-target
Clippy and Console all-target Clippy with `runtime-http` passed with warnings denied.
The workspace build and default-feature-disabled runtime check also passed.
The broad workspace test suite was not repeated for this follow-up.

The installed CLI/HTTP/scheduler fixture passes 18 checks with local synthetic
inference. It exercises explicit version selection, DSL cron registration and a
manual trigger, signed attribution, queue and saved-result behavior, promotion,
rollback, disablement, exact-current-version checks and refusals. Independent
temporary Ed25519 keys exercise actual CLI/API receipt import, pinned trust,
reviewer tampering and the separation of approval from activation. The final
workspace debug binary passed the complete fixture with SHA-256
`01af646786b5bf10df309015e0334a39b37f5b10f2e35a790e9f1f9522f4dd03`.

Console tests use the real router, authenticated sessions, approval storage,
signing and HTTP connector with a synthetic runtime transport. They verify the
review/decision/import/activation sequence, exact evidence binding, authenticated
reviewer identity, CSRF, role and workspace restrictions, and denied egress.
Core store tests separately verify signed receipt acceptance against actual
passing evaluation evidence. This is not a deployed identity-provider or browser
certification; the UI checks use its test harness. Workspace switching discards
prior controls, and requests retain their displayed workspace binding.

No new live-model quality, regulatory suitability or sandbox compatibility claim
is made by these synthetic checks. The registered execution sandbox matrix is
unchanged; only explicitly selected ordinary ORGA agents consume these versions.

### Publication gate

Publishing crates, release binaries and release containers now requires successful
Test and Linux Landlock release qualification workflows for the exact canonical
main SHA. Main container builds require Test. Jobs check out the qualified SHA;
PR runs, different commits, missing evidence, and a failed or unfinished latest
run cannot authorize publication. The manual Linux workflow must be run on the
intended main commit before tagging.

The Linux driver exercises onboarding, filesystem grants, source requests, Git
snapshots, managed CLI containment, approved and denied actions, interruption and
operator reconciliation. It uses synthetic providers with actual kernel isolation
and requires the host prerequisites instead of skipping unavailable containment.
Reports bind the source revision, dirty-tree status, binary hash, driver hashes
and host details. Only top-level reports and logs are public artifacts; private
action fixtures contain signing keys and must not be uploaded.

Offline gate regressions cover exact-source matching, both required workflows,
failure and rerun precedence, and unreadable or ambiguous evidence. Workflow
syntax, expression and shell linting pass. The fourteen-crate publication graph and
the offline release-tool regressions pass.

Local tests do not qualify a GitHub release commit. The manual GitHub job must
pass on the intended release commit; the completed 2.0.0 qualification is recorded
below. This evidence does not claim macOS/Windows Landlock support, third-party CLI
compatibility, model quality, human approval quality, or exactly-once effects in
arbitrary services.

## Combined security and runtime integration — 2026-10-23

The integration includes the optional prompt-cache branch, the security follow-up,
the 2.0 preparation and the sandbox documentation updates. New instruction-review
routes now consume the middleware's authorized tenant/workspace and require matching
runtime ownership for listing, inspection, reviews, receipts and activation. Their
HTTP transport reuses the shared DNS/IP policy instead of creating a separate client.
The release-binary job also requires its reviewed tag SHA to equal the qualified SHA.

Workspace build and all-target Clippy passed with warnings denied. The 21 Console
approval API tests passed, including the complete signed-review/import/activation
flow and requests from foreign tenants, mismatched workspaces, unassigned runtimes
and private addresses without a CIDR grant. Denied requests made no runtime calls.
The release graph, eleven offline release-tool tests and workflow lint passed.
Both UIs build, and Console type checking passed. All 326 Console UI tests passed
across the initial run and a focused rerun: Node 26's global storage behavior broke
two existing suites, whose 19 tests passed under Node 22 without code changes.
The operations UI WebSocket ticket regression also passed.

All six groups of the Linux release driver passed with the merged shipping binary:
onboarding, filesystem grants, source access, Git snapshots/recovery, managed CLI
containment and approved/denied actions. The binary SHA-256 was
`a3ec138a57e64ac12f312088d8e48fee1f797cbd9596deadcd48f55d3ff9db8d`, unchanged
throughout testing on Linux 7.0.0-38-generic. The run recorded the development tree
as dirty; it is local integration evidence, not an exact-commit release approval.
The initial attempt placed executable fixtures beneath private `/tmp` and correctly
failed onboarding and managed launch. Moving the evidence directory outside `/tmp`
made the complete driver pass; the driver now rejects that invalid location early.
No containment restriction was relaxed.

The installed CLI governed-improvement driver passed all 18 checks, including real
HTTP and scheduled admission, signed receipt import, explicit activation, version
pinning, retry retention, rollback, disabled admissions and tamper refusal. Its
provider was synthetic; this does not establish model quality or financial savings.
The published skill bundle was re-signed in the new envelope format using its
existing publisher key, and the signing script independently verified the envelope
and manifest. Live publisher discovery was not part of that local verification.

The combined Enterprise server run passed 472 unit tests and 203 focused API tests
covering provisioning, signup, platform administration, runtime ownership, tenant
isolation, audit, ORGA and RAG. Its all-target Clippy and the runtime's minimal
all-target Clippy passed with warnings denied. The earlier isolated security-branch
checks also exercised trusted provisioning on PostgreSQL with an actual restricted
application role; they were not repeated for the integration-only route changes.

The runtime library run passed 1,415 tests and retained ten explicit ignores. One
cancellation test raced the journal's concurrent append and hit the intended
"changed during verification" refusal. Its polling now retries only that specific
snapshot-race error; malformed evidence still fails. All thirteen coordinator
WebSocket/delegation tests passed after the fixture correction. Cache HTTP-loop,
budget recovery and ToolClad reference-vector integration checks also passed.
The two shipping-CLI skill tests passed on the combined branch, confirming truthful
failure exits and confined scanning. Existing ignored external-service tests were
not counted as successes. No workspace-wide test rerun was used for this integration.

## Published 2.0.0 source qualification — 2026-10-23

The public release is bound to commit
`21471d3a9dea8ccf24d9248ff9338b35a79215e5`. Its corresponding Gitea commit is
`262c02a8411b8f3491af23e32667d8716d13c3d1`; OSS filtering creates a separate history.
[Test](https://github.com/ThirdKeyAI/Symbiont/actions/runs/40024802750),
[container build](https://github.com/ThirdKeyAI/Symbiont/actions/runs/40024802803),
[documentation](https://github.com/ThirdKeyAI/Symbiont/actions/runs/40024802890), and
[Linux release qualification](https://github.com/ThirdKeyAI/Symbiont/actions/runs/40024828887)
all passed on that exact public commit before publication.

The qualification ran the release-profile binary from a clean checkout on Linux
6.17.0-1022-azure with systemd 255. All six groups passed: onboarding, filesystem
grants, source access, Git snapshots, managed CLI containment and governed actions.
The binary hash was
`409fab080c0fa3b6e92cbd05575987b6da07ee65f556c84839d86844e91b7e72`,
unchanged throughout the run. The driver ran as a transient service inside the
delegated systemd user manager; starting that manager without moving the driver
into its delegated tree had correctly failed the earlier attempt.

The workflow enables unprivileged namespaces only on its disposable runner. It
does not change operator-host policy. Providers and CLI fixtures are synthetic;
this is real Linux isolation and lifecycle coverage, not a live-provider cost or
model-quality evaluation. It does not establish equivalent containment on macOS
or Windows. See [release verification](release-verification.md) for independent
artifact checks and [migration](migration-2.0.md) for deployment changes.
