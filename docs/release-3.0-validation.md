# 3.0 release preparation

Validation started on 2026-10-27. The 3.0 version reflects Rust API changes:
new outbound fields in public structs, explicit Landlock enum variants, and
replacement of the total `SecurityTier` ordering with a partial ordering.
See [migration guidance](outbound-capabilities.md#coverage-and-migration).
Existing serialized configurations do not require an outbound profile.

The updated packages are `symbi`, `symbi-dsl`, `symbi-runtime`, `repl-core`,
`repl-cli`, `repl-lsp`, and `symbi-shell`. Unchanged helpers retain the versions
listed in [release verification](release-verification.md).

## Local validation

The candidate was built on Linux using one Cargo build job and reduced debug
information (`CARGO_PROFILE_DEV_DEBUG=1`). This limits build memory use without
changing runtime features. It is not a release-profile binary qualification.

| Check | Result |
| --- | --- |
| `cargo build --workspace` | Passed |
| `cargo test --workspace` | 2494 passed, zero failures, 70 ignored across 88 suite summaries on the final source |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed with zero warnings |
| `cargo fmt --check` | Passed |
| Package graph and cross-crate version references | Passed |
| Outbound installed-CLI workflow | All 16 cases passed again after the layout fix |
| Restricted-host platform diagnostics | Preliminary binary: expected namespace refusal, cleanup and no fallback confirmed; not a qualified native-worker platform |
| Security catalogue validation and six regression tests | Passed; current source digest matches the refreshed report |
| Release-tool tests | Eight passed |
| Release gate, evidence selection and scoring rejection tests | Four, two and eight passed respectively |
| Historical containment-evidence tampering checks | Five broker and four lifecycle mutations rejected |
| English and five translated documentation builds | Passed, including each search-index language |
| Filtered public export | Validated; no publication performed by export validation |

The [outbound report](evidence/2026-10-27/release-3.0-outbound/README.md) identifies
the exact source-content and binary digests, configuration, driver and platform.
It covers synthetic local services, policy, exact approvals, credential handling,
response filtering, signed journals and interruption/retry refusal. It does not
qualify worker containment, production credentials or real provider behavior.

The historical containment bundles test the independent verifiers, not this
candidate binary. This development host refuses the unprivileged namespaces
needed for full Landlock qualification; host security settings were not changed.

## Publication gates

The exact public main commit must pass the canonical Test workflow, Linux
Landlock qualification, and Docker build before tagging. Run the release-binary
workflow with `verify_only` before publication to check all supported targets.
The Ubuntu 22.04 builder enforces the glibc 2.35 ceiling. A separate Ubuntu 24.04
job downloads that job's publication archive and qualifies its exact binary,
without rebuilding it. This supplies the newer Landlock kernel and Python
needed by the qualification driver while retaining the older userspace baseline.
Publication depends on this job and its independently verified containment evidence.
Failures or incomplete reports prevent publication; previous-release evidence
cannot substitute for these checks.

Use the [release verification guide](release-verification.md) after publication
to bind downloaded assets and packages to the public tag and source commit.
Gitea and the filtered public mirror have different commit identities.
