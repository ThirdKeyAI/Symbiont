# 3.0 candidate outbound CLI qualification

All 16 synthetic outbound CLI cases passed on 2026-10-27 after the final
Rust test-module layout correction. The workflow used the 3.0.0 debug binary
built with one Cargo job and reduced debug information. It exercises real
manifest loading, policy, exact terminal approvals, credentials, response
filtering, signed journals and interrupted-invocation retry refusal. It does
not qualify worker containment or the eventual release-profile binary.

The candidate was uncommitted when tested, based on `89f5e2dc7c22be38997c31febb0dbda19c97a89c`.
Its source-content digest, rather than that base revision alone, identifies the
code under test. The final release still requires exact-commit CI qualification.

- Source digest: `ad54fbff4a67b4df1a54edcafa19562ac9ffb1fff9b3b7d8f50b361e37dea80c`
- Binary SHA-256: `f57c2c4387dc84e4b8fa730284c29dbf036323e1780ef6e1908226941a48f1d9`
- Report SHA-256: `d3c0fc7bb3590bf4e8243f28d55199028cb489dcff378e76f17eac189a63ccd2`
- [Final candidate report](final-workflow.json)
- [Preliminary report before the layout correction](workflow.json)

Reproduce with `scripts/test-outbound-capabilities.py --binary /absolute/path/to/symbi --report /absolute/new/report.json`.
The fixture uses synthetic credentials and local services; it makes no paid
provider calls. Reports retain hashes and verification observations, not private
signing keys. Historical containment bundles remain separately scoped.

## Restricted-host diagnostic

The [preliminary binary's platform diagnostic](platform-refusal.json) passed the
expected namespace-refusal check with Landlock ABI 8. Supervised launch and
cleanup succeeded; private native-workspace setup was refused with no host
fallback. Cleanup, unchanged binary and unchanged host policy were confirmed.
`qualified` remains `false`: this is not a passing native-worker qualification.
The report predates the test-module layout correction and identifies its own
binary hash. Publication qualification must independently exercise the final
release artifact on a supported host.
