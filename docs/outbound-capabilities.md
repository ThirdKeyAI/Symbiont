# Optional outbound capabilities

Introduced in 3.0, this feature adds named outbound operations to governed ToolClad HTTP
tools. It is opt-in: existing configurations retain their current behavior.
Once an outbound profile is selected, every HTTP tool in that executor must name
a valid capability. Missing configuration, unavailable credentials and unsupported
worker networking fail closed. The runtime never falls back to direct networking
or another credential source.

All enforcement, local diagnostics and evidence are OSS. Linux desktop users can
keep the existing Landlock setup; no separate proxy, container, VM or cluster is
required for the runtime HTTP broker. Other sandbox choices retain their existing
prerequisites. This feature does not qualify a sandbox or change its default tier.

## Configure an operation

Keep this operator-owned control project and all credential-store files outside
worker-readable sources and mount grants. The labels below name the profile's
configured ownership; they are not identity claims accepted from an agent.

```toml
[outbound]
schema_version = 1
tenant = "operations"
workspace = "case-review"

[outbound.credentials.case_service]
source = "environment"
variable = "CASE_SERVICE_KEY"
version = "v1"

[outbound.capabilities.read_case]
tool = "read_case"
origin = "https://cases.example.com"
method = "GET"
path_prefix = "/records/"
credential = "case_service"
header = "authorization"
prefix = "Bearer "
max_requests = 32
max_request_bytes = 65536
max_response_bytes = 262144
timeout_seconds = 15
```

The origin is canonical scheme, host and effective port, without a trailing
slash, path, query or credentials. Public origins require HTTPS and DNS/IP SSRF
checks. An explicit `private_origin = true` permits only an exact loopback or
private IP origin, including its port. Hostnames and cloud metadata addresses
cannot use that exception. Private business APIs can use HTTPS; local fixture or
model endpoints may use HTTP within that explicit private-origin contract.

Bind the operation in `tools/read_case.clad.toml`:

```toml
[tool]
name = "read_case"
version = "1"
description = "Read a case record"

[args.record]
type = "integer"
required = true
min = 1

[http]
capability = "read_case"
method = "GET"
url = "https://cases.example.com/records/{record}"

[output]
format = "json"
max_bytes = 262144
```

`path_prefix` matches a path-segment boundary. Methods, origins and ports must
match exactly. A capability method is GET, HEAD, POST, PUT, PATCH or DELETE.
Write the origin in lowercase and omit the scheme's default port; validation
names the canonical form when it differs. Encoded path separators, ambiguous double encoding, `.`/`..`
segments (also after percent-decoding and with `;` parameters stripped),
fragments, transport-header overrides and streaming request bodies are refused.
Path parameters (`;` or an encoded `%3B` anywhere in the path) are refused
unless the capability sets `allow_path_parameters = true`; servers disagree on
whether a segment such as `..;` is a traversal. Typed manifest arguments remain
mandatory. A path ceiling permits the operations below it; use narrow prefixes,
schemas and Cedar rules appropriate to the business API.

By default a capability does not constrain the query string: any query the
manifest renders is sent. To restrict it, list the permitted keys:

```toml
allowed_query_keys = ["page", "limit"]
```

With `allowed_query_keys` set, every decoded query key must be listed and may
appear at most once; an empty list permits no query at all.

Run `symbi tools validate`, then `symbi doctor` for project and platform checks.
Neither command fetches the application credential or contacts the application
service. A governed run performs that check. Missing secrets produce an explicit
tool error and no upstream call.

For writes, define a separate capability and manifest with
`human_approval = true`. The existing `--approval-terminal` relay displays the
prepared request and requires its exact approval ID. Cedar sees the normalized
operation, profile, credential reference/version and request in
`resolved.outbound`. Approval does not authorize arbitrary requests to the same
hostname. The credential-bearing header is supplied only by the runtime; the
strict manifest cannot override it or use legacy secret placeholders.

## Select a credential store explicitly

Replace the environment source with:

```toml
[outbound.credentials.case_service]
source = "secret_store"
config_file = "/etc/symbiont/case-store.toml"
key = "case_service"
version = "v2"
```

The absolute configuration file uses the existing `SecretsConfig` format for
Vault or the file backend. For example, an encrypted file store configuration:

```toml
type = "file"
path = "/var/lib/symbiont/secrets/cases.json"
format = "json"
[encryption]
enabled = true
[encryption.key]
provider = "env"
env_var = "CASE_STORE_PASSWORD"
```

Provision the store using the existing secrets tooling; do not place plaintext
credentials in tool manifests. Store configuration is captured when the profile
loads and its digest participates in the prepared contract. Its authentication
material is never serialized into that contract. Backend-specific key lookup
remains unchanged, including Vault's per-agent namespace. A store outage, missing
key or decrypt failure is an error; environment variables are never a fallback.
The optional environment source remains available for explicit development use.

`version` is an operator-declared credential identity revision, not an automatic
upstream lease or cryptographic proof of rotation. Change it when changing the
intended credential identity, then start a new executor/session. This feature
does not mint temporary keys for providers that only support static API keys.
A resolved credential is cached only for the lifetime of one run's execution
authority: the store is constructed and the value fetched on that run's first
request, reused by later requests in the same run, and dropped when the run is
revoked or its authority ends. Nothing is cached across runs. The secret store
still controls its own credential expiry and rotation.
Concurrent calls in the same run share successful resolution; failures can be
retried while authority remains live. The store configuration digest covers
connection settings and key references, excluding Vault tokens and AppRole secret
IDs. Changing a secret value alone does not change this digest: update the
declared `version` to identify an intentional rotation.
Vault stores authenticate with the configured method before reading KVv2 data.
Ambient `VAULT_TOKEN`, `VAULT_SKIP_VERIFY` and certificate environment variables
do not override this configuration. Configure `tls.ca_cert` and, when needed,
both `tls.client_cert` and `tls.client_key` explicitly. Unsupported authentication
methods fail before secret retrieval.

