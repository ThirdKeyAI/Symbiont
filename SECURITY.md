# Security Policy

## Supported Versions

The following versions of Symbiont are currently supported with security updates:

| Version | Supported          | Notes |
| ------- | ------------------ | ----- |
| 1.14.x  | :white_check_mark: | Current. v1.14.0 is the security-audit response release — fixes 5 CRITICAL / 7 HIGH / 10 MEDIUM / 9 LOW findings (see `SECURITY_AUDIT.md`). v1.14.1 is the release-workflow + integration-test hotfix that ships v1.14.0's source-level posture as crates.io / binary release artifacts. v1.14.2 closes the fail-closed-without-a-policy-backend trap: published binaries now ship with Cedar in the default feature set and `symbi up` / `symbi run` auto-wire `CedarPolicyGate` from `policies/*.cedar` at startup; the fail-closed `DefaultPolicyGate::new()` fallback remains in place when no policy files are present. |
| 1.13.x  | :white_check_mark: | Previous. Affected by every finding fixed in v1.14.0. Operators on 1.13 should upgrade. |
| < 1.13  | :x:                | Unsupported. Several exploitable issues are documented in `SECURITY_AUDIT.md`. |

> **Upgrade guidance.** v1.14.0 contains breaking changes — Composio MCP / SymbiBot integration removed, reasoning-loop policy gate now fail-closed by default, JWT verifier enforces an ES256 / EdDSA / HS256 algorithm allowlist, `docker-compose.test.yml` requires `SYMBIONT_API_TOKEN` with no default. See `CHANGELOG.md` (the v1.14.0 section) for the full upgrade and migration notes, and `SECURITY-OPS.md` for out-of-band operator actions (e.g. Homebrew PAT rotation).

*Last updated: 2026-05-18*

## Reporting a Vulnerability

We take security vulnerabilities seriously. If you discover a security vulnerability in Symbiont, please report it to us privately.

### How to Report

**DO NOT** create a public GitHub issue for security vulnerabilities.

Instead, please:

1. **Email**: Send details to security@thirdkey.ai
2. **Subject**: Include "SECURITY" in the subject line
3. **Content**: Include the following information:
   - Description of the vulnerability
   - Steps to reproduce the issue
   - Potential impact
   - Any suggested fixes (if you have them)

### What to Expect

- **Acknowledgment**: We will acknowledge receipt of your report within 48 hours
- **Assessment**: We will assess the vulnerability and provide an initial response within 5 business days
- **Updates**: We will keep you informed of our progress throughout the process
- **Resolution**: We aim to resolve critical vulnerabilities within 30 days

### Disclosure Policy

- We follow responsible disclosure practices
- We will work with you to understand and resolve the issue before any public disclosure
- We will credit you for the discovery (unless you prefer to remain anonymous)
- We will coordinate with you on the timing of public disclosure

### Security Best Practices

When using Symbiont in production:

