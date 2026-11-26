# ClawHavoc rule maintenance

The current scanner is an offline, deterministic content scanner. Its rules ship
with Symbiont. This foundation adds validation, explicit deny/advisory semantics,
policy identity and regression evidence; it does not download new rules or claim
complete coverage of current malicious skills.

## Updating a rule

1. Record the observed technique and a primary source. Use synthetic payloads;
   never commit credentials, live malware or fetched executable content.
2. Keep the built-in rule ID stable when refining the same technique. A new
   technique needs a new ID. Document severity and expected false positives.
3. Add positive, evasive-variant and legitimate-control cases to
   `crates/runtime/tests/fixtures/clawhavoc/cases.json`. Existing inline tests
   provide additional coverage. Each corpus entry records provenance and an
   expected directory verdict, rule and severity. The corpus is a regression
   baseline, not a detection-rate benchmark.
4. Run `cargo test -p symbi-runtime --lib skills::` and the workspace gates.
   Run `python3 scripts/test-skills-scanner.py --binary target/debug/symbi`
   against the built shipping CLI. The main CI workflow runs both paths.
5. Bump `BUILTIN_RULESET_VERSION` when bundled definitions change and
   `SCANNER_SEMANTICS_VERSION` when matching, file coverage, validation, identity
   encoding or verdict semantics change. Changing limits or custom rules changes
   the policy digest automatically. Preserve tests for previously supported
   legitimate behavior and explain intentional verdict changes.

The digest uses SHA-256 over compact serde JSON for an ordered tuple containing
the domain string `symbi-skill-scan-policy-v1`, semantics version, ruleset version,
built-in definitions, requested custom rules, and limits. Preserve this encoding
or bump its domain/version. It is order-sensitive by design. It is not a signature
or proof that a skill was scanned; callers must retain the actual scan result.

## Compatibility

Custom content/file deny patterns now produce High findings and block. Callers
that intended advisory matching should explicitly use `WarnContentPattern`.
Malformed patterns, including advisory patterns, reject the whole configuration.
Fallible constructors return an error. Legacy constructors preserve their return
type but produce failed scans, including for empty content or a missing directory.
Multiple executable allowlists are rejected instead of using the last list.

Custom IDs use a kind prefix and SHA-256 of the effective regex rather than the
raw pattern. `ScanResult` has a new `identity` field; Rust consumers constructing
struct literals need to supply it. JSON consumers should accept the additional
field. The engine version is the runtime crate version, separate from policy
identity. Rule validation does not change signature verification or automatically
enforce a loader's verdict: SDK consumers must inspect `scan_result.passed`.

Matching is still line-based and context-insensitive. A dangerous command quoted
in security documentation can match; multiline, encoded and indirect behavior
can evade these signatures. An interpreter allowlist checks recognized shebangs,
not all command execution. A passing scan never replaces runtime authorization,
filesystem/network restrictions or independently verified publisher identity.

## AgentSniff assessment and next stage

Reviewed AgentSniff at `4d58d71d68273e241894c84db112ffc189424287` (Apache-2.0):

- `crates/agentsniff-signatures/src/lib.rs` independently pins its verification
  key, verifies datasets, and accepts runtime overlays as a complete set.
- `crates/agentsniff/src/signatures/updater.rs` bounds responses, validates the
  full download, stages a complete set and supports restoration on activation
  failure.

This change adopts the complete-set validation principle. No AgentSniff code,
keys or datasets are copied, and no dependency is added. Its network-service,
framework and domain fingerprints are not malicious-skill indicators; importing
them as deny rules would confuse detection of agent software with maliciousness.

For a future signed rule-pack updater, reuse those architectural principles with
an independently pinned ClawHavoc publisher key, one signed manifest binding every
file and version, strict schema/regex validation, bounded reads, rollback/freeze
protection, and crash-safe activation with a last-known-good generation. Do not
copy AgentSniff's keys or wire format without a separate compatibility review.
Test interrupted updates, mixed generations, wrong keys, invalid rules, oversized
packs and offline recovery before enabling downloads. Rule updates should enter
review through tested PRs; this foundation installs no scheduled updater.

Primary research and comparison sources for subsequent coverage work:

- [ClawHavoc campaign research](https://www.paloaltonetworks.com/blog/security-operations/clawhavoc-ai-agent-skills-poisoning/)
- [Cisco Skill Scanner](https://github.com/cisco-ai-defense/skill-scanner)
- [AgentSniff](https://github.com/ThirdKeyAI/agentsniff)
