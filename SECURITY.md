# Security Policy

## Supported Versions

The following versions of Symbiont are currently supported with security updates:

| Version | Supported          |
| ------- | ------------------ |
| 1.4.x   | :white_check_mark: |
| 1.1.x   | :white_check_mark: |
| 1.0.x   | :x:                |
| < 1.0   | :x:                |

*Last updated: 2026-02-15*

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

1. **Keep Updated**: Always use the latest supported version
2. **Secrets Management**: Use the built-in encrypted secrets store with a strong KDF password; prefer OS keychain or Vault key providers over environment variables
3. **Sandboxing**: Use Docker, gVisor, or Firecracker for untrusted code; never use the native sandbox in production
4. **Tool Verification**: Run in strict enforcement mode to ensure only verified MCP tools are executed
5. **API Keys**: Enable per-agent API key authentication for all HTTP endpoints; rotate keys regularly
6. **Rate Limiting**: Keep rate limiting enabled to prevent abuse and resource exhaustion
7. **Audit Logging**: Enable audit logging in strict mode and monitor for suspicious activity
8. **Network Security**: Run Symbiont in a properly secured network environment with TLS
9. **Policy Rules**: Define explicit allow/deny policies for agent capabilities and tool access

### Security Features

Symbiont includes several security features:

- **Sandboxed Execution**: Tiered isolation (Docker, gVisor, Firecracker, E2B) with resource limits via rlimit and process-group kill on timeout
- **Native Sandbox Hardening**: Process-group isolation (`setpgid`/`killpg`), CPU/memory/file-size limits, empty-by-default allowed executables, shell warnings
- **Secrets Management**: AES-256-GCM encrypted file store with Argon2 KDF, file locking (fd-lock), mtime-based decryption cache, env/keychain/file/Vault key providers
- **Per-Agent API Key Authentication**: Argon2-hashed API keys with file-backed key store
- **Per-IP Rate Limiting**: Governor-based rate limiting middleware (configurable, default 100 req/min)
- **Tool Verification (SchemaPin)**: Cryptographic schema verification for MCP tool invocations with configurable enforcement policies (strict/permissive/development/disabled)
- **Agent Identity (AgentPin)**: Domain-anchored ES256 cryptographic identity verification for AI agents
- **Webhook Signature Verification**: HMAC-SHA256 and JWT verification with provider presets (GitHub, Stripe, Slack) and constant-time comparison
- **AgentSkills Security**: Verified skill loading with SchemaPin signatures, content scanning with ClawHavoc defense rules
- **Sensitive Argument Redaction**: Schema-driven masking of sensitive tool parameters in logs
- **Audit Logging**: Comprehensive logging of security-relevant events with strict/permissive failure modes
- **Policy Engine**: Fine-grained access control and security policies with DSL-defined rules
- **Model I/O Logging**: Encrypted interaction logs with configurable retention

### Security Considerations

- Symbiont executes arbitrary code as defined in agent configurations
- The native sandbox provides resource limits but **not** full isolation — use Docker/gVisor/Firecracker/E2B for untrusted code
- Native sandbox is blocked in production (`SYMBIONT_ENV=production`)
- Ensure proper network isolation and access controls
- Regularly review and audit agent configurations and policy rules
- Monitor system resources and API usage
- Use encryption for data at rest and in transit
- Rotate API keys and secrets periodically

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