1. **Keep Updated**: Always use the latest supported version. v1.14.0 fixes several CRITICAL findings — see `SECURITY_AUDIT.md`.
2. **Secrets Management**: Use the built-in encrypted secrets store with a strong KDF password; prefer OS keychain or Vault key providers over environment variables. Use `*_API_KEY_REF` env vars (e.g. `OPENROUTER_API_KEY_REF`) to point at secret-store entries instead of inlining keys in `*_API_KEY`.
3. **Sandboxing**: Use Docker, gVisor, Firecracker, or E2B for untrusted code; never use the native sandbox in production. The `native-sandbox` Cargo feature fails to compile in release builds (v1.14.0+); runtime construction additionally requires `SYMBI_UNSAFE_NATIVE_SANDBOX=1` so accidental use is impossible in a hardened binary.
4. **Policy Gate**: `symbi up` / `symbi run` default to a fail-closed reasoning-loop policy gate (v1.14.0+) — every `ToolCall` and `Delegate` action is denied until an explicit `ReasoningPolicyGate` implementation (e.g. `CedarPolicyGate`, `OpaPolicyGateBridge`) is wired in. The dev-only `--insecure-allow-all` / `SYMBI_INSECURE_ALLOW_ALL=1` opt-in prints a loud stderr banner; never use it in production.
5. **Tool Verification**: Run in strict enforcement mode to ensure only verified MCP tools are executed.
6. **API Keys**: Enable per-agent API key authentication for all HTTP endpoints; rotate keys regularly. Use the `keyid.secret` format — v1.14.0 added `SYMBI_REJECT_LEGACY_API_KEYS=1` to immediately reject the deprecated O(n) Argon2 scan path used for unprefixed legacy keys.
7. **JWT / Webhook Verification**: v1.14.0 enforces an algorithm allowlist (ES256 / EdDSA for asymmetric `Authorization: Bearer`, HS256 for HMAC webhooks). RSA-signed JWTs are refused on every operator-controlled path, which mitigates RUSTSEC-2023-0071 (`rsa` Marvin Attack via `jsonwebtoken`). Audience claims are required unconditionally.
8. **Rate Limiting**: Keep rate limiting enabled to prevent abuse and resource exhaustion.
9. **Audit Logging**: Enable audit logging in strict mode and monitor for suspicious activity.
10. **Network Security**: Run Symbiont in a properly secured network environment with TLS. Bind ports to `127.0.0.1` behind a reverse proxy; the bundled `docker-compose.test.yml` does this by default and refuses to start without `SYMBIONT_API_TOKEN` (no `testtoken123` default; the runtime additionally rejects any token starting with `test` shorter than 20 chars).
11. **Policy Rules**: Define explicit allow/deny policies for agent capabilities and tool access. Cedar policies are the recommended path.
12. **External Tools**: Symbiont does not ship a built-in MCP tool client (the Composio integration was removed in v1.14.0 — see `SECURITY_AUDIT.md` C3). Bring your own `ActionExecutor` for external tool dispatch, and never dispatch LLM-supplied tool names against an unbounded backend without a static allowlist + TLS pinning.

### Security Features

Symbiont includes several security features:

- **Sandboxed Execution**: Tiered isolation (Docker, gVisor, Firecracker, E2B) with resource limits via rlimit and process-group kill on timeout. Firecracker working directory is per-uid `/run/symbi/agent_<id>` (0700) with a per-uid `<temp>/symbi-<uid>/...` fallback (v1.14.0+).
- **Native Sandbox Hardening**: Process-group isolation (`setpgid`/`killpg`), CPU/memory/file-size limits, empty-by-default allowed executables, shell warnings. The `native-sandbox` Cargo feature fails to compile in release builds, and runtime construction requires `SYMBI_UNSAFE_NATIVE_SANDBOX=1` regardless of `SYMBI_ENV` (v1.14.0+).
- **Fail-Closed Reasoning Policy Gate**: `DefaultPolicyGate::new()` denies every `ToolCall` and `Delegate` action with an explicit reason; production deployments must wire `CedarPolicyGate`, `OpaPolicyGateBridge`, or a custom `ReasoningPolicyGate`. The dev-only permissive constructor is `permissive_for_dev_only()` (`#[doc(hidden)]`) and emits `tracing::warn!` on every action it evaluates (v1.14.0+).
- **JWT Algorithm Allowlist**: ES256 / EdDSA for asymmetric `Authorization: Bearer` paths, HS256 for the HMAC webhook-signature path. RS / PS / `none` algorithms are refused at both the header-inspection guard and the `Validation::algorithms` allowlist. Audience claims required unconditionally — the `SYMBIONT_ALLOW_NO_JWT_AUDIENCE` env-var escape hatch was removed in v1.14.0. Mitigates RUSTSEC-2023-0071 (`rsa` Marvin Attack via `jsonwebtoken`).
- **Tool-Call Argument Validation**: LLM-produced tool arguments are validated against the tool's declared JSON Schema before the policy gate runs; schema-violating or non-object arguments are rejected as `LoopDecision::Deny` (v1.14.0+).
- **Secrets Management**: AES-256-GCM encrypted file store with Argon2id KDF (OWASP 2024 at-rest parameters: 64 MiB / 3 iters), file locking (fd-lock), mtime-based decryption cache, env/keychain/file/Vault key providers. `Secret` type emits `"value": "[REDACTED]"` on `Serialize` so structured loggers can't leak plaintext (v1.14.0+).
- **Per-Agent API Key Authentication**: Argon2id-hashed API keys (OWASP interactive: 19 MiB / 2 iters) with file-backed key store. `keyid.secret` format gives O(1) lookup by key ID; the legacy O(n) scan path emits a loud `error!` and can be hard-disabled with `SYMBI_REJECT_LEGACY_API_KEYS=1` (v1.14.0+).
- **Per-IP Rate Limiting**: Governor-based rate limiting middleware (configurable, default 100 req/min).
- **Tool Verification (SchemaPin)**: Cryptographic schema verification for MCP tool invocations with configurable enforcement policies (strict/permissive/development/disabled).
- **Agent Identity (AgentPin)**: Domain-anchored ES256 cryptographic identity verification for AI agents.
- **Webhook Signature Verification**: HMAC-SHA256 (`subtle::ConstantTimeEq`) and JWT verification with provider presets (GitHub, Stripe, Slack, Mattermost) and constant-time comparison. Replay protection: 5-minute timestamp freshness window with `i128`-widened delta to defeat `i64` overflow attacks at the boundary.
- **Trusted Proxy Allowlist**: `X-Forwarded-For` is only honored from CIDRs configured via `SYMBI_TRUSTED_PROXIES`.
- **Invisible-Character Sanitization (`symbi-invis-strip`)**: Strips C0/C1 controls, zero-width chars, bidi overrides, variation selectors, the Unicode Tag block, soft hyphen, combining marks, and superscript/subscript forms. `detect_injection_patterns` NFKC-normalises input (closes fullwidth + math-alphanumeric homoglyph bypasses), adds compact-projection matching (catches post-strip word concatenation), and flags Latin+Cyrillic mixing with a synthetic `mixed-script` marker (v1.14.0 / invis-strip 0.3.0).
- **Human Approval Relay (`symbi-approval-relay`)**: Dual-channel human approval relay with Slack signature verification using `subtle::ConstantTimeEq` and `i128`-widened timestamp delta.
- **AgentSkills Security**: Verified skill loading with SchemaPin signatures, content scanning with ClawHavoc defense rules.
- **Sensitive Argument Redaction**: Schema-driven masking of sensitive tool parameters in logs.
- **Audit Logging**: Comprehensive logging of security-relevant events with strict/permissive failure modes.
- **Policy Engine**: Fine-grained access control and security policies with Cedar and DSL-defined rules.
- **Model I/O Logging**: Encrypted interaction logs with configurable retention.

