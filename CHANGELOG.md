# Changelog

All notable changes to the Symbiont project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.14.1] - 2026-05-18

**Hotfix for v1.14.0 release workflow + integration tests.** v1.14.0's binaries and `cargo publish` to crates.io did not ship: the GitHub Actions workflow YAML captured at the v1.14.0 tag commit had two issues that prevented the tag-triggered jobs from running. v1.14.1 is identical in source-level security posture to v1.14.0 — see the v1.14.0 entry below for the full audit response.

### Fixed
- **`dtolnay/rust-toolchain` action invocation**: the SHA-pinned form (added in v1.14.0 H6) requires an explicit `with: toolchain: <version>` input — the older ref-as-toolchain inference (`@stable`) only works when the action is consumed by ref name, not by SHA. Three workflow sites in `.github/workflows/{test,publish,release-binaries}.yml` now pass `toolchain: stable`. (The `fuzz` job's second invocation was already correct, pinning `nightly-2026-02-21`.)
- **`tools/fuzz` CI list**: dropped `sse_jsonrpc_parsing` from the hardcoded fuzz-target list in `.github/workflows/test.yml`. The corresponding source was removed in v1.14.0 as part of the Composio MCP removal but the workflow YAML was not updated, causing the fuzz job to fail on `cargo fuzz run sse_jsonrpc_parsing`.
- **`crates/runtime/tests/http_input_integration_tests.rs`**: the test fixture seeded `cors_origins: ["*"]`, but v1.14.0's M1 fix refuses the wildcard at server startup. The integration tests therefore failed with `Connection refused` for the whole file (9/9 tests). Test fixture now uses the explicit loopback origin (`http://127.0.0.1:<port>`); the `test_cors_headers_when_enabled` preflight is rewritten to send the matching `Origin` header so it actually exercises the allowlist path. Local `cargo test -p symbi-runtime --test http_input_integration_tests --features http-input` is green (9/9 pass).

### Crate versions
| Crate | Version |
|-------|---------|
| `symbi` | 1.14.1 |
| `symbi-runtime` | 1.14.1 |
| `symbi-dsl` | 1.14.1 |
| `repl-core` | 1.14.1 |
| `repl-cli` | 1.14.1 |
| `repl-proto` | 1.14.1 |
| `repl-lsp` | 1.14.1 |
| `symbi-shell` | 1.14.1 |
| `symbi-invis-strip` | 0.3.0 (unchanged) |
| `symbi-approval-relay` | 0.1.1 (unchanged) |
| `symbi-channel-adapter` | 0.1.3 (unchanged) |

## [1.14.0] - 2026-05-18

**Security audit response release.** Implements every finding in `SECURITY_AUDIT.md` (5 CRITICAL, 7 HIGH, 10 MEDIUM, 9 LOW). Out-of-band operator items live in `SECURITY-OPS.md`.

### Removed (BREAKING)
- **Composio MCP integration + SymbiBot autonomous-posting feature** removed entirely (SECURITY_AUDIT.md C3). Affected surface: the `composio` Cargo feature flag, `ComposioToolExecutor`, the `crates/runtime/src/integrations/composio` module, the `symbiont_mcp add` / `symbiont_mcp list` CLI subcommands, the `crates/runtime/examples/composio_smoke_test.rs` example, and the `tools/fuzz/fuzz_targets/sse_jsonrpc_parsing.rs` fuzz target. Env vars `COMPOSIO_API_KEY` and `COMPOSIO_MCP_URL` are no longer read. Bring your own `ActionExecutor` for external tool dispatch — Composio dispatched LLM-supplied tool names without a static allowlist or TLS pinning.
- **`SYMBIONT_ALLOW_NO_JWT_AUDIENCE` env-var escape hatch** removed (SECURITY_AUDIT.md M2). Every JWT verifier now requires an explicit `aud` configuration unconditionally.

### Changed (BREAKING)
- **`symbi up` / `symbi run` default policy gate is now fail-closed** (SECURITY_AUDIT.md C2 / M3). `DefaultPolicyGate::new()` returns `LoopDecision::Deny` for every `ToolCall` and `Delegate` action with an explicit reason; `Respond` actions remain allowed. The previous binary hard-coded `DefaultPolicyGate::permissive()`, which silently allowed every tool call and delegation. Wire `CedarPolicyGate` / `OpaPolicyGateBridge` / your own `ReasoningPolicyGate` impl, or opt into the dev-only permissive mode via `--insecure-allow-all` (or `SYMBI_INSECURE_ALLOW_ALL=1`) with a loud stderr banner.
- **`DefaultPolicyGate::permissive()` renamed to `DefaultPolicyGate::permissive_for_dev_only()`** and marked `#[doc(hidden)]`. It now emits a `tracing::warn!` on every evaluated action so insecure permissive mode is visible in production logs.
- **`bundled docker-compose.test.yml` now requires `SYMBIONT_API_TOKEN`** (no `testtoken123` default) and binds published ports to `127.0.0.1` rather than `0.0.0.0` (SECURITY_AUDIT.md C5). `VM_HOST` is also required; `.env.example` is included.
- **JWT verifier algorithm allowlist** (SECURITY_AUDIT.md C4). ES256 and EdDSA only for the asymmetric `Authorization: Bearer` path; HS256 only for the HMAC webhook-signature path. `RS256`/`RS384`/`RS512`/`PS256`/`PS384`/`PS512` and `none` are rejected at both the header-inspection guard and the `Validation::algorithms` allowlist. Neutralizes RUSTSEC-2023-0071 (`rsa` Marvin Attack reachable through `jsonwebtoken`) on every path operators control. The Microsoft Teams adapter (`crates/channel-adapter/src/adapters/teams/auth.rs`) still uses RS256 because the Bot Framework protocol requires it; that surface is bounded to MS-signed tokens.
- **`symbi-invis-strip` 0.3.0**: forbidden range expanded with U+00AD (soft hyphen), U+0300..=U+036F (combining diacritical marks), and U+2070..=U+209F (superscript/subscript forms); `detect_injection_patterns` now NFKC-normalises input (closes fullwidth and math-alphanumeric homoglyph bypasses), adds a compact-projection scan (catches post-strip word concatenation), and flags Latin+Cyrillic mixing with a synthetic `mixed-script` marker. New `unicode-normalization` dependency. 7 new `bypass_proofs` regression tests cover each bypass class. (SECURITY_AUDIT.md H5)

### Added
- **`SYMBI_REJECT_LEGACY_API_KEYS=1` env var**: short-circuits the deprecated O(n) Argon2 scan for unprefixed API keys (returns `None` with a warn log). Operators should re-issue every key in `keyid.secret` format. The legacy path will be removed in the next minor release. (SECURITY_AUDIT.md M6)
- **`SYMBI_UNSAFE_NATIVE_SANDBOX=1` env var**: required (in addition to `SYMBI_ENV` not being `production`) to construct the `native` sandbox runner at runtime. The `native-sandbox` Cargo feature now also fails to compile in release builds via a top-of-module `compile_error!`. (SECURITY_AUDIT.md H4)
- **Tool-call argument validation against the declared JSON Schema** (SECURITY_AUDIT.md M4). Arguments produced by the LLM are validated before the policy gate runs; non-object arguments and schema-violating arguments are rejected as `LoopDecision::Deny`.
- **`Secret` has a hand-written `Serialize` impl that emits `"value": "[REDACTED]"`** (SECURITY_AUDIT.md M7). Derived `Deserialize` retained. Regression test asserts the JSON output never leaks the plaintext.
- **`symbi-approval-relay` 0.1.1**: Slack timestamp delta widened to `i128` + `saturating_sub` to mirror the channel-adapter pattern. (SECURITY_AUDIT.md L4)

### Fixed
- **Scheduler shell-injection sink** (SECURITY_AUDIT.md C1). `scheduler/task_manager.rs` no longer interpolates `task.config.dsl_source` into `sh -c`. The DSL is written to disk and passed via `$1` argv with a quoted-literal script.
- **Toolclad `session.startup_command` shell-injection** (SECURITY_AUDIT.md H1). Parsed via `shlex::split` and executed as argv; empty / metacharacter-bearing tokens rejected.
- **Firecracker host `/tmp` working directory** (SECURITY_AUDIT.md H3). Production path now uses per-uid `/run/symbi/agent_<id>` (0700) with `<temp>/symbi-<uid>/agent_<id>` fallback. Avoids cross-user `/tmp` races.
- **Docker sandbox `-e KEY=VALUE` env smuggling** (SECURITY_AUDIT.md M5). Environment is now written to a 0600 tempfile and passed via `--env-file`. Env keys containing `=`, newlines, or NULs are rejected.
- **HTTP-input CORS wildcard `"*"` accepted at startup** (SECURITY_AUDIT.md M1). Server now returns a `RuntimeError::Configuration` and refuses to start when the wildcard is configured.
- **HTTP-input error response logging** (SECURITY_AUDIT.md L1). Replaced `tracing::debug!` of the full `Display` string with `tracing::info!` of a stable enum-tag (`Configuration`, `Security`, etc.) plus the public message. No more internal path leaks.
- **Empty `ApiKeyStore` silent legacy fallback** (SECURITY_AUDIT.md L3). When a key store is configured but contains no records, a one-shot `tracing::error!` fires before falling back to the legacy env-var auth.
- **`approval-relay` Slack timestamp `i64::abs` overflow** (SECURITY_AUDIT.md L4). Widened to `i128 + saturating_sub`.
- **`Dockerfile` base images pinned by `@sha256:` digest** (SECURITY_AUDIT.md L6). `rust:1.88-slim-bookworm@sha256:38bc5a86…`, `debian:bookworm-slim@sha256:67b30a61…`.
- **`Dockerfile` HEALTHCHECK probes the HTTP server** (SECURITY_AUDIT.md L7). `curl -fsS http://127.0.0.1:8080/api/v1/health` with the original `/proc/net/tcp` socket-listen check as a fallback for HTTP-Input-only deploys.
- **All third-party GitHub Actions SHA-pinned** (SECURITY_AUDIT.md H6). Every `uses:` line carries a 40-char commit SHA with a trailing version comment. `cargo install cargo-fuzz` pinned to `0.13.1 --locked`. PR-only jobs now run with `permissions: contents: read` only (SECURITY_AUDIT.md M9).
- **`deny.toml`**: stale ignores `RUSTSEC-2026-0097` and `RUSTSEC-2026-0002` removed; `RUSTSEC-2023-0071` (`rsa` Marvin Attack) documented as a runtime-mitigated ignore pointing at the JWT verifier allowlist (SECURITY_AUDIT.md M8).
- **`config.rs` weak-token guard extended** (SECURITY_AUDIT.md C5 belt-and-braces). Rejects `testtoken123` literally and any token starting with `test` (case-insensitive) shorter than 20 chars. Prevents re-introduction of the historical compose default.

### Migration notes
- **No replacement for Composio.** Implement your own `ActionExecutor` for external tool dispatch. If you were using `ComposioToolExecutor` directly, see the v1.7.0 phase guidance in `ROADMAP.md` for the `ActionExecutor` trait surface.
- **If `symbi up` / `symbi run` denies every tool call after upgrade**, that is expected: wire a real policy backend (`CedarPolicyGate`, `OpaPolicyGateBridge`, or a custom `ReasoningPolicyGate`), or opt into permissive mode for local development via `--insecure-allow-all` / `SYMBI_INSECURE_ALLOW_ALL=1`.
- **Operators who issued API keys before the `keyid.secret` format** should re-issue all keys in the new format and set `SYMBI_REJECT_LEGACY_API_KEYS=1`. The legacy O(n) scan path will be removed in the next minor release.
- **JWT verifier**: tokens without `aud` are rejected unconditionally; remove `SYMBIONT_ALLOW_NO_JWT_AUDIENCE` from any environment file. RSA-signed JWTs are refused on every path under operator control.
- **`docker-compose.test.yml`** now requires explicit `SYMBIONT_API_TOKEN` and `VM_HOST` env vars and binds to `127.0.0.1`. See `.env.example`.
- **`native-sandbox` Cargo feature** fails to compile in release builds. The feature is intended only for local debugging — use the `docker`, `gvisor`, `firecracker`, or `e2b` runners in CI / staging / production.

### Crate versions
| Crate | Version |
|-------|---------|
| `symbi` | 1.14.0 |
| `symbi-runtime` | 1.14.0 |
| `symbi-dsl` | 1.14.0 |
| `repl-core` | 1.14.0 |
| `repl-cli` | 1.14.0 |
| `repl-proto` | 1.14.0 |
| `repl-lsp` | 1.14.0 |
| `symbi-shell` | 1.14.0 |
| `symbi-invis-strip` | 0.3.0 |
| `symbi-approval-relay` | 0.1.1 |
| `symbi-channel-adapter` | 0.1.3 (unchanged) |

## [1.13.0] - 2026-05-07

### Added
- **`symbi fmt`** — canonical formatter for `.symbi` source files. Reuses the tree-sitter parse tree from `symbi-dsl` to emit a stable canonical layout (4-space indent, single blank line between top-level items, trailing commas in metadata blocks, normalised spacing around `:`, `=`, `->`, `,`). Modes: rewrite-in-place by default; `--check` exits with code 2 if changes are needed (CI gate); `--stdin` reads from stdin and writes formatted output to stdout for editor integration. The formatter only emits output when the source parses without syntax errors and otherwise leaves the file untouched. v1 covers `metadata`, `agent`, `capabilities`, and `with` blocks; other top-level constructs (policies, schedules, channels, memories, webhooks, free-standing functions) round-trip via trimmed verbatim source.
- **Tree-sitter grammar v2 for the Symbiont DSL** — covers the full surface used by every example agent in `agents/*.symbi` plus documented Symbiont features: `=` and `:` separators in metadata/capabilities/channel properties; array and record values; full expression precedence ladder (`||`, `&&`, `==`/`!=`/`in`, `<`/`>`/`<=`/`>=`, `+`/`-`/`*`/`/`/`%`, unary `!`/`not`/`-`, postfix `.`/`(...)`/`[...]`); type literals (`Foo { k: v }`) and bare records with both identifier and string keys; lambdas (`x => expr`); vault URLs (`vault://...`); statements (`for in`, `match`, `try`/`catch`, assignment, compound assignment, `let`/`return`); block tail expressions; multi-arg generics (`Map<K, V>`, `Result<T, E>`); `if let Pattern(...) = expr { ... }` destructuring; named arguments accept `=` or `:`; underscore separators in numeric literals (`100_000_000`).
- **Tolerant formatter fallback** — `format_source` now returns the input unchanged on any parse error rather than failing, so `symbi fmt` degrades gracefully on novel grammar surfaces while a follow-up grammar bump catches up.
- **LLM API keys via `SecretStore`**: `CloudInferenceProvider` and `LlmClient` now accept an optional `Arc<dyn SecretStore + Send + Sync>` (HashiCorp Vault, OpenBao, or the file-encrypted backend) for resolving provider API keys. Operators point at secret-store keys with new env vars (`OPENROUTER_API_KEY_REF`, `OPENAI_API_KEY_REF`, `ANTHROPIC_API_KEY_REF`); when the corresponding `*_REF` is set the value is fetched from the store, otherwise the existing `*_API_KEY` env var is used. On store miss/failure the loader falls back to the env var so dev and CI workflows are unaffected. New helper `secrets::resolve_secret_or_env(env_var, secret_key, store)` exposes the same logic for other call sites. New constructors `LlmClient::from_env_or_secrets(store)` and `CloudInferenceProvider::from_env_or_secrets(store)` (async); `from_env()` is unchanged. Side benefit: the cloud provider no longer re-reads `*_API_KEY` / `*_BASE_URL` from the env on every inference call.
- **ToolClad `agent_summary` arg type** — typestate fence for orchestrator-injection that prevents adversarial agent output from being repackaged into trusted orchestration arguments.
- **`symbi-invis-strip` 0.2.0 — INJECTION_MARKERS expanded** from tier1-v3 bypass forensics. Runtime tracks the new `symbi-invis-strip` version.

### Fixed
- **Slack signature timestamp overflow** (`crates/channel-adapter/src/adapters/slack/signature.rs`): the freshness check computed `(now - ts).abs()` on `i64`. When the fuzzer fed an adversarial timestamp at the i64 extremes (e.g. `i64::MAX`) the subtraction overflowed — debug builds panicked with `attempt to subtract with overflow`, **release builds wrapped silently and could let stale or future requests slip through the freshness check**. The delta is now computed in `i128` so adversarial timestamps near the i64 boundaries cannot escape the rejection. Surfaced by the `slack_signature_verification` fuzz target; the matching property check inside the harness was widened to keep the assertion truthful at the boundary. New regression test `extreme_timestamps_do_not_panic` covers `i64::MIN`, `i64::MAX`, and adjacent values.
- **`jsonwebtoken` v10 crypto backend**: pinned `rust_crypto` feature on `jsonwebtoken` in `crates/runtime` and `crates/channel-adapter`. The v10 release dropped a default backend and required exactly one of `rust_crypto` or `aws_lc_rs` to be enabled; without it, JWT verifier tests panicked at runtime. `rust_crypto` was selected because the workspace already depends on RustCrypto crates (`hmac`, `sha2`, `ecdsa`, `p256`) and avoids pulling in `aws-lc-sys`.
- **rustfmt drift** in `crates/runtime/src/toolclad/validator.rs` and `crates/symbi-invis-strip/src/lib.rs` (test bodies wrapping long inline strings).

### Editor / tooling ecosystem (separate repos, released this cycle)
- **[`tree-sitter-symbiont`](https://github.com/ThirdKeyAI/tree-sitter-symbiont) v0.1.0** — standalone tree-sitter grammar repo published to npm (`tree-sitter-symbiont`) and crates.io (`tree-sitter-symbiont`). Carries the v2 grammar, highlights query, Node and Rust bindings, and corpus tests.
- **[`vscode-symbiont`](https://github.com/ThirdKeyAI/vscode-symbiont) v0.1.0** — VS Code extension with a TextMate grammar covering the full keyword surface. `.vsix` published as a GitHub release asset; marketplace publish is a separate step.
- **[`symbiont-sdk-js`](https://github.com/ThirdKeyAI/symbiont-sdk-js) v1.13.0** — `symbi agent create` CLI now accepts `.symbi` files alongside `.json` and the legacy `.dsl`.

### Notes
- The `symbiont-sdk-python` SDK (currently v1.11.0) does not require a release in this cycle: the runtime HTTP API surface has not changed since v1.11.0 and the Python SDK does not read agent files from disk in a way that the canonical-extension change affects.

## [1.12.0] - 2026-04-29

### Added
- **Sandbox tier selection — all OSS** (PR #54): Operators now pick the host-isolation tier per agent via the DSL `with { sandbox = "tier1" | "gvisor" | "firecracker" }` block, or set a project default via `[sandbox] tier = "..."` in `symbiont.toml`. All three tiers — Docker, gVisor, and Firecracker — ship in the OSS runtime; gVisor and Firecracker are no longer Enterprise-gated. New runtime modules: `crates/runtime/src/sandbox/gvisor.rs` (delegates to Docker with the `runsc` runtime) and `crates/runtime/src/sandbox/firecracker.rs` (per-execution Firecracker microVM with operator-supplied kernel + rootfs).
- **`.symbi` canonical agent file extension** (PR #53): Agent files use `.symbi` going forward. `.dsl` continues to be recognized indefinitely for backward compatibility — no migration required. New helpers `dsl::is_symbi_file` and `dsl::strip_symbi_extension`. Scaffolding (`symbi init`, `symbi new`) now emits `.symbi`. The 14 example agents in `agents/` were renamed.
- **Tier 3 `symbi init` flags** (PR #55): `symbi init --sandbox tier3 --firecracker-kernel <PATH> --firecracker-rootfs <PATH>` validates both files exist before scaffolding `[sandbox.firecracker]` into `symbiont.toml`. New guide `docs/firecracker-setup.md` covers prerequisites, a quickstart recipe (prebuilt vmlinux + minimal Alpine rootfs), the in-VM init contract, transport patterns for `/work` (vsock vs. second block device), a hardening checklist, and troubleshooting.
- **`SecurityTier::Hosted` variant** (PR #56): E2B is no longer modeled as a peer of Tier 1/2/3 — it is a separate hosted-cloud backend with no on-host isolation. `E2B → SecurityTier::Hosted` sorts **below** `Tier1` for ordering, so policies requiring host isolation (`tier >= Tier1`) now correctly reject hosted execution. E2B remains opt-in only via DSL (`with { sandbox = "e2b" }`); it is intentionally not exposed as an `[sandbox] tier` value or a `--sandbox` flag.

### Changed
- **`symbi doctor`** now reports reachability of `runsc` and `firecracker` binaries in addition to `docker`.
- **Documentation** updated across English + 5 translations (zh-cn, es, pt, ja, de): `docs/security-model.md` rewritten with the three-tier ladder + hosted-execution sidebar, `docs/getting-started.md` documents `.symbi` and `tier3`, `docs/api-reference.md` notes `agents-md` reads `.symbi` first then `.dsl`, `docs/docker.md` and `docs/native-execution-guide.md` updated for the new file extension, `docs/http-input.md` clarifies on-demand LLM invocation reads both extensions.

### Migration notes
- No breaking changes. Existing `.dsl` files continue to work without modification.
- Operators using E2B should review any policies that relied on `E2B → SecurityTier::Tier1` parity — those policies will now correctly fail on hosted execution unless re-scoped.

## [1.11.0] - 2026-04-24

### Added
- **`symbi init` Docker ergonomics**: `init` now accepts `--dir <PATH>` for targeting a mounted volume from inside a container (`docker run -v $(pwd):/workspace ... init --dir /workspace`), generates a ready-to-run `docker-compose.yml` with correct volume mounts and env wiring, and writes a `.env` with a freshly generated `SYMBIONT_MASTER_KEY` (0600 perms) plus a safe-to-commit `.env.example`. Opt out with `--no-docker-compose`. `symbi up` in an empty directory now points the user at `symbi init` instead of silently starting with no agents. `init` is promoted to the first subcommand in `symbi --help`. See `docs/docker.md` for the new 2-command Docker quickstart.
- **`symbi shell` — interactive TUI**: New first-class subcommand providing a ratatui-based terminal UI for building, orchestrating, and operating agents. Inline viewport with live-streaming tool-call cards, async throbber during LLM calls, markdown + diff renderers, toggleable project-structure sidebar, agent-card widget, diff view, and ORGA-phase-colored trace timeline. Command registry with `/help`, `/clear`, `/quit`, `/dsl` toggle, `/model`, `/cost`, `/status`, input history, and session UUIDs. Agent lifecycle: `/agents`, `/debug`, `/stop`, `/pause`, `/destroy`. AI-assisted authoring: `/spawn`, `/policy`, `/tool`, `/behavior` (artifacts are persisted to disk). Orchestration: async orchestrator wired for conversational mode, `/audit` command wired to the ORGA journal, automatic context compaction with `/compact` and `/context`. Ops: `/deploy`, `/ask`, `/send`, `/memory`, `/run`, `/chain`, `/debate`, `/tools`, `/skills`, `/doctor`, `/logs`, `/new`. Remote attach: `/attach`, `/detach`, `/cron` over HTTP; `/channels` via remote attach; `/secrets` via local encrypted store. Session persistence: `/snapshot`, `/resume`, `/export`. Fuzzy `@mention` + `/command` completion with grouped popup, auto-trigger on `/` and `@`, arrow navigation, `@path` completion, DSL-aware completion, in-process DSL evaluation in `/dsl` mode. `/init` with deterministic profiles and conversational mode. Tree-sitter syntax highlighting for the Symbiont DSL plus Cedar and ToolClad. Artifact validation pipeline: constraint loader, DSL validator, Cedar and ToolClad validators. Theme system, OSC-8 hyperlinks, resize handling, transient-retry, Zellij detection with inline-viewport warning, `--yes`, `--profile`.
- **Agent deployment stack**: `/deploy local` via Docker with a hardened sandbox runner, `/deploy cloudrun` for Google Cloud Run (OSS single-agent), and `/deploy aws` for AWS App Runner (OSS single-agent).
- **Cross-instance agent messaging**: `RemoteCommunicationBus` with HTTP messaging endpoints wired into `RuntimeBridge`'s default context. Cron + heartbeat architecture documented in the spec.
- **`symbi-approval-relay` crate**: Dual-channel human approval relay.
- **`symbi schemapin` and `symbi policy` CLI subcommands**.
- **`symbi-invis-strip` crate**: Zero-dependency Unicode invisible-stripping helper (ASCII C0/DEL, C1, zero-width, bidi overrides, word-joiner/invisible-operator block, BOM, variation selectors, Unicode Tag block, supplementary variation selectors). Opt-in `sanitize_field_with_markup` variant additionally strips `<!-- ... -->` HTML comments and triple-backtick fenced blocks for surfaces where renderer-hidden markup has no legitimate use.
- **Cedar policy linter** (`scripts/lint-cedar-policies.py`): Detects homoglyph identifiers and invisible control chars in `.cedar` files. Wired to the repo pre-commit hook and CI test job.
- **AgentPin fully wired; SchemaPin enforcement hardened**.
- **`symbi-e2e` end-to-end test crate**: Covers AgentPin messaging, API auth scope, cross-runtime bus, Docker volumes, messaging ingress, rate limit, and webhook signature verification.
- **Opt-in OpenRouter app attribution**: Runtime now sets the OpenRouter app-name headers when enabled.
- **`symbi repl` shim subcommand** forwards to the `repl-cli` binary (mirrors the existing `symbi shell` shim) so the command every docs page has referenced is now a first-class subcommand rather than a separately-built binary.

### Changed
- **OSS vs Enterprise licensing**: Documented in the spec and plan.
- **Docs rewrite**: `docs/index.md`, `docs/getting-started.md`, `docs/docker.md` lead with a 2-command Docker init flow. New `docs/symbi-shell.md` covers the Beta interactive TUI end-to-end. `docs/repl-guide.md` cross-links to the shell. `docs/api-reference.md` gains a `CLI subcommands` section covering `symbi schemapin`, `symbi policy`, and `symbi agents-md`. `docs/runtime-architecture.md` gains a `Cross-instance agent messaging` subsection. `docs/security-model.md` gains `Invisible-Character Sanitization (symbi-invis-strip)`, `Cedar Policy Linter`, and `Human Approval Relay (symbi-approval-relay)` sections. All five translations (zh-cn, es, pt, ja, de) synced.
- **Staleness sweep**: Fixed broken copy-paste commands across all language variants — `symbiont-runtime` → `symbi-runtime` package name, rewrote the Runtime HTTP API quickstart to use `symbi up --http-bind 0.0.0.0` + `$SYMBI_HTTP_TOKEN` (not the non-existent `symbiont-runtime --http-api`), `docker build -f runtime/Dockerfile .` → `docker build .`, and `symbi-runtime = { version = "1.6" }` snippet → `"1.11"`. Documented the previously-undocumented `symbi new` templates (`webhook-min`, `webscraper-agent`, `slm-first`, `rag-lite`) and the `OPENROUTER_REFERER` / `OPENROUTER_TITLE` env vars.
- **`/attach` scheme policy**: Documentation clarified that `/attach` accepts HTTP **or** HTTPS; `https://` is required for any remote or production target.

### Fixed
- `symbi-shell`: `/spawn`, `/policy`, `/tool`, `/behavior` now actually persist their artifacts; Enter submits on first press even when the completion popup is visible; content scroll fix with all warnings eliminated; batched UX fixes.
- CI: Unblocked minimal build and Docker build, added 4 missing fuzz targets, normalised `cargo fmt` across the workspace, silenced `approx_constant` lint, fixed three release-workflow + test issues exposed by v1.10.0.
- OSS sync: Include `tests/e2e` workspace member in the OSS allowlist and Docker context.

### Security
- **2026-04-18 audit remediation**: Closed H-2/H-3/H-4 (reasoning policy gate, SchemaPin SSRF/TLS, parallel-cap enforcement). Hardened medium-severity findings (M-2..M-11 subset) and low-severity findings (L-1, L-3, L-5, L-6).
- **SystemTime overflow DoS** in the remote envelope parser fixed; Docker proto dependencies and fuzz-target tokio runtime aligned.
- **Agent scope enforcement**: Applied to every `/api/v1` agent, schedule, and channel route.
- **Bus signature verification** enforced; ToolClad custom parsers gated.
- **4 new fuzz targets** for the messaging attack surface.
- **Dependency CVE patches**; remote-bus env var unified; env-touching tests serialised to prevent cross-test interference.

## [1.10.0] - 2026-04-13

### Added
- **HTTP Input LLM invocation with ToolClad**: When the target agent is not `Running` on the communication bus, the webhook handler now falls back to an on-demand LLM invocation path that runs an ORGA-style tool-calling loop against ToolClad manifests. Tools execute on a blocking thread pool with a 120-second per-tool timeout. Duplicate `(tool, input)` pairs within a single iteration are deduplicated. Provider auto-detected from `OPENROUTER_API_KEY`, `OPENAI_API_KEY`, or `ANTHROPIC_API_KEY`.
- **Normalized LLM tool-calling client**: `LlmClient::chat_with_tools` returns a unified content-block shape across Anthropic (native `tool_use`) and OpenAI/OpenRouter (function calling normalized to the same format).
- **Webhook response metadata**: LLM-invoked responses include `response`, `tool_runs`, `model`, `provider`, `latency_ms`, and `status: completed`.

### Fixed
- **HTTP Input: agent state check before communication bus dispatch**: `invoke_agent` now verifies the target agent is in the `Running` state via `scheduler.get_agent_status()` before sending a message. Previously `send_message` returned `Ok` for unregistered agents and delivery failed silently, producing a false `"execution_started"` response.
- **HTTP Input: UTF-8 safe string truncation**: Tool output previews and caller-supplied `system_prompt` values are truncated on UTF-8 character boundaries to prevent panics on multi-byte output.
- **HTTP Input: system_prompt length cap**: Caller-supplied `system_prompt` is now capped at 4096 bytes and logged; remains a prompt-injection surface when exposed to untrusted callers.

## [1.9.1] - 2026-04-01

### Changed
- **LanceDB is now an optional build feature**: The `lancedb`, `arrow-array`, and `arrow-schema` dependencies are gated behind the `vector-lancedb` feature flag. LanceDB remains in the default feature set so existing builds are unaffected. Build without vector backends using `--no-default-features` for lighter binaries.

### Fixed
- **README restructured**: Tighter positioning as "policy-governed agent runtime", trimmed capabilities table, simplified DSL example, softened benchmark claims, clarified Community/Enterprise editions
- **Documentation alignment**: Docs index, SECURITY.md support matrix, Dockerfile port comments, and all translations updated to match
- **Version consistency**: Fixed Rust version mismatch (1.88 → 1.82) across all docs and READMEs
- **Dead link**: Removed enterprise/README.md reference from public READMEs
- **Speculative docs**: Removed planned Risk Assessment Algorithm sections from security-model and runtime-architecture docs

## [1.9.0] - 2026-03-29

### Added
- **ToolClad runtime integration**: Manifest loading, argument validation, and command execution for declarative tool contracts
- **ToolClad extended types**: Output parsers, custom types, schema validation, and tools init
- **ToolClad session and browser mode**: SessionExecutor and BrowserExecutor support for ToolClad v0.4.0+ spec
- **HTTP backend and MCP proxy backend**: ToolClad backends with secrets injection
- **`symbi tools` CLI**: Scope enforcement, Cedar policy generation, and hot-reload file watcher
- **ToolClad manifests**: Built-in tool contracts for whois, nmap, dig, and curl
- **W3C traceparent propagation**: OpenTelemetry distributed trace context across agent boundaries with integration tests
- **Production readiness**: Bounded channels, health probes, secrets TTL, Cedar reload, audit export, and rate limiting
- **A2UI v0.2.0**: Updated Agent-to-UI interface for Symbiont v1.9.0 compatibility

### Changed
- **BrowserDef v0.5.1**: Add connect, extract_mode, and default engine to CDP configuration

### Fixed
- **Critical security fixes**: DoS vector mitigation, JWT validation hardening, environment variable leakage prevention, sandbox guard improvements
- **Concurrency and resource exhaustion**: Address priority queue, Composio auth, and vector DB allocation issues
- **Memory efficiency**: Resource exhaustion and memory efficiency review improvements
- **Documentation accuracy**: Fix 6 documentation accuracy issues from external review
- **Fuzz workflow**: ComposioError variant, unused imports, cfg-gated vars

### Crate Versions
| Crate | Version |
|-------|---------|
| `symbi` | 1.9.0 |
| `symbi-dsl` | 1.9.0 |
| `symbi-runtime` | 1.9.0 |
| `symbi-channel-adapter` | 0.1.2 |
| `repl-core` | 1.9.0 |
| `repl-proto` | 1.9.0 |
| `repl-cli` | 1.9.0 |
| `repl-lsp` | 1.9.0 |

## [1.8.1] - 2026-03-16

### Added
- **`symbi run` command**: Execute any agent directly from the CLI without starting the full runtime
- **Hash comments**: Support `#` line comments in DSL files

### Fixed
- **Default Anthropic model name**: Corrected default model identifier
- **Agent names in API list**: Proper name display in agent listing endpoint
- **Usability improvements**: Auth clarity, cedar flag docs, auto-routing, rust-version alignment
- **Formatting**: cargo fmt compliance

### Crate Versions
| Crate | Version |
|-------|---------|
| `symbi` | 1.8.1 |
| `symbi-dsl` | 1.8.1 |
| `symbi-runtime` | 1.8.1 |
| `symbi-channel-adapter` | 0.1.2 |
| `repl-core` | 1.8.1 |
| `repl-proto` | 1.8.1 |
| `repl-cli` | 1.8.1 |
| `repl-lsp` | 1.8.1 |

## [1.8.0] - 2026-03-13

### Added
- **`symbi init` command**: Interactive project scaffolding with profile-based templates (minimal, assistant, dev-agent, multi-agent)
- **Agent catalog**: Built-in catalog with list and import for pre-built governed agents
- **Inter-agent communication bus**: CommunicationBus with policy evaluation for all builtins (`ask`, `delegate`, `send_to`, `parallel`, `race`)
- **CommunicationPolicyGate**: Cedar-style rule enforcement for inter-agent calls with priority-based evaluation and hard deny

### Changed
- **CI workflow**: Replace arduino/setup-protoc with native package managers (Node.js 20 deprecation)

### Crate Versions
| Crate | Version |
|-------|---------|
| `symbi` | 1.8.0 |
| `symbi-dsl` | 1.8.0 |
| `symbi-runtime` | 1.8.0 |
| `symbi-channel-adapter` | 0.1.2 |
| `repl-core` | 1.8.0 |
| `repl-proto` | 1.8.0 |
| `repl-cli` | 1.8.0 |
| `repl-lsp` | 1.8.0 |

## [1.7.1] - 2026-03-11

### Added
- **AI Assistant Plugin docs**: Document symbi-claude-code and symbi-gemini-cli governance plugins in README, getting-started, and index docs
- **SchemaPin discovery JSON**: Support SchemaPin discovery JSON format in `fetch_public_key`
- **Cosign binary signing**: Release workflow now signs binaries with cosign

### Changed
- **Drop Intel macOS builds**: Remove x86_64-apple-darwin target from release workflow; install script provides source/Homebrew guidance
- **Cross-build optimization**: Use thin LTO and 4 codegen units for cross builds to avoid OOM during linking
- **README images**: Use absolute GitHub URLs for logo images

### Fixed
- **Release workflow**: Multiple fixes for cross-compilation (protoc in cross container, vcpkg OpenSSL on Windows, NASM for Windows builds)
- **Publish workflow**: Improved reliability for crates.io publishing

### Crate Versions
| Crate | Version |
|-------|---------|
| `symbi` | 1.7.1 |
| `symbi-dsl` | 1.7.1 |
| `symbi-runtime` | 1.7.1 |
| `symbi-channel-adapter` | 0.1.2 |
| `repl-core` | 1.7.1 |
| `repl-proto` | 1.7.1 |
| `repl-cli` | 1.7.1 |
| `repl-lsp` | 1.7.1 |

## [1.7.0] - 2026-03-08

### Added

#### Standalone Agent SDK (Phase 1)
- **`symbi_runtime::prelude`**: One-import module for standalone agent development — re-exports reasoning loop, executors, providers, policy gates, and types
- **`ReasoningLoopRunner::builder()`**: Typestate builder pattern with compile-time enforcement of required fields (provider → executor → build)
- **`ToolFilterPolicyGate`**: Tool-name whitelisting gate — restricts which tools an agent can invoke without requiring full Cedar policies
- **`tool_definitions()` on `ActionExecutor` trait**: Enables executors to self-describe available tools for LLM function-calling
- **`cloud-llm` and `standalone-agent` feature flags**: Lighter builds for agents that don't need the full runtime

#### External Agent Integration (Phase 2)
- **External execution mode**: New `ExecutionMode::External` for agents running outside the coordinator
- **`Unreachable` agent state**: Detects when external agents stop sending heartbeats
- **Heartbeat and push-event HTTP endpoints**: `/agents/{id}/heartbeat` and `/agents/{id}/events` for external agent liveness and event reporting
- **Scheduler support**: External agents register with the scheduler but skip the execution queue — coordinator tracks their status without managing their lifecycle
- **Extended `CreateAgentRequest`**: DSL field now optional for external agents; `AgentStatusResponse` includes new fields for external agent metadata

#### Advanced Reasoning Primitives (`orga-adaptive` feature)
- **Tool profiling**: Runtime performance tracking per tool behind feature gate
- **Step iteration**: Per-step iteration controls for reasoning loop
- **Pre-hydration**: Context pre-loading before reasoning loop execution
- **Scoped knowledge bridge**: Directory and scope-based knowledge routing

#### Pre-Built Binary Releases
- **`vendored-openssl` feature**: Static OpenSSL linking for portable binary distribution
- **Release workflow**: GitHub Actions cross-compilation for 5 targets (linux x86_64/aarch64, macOS x86_64/aarch64, Windows x86_64)
- **SHA256 checksum verification**: Install script verifies binary integrity
- **Homebrew tap**: `brew tap thirdkeyai/tap && brew install symbi`
- **Cross.toml**: aarch64-linux cross-compilation configuration

#### Coordinator Chat
- **WebSocket-based coordinator chat panel**: Real-time agent interaction with Markdown rendering and thinking indicators
- **`ComposioToolExecutor`**: Integration with Composio MCP for external tool execution
- **Observation `call_id` tracking**: Proper correlation of tool calls to observations

### Fixed
- **Context test isolation**: Tests use isolated temp dirs to prevent cross-test interference from shared filesystem state
- **Reasoning loop robustness**: Token estimation, context management, and Anthropic protocol compliance improvements
- **Formatting**: Fixed `cargo fmt` issues in context_manager, conversation, and phases modules
- **Production safety**: Fixed path traversal, panic-on-unwrap, and potential secret leaks
- **Async I/O**: Replaced blocking `std::fs` calls with async equivalents in async contexts

### Changed
- **Feature flag rename**: `symbi-dev` → `orga-adaptive` for advanced reasoning primitives
- **Apache 2.0 license**: Project relicensed from MIT to Apache 2.0
- **Copyright update**: 2024-2026 Jascha Wanger / ThirdKey AI

### Crate Versions
| Crate | Version |
|-------|---------|
| `symbi` | 1.7.0 |
| `symbi-dsl` | 1.7.0 |
| `symbi-runtime` | 1.7.0 |
| `symbi-channel-adapter` | 0.1.2 |
| `repl-core` | 1.7.0 |
| `repl-proto` | 1.7.0 |
| `repl-cli` | 1.7.0 |
| `repl-lsp` | 1.7.0 |

## [1.6.1] - 2026-02-27

### Fixed
- **qdrant-client version pin**: Pin `qdrant-client` to `>=1.14.0, <1.16.0` to prevent API breakage from v1.16+ (fields added to `CreateCollection`, `UpsertPoints`, `DeletePoints`, `CreateFieldIndexCollection`)

### Crate Versions
| Crate | Version |
|-------|---------|
| `symbi` | 1.6.1 |
| `symbi-dsl` | 1.6.1 |
| `symbi-runtime` | 1.6.1 |
| `symbi-channel-adapter` | 0.1.1 |
| `repl-core` | 1.6.1 |
| `repl-proto` | 1.6.1 |
| `repl-cli` | 1.6.1 |
| `repl-lsp` | 1.6.1 |

## [1.6.0] - 2026-02-27 [YANKED]

> **Yanked from crates.io**: All 7 crates at v1.6.0 have been yanked due to a
> `qdrant-client` semver breakage that caused `cargo install` to fail. Use v1.6.1 instead.

### Added

#### ClawHavoc Scanner Expansion
- **30 new detection rules** across 7 attack categories: reverse shells (7 rules), credential harvesting (6), network exfiltration (3), process injection (4), privilege escalation (5), symlink/path traversal (2), downloader chains (3)
- **5-level severity model**: Critical, High, Medium, Warning, Info — scans fail on Critical or High findings (previously only Critical)
- **`AllowedExecutablesOnly` custom rule type**: Whitelist-based executable filtering for strict sandboxed environments

#### Agent Registry & Lifecycle
- **Persistent `AgentRegistry`**: Store and retrieve agent metadata with delete and re-execute lifecycle support

#### AGENTS.md Support
- **Full bidirectional AGENTS.md**: Generate and parse agent manifest files for ecosystem interoperability

#### Performance Verification
- **Benchmarked performance claims**: Policy evaluation <1ms, ECDSA P-256 <5ms, SchemaPin verification <5ms, 10k agent scheduling <2% CPU overhead
- **Debug/release threshold split**: Relaxed thresholds for debug builds (unoptimized crypto) while preserving real claims for release

#### Fuzzing Expansion
- **6 new fuzz targets**: `dsl_evaluator`, `mattermost_signature_verification`, `crypto_roundtrip`, `webhook_verify_generic`, `api_key_store`, `policy_evaluation` — total now 18 targets

#### Agentic Reasoning Loop (Phases 1–5)
- **Typestate-enforced ORGA cycle**: Observe-Reason-Gate-Act loop with compile-time phase transition safety (`AgentLoop<Reasoning>` → `PolicyCheck` → `ToolDispatching` → `Observing`). Invalid transitions are caught at compile time via zero-sized type markers
- **Unified inference providers**: `InferenceProvider` trait with `CloudInferenceProvider` (OpenRouter, OpenAI, Anthropic) and local SLM support. Model auto-detection from `OPENROUTER_API_KEY` / `OPENROUTER_MODEL` environment variables
- **Policy-gated reasoning**: Every proposed action evaluated by `ReasoningPolicyGate` before execution — deny, allow, or modify. Cedar policy engine integration via `CedarGate`
- **Action executor with circuit breakers**: Parallel tool dispatch via `FuturesUnordered`, per-tool timeouts, and `CircuitBreakerRegistry` with configurable failure thresholds and recovery windows
- **Durable execution journal**: `BufferedJournal` with sequenced `JournalEntry` events for loop replay and debugging. Replaces `NoOpJournal`
- **Human-in-the-loop critic**: `HumanCritic` integration for approval workflows within the reasoning loop
- **Multi-agent patterns**: `AgentRegistry` for persistent agent metadata, `Saga` pattern for multi-step distributed operations with checkpoints
- **Structured output validation**: `OutputSchema` + `ValidationPipeline` for schema-validated LLM responses
- **Context token budget enforcement**: In-loop `ContextManager` with sliding window, observation masking, and anchored summary strategies
- **DSL reasoning builtins**: `reason`, `llm_call`, `parse_json`, `tool_call`, and `delegate` builtins wired into the REPL
- **Observability**: OpenTelemetry tracing spans (`tracing_spans`), reasoning loop metrics (`metrics`), and phase scheduling (`scheduler`)
- **Live integration tests**: Full loop tests with real LLM inference via OpenRouter

#### Knowledge-Reasoning Bridge
- **`KnowledgeBridge`**: Opt-in bridge between `context::ContextManager` (agent memory/knowledge) and the reasoning loop. Configurable via `KnowledgeConfig` (max items, relevance threshold, auto-persist)
- **Context injection**: Retrieves relevant knowledge via `query_context()` + `search_knowledge()` and injects as a replaceable system message before each reasoning step
- **`recall_knowledge` tool**: LLM-callable tool that searches the agent's knowledge base with configurable result limits
- **`store_knowledge` tool**: LLM-callable tool that stores new facts (subject/predicate/object triples) into the agent's knowledge base
- **`KnowledgeAwareExecutor`**: Wraps the inner `ActionExecutor`, intercepts knowledge tool calls locally, delegates all others to the real executor
- **Post-loop persistence**: Automatically stores conversation learnings as episodic memory after loop completion (when `auto_persist` is enabled)
- **Backward compatible**: `ReasoningLoopRunner` works identically without a knowledge bridge (`knowledge_bridge: None`)

#### Infrastructure
- **Docker build optimization**: cargo-chef caching, split CI/release build profiles, nproc-based parallelism auto-detection
- **v1.6.0 roadmap**: Agent discovery, remote transport, and DSL A2A primitives planned across 5 phases

### Fixed
- **cargo-chef cook**: Create stub for `[[example]]` entries not handled by cargo-chef
- **ECDSA benchmark threshold**: Debug builds no longer fail due to unoptimized crypto exceeding release-only 5ms threshold
- **SchemaPin verification threshold**: Same debug/release split applied to pinned-key verification benchmark

### Crate Versions
| Crate | Version |
|-------|---------|
| `symbi` | 1.6.0 |
| `symbi-dsl` | 1.6.0 |
| `symbi-runtime` | 1.6.0 |
| `symbi-channel-adapter` | 0.1.1 |
| `repl-core` | 1.6.0 |
| `repl-proto` | 1.6.0 |
| `repl-cli` | 1.6.0 |
| `repl-lsp` | 1.6.0 |

## [1.5.0] - 2026-02-22

### Added

#### LanceDB Embedded Vector Backend
- **`VectorDb` trait abstraction**: Backend-agnostic async trait (`initialize`, `store`, `store_batch`, `search`, `delete`, `count`, `drop_collection`, `health_check`) with unified `VectorSearchResult` and typed `VectorDbError`
- **LanceDB as default embedded backend**: Zero-config vector search using Arrow-based `lancedb` crate — no Docker, no external services required. Default data path: `./data/vector_db/`
- **Qdrant moved to optional backend**: Existing Qdrant support preserved behind `vector-qdrant` feature flag (`qdrant-client` dep gated on `#[cfg(feature = "vector-qdrant")]`)
- **Backend factory**: `resolve_vector_config()` and `create_vector_backend()` select backend via `SYMBIONT_VECTOR_BACKEND` env var, config file, or default to LanceDB
- **Consumer updates**: `StandardRAGEngine` and `StandardContextManager` now accept `Arc<dyn VectorDb>` instead of concrete `QdrantClientWrapper`

#### Context Compaction Pipeline
- **`TokenCounter` trait**: Pluggable token counting with `TiktokenCounter` (model-aware via `tiktoken-rs`) and `HeuristicTokenCounter` fallback
- **`create_token_counter` factory**: Tiered resolution — tiktoken for known models, heuristic for unknown
- **Tier 1 Summarize**: LLM-driven condensation of oldest conversation items when context exceeds threshold
- **Tier 4 Truncate**: Drop oldest conversation items as last-resort when budget is critically exceeded
- **Enterprise tier stubs**: Tier 2 (episodic compression) and Tier 3 (archive to memory) gated behind `enterprise-compaction` feature
- **`select_tier` pipeline orchestrator**: Evaluates tiers in order, returns first applicable `CompactionResult`
- **`check_and_compact` integration**: Wired into `StandardContextManager` for automatic compaction on context operations
- **`CompactionMetrics`**: Token counts and tier usage exposed in runtime metrics snapshot

#### Composio MCP Integration
- **Feature-gated `composio` module**: SSE-based connection to Composio MCP server for external tool access (uses existing `reqwest` dependency)

#### Security Hardening
- **Structure-aware fuzz targets**: 5 new fuzz targets for DSL parsing, SSE/JSON-RPC protocol, SchemaPin verification, Slack signature validation, and TOFU key substitution
- **API middleware hardening**: Trusted proxy configuration, fail-closed rate limiting, deprecated static token auth
- **Audit TOCTOU fix**: Eliminated time-of-check/time-of-use race in audit trail writes
- **Vault secret heuristic**: Improved detection of secret values in Vault backend responses

### Changed
- Default vector backend is now LanceDB (previously Qdrant was required)
- Docker Compose examples no longer include Qdrant by default
- Development setup no longer requires `docker-compose up -d qdrant`
- **`RoutingStatistics`**: Replaced `Arc<RwLock<RoutingStatistics>>` with lock-free `AtomicU64` counters — eliminates write-lock contention on every routed request
- **`SlmExecutor` trait**: Extracted from inline mock — enables dependency injection for SLM execution
- **`LLMClient` trait + `LLMClientPool`**: Public trait and registry pattern replace hardcoded `MockLLMClient` — empty pool by default, consumers call `register()`
- **Fallback tracking**: Consolidated duplicate fallback counting into single `fallback_to_llm` helper
- **Relaxed `base64ct` pin**: Changed from `=1.6.0` to `^1` to allow compatible upgrades

### Removed
- **`SchemaPinCliWrapper`**: Deleted legacy Go CLI binary wrapper (516 lines) — native Rust `schemapin` crate handles all operations
- **`ConfidenceMonitor` stub**: Removed dead `ConfidenceConfig`, `ConfidenceStatistics`, and `ConfidenceMonitor` types — trait + `NoOpConfidenceMonitor` retained
- **`MockLLMClient` from public API**: Moved behind `#[cfg(test)]` — use `LLMClientPool::register()` for production clients
- **`execute_slm_mock`**: Deleted — replaced by `SlmExecutor` trait injection
- **Enterprise dead code**: Removed commented-out enterprise module stubs and unused re-exports from OSS build

### Crate Versions
| Crate | Version |
|-------|---------|
| `symbi` | 1.5.0 |
| `symbi-dsl` | 1.5.0 |
| `symbi-runtime` | 1.5.0 |
| `symbi-channel-adapter` | 0.1.1 |
| `repl-core` | 1.5.0 |
| `repl-proto` | 1.5.0 |
| `repl-cli` | 1.5.0 |
| `repl-lsp` | 1.5.0 |

## [1.4.0] - 2026-02-16

### Added

#### HTTP Input Security Hardening
- **Loopback-only default binding**: `bind_address` defaults to `127.0.0.1` instead of `0.0.0.0`
- **Explicit CORS origin allow-lists**: Replaced `cors_enabled` boolean with `cors_origins: Vec<String>`
- **JWT EdDSA validation**: Full Ed25519 public key loading and JWT verification in auth middleware
- **Health endpoint separation**: `/health` exempt from authentication for load balancers
- **PathPrefix route matching**: Implement `RouteMatch::PathPrefix` in HTTP input routing
- **Runtime agent execution**: Replace `invoke_agent` stub with real runtime dispatch

#### Persistent Memory (`MarkdownMemoryStore`)
- **Markdown-backed agent memory** implementing `ContextPersistence` trait
  - Facts, Procedures, and Learned Patterns sections in `memory.md`
  - Daily log append with timestamped entries
  - Retention-based compaction with configurable max age
- **DSL `memory` block**: Declarative memory configuration in agent definitions
  - `store`: Storage format (`markdown`)
  - `path`: File path for memory persistence
  - `retention`: Duration-based retention (`90d`, `6months`, etc.) via `humantime`
  - `compact_after`: Compaction threshold
- **REPL `:memory` command**: Inspect, compact, and purge agent memory at runtime

#### Webhook DX (`SignatureVerifier`)
- **`SignatureVerifier` trait** with two implementations:
  - `HmacVerifier`: HMAC-SHA256 with constant-time comparison via `subtle` crate
  - `JwtVerifier`: HS256 JWT token verification
- **`WebhookProvider` presets**: GitHub, Stripe, Slack, Custom — each maps provider name to correct header and signing scheme
- **DSL `webhook` block**: Declarative webhook endpoint configuration
  - `provider`: Provider preset name
  - `secret`: Secret key or environment variable reference (`$ENV_VAR`)
  - `path`: HTTP endpoint path
  - `filter`: Event type filtering
- **Wired into `HttpInputServer`**: Pre-handler signature verification on raw `Bytes` before JSON parsing. Returns 401 on failure, 400 on bad JSON.
- **REPL `:webhook` command**: List configured webhook endpoints

#### Skill Scanning (ClawHavoc)
- **`SkillScanner`** with 10 built-in defense rules for detecting malicious patterns in agent skills:
  - `pipe-to-shell` (Critical): `curl ... | sh`
  - `wget-pipe-to-shell` (Critical): `wget ... | sh`
  - `env-file-reference` (Warning): References to `.env` files
  - `soul-md-modification` (Critical): Attempts to rewrite `SOUL.md`
  - `memory-md-modification` (Critical): Attempts to rewrite `MEMORY.md`
  - `eval-with-fetch` (Critical): `eval()` + network fetch
  - `fetch-with-eval` (Critical): Network fetch + `eval()`
  - `base64-decode-exec` (Critical): Base64 decode piped to shell
  - `rm-rf-pattern` (Critical): `rm -rf /`
  - `chmod-777` (Warning): World-writable permissions
- **Automatic scanning on skill load**: Every text file in the skill directory scanned line-by-line
- **Custom rules**: Add domain-specific regex patterns alongside ClawHavoc defaults
- **SchemaPin integration**: Skills are both signature-verified and content-scanned

#### Metrics & Telemetry
- **`FileMetricsExporter`**: Atomic JSON file writes (tempfile + rename) for metric snapshots
- **`OtlpExporter`**: Send metrics to any OpenTelemetry-compatible endpoint via gRPC or HTTP (behind `metrics` feature flag)
- **`CompositeExporter`**: Fan-out to multiple backends simultaneously; individual failures logged but don't block others
- **`MetricsCollector`**: Background thread for periodic snapshot collection from scheduler, task manager, load balancer, and system resources
- **`/api/v1/metrics` endpoint**: Full snapshot covering job counts, task queue depths, worker utilization, CPU, and memory usage

#### DSL Parser Fixes
- **Bare identifier in `value` rule**: `store markdown`, `provider github` now parse correctly
- **Short-form duration literals**: `90d`, `6m`, `1y` alongside existing `90.seconds` form
- **Conflict resolution**: `conflicts` declaration for `expression`/`value` ambiguity

### SDK Parity (v0.6.0)

Both SDKs ship at v0.6.0 with full feature parity:

- **Python SDK** ([PyPI](https://pypi.org/project/symbiont-sdk/0.6.0/)): `MarkdownMemoryStore`, `HmacVerifier`, `JwtVerifier`, `WebhookProvider`, `SkillScanner`, `SkillLoader` with SchemaPin integration, `MetricsClient`, `FileMetricsExporter`, `CompositeExporter` — 120 tests passing
- **JavaScript SDK** ([npm](https://www.npmjs.com/package/symbiont-sdk-js)): `MarkdownMemoryStore`, `HmacVerifier`, `JwtVerifier`, `WebhookProvider`, `SkillScanner` with all 10 ClawHavoc rules, `MetricsApiClient`, `FileMetricsExporter` — 1,037 tests passing

### Crate Versions
| Crate | Version |
|-------|---------|
| `symbi` | 1.4.0 |
| `symbi-dsl` | 1.4.0 |
| `symbi-runtime` | 1.4.0 |
| `symbi-channel-adapter` | 0.1.1 |
| `repl-core` | 1.4.0 |
| `repl-proto` | 1.4.0 |
| `repl-cli` | 1.4.0 |
| `repl-lsp` | 1.4.0 |

## [1.1.0] - 2026-02-12

### Added

#### Security Hardening v2 (symbi-runtime)
- **Per-agent API key authentication** with Argon2 hashing and file-backed key store
- **Per-IP rate limiting middleware** wired into HTTP router (governor, 100 req/min)
- **Schema-driven argument redaction** via `sensitive_params` on MCP tools
- **File locking for secret store** reads (fd-lock shared read locks)
- **Safe sandbox defaults**: empty `allowed_executables`, shell warnings

#### DSL Improvements (symbi-dsl)
- **Structured `DslDiagnostic` type** replacing println-based error reporting
- **Humantime-based timeout parsing** with backward-compatible `.seconds` suffix

#### symbi-a2ui (experimental/alpha)
- New Lit-based admin UI for fleet management, compliance dashboards, and audit trail viewing
- Not published to npm — private, experimental

### Fixed
- **Teams Auth** (symbi-channel-adapter): Migrated to jsonwebtoken v10 API with proper claim validation

### Crate Versions
| Crate | Version |
|-------|---------|
| `symbi` | 1.1.0 |
| `symbi-dsl` | 1.1.0 |
| `symbi-runtime` | 1.1.0 |
| `symbi-channel-adapter` | 0.1.1 |
| `repl-core` | 1.0.1 |

## [1.0.1] - 2026-02-11

### Added

#### AgentPin Integration
- **DiscoveryMode Resolver Dispatch**: Multi-strategy agent identity resolution
  - `StaticDocument`: Use a pre-loaded discovery document (offline/testing)
  - `WellKnown`: Fetch `.well-known/agent-identity.json` over HTTPS (default)
  - `DnsRecord`: DNS TXT record lookup (future)
  - Automatic fallback chain: Static → WellKnown → DnsRecord
- **agentpin 0.2.0**: Switched from local path dependency to crates.io release
  - Trust bundle support for fully offline verification
  - Enhanced discovery document validation

#### MCP Server
- **Real MCP Server**: Replaced stub with full MCP server over stdio using `rmcp` SDK
  - `symbi mcp` command now serves a proper MCP protocol endpoint
  - Tool registration, invocation, and result marshalling

#### Channel Adapters
- **Bidirectional Slack Adapter**: Real-time Slack integration (Phase 1)
  - Socket Mode for event streaming
  - Message sending and channel management
- **Teams & Mattermost Adapters**: Additional chat platform support
  - Microsoft Teams webhook and Bot Framework integration
  - Mattermost WebSocket and REST API integration
- **Channel Management REST API**: CRUD endpoints for channel configurations
- **Declarative `channel {}` Block**: DSL grammar support for channel definitions
- **Enterprise Channel Governance**: Policy enforcement for channel operations

#### Infrastructure
- **`.claude/` Release Documentation**: Added CLAUDE.md development guidelines and RELEASE_RUNBOOK.md
- **ROADMAP.md**: v1.1.0+ release planning document

### Fixed
- **Docker Build Cache**: Fixed cleanup glob to include `libsymbi*` and `.fingerprint/symbi*` cached artifacts
- **Clippy**: Use `derive(Default)` for DiscoveryMode enum instead of manual impl
- **CI Tests**: Fixed all failing CI tests and runtime bugs
- **Compilation Warnings**: Resolved warnings, made Qdrant optional
- **Runtime Init**: Fixed auth header and model defaults for HTTP-only mode

### Changed
- **Docker Base Image**: Bumped Rust image to 1.88 for dependency compatibility
- **OSS Sync**: Hardened sync script with dry-run mode, interactive prompts, expanded safety checks

### Crate Versions
| Crate | Version |
|-------|---------|
| `symbi` | 1.0.1 |
| `symbi-dsl` | 1.0.1 |
| `symbi-runtime` | 1.0.2 |
| `repl-core` | 1.0.1 |

## [1.0.0] - 2026-02-07

### Added

#### 🎯 Production-Ready Scheduling (v1.0.0)
- **Session Isolation**: Per-run AgentContext with HeartbeatContextMode control
  - `EphemeralWithSummary`: Fresh context per iteration with summary carryover (default)
  - `SharedPersistent`: Persistent context across all iterations
  - `FullyEphemeral`: Stateless execution with no context carryover
  - Prevents unbounded memory growth in long-running heartbeat agents
- **Jitter Support**: Random 0-N second delay to prevent thundering herd
  - Configurable `max_jitter_seconds` in CronSchedulerConfig
  - Spreads job starts across time window when multiple jobs share a schedule
- **Per-Job Concurrency Guards**: Limit concurrent runs per job
  - `max_concurrent` field on CronJobDefinition
  - Prevents resource exhaustion from overlapping executions
  - Scheduler skips tick when job at max concurrency
- **Dead-Letter Queue**: Jobs exceeding max_retries move to DeadLetter status
  - Manual review and recovery workflow via CLI
  - Audit trail of failure patterns
  - `symbi cron reset <job-id>` to reactivate after fixing
- **CronMetrics Observability**: Comprehensive metrics collection
  - `runs_total`, `runs_succeeded`, `runs_failed` counters
  - `execution_duration_seconds` histogram
  - `in_flight_jobs` gauge
  - `dead_letter_total` counter
  - Prometheus-compatible export
- **CronSchedulerHealth Endpoint**: `/api/v1/health/scheduler` for monitoring
  - Active/paused/in-flight job counts
  - Aggregated metrics and performance data
  - Integration with ops monitoring systems
- **AgentPin JWT Field**: Cryptographic identity verification on CronJobDefinition
  - ES256 (ECDSA P-256) signature verification
  - Domain-anchored agent identity
  - `require_agent_pin` policy enforcement
  - Prevents unauthorized agent execution

#### 🔒 Security Enhancements
- **New SecurityEventType Variants**:
  - `CronJobDeadLettered`: Job moved to dead-letter queue after max retries
  - `AgentPinVerificationFailed`: AgentPin JWT validation failure
- **Policy Enforcement**: Enhanced security checks before scheduled execution
  - Time window restrictions
  - Capability requirements
  - Human approval workflows
  - AgentPin cryptographic verification

#### 📋 Documentation
- **Comprehensive Scheduling Guide**: [`docs/scheduling.md`](docs/scheduling.md)
  - Complete architecture overview
  - DSL syntax reference (cron and at/one-shot)
  - CLI command reference
  - Heartbeat pattern guide
  - Session isolation strategies
  - Delivery routing configuration
  - Policy enforcement examples
  - Production hardening best practices
  - HTTP API endpoint reference
  - SDK examples (JS + Python)
  - Configuration reference

### Improved

#### Reliability & Stability
- **Graceful Shutdown**: Enhanced in-flight job tracking during scheduler shutdown
- **Error Recovery**: Better retry logic with exponential backoff
- **State Management**: More robust job state transitions with ACID guarantees
- **Audit Trail**: Complete lifecycle tracking for all scheduled jobs

#### Performance
- **Optimized Tick Loop**: Reduced scheduler overhead with efficient job selection
- **Concurrent Execution**: Improved throughput with configurable global concurrency
- **Resource Management**: Better CPU and memory utilization tracking

### Fixed
- **Scheduler Stability**: Resolved edge cases in job state management
- **Heartbeat Memory Leaks**: Fixed unbounded context growth in long-running agents
- **Concurrency Deadlocks**: Eliminated potential deadlocks under high load
- **Metric Collection**: Fixed race conditions in metrics aggregation

### Breaking Changes
- **CronJobDefinition Schema**: Added new fields (`max_concurrent`, `agent_pin_jwt`)
  - Migration: Existing jobs work without changes (optional fields default to safe values)
- **HeartbeatContextMode**: New enum for session isolation control
  - Migration: Defaults to `EphemeralWithSummary` (previous behavior)

### Migration from v0.9.0
No breaking API changes. All v0.9.0 scheduled jobs continue to work.

Optional enhancements:
1. Add `max_concurrent` limits to high-frequency jobs
2. Enable `max_jitter_seconds` to spread job starts
3. Configure `agent_pin_jwt` for identity verification
4. Set `default_max_retries` to enable dead-letter queue

## [0.9.0] - 2026-01-15

### Added

#### 🚀 Delivery Routing & Policy Enforcement
- **DeliveryRouter Trait**: Pluggable output routing system
  - [`crates/runtime/src/scheduler/delivery.rs`](crates/runtime/src/scheduler/delivery.rs): Core delivery abstractions
  - DefaultDeliveryRouter implementation with multiple channels
- **Delivery Channels**: Six built-in output destinations
  - **Webhook**: HTTP POST with configurable headers and authentication
  - **Slack**: Slack webhook and API integration
  - **Email**: SMTP email delivery with templates
  - **Custom**: User-defined delivery handlers
  - **Stdout**: Console output for development
  - **LogFile**: Job-specific log file persistence
- **PolicyGate**: Schedule policy enforcement before execution
  - [`crates/runtime/src/scheduler/policy.rs`](crates/runtime/src/scheduler/policy.rs): Policy evaluation engine
  - Integration with RealPolicyParser (replacing stub in repl-core)
  - Support for time windows, capabilities, approvals, and AgentPin requirements
- **Real Policy Parser**: Production-grade policy evaluation
  - Replaced stub implementation with full policy DSL support
  - Condition evaluation with complex boolean logic
  - Integration with security audit trail

#### 🌐 HTTP API Schedule Endpoints
- **Complete Schedule Management API**: 10 RESTful endpoints with OpenAPI annotations
  - `POST /api/v1/schedule`: Create new scheduled job
  - `GET /api/v1/schedule`: List all jobs (filterable by status, agent ID)
  - `GET /api/v1/schedule/{job_id}`: Get job details
  - `PUT /api/v1/schedule/{job_id}`: Update job configuration
  - `DELETE /api/v1/schedule/{job_id}`: Delete job
  - `POST /api/v1/schedule/{job_id}/pause`: Pause job
  - `POST /api/v1/schedule/{job_id}/resume`: Resume paused job
  - `POST /api/v1/schedule/{job_id}/run`: Trigger immediate execution
  - `GET /api/v1/schedule/{job_id}/history`: Get run history
  - `GET /api/v1/schedule/{job_id}/next_run`: Get next scheduled run time
- **OpenAPI Integration**: Full Swagger/OpenAPI 3.0 documentation
  - Interactive API explorer with Swagger UI
  - Request/response schema definitions
  - Authentication examples

#### 📦 SDK Integration
- **JavaScript SDK ScheduleClient**: Complete TypeScript SDK for schedule management
  - [`symbiont-sdk-js/src/schedule.ts`](../symbiont-sdk-js/src/schedule.ts): Schedule client implementation
  - Full CRUD operations for jobs
  - Run history and status queries
  - Webhook and Slack delivery configuration
- **Python SDK ScheduleClient**: Full Python SDK with async support
  - [`symbiont-sdk-python/src/symbiont/schedule.py`](../symbiont-sdk-python/src/symbiont/schedule.py): Schedule client implementation
  - Type hints and dataclass models
  - Idiomatic Python API design

### Improved

#### Developer Experience
- **SDK Examples**: Comprehensive examples in both JavaScript and Python
  - Job creation and lifecycle management
  - Delivery routing configuration
  - Policy enforcement patterns
- **API Documentation**: Enhanced endpoint documentation with usage examples
- **Error Handling**: Better error messages for delivery failures and policy violations

#### Operational Excellence
- **Delivery Retry Logic**: Configurable retry with exponential backoff
- **Webhook Timeout**: Configurable timeout for webhook delivery
- **Channel Fallback**: Graceful degradation when delivery channels fail

### Fixed
- **Policy Parser Integration**: Fixed integration issues between scheduler and policy engine
- **Delivery Error Handling**: Improved error propagation for failed deliveries
- **SDK Type Safety**: Enhanced type definitions in both JS and Python SDKs

## [0.8.0] - 2025-12-10

### Added

#### 💓 Heartbeat Pattern & DSL
- **Heartbeat Agent Pattern**: Continuous monitoring with assessment-action-sleep cycles
  - [`crates/runtime/src/scheduler/heartbeat.rs`](crates/runtime/src/scheduler/heartbeat.rs): Heartbeat execution engine
  - HeartbeatConfig for iteration limits and context management
  - HeartbeatContextMode enum for session isolation strategies
  - HeartbeatAssessment tracking for agent decisions
- **DSL Grammar**: Schedule definition blocks in Symbiont DSL
  - [`crates/dsl/src/grammar/schedule.pest`](crates/dsl/src/grammar/schedule.pest): Pest grammar for schedule blocks
  - Cron expression syntax with validation
  - At/one-shot timestamp syntax (ISO 8601)
  - Nested policy and heartbeat configuration blocks
- **DSL Schedule Extraction**: Parse and validate schedule definitions from DSL files
  - [`crates/dsl/src/schedule.rs`](crates/dsl/src/schedule.rs): Schedule AST and validation
  - Integration with existing DSL parser infrastructure
  - Semantic validation (cron syntax, timestamp format, policy rules)

#### ⌨️ CLI Subcommands
- **`symbi cron` Command Group**: Complete CLI for schedule management
  - [`src/commands/cron/mod.rs`](src/commands/cron/mod.rs): Command router and shared utilities
  - **`symbi cron list`**: List jobs with filtering (status, agent ID)
  - **`symbi cron add`**: Create job from DSL file or JSON
  - **`symbi cron remove`**: Delete job by ID or name
  - **`symbi cron pause`**: Pause job scheduling
  - **`symbi cron resume`**: Resume paused job
  - **`symbi cron status`**: Job details with next run time
  - **`symbi cron run`**: Trigger immediate execution
  - **`symbi cron history`**: View run history with filtering
- **Interactive CLI**: Rich terminal output with colors and formatting
  - Table views for job lists and history
  - Human-readable timestamps and durations
  - JSON output mode for scripting

### Improved

#### DSL Integration
- **Unified Configuration**: Schedule definitions colocated with agent definitions
- **Validation**: Comprehensive validation at parse time vs runtime
- **Error Messages**: Clear error reporting for invalid schedules

#### Developer Experience
- **CLI Discoverability**: Intuitive command structure with helpful error messages
- **Documentation**: Inline help for all CLI commands
- **Testing**: Integration tests for CLI workflows

### Fixed
- **DSL Parser**: Fixed parsing of nested schedule blocks
- **Cron Validation**: Improved cron expression validation with better error messages
- **CLI Error Handling**: Better error propagation from runtime to CLI

## [0.7.0] - 2025-11-20

### Added

#### ⏰ Cron Foundation
- **CronScheduler**: Background tick loop for scheduled execution
  - [`crates/runtime/src/scheduler/cron.rs`](crates/runtime/src/scheduler/cron.rs): Core scheduler implementation
  - 1-second tick interval with job selection by next run time
  - Concurrent execution with configurable limits
  - Graceful shutdown with in-flight job tracking
- **SQLite Persistent Job Store**: Durable job storage with ACID guarantees
  - [`crates/runtime/src/scheduler/store.rs`](crates/runtime/src/scheduler/store.rs): SqliteJobStore implementation
  - Transaction support for atomic state updates
  - Query capabilities (filter by status, agent ID, name)
  - Job run history with audit trail
- **CronJobDefinition**: Complete job lifecycle management
  - Cron expression parsing and validation
  - Job states: Active, Paused, Completed, Failed
  - One-shot job support with `at` timestamp field
  - Delivery configuration (channels, webhooks, Slack, email)
- **CronScheduled ExecutionMode**: New variant in AgentExecutionMode enum
  - Integration with existing scheduler infrastructure
  - Session management for scheduled agents
  - Context lifecycle hooks for pre/post execution
- **One-Shot Job Support**: Jobs that run once at a specific time
  - ISO 8601 timestamp parsing
  - Automatic job completion after successful execution
  - Failure handling with retry logic
- **Audit-Aware Run Records**: JobRunRecord with security event integration
  - Execution metadata (start time, duration, status)
  - Output capture and storage
  - Error message tracking
  - Integration with SecurityEventType for audit trail

#### 📊 Monitoring & Observability
- **Job Status Tracking**: Real-time job state monitoring
  - Next run time calculation
  - Last run status and duration
  - Failure count and retry tracking
- **Run History**: Persistent execution history per job
  - Queryable by status, time range
  - Success/failure statistics
  - Performance metrics (execution time)

### Improved

#### Scheduler Architecture
- **Separation of Concerns**: Clean separation between scheduler, store, and execution engine
- **Testability**: Mockable components for unit testing
- **Configuration**: Flexible CronSchedulerConfig for operational tuning

#### Runtime Integration
- **AgentContext Integration**: Seamless integration with existing context management
- **Policy Enforcement**: Placeholder for policy gates (implemented in v0.9.0)
- **Delivery Routing**: Framework for output delivery (implemented in v0.9.0)

### Fixed
- **Cron Expression Parsing**: Robust parsing with validation
- **Timezone Handling**: UTC-based scheduling with clear timezone semantics
- **Concurrency Safety**: Thread-safe job state management

### Dependencies
- **Added**: `cron` crate for expression parsing and scheduling
- **Added**: `rusqlite` for persistent job storage

## [0.6.1] - 2025-11-16

### Fixed
- **Compilation Issues**: Resolved crates.io publishing compilation errors
  - Fixed SecureMessage API usage with correct field names and types
  - Added missing SystemTime import for timestamp handling
  - Fixed ModelLogger API compatibility
  - Added missing dependencies (tokio, serde) to REPL crates
  - Fixed HttpInputConfig struct with required fields
  - Resolved match arm type compatibility issues in JSON-RPC server

### Dependencies
- **Version Specifications**: Added proper version specifications to all workspace dependencies for crates.io publishing

## [0.6.0] - 2025-11-15

### Added

#### 🧠 Complete REPL System (New)
- **Interactive Development Environment**: Full REPL (Read-Eval-Print Loop) system for Symbiont DSL
  - [`crates/repl-core`](crates/repl-core): Core REPL engine with DSL evaluation, session management, and policy enforcement
  - [`crates/repl-cli`](crates/repl-cli): Interactive CLI interface and JSON-RPC server for programmatic access
  - [`crates/repl-proto`](crates/repl-proto): JSON-RPC protocol definitions for client-server communication
  - [`crates/repl-lsp`](crates/repl-lsp): Language Server Protocol implementation for IDE integration
- **Agent Lifecycle Management**: Create, start, stop, pause, resume, and destroy agents through REPL
- **Real-time Monitoring**: Execution monitoring with statistics, traces, and performance metrics
- **Session Management**: Snapshot and restore REPL sessions with persistent state
- **Policy Integration**: Built-in policy checking and capability gating for security

#### 🏢 Enterprise Features (New)
- **Suspended Agent Tracking**: Enterprise scheduler with advanced agent state management
  - [`enterprise/src/scheduler.rs`](enterprise/src/scheduler.rs): Enhanced scheduler with suspension tracking
  - Configurable suspension criteria and automatic resume capabilities
  - Integration with base runtime scheduler maintaining full compatibility
- **Retention Policy Scheduler**: Automated data lifecycle management
  - [`enterprise/docs/RETENTION_POLICY_SCHEDULER.md`](enterprise/docs/RETENTION_POLICY_SCHEDULER.md): Comprehensive retention policy system
  - Automatic cleanup of expired context items and memories
  - Configurable retention policies with compliance support
  - Background task execution with monitoring and metrics

#### 🛡️ AI-Driven Tool Review System (New)
- **Automated Security Analysis**: Complete workflow for MCP tool review and signing
  - [`enterprise/src/tool_review/`](enterprise/src/tool_review/): Tool review orchestrator and components
  - AI-powered security analysis with RAG (Retrieval-Augmented Generation)
  - Human oversight integration with streamlined review interface
  - Digital signing and verification of approved tools
- **Security Assessment**: Risk-based analysis with configurable severity levels
  - Vulnerability detection and impact assessment
  - Automated recommendations with confidence scoring
  - Audit trail and compliance reporting

#### ☁️ E2B Sandbox Integration (New)
- **Cloud Sandbox Support**: E2B.dev integration for secure code execution
  - [`crates/runtime/src/sandbox/e2b.rs`](crates/runtime/src/sandbox/e2b.rs): E2B sandbox implementation
  - Multi-tier sandbox architecture (Docker, gVisor, Firecracker, E2B)
  - Automatic tier selection based on risk assessment
  - Remote execution capabilities with enhanced isolation

#### 📊 Enhanced Scheduler Features
- **Real Task Execution**: Production-grade task processing capabilities
  - Process spawning with secure execution environments
  - Resource monitoring (CPU, memory) with 5-second intervals
  - Health checks and automatic failure detection
  - Support for ephemeral, persistent, scheduled, and event-driven execution modes
- **Graceful Shutdown**: Enhanced termination handling
  - 30-second graceful termination period with force termination fallback
  - Resource cleanup and metrics persistence
  - Queue cleanup and state synchronization

#### 📋 Documentation & Architecture
- **Data Directory Design**: Comprehensive directory structure specification
  - [`data_directory_structure_design.md`](data_directory_structure_design.md): Enhanced data persistence architecture
  - Unified management of agent contexts, logs, prompts, and vector database storage
  - Migration utilities and backward compatibility support
- **Tool Review Documentation**: Complete workflow documentation
  - [`docs/tool_review_workflow.md`](docs/tool_review_workflow.md): AI-driven tool review process
  - Security analysis procedures and human oversight protocols
- **REPL Guide**: Comprehensive user and developer documentation
  - [`docs/repl-guide.md`](docs/repl-guide.md): Complete REPL usage guide
  - Interactive examples and integration patterns

#### 🔧 Release Management
- **Version Bump**: Updated to 0.6.0 across all workspace crates
- **Documentation Updates**: Updated version references in documentation and examples

### Improved

#### Developer Experience
- **Unified Workspace**: Enhanced project organization with REPL crates
  - Consistent versioning across all workspace members
  - Improved dependency management between crates
- **IDE Integration**: Language Server Protocol support for enhanced development
  - Syntax highlighting and completion for Symbiont DSL
  - Real-time error checking and validation
  - Integrated debugging capabilities

#### Enterprise Scheduler
- **Advanced State Management**: Enhanced agent lifecycle tracking
  - Suspension and resume capabilities with configurable criteria
  - Resource optimization during agent suspension periods
  - Seamless integration with existing scheduler infrastructure
- **Compliance & Monitoring**: Enterprise-grade operational capabilities
  - Comprehensive audit trails and compliance reporting
  - Advanced metrics collection and performance monitoring
  - Retention policy enforcement with automated cleanup

#### Security & Compliance
- **Enhanced Tool Security**: AI-driven security analysis and verification
  - Automated vulnerability detection with high confidence scoring
  - Human-in-the-loop verification for critical security decisions
  - Digital signing and integrity verification for tool distribution
- **Multi-tier Sandboxing**: Advanced isolation capabilities
  - Automatic risk assessment and tier selection
  - Enhanced security boundaries with cloud sandbox options
  - Improved resource management and monitoring

### Fixed
- **Scheduler Integration**: Resolved enterprise scheduler compatibility issues
- **REPL Session Management**: Fixed session persistence and restoration
- **Tool Review Workflow**: Enhanced error handling and timeout management
- **E2B Integration**: Improved authentication and endpoint configuration
- **Version References**: Updated all version references from 0.5.0 to 0.6.0 in documentation

### Breaking Changes
- **Workspace Structure**: New REPL crates require updated import statements
- **Enterprise Scheduler**: Enhanced scheduler interface with additional methods
- **Sandbox Architecture**: Updated sandbox tier enumeration with E2B support

### Dependencies
- **Added**: REPL system dependencies for interactive development
- **Updated**: Enterprise features with enhanced scheduling capabilities
- **Enhanced**: Tool review system with AI-powered analysis

### Performance Improvements
- **REPL Performance**: Optimized DSL evaluation and session management
- **Scheduler Throughput**: Enhanced task processing with real execution support
- **Tool Review Efficiency**: Streamlined security analysis workflow

## [0.5.0] - 2025-10-14

### Added

#### 🛠️ Enhanced CLI Experience
- **System Health Diagnostics**: New `symbi doctor` command for comprehensive system health checks
  - [`src/commands/doctor.rs`](src/commands/doctor.rs): Validates system dependencies, configuration, and runtime environment
  - Checks for required tools, permissions, and connectivity
  - Provides actionable recommendations for fixing issues
- **Log Management**: New `symbi logs` command for viewing and filtering application logs
  - [`src/commands/logs.rs`](src/commands/logs.rs): Real-time log streaming and filtering
  - Support for log levels, time ranges, and pattern matching
  - Integration with system logging infrastructure
- **Project Scaffolding**: New `symbi new` command for creating new agent projects
  - [`src/commands/new.rs`](src/commands/new.rs): Interactive project creation with templates
  - Pre-configured project structure with best practices
  - Multiple project templates (basic, advanced, custom)
  - Automatic dependency setup and configuration
- **Status Monitoring**: New `symbi status` command for real-time system status
  - [`src/commands/status.rs`](src/commands/status.rs): Display running agents, resource usage, and system health
  - Quick overview of active components and their states
- **Quick Start**: New `symbi up` command for rapid environment initialization
  - [`src/commands/up.rs`](src/commands/up.rs): One-command setup for development and production
  - Automatic dependency installation and service startup
  - Health checks and validation after startup

#### 📦 Installation & Distribution
- **Automated Installation Script**: New [`scripts/install.sh`](scripts/install.sh) for easy setup
  - Cross-platform installation support (Linux, macOS)
  - Automatic dependency detection and installation
  - Version management and upgrade capabilities
  - Configurable installation paths and options

#### 📋 Documentation
- **Version 1.0 Planning Documents**: Comprehensive planning for next major release
  - [`docs/v1-plan.md`](docs/v1-plan.md): Detailed roadmap and feature planning
  - [`docs/v1-plan-original.md`](docs/v1-plan-original.md): Original design documents and architecture decisions

### Improved

#### User Experience
- **CLI Interface**: Enhanced command-line interface with improved help text and error messages
  - Better command organization and discoverability
  - Consistent command structure across all operations
  - Improved error messages with actionable guidance
- **README Documentation**: Streamlined and updated README files across all languages
  - Simplified getting started guide
  - Clearer feature descriptions and use cases
  - Updated installation instructions
  - Better examples and quick start guides

#### Developer Experience
- **Project Structure**: Enhanced organization for better maintainability
  - Clearer separation of concerns in command modules
  - Improved code organization in [`src/commands/mod.rs`](src/commands/mod.rs:5)
- **Main CLI Entry Point**: Updated [`src/main.rs`](src/main.rs) with new command routing
  - Better command registration and handling
  - Enhanced error handling and logging
  - Improved startup performance

### Fixed
- **CLI Command Registration**: Properly integrated new commands into main CLI interface
- **Error Handling**: Improved error messages and recovery in CLI commands
- **Documentation Links**: Fixed broken references in README files across all language versions

### Performance Improvements
- **Startup Time**: Optimized CLI initialization and command loading
- **Log Processing**: Enhanced log streaming performance for real-time monitoring
- **Status Checks**: Faster system status queries and health checks

## [0.4.0] - 2025-08-28

### Added

#### 🧠 SLM-First Architecture (New)
- **Policy-Driven Routing Engine**: Intelligent routing between Small Language Models (SLMs) and Large Language Models (LLMs)
  - [`crates/runtime/src/routing/engine.rs`](crates/runtime/src/routing/engine.rs): Core routing engine with SLM-first preference and LLM fallback
  - [`crates/runtime/src/routing/policy.rs`](crates/runtime/src/routing/policy.rs): Configurable policy evaluation with rule-based decision logic
  - [`crates/runtime/src/routing/config.rs`](crates/runtime/src/routing/config.rs): Comprehensive routing configuration management
  - [`crates/runtime/src/routing/decision.rs`](crates/runtime/src/routing/decision.rs): Route decision types and execution paths
- **Task Classification System**: Automatic categorization of requests for optimal model selection
  - Task-aware routing with capability matching
  - Pattern recognition and keyword analysis for task classification
- **Confidence-Based Quality Control**: Adaptive learning system for model performance tracking
  - [`crates/runtime/src/routing/confidence.rs`](crates/runtime/src/routing/confidence.rs): Confidence monitoring and threshold management
  - Real-time quality assessment with configurable confidence thresholds
  - Automatic fallback on low-confidence responses

#### ⚡ Performance & Reliability
- **Thread-Safe Operations**: Full async/await support with proper concurrency handling
- **Error Recovery**: Graceful fallback mechanisms with exponential backoff retry logic
- **Runtime Configuration**: Dynamic policy updates and threshold adjustments without restart
- **Comprehensive Logging**: Detailed audit trail of routing decisions and performance metrics

### Improved

#### Routing & Model Management
- **Model Catalog Integration**: Deep integration with existing model catalog for SLM selection
- **Resource Management**: Intelligent resource allocation and constraint handling
- **Load Balancing**: Multiple strategies for distributing requests across available models
- **Scheduler Integration**: Seamless integration with the existing agent scheduler

#### Developer Experience
- **Comprehensive Testing**: Complete test coverage for all routing components with mock implementations
- **Documentation**: Extensive design documents and implementation guides
  - [`docs/slm_config_design.md`](docs/slm_config_design.md): SLM configuration architecture
  - [`docs/router_design.md`](docs/router_design.md): Router design and implementation guide
  - [`docs/unit_testing_guide.md`](docs/unit_testing_guide.md): Testing methodology and coverage
- **Configuration Validation**: Enhanced validation of routing policies and model configurations

### Fixed
- **Module Exports**: Fixed routing module structure in [`crates/runtime/src/routing/mod.rs`](crates/runtime/src/routing/mod.rs:5)
  - Added missing `pub mod config;` and `pub mod policy;` declarations
  - Added corresponding `pub use` statements for proper re-exports
- **Task Type Updates**: Replaced deprecated `TaskType::TextGeneration` with `TaskType::CodeGeneration`
  - Updated routing engine references throughout codebase
  - Fixed task type usage in test modules and policy evaluation
- **Import Resolution**: Resolved compilation errors in routing components
  - Updated ModelLogger constructor calls to match current API
  - Fixed import paths in test modules for proper dependency resolution
- **Code Quality**: Applied clippy suggestions and resolved all warnings
  - Improved code patterns and removed unused imports
  - Enhanced error handling and async operation safety

### Performance Improvements
- **Routing Throughput**: Optimized routing decision performance with efficient policy evaluation
- **Memory Efficiency**: Reduced memory overhead in confidence monitoring and statistics tracking
- **Async Operations**: Enhanced async runtime efficiency for concurrent request handling
- **Configuration Loading**: Optimized configuration parsing and validation performance

### Breaking Changes
- **Routing API**: New routing engine interface with SLM-first architecture
- **Task Classification**: Updated task type enumeration with `CodeGeneration` replacing `TextGeneration`
- **Configuration Schema**: Enhanced routing configuration structure with policy-driven settings

## [0.3.1] - 2025-08-10

### Added

#### 🔒 Security Enhancements
- **Centralized Configuration Management**: New [`config.rs`](crates/runtime/src/config.rs) module for secure configuration handling
  - Environment variable abstraction layer with validation
  - Multiple secret key providers (environment, file, external services)
  - Centralized configuration access patterns
- **Enhanced CI/CD Security**: Automated security scanning in GitHub Actions
  - Daily cargo audit vulnerability scanning
  - Clippy security lints integration
  - Secret leak detection in build pipeline

#### 📋 API Documentation
- **SwaggerUI Integration**: Interactive API documentation for HTTP endpoints
  - Auto-generated OpenAPI specifications
  - Interactive API testing interface
  - Complete endpoint documentation with examples

### Security Fixes

#### 🛡️ Critical Vulnerability Resolutions
- **RUSTSEC-2022-0093**: Fixed ed25519-dalek Double Public Key Signing Oracle Attack
  - Updated from v1.0.1 → v2.2.0
- **RUSTSEC-2024-0344**: Resolved curve25519-dalek timing variability vulnerability
  - Updated from v3.2.0 → v4.1.3 (transitive dependency)
- **RUSTSEC-2025-0009**: Fixed ring AES panic vulnerability
  - Updated from v0.16 → v0.17.12
- **Timing Attack Prevention**: Implemented constant-time token comparison
  - Replaced vulnerable string comparison in authentication middleware
  - Added `subtle` crate for constant-time operations
  - Enhanced authentication logging and error handling

### Improved

#### Configuration Management
- **Environment Variable Security**: Eliminated direct `env::var` usage throughout codebase
- **Secret Handling**: Secure configuration management with validation
- **Error Handling**: Enhanced configuration error reporting and validation

#### Authentication & Security
- **Middleware Security**: Updated authentication middleware to use configuration management
- **Request Logging**: Enhanced security logging for authentication failures
- **Token Validation**: Improved bearer token validation with timing attack prevention

### Dependencies

#### Security Updates
- **Updated**: `ed25519-dalek` from v1.0.1 to v2.2.0 (critical security fix)
- **Updated**: `reqwest` from v0.11 to v0.12 (security and performance)
- **Updated**: `ring` from v0.16 to v0.17.12 (AES panic fix)
- **Added**: `subtle` v2.5 for constant-time cryptographic operations

#### Documentation & Tooling
- **Added**: `utoipa` and `utoipa-swagger-ui` for API documentation generation
- **Enhanced**: CI/CD security workflow with automated vulnerability scanning

### Verification
- ✅ **cargo audit**: All critical vulnerabilities resolved
- ✅ **cargo clippy**: No security or performance warnings
- ✅ **Timing attack tests**: Constant-time comparison verified
- ✅ **Configuration migration**: Seamless upgrade path from v0.3.0

## [0.3.0] - 2025-08-09

### Added

#### 🚀 HTTP API Server (New)
- **Complete API Server**: Full-featured HTTP server implementation using Axum framework
  - RESTful endpoints for agent management, execution, and monitoring
  - Authentication middleware with bearer token and JWT support
  - CORS support and comprehensive security headers
  - Request tracing and structured logging
  - Graceful shutdown with active request completion
- **Agent Management API**: Create, update, delete, and monitor agents via HTTP
  - Agent status tracking with real-time metrics
  - Agent execution history and performance data
  - Agent configuration updates without restart
- **System Monitoring**: Health checks, metrics collection, and system status endpoints
  - Real-time system resource utilization
  - Agent scheduler statistics and performance metrics
  - Comprehensive health check with component status

#### 🧠 Advanced Context & Knowledge Management (New)
- **Hierarchical Memory System**: Multi-layered memory architecture for agents
  - **Working Memory**: Variables, active goals, attention focus for immediate processing
  - **Short-term Memory**: Recent experiences and temporary information
  - **Long-term Memory**: Persistent knowledge and learned experiences
  - **Episodic Memory**: Structured experience episodes with events and outcomes
  - **Semantic Memory**: Concept relationships and domain knowledge graphs
- **Knowledge Base Operations**: Comprehensive knowledge management capabilities
  - **Facts**: Subject-predicate-object knowledge with confidence scoring
  - **Procedures**: Step-by-step procedural knowledge with error handling
  - **Patterns**: Learned behavioral patterns with occurrence tracking
  - **Knowledge Sharing**: Inter-agent knowledge sharing with trust scoring
- **Context Persistence**: File-based and configurable storage backend
  - Automatic context archiving and retention policies
  - Compression and encryption support for sensitive data
  - Migration utilities for legacy storage formats
- **Vector Database Integration**: Semantic search and similarity matching
  - Qdrant integration for high-performance vector operations
  - Embedding generation and storage for context items
  - Batch operations for efficient data processing
- **Context Examples**: Comprehensive [`context_example.rs`](crates/runtime/examples/context_example.rs) demonstration

#### ⚡ Production-Grade Agent Scheduler (New)
- **Priority-Based Scheduling**: Multi-level priority queue with resource-aware scheduling
  - Configurable priority levels and scheduling algorithms
  - Resource requirements tracking and allocation
  - Load balancing with multiple strategies (round-robin, resource-based)
- **Task Management**: Complete lifecycle management for agent tasks
  - Task health monitoring and failure detection
  - Automatic retry logic with exponential backoff
  - Timeout handling and graceful termination
- **System Monitoring**: Real-time scheduler metrics and health monitoring
  - Agent performance tracking (CPU, memory, execution time)
  - System capacity monitoring and utilization alerts
  - Comprehensive scheduler statistics and dashboards
- **Graceful Shutdown**: Production-ready shutdown with active task completion
  - Resource cleanup and allocation tracking
  - Metrics persistence and system state preservation
  - Configurable shutdown timeouts and force termination

#### 📊 Enhanced Documentation & Examples
- **Production Examples**: Real-world usage patterns and best practices
  - RAG engine integration with [`rag_example.rs`](crates/runtime/examples/rag_example.rs)
  - Context persistence and management workflows
  - Agent lifecycle and resource management
- **API Reference**: Complete HTTP API documentation with examples
  - OpenAPI-compatible endpoint specifications
  - Authentication and authorization guides
  - Integration examples for common use cases

### Improved

#### Runtime Stability & Performance
- **Memory Management**: Optimized memory usage with configurable limits
- **Error Handling**: Enhanced error propagation and recovery mechanisms
- **Async Performance**: Improved async runtime efficiency and task scheduling
- **Resource Utilization**: Better CPU and memory resource management

#### Configuration & Deployment
- **Feature Flags**: Granular feature control for different deployment scenarios
  - `http-api`: HTTP server and API endpoints
  - `http-input`: Webhook input processing
  - `vector-db`: Vector database integration
  - `embedding-models`: Local embedding model support
- **Directory Structure**: Standardized data directory layout
  - Separate directories for state, logs, prompts, and vector data
  - Automatic directory creation and permission management
  - Legacy migration utilities for existing deployments

#### Developer Experience
- **Examples**: Comprehensive example implementations for all major features
- **Testing**: Enhanced test coverage with integration tests
- **Logging**: Structured logging with configurable verbosity levels
- **Debugging**: Improved debugging capabilities with detailed metrics

### Fixed
- **Scheduler Deadlocks**: Resolved potential deadlock conditions in agent scheduling
- **Memory Leaks**: Fixed memory leaks in context management and vector operations
- **Graceful Shutdown**: Improved shutdown reliability under high load
- **Configuration Validation**: Enhanced validation of configuration parameters
- **Error Recovery**: Better error recovery in network and storage operations

### Dependencies
- **Added**: Axum 0.7 for HTTP server implementation
- **Added**: Tower and Tower-HTTP for middleware and CORS support
- **Added**: Governor for rate limiting capabilities
- **Added**: Qdrant-client 1.14.0 for vector database operations
- **Updated**: Tokio async runtime optimizations
- **Updated**: Enhanced serialization with serde improvements

### Breaking Changes
- **Context API**: Updated context management API with hierarchical memory model
- **Scheduler Interface**: New scheduler trait with enhanced lifecycle management
- **Configuration Format**: Updated configuration structure for directory management

### Performance Improvements
- **Scheduler Throughput**: Up to 10x improvement in agent scheduling performance
- **Memory Efficiency**: 40% reduction in memory usage for large context operations
- **Vector Search**: Optimized vector database operations with batch processing
- **HTTP Response Time**: Sub-100ms response times for standard API operations

### Security Enhancements
- **Authentication**: Multi-factor authentication support for HTTP API
- **Encryption**: Enhanced encryption for data at rest and in transit
- **Access Control**: Improved permission management for context operations
- **Data Protection**: Secure handling of sensitive agent data and configurations

## Installation

### Docker
```bash
docker pull ghcr.io/thirdkeyai/symbi:v0.3.0
```

### Cargo (with all features)
```bash
cargo install symbi-runtime --features full
```

### Cargo (minimal installation)
```bash
cargo install symbi-runtime --features minimal
```

### From Source
```bash
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont
git checkout v0.3.0
cargo build --release --features full
```

## Quick Start - HTTP API

```rust
use symbi_runtime::api::{HttpApiServer, HttpApiConfig};

let config = HttpApiConfig {
    bind_address: "0.0.0.0".to_string(),
    port: 8080,
    enable_cors: true,
    enable_tracing: true,
};

let server = HttpApiServer::new(config);
server.start().await?;
```

## Quick Start - Context Management

```rust
use symbi_runtime::context::{StandardContextManager, ContextManagerConfig};

let config = ContextManagerConfig {
    max_contexts_in_memory: 1000,
    enable_auto_archiving: true,
    enable_vector_db: true,
    ..Default::default()
};

let context_manager = StandardContextManager::new(config, "system").await?;
let session_id = context_manager.create_session(agent_id).await?;
```

---

**Full Changes**: [v0.1.2...v0.3.0](https://github.com/thirdkeyai/symbiont/compare/v0.1.2...v0.3.0)

## [0.1.1] - 2025-07-26

### Added

#### Secrets Management System
- HashiCorp Vault backend with multiple authentication methods:
  - Token-based authentication
  - Kubernetes service account authentication
  - AWS IAM role authentication (framework ready)
  - AppRole authentication
- Encrypted file backend with AES-256-GCM encryption
- OS keychain integration for master key storage
- Audit trail for all secrets operations
- Agent-scoped secret namespaces
- CLI subcommands for encrypt/decrypt/edit operations

#### Security & Compliance
- Code of Conduct and Security Policy documentation
- Cosign container image signing
- Container security scanning with Trivy

#### Infrastructure
- Tag-based Docker builds with semantic versioning
- Multi-architecture container support (linux/amd64, linux/arm64)
- GitHub Container Registry integration

### Improved

#### Runtime Components
- MCP client error handling and stability
- RAG engine async context manager API
- HTTP API reliability (optional feature)
- Tool execution and sandboxing integration
- Vector database integration with Qdrant

#### Documentation
- Security model documentation
- API reference with examples
- Clear OSS vs Enterprise feature distinction
- Development and contribution guidelines

#### Development Experience
- Environment configuration with `.env` support
- Test coverage (17/17 secrets management tests passing)
- Error messages and debugging capabilities

### Fixed
- Import naming conflicts in test modules
- RAG engine async context manager issues
- Docker registry naming for lowercase compliance
- Documentation link references
- Cargo clippy warnings and compilation errors

### Dependencies
- Added vaultrs for Vault integration
- Updated tokio for async runtime
- Added serde for configuration serialization
- Added thiserror for error handling

### Known Issues
- Windows keychain integration pending

## Installation

### Docker
```bash
docker pull ghcr.io/thirdkeyai/symbi:v0.1.1
```

### From Source
```bash
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont
git checkout v0.1.1
cargo build --release
```

For the complete list of changes, see the [commit history](https://github.com/thirdkeyai/symbiont/compare/v0.1.0...v0.1.1).