# 1.21.0 release preparation

Validation date: 2026-10-06. This records the checks run to prepare 1.21.0.
The improvement lifecycle remains optional and explicitly selected.

## Changes

- Integrated project-bounded `.env` loading, isolated initialization tests,
  serialized native-runner environment tests and rustls 0.23.45.
- Manual cron admission now registers ownership before publishing its intent to
  recovery. Durable invocation claims still enforce ownership across processes.
  Policy and identity refusals retain their specific reason; changing policy
  does not reopen a refused invocation identity.
- Prepared the main workspace crates at 1.21.0 and the changed channel adapter at
  0.1.6. Unchanged independent helpers retain their existing versions.
- Publishing derives the complete dependency order from Cargo metadata, including
  optional dependencies and both sandbox crates. Registry checks use exact
  versions; upload progress cannot turn a failed command into a successful release.
- OSS sync validates an offline export of tracked, allowed files before signing
  or pushing. Public workflow drivers and their helpers are included. Private
  directories, untracked artifacts and signing-key files stay out. The existing
  permitted alias is materialized from its target file; other links are refused.

## Completed checks

- Workspace build; workspace Clippy across all targets with warnings denied;
  formatting and Git whitespace checks.
- 155 scheduler tests, including admission/recovery at the pending-intent boundary,
  plus 30 consecutive runs of the policy-refusal regression.
- Ten native-runner tests with the optional feature enabled and four test threads;
  two subprocess initialization tests; five installed-CLI `.env` boundary cases.
- Fifteen installed-CLI cron E2E checks with seven independently verified journals:
  concurrent retries, timer recovery, interrupted publication, signed resolution,
  explicit new work and unchanged original evidence. A documented transient
  history-repair refusal permits only bounded retries of the same operator review.
- Eight real-model Landlock action E2E checks across seven executions, covering
  exact approvals, denial, missing relay, interruption before approval and after
  publication, signed reconciliation without replay and a fresh approved recovery
  invocation. Exactly four receipts remained; the workflow was disabled and its
  worker pool cleaned up. The independent routing candidate stays unapproved.
- Four offline release-tool regressions; a complete filtered OSS export; the
  fourteen-crate publish graph, also validated with an empty Cargo cache offline.
  Read-only registry checks correctly distinguished existing and absent exact
  versions; no upload command was executed.
- Cached advisory, dependency-ban, license and source checks passed. This is not
  a claim that the advisory database was refreshed during the run.

The installed CLI used for E2E reports `symbi 1.21.0`, with SHA-256
`a01ca2da7a398b7c65d307b769890ced985a1f46e5e13451c6a738b02ccfb726`.
Private test projects, signing keys, transcripts and machine-readable reports are
retained outside the repository. The earlier routing experiment and its failed
acceptance results remain unchanged; see [phase 2 findings](governed-intake-phase2.md).

## Publishing prerequisites

Offline validation does not verify external credentials or perform a publication.
The Gitea `github-sync` environment needs `GITHUB_SSH_KEY` and
`OSS_GPG_PRIVATE_KEY` for the existing `7B0BF546C173D14B` signing identity. The
workflow imports the unattended signing key into a temporary private keyring and
removes it afterward. Crates publishing requires `CARGO_REGISTRY_TOKEN`.

Tagging and public mirror synchronization are not part of these checks.

## Known issue: load-dependent test flakiness

`cargo test --workspace` is reliable at moderate parallelism and intermittently
fails when the test binary saturates every core. Measured on a 24-core host:
roughly one full-suite run in twenty fails in an invocation-ownership test,
while six consecutive runs at `--test-threads=4` were clean.

Two causes were found and fixed for this release. Both lock helpers treated
every `flock` errno other than `EWOULDBLOCK` as a hard failure, so a signal
arriving during the call refused a claim that nothing held; they now reissue an
interrupted lock. Separately, the embedding suite mutates the process-global
`SYMBIONT_REQUIRE_REAL_EMBEDDINGS` while a context manager test reads it, which
is now serialized against that suite.

The residual failures are refusals — reconciliation declines an invocation it
still considers owned — rather than lost or duplicated ownership. The working
hypothesis is that the owner's descriptor is closed on a blocking thread, so the
lock is released slightly after the handle is dropped. This is unresolved and
should be confirmed before any claim that releasing a claim is synchronous.