### Security Considerations

- Symbiont executes arbitrary code as defined in agent configurations
- The native sandbox provides resource limits but **not** full isolation — use Docker/gVisor/Firecracker/E2B for untrusted code
- Native sandbox is blocked in production (`SYMBIONT_ENV=production`); v1.14.0 additionally requires `SYMBI_UNSAFE_NATIVE_SANDBOX=1` at runtime and makes the `native-sandbox` Cargo feature fail to compile in release builds
- Ensure proper network isolation and access controls; bind to `127.0.0.1` behind a reverse proxy, not `0.0.0.0`
- Regularly review and audit agent configurations and policy rules
- Monitor system resources and API usage
- Use encryption for data at rest and in transit
- Rotate API keys and secrets periodically — see `SECURITY-OPS.md` for the operator-side action items (e.g. Homebrew PAT rotation)
- The Microsoft Teams adapter (`crates/channel-adapter/src/adapters/teams/auth.rs`) still uses RS256 because the Bot Framework protocol requires it; that surface is bounded to MS-signed tokens issued by Azure

## Vulnerability Management

We maintain an internal vulnerability management process:

1. **Triage**: Initial assessment and severity classification
2. **Investigation**: Technical analysis and impact assessment  
3. **Remediation**: Development and testing of fixes
4. **Release**: Security patches and coordinated disclosure
5. **Post-mortem**: Review process improvements

### Severity Classification

- **Critical**: Remote code execution, privilege escalation
- **High**: Information disclosure, authentication bypass
- **Medium**: Denial of service, local privilege escalation
- **Low**: Information leakage, minor security issues

## Third-Party Dependencies

We monitor our dependencies for known vulnerabilities:

- **cargo-deny**: License and vulnerability auditing via `deny.toml`
- Regular dependency updates with Cargo lockfile pinning
- Automated vulnerability scanning in CI
- Review of dependency security advisories
- Prompt patching of vulnerable dependencies

## Contact

For security-related questions or concerns:

- Security Email: security@thirdkey.ai
- General Contact: oss@symbiont.dev
- Website: https://symbiont.dev

---

*This security policy is subject to change. Check this document regularly for updates.*