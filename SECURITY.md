# Security Policy

## Supported Versions

Symbiont has two supported release lines.

| Version | Status | Security fixes |
|---------|--------|----------------|
| 3.0.x | Current release line and default install | Yes |
| 2.0.x | Supported for existing deployments | Yes, as 2.0.x patch releases |
| < 2.0 | Not supported | No |

Start new deployments on 3.0. Existing 2.0 deployments can stay on 2.0 while
they plan the upgrade. Security fixes are made on main for 3.0 and backported
to 2.0 when the affected code exists there. When 2.0 support is due to end,
the date will be announced on this page in advance.

This document describes the **3.0** runtime contract. The outbound
capabilities and registered Landlock selection described here are new in 3.0
and are not present in 2.0.x. Before upgrading from 2.0, read the
[3.0 release notes](CHANGELOG.md); from 1.x, start with the
[2.0 migration guide](docs/migration-2.0.md).
Verify the installed binary and its release provenance using the
[release verification guide](docs/release-verification.md).

*Last updated: 2026-10-30*

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

### Deployment responsibilities

Keep operator-owned configuration, policy, credentials, approval state and audit
keys outside worker-readable source trees and file grants. Select a sandbox
appropriate to the threat model and verify its prerequisites. Runtime and host
administrators remain trusted. A compromised trusted runtime or host kernel is
not contained by an application policy engine.

Landlock is a Linux-native option for desktop and laptop operation without a
container or VM. It is separate from the unsafe development-only native backend,
and shares the host kernel. Docker, gVisor and Firecracker provide different
boundaries and requirements; do not infer one backend's guarantees from another's
passing tests. Hosted execution has separate infrastructure and data-handling
implications. See [Landlock setup](docs/landlock-development.md),
[container networking](docs/toolclad.md) and [VM setup](docs/firecracker-setup.md).

Require authenticated entry points, configured token issuer/audience, trusted
proxy configuration and explicit runtime/source ownership where applicable.
See the [2.0 migration guide](docs/migration-2.0.md) for current authentication
and ownership requirements. Development bypasses are not production controls.

### Runtime controls and their scope

- **Governed actions:** supported tool paths prepare immutable requests, apply
  policy and any exact-call human approval, then dispatch with required evidence.
  A tool registry or signed schema does not by itself authorize an action.
- **Worker containment:** governed workers receive bounded source/file grants,
  private scratch space and owned cleanup. Managed CLI workers keep tool,
  inference and approval authority in the runtime. Missing prerequisites fail
  the selected operation; no host fallback is implied.
- **Network and credentials:** Docker defaults to disabled networking and rejects
  bridge networking in production. Managed inference holds credentials outside
  the worker. The [optional outbound capability contract](docs/outbound-capabilities.md)
  adds named HTTP operations, explicit credential sources and execution-scoped
  authority. It does not cover every transport in the runtime.
- **External tools:** the built-in MCP client supports governed, schema-verified
  invocation. See [MCP tools](docs/mcp-tools.md) for enforcement, file grants and
  compatibility limits. Provider-specific remote services require their own
  destination and identity controls.
- **Audit and recovery:** protected journals bind run identity, prepared calls and
  effect records. Verify them using a separately trusted public key. An intact
  signed prefix is not a completed run; unknown effects require reconciliation
  and cannot justify automatic replay. See [run audit](docs/run-audit.md) and
  [invocation reconciliation](docs/invocation-reconciliation.md).
- **Data handling:** constrain returned fields and permitted operations, protect
  retained journals and configure retention appropriate to the deployment.
  Credential isolation, response filtering and hostname restrictions are not
  general guarantees against data exfiltration through an approved service.

The [machine-readable control catalogue](docs/security/controls.json) records
threats, invariants, implementation references, route/backend scope and limitations.
Implementation and validation status are separate. A test driver is not a passing
qualification, and a qualification applies only to its recorded source, binary,
configuration and platform. [Catalogue validation](docs/security/README.md) checks
these evidence claims without claiming formal verification or exhaustive testing.

The catalogue is an initial scoped inventory, not a complete audit of all code.
Ordinary inference providers, channel delivery, remote communication, browser
execution and future resume mechanisms must not inherit guarantees from unrelated
routes. Review [route coverage](docs/containment-branch-guide.md) when selecting an
entry point. Core containment and local verification remain available in OSS.

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