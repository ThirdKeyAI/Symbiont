# Symbiont Release Runbook

Last updated: 2026-10-03

## Pre-Release Checklist

### 1. Determine Version Scope

All crates in this workspace that had **source code changes** (not just Cargo.toml dependency bumps) since their last published version need a version bump.

Run `python3 scripts/publish-crates.py` to validate and list all publishable
workspace crates in dependency order. It includes optional dependencies and both
sandbox crates. `tests/e2e` remains unpublished. All local version references must
match the corresponding package version.

The 1.21.0 preparation updates the main workspace crates and
`symbi-channel-adapter` to 0.1.6. Unchanged independent helpers retain their versions.

### 2. Check Published Versions vs Source

```bash
# Crates.io — check what's already published
cargo search symbi --limit 10
cargo search repl-core --limit 1
cargo search repl-proto --limit 1
cargo search repl-cli --limit 1
cargo search repl-lsp --limit 1

# npm (JS SDK is a separate repo)
npm view symbiont-sdk-js versions --json

# PyPI (Python SDK is a separate repo)
pip index versions symbiont-sdk

# AgentPin (separate repo)
cargo search agentpin --limit 3
npm view agentpin versions --json
pip index versions agentpin
```

### 3. Identify Changed Crates Since Last Tag

```bash
# List all files changed since last release
git diff v<LAST_TAG>..HEAD --name-only | sort

# Count changes per crate
for crate in dsl runtime channel-adapter repl-core repl-cli repl-lsp repl-proto; do
  echo "=== $crate ===" && git diff v<LAST_TAG>..HEAD --name-only -- crates/$crate/ | wc -l
done
echo "=== workspace root ===" && git diff v<LAST_TAG>..HEAD --name-only -- src/ Cargo.toml Dockerfile | wc -l
```

**Important**: A crate may already have been bumped during the development cycle (e.g., `symbi-runtime` was bumped to 1.0.1 mid-cycle and published). Check whether the *published* version on crates.io already includes the latest changes. If the source version matches crates.io but there are unpublished source changes, bump again.

## Bumping Versions

### 4. Update Cargo.toml Files

When bumping a crate, you must also update **all cross-crate references** to it:

| If you bump... | Also update references in... |
|----------------|------------------------------|
| `symbi-dsl` | `Cargo.toml` (root), `crates/runtime/Cargo.toml` |
| `symbi-runtime` | `Cargo.toml` (root), `crates/repl-core/Cargo.toml` |
| `repl-core` | `crates/repl-cli/Cargo.toml`, `crates/repl-lsp/Cargo.toml` |
| `repl-proto` | `crates/repl-cli/Cargo.toml` |

Find all references with:
```bash
grep -r 'CRATE_NAME.*version = "' crates/*/Cargo.toml Cargo.toml
```

### 5. Update Cargo.lock and Validate

```bash
cargo update --workspace
cargo clippy --workspace   # must pass with zero warnings
cargo test --workspace     # must pass
```

## Publishing

### 6. Commit, Push, Sync to GitHub

```bash
git add Cargo.lock Cargo.toml crates/*/Cargo.toml
git commit -m "Bump workspace crates to v<VERSION>"
git push origin main

# Sync to GitHub (triggers CI)
bash scripts/sync_oss_to_github.sh --force
```

### 7. Wait for CI

```bash
# Watch both workflows
gh run list --repo ThirdKeyAI/Symbiont --limit 4

# Or watch specific runs
gh run watch <RUN_ID> --repo ThirdKeyAI/Symbiont --exit-status
```

Record the intended full commit SHA on GitHub `main`. Both **Test** and **Build
and Push Docker Image** must pass for that commit. Then dispatch **Linux Landlock
release qualification** on `main`, verify that its `head_sha` is the recorded SHA,
and wait for success before tagging. If `main` moves, do not reuse evidence for a
different commit. The Linux job exercises the shipping binary on a disposable
host and retains public reports; missing host prerequisites fail the job.

```bash
gh workflow run landlock-release.yml --repo ThirdKeyAI/Symbiont --ref main
# Check the run's head SHA and final result, then validate both workflow results:
python3 scripts/require-release-checks.py --repository ThirdKeyAI/Symbiont --sha <FULL_SHA>
```

The gate requires `GH_TOKEN` or `GITHUB_TOKEN` with read access to Actions. Tag
publication, release assets, and release containers use a reusable gate and check
out that qualified SHA. A failed, cancelled, or unfinished latest run blocks
publication even when an earlier run passed. Main container builds require Test;
they do not trigger the manual Linux qualification on every commit.

### 8. Create GitHub Release

Gitea and GitHub have different histories because GitHub receives only the OSS
export. Create the public tag at the qualified **GitHub** commit; never push the
private source tag object to the public mirror. Check for an existing tag first
and verify its target instead of replacing it. Creating the public tag explicitly
also triggers versioned container publication.

```bash
gh api repos/ThirdKeyAI/Symbiont/git/refs \
  -f ref=refs/tags/v<VERSION> -f sha=<FULL_SHA>
gh release create v<VERSION> --repo ThirdKeyAI/Symbiont \
  --verify-tag \
  --title "Symbiont v<VERSION>" \
  --notes-file /path/to/release-notes.md
```

Prepare the notes file with the release overview, migration link, opt-in features
and changelog before this step. Published releases trigger crates.io and binary
publication automatically. Keep the tag fixed once publication begins.

### 9. Publish to crates.io (if needed)

The publish workflow derives the dependency order from Cargo metadata, including
`symbi-sandbox-guest` and `symbi-sandbox-supervisor` before `symbi-runtime`.

```bash
# Validation only: no registry requests or uploads.
python3 scripts/publish-crates.py
python3 scripts/test-release-tools.py

# Only as part of an authorized release, with CARGO_REGISTRY_TOKEN configured:
python3 scripts/publish-crates.py --publish
```

Registry checks use the exact package version, not search ranking or the newest
published version. A nonzero Cargo exit fails unless the registry confirms that
exact version now exists. Upload progress text cannot turn a failed upload green.

### 10. Verify publication and documentation

Wait for **Publish to crates.io**, **Release Binaries** (including the Homebrew
update), and the tag's **Build and Push Docker Image** run to succeed. Check their
source SHA, not just the workflow name. Publication is not complete merely because
the GitHub release page exists.

Use [release verification](release-verification.md) to check all four archive
checksums and signatures, the signed source record, and both container identities.
Check each crate's exact registry version, checksum and source provenance. The
Homebrew formula's three archive hashes must match the release manifest. Exercise
the downloaded native binary and the documented installer in a temporary home.

Update the English installation, migration and validation pages first, then their
translations. Build all six documentation sites and verify separate search indexes
as in `.github/workflows/docs.yml`. Update the release notes after asset upload if
needed. Documentation or installer fixes on main must not silently move an already
published tag or replace a published crate version.


## Lessons Learned

### Docker Build Cache Invalidation
The Dockerfile uses a dependency caching strategy with stub source files. The cleanup glob **must** include:
- `target/release/deps/symbi*` — direct matches
- `target/release/deps/libsymbi*` — Rust prefixes library artifacts with `lib`
- `target/release/.fingerprint/symbi*` — build fingerprints
- `target/release/symbi*` — built binaries

Missing any of these causes stale artifacts that fail the real build.

### Version Drift Between Published and Source
A crate version in `Cargo.toml` may match crates.io, but if there are source changes after the publish, the version must be bumped again. Always compare:
- The *published* version on crates.io
- The *source* version in Cargo.toml
- Whether there are *unpublished source changes* since the version was last set

### Cross-Crate Version References
Cargo workspace path dependencies with `version = "x.y.z"` are used for crates.io compatibility. When bumping any crate, grep for all references and update them too, or `cargo publish` will fail.

### OSS Sync Script
Validate the actual filtered working tree locally before a public sync:

```bash
bash scripts/sync_oss_to_github.sh --export-dir /absolute/new/oss-export
```

This mode has no network, Git commit, signing or push step. It exports only tracked
files and explicit public tooling. Stage new public files before validating them.
Private source trees, untracked artifacts, linked files and sensitive key names
are refused or excluded. The same filter runs in CI and in the actual sync.
A nonzero exit is a failure; cleanup no longer changes a successful status.

The Gitea `github-sync` environment must provide `GITHUB_SSH_KEY` and
`OSS_GPG_PRIVATE_KEY`, the unattended signing key for `7B0BF546C173D14B`.
The workflow imports it into a temporary private keyring and removes that keyring
afterward. Public crates publishing separately needs `CARGO_REGISTRY_TOKEN`.
Offline checks do not prove that these external credentials are configured.

### CI Must Pass Before Tagging
Wait for Test, Docker Build and the manual Linux Landlock release qualification
to succeed for the intended commit before creating its tag/release. A failed
workflow after tagging requires extra cleanup.