## Managed inference

Managed inference uses the same credential type, source resolver, reflection
filter and execution-authority implementation. Existing `api_key_env` settings
remain supported. To select the store explicitly, omit `api_key_env` and add:

```toml
[managed_cli.inference.credential]
source = "secret_store"
config_file = "/etc/symbiont/inference-store.toml"
key = "inference"
version = "v1"
```

Keep the existing endpoint, model and request/output bounds in
`[managed_cli.inference]`. Selecting both credential settings is refused. This
adapter retains its fixed routes, model checks, permitted request fields and
output-token reservations. Its bounded event streams are buffered before delivery
so credential reflection is checked before bytes reach the worker. This setting
does not change ordinary provider adapters used by other runtime routes.

## Lifetime, evidence and limits

Each execution has a runtime-issued assignment epoch. A governed session issues
it once and records it as `outbound_assignment` in the run's trusted context;
the ToolClad HTTP authority for that run adopts the same epoch, so the
`assignment_epoch` in its effect records equals the session's
`outbound_assignment`, and a dispatch bound to a different epoch revokes the
run's authority. Runs outside a governed session receive an epoch from the
executor. The session and executor hold separate in-memory authority objects
(the executor interface cannot carry the session's object); stopping the
session revokes its own authority and cancels the run in the executor, which
revokes the executor's. HTTP authority also binds the run's trusted context and
executor instance. A closed run is remembered until its run deadline, so a
queued call cannot recreate authority, including when the run closed before its
first HTTP request. Cancellation interrupts credential retrieval and network waiting, and prevents
subsequent admissions. Replacing an executor/session produces fresh authority;
these grants are not serialized, transferable worker tokens.

An already admitted remote request may still take effect after cancellation.
Unknown effects retain their request charge and require inspection/reconciliation;
they cannot be made safe by automatic retry. Per-capability request allowances
supplement existing run and inference budgets. The registry permits at most 1,024
concurrently active runs and 128 capabilities per assignment; closed runs are
tracked in a separate bounded set. At either bound it refuses only new runs
until entries expire; existing runs are unaffected. Limits on response bytes and
request duration remain independent of token accounting.

Protected effect records include the capability contract, configured ownership,
credential reference/version, assignment epoch and request/response hashes.
Credential values are absent, including from the request hash, which covers the
credential's reference and version instead of its value. Bodies of responses
whose status is neither 2xx nor listed in the manifest's `success_status` are
replaced with a generic error. Response header names and values are checked for
exact credential reflection before any header hash is retained, including on
oversized or incomplete responses. Header-name checks account for HTTP's ASCII
lowercasing and trimmed header-value whitespace; bodies use exact matching. Safe incomplete responses still retain status,
received-byte counts and a header hash. Returned bodies are also checked before
delivery; refused responses have no body hash. This
does not detect every encoding or transformation of a credential, and an allowed
service can still misuse submitted data. Trust the configured upstream, constrain
operations and apply appropriate data policies.

Use `symbi audit inspect` and the existing invocation reconciliation commands.
The inspector exposes outbound metadata alongside network effects. A signed
prefix without a confirmed terminal outcome remains incomplete.

## Coverage and migration

| Route | Optional contract |
| --- | --- |
| Governed ToolClad HTTP in CLI or runtime integrations that load the project executor | Named capabilities, explicit credential sources, execution authority and effect evidence |
| Managed CLI inference | Shared explicit credential resolver and lifecycle; existing fixed inference contract |
| Governed command/MCP workers in an outbound-profile executor | Independent network denial; no general HTTP proxy is added |
| Browser adapter | Unavailable for this profile; configuration is refused |
| Ordinary inference providers, channels, webhooks and remote buses | Existing route-specific contracts; not covered by the new ToolClad profile |
| Logical workflow resume or VM memory hibernation | Not implemented by this feature |

Profiles are frozen when an executor is constructed. Updating files is not live
revocation of existing executors; cancel/drain them and construct replacements.
Legacy HTTP manifests continue to work without `[outbound]`. Migrating requires
named capabilities and removal of legacy secret templates from those manifests.
Do not use a binary that predates this feature for an enabled configuration:
older versions may ignore a top-level section, although they reject the new
manifest capability field. Downgrading requires an explicit operator migration,
never an automatic runtime fallback.

Rust callers constructing `HttpDef` or `InferenceBrokerConfig` with struct
literals must add `capability: None` or `credential: None`, respectively, to retain
the legacy selection. Existing serialized configurations need no new fields.
Callers constructing `ToolEffect::NetworkRequestStarted` must also add
`outbound: None` when no outbound profile applies.

Registered Landlock execution introduces a `Landlock` variant in both
`SecurityTier` and `dsl::SandboxTier`; update exhaustive matches. `SecurityTier`
no longer implements `Ord`: Landlock is incomparable with hosted execution and
the container/VM ladder. Use `selected.satisfies(&required)` for minimum
requirements, rather than sorting tiers or assuming every backend has a numeric
rank. Landlock satisfies `Landlock` and `None`; Docker, gVisor and Firecracker
do not satisfy an explicit Landlock requirement, and Landlock does not satisfy
their requirements.

See the [control catalogue](security/README.md) for exact evidence scope. Run
`python3 scripts/test-outbound-capabilities.py --binary /absolute/path/to/symbi --report /absolute/new/report.json`
for the local installed-CLI workflow. The fixture uses synthetic data and no paid
providers. Its report does not establish worker sandbox isolation.
