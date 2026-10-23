# Optional governed improvements

This OSS foundation adds an explicit lifecycle for versioned workflow instructions:
candidate → signed trial evidence → independent acceptance checks → exact operator
approval → activation → a version pinned for each run. It does not modify model
weights, permissions, tool contracts or executable agent definitions.

Ordinary agents do not opt in automatically. Only `symbi improvement init` creates
an enabled workflow, and only a run explicitly selecting that workflow uses it.
Existing invocations, installations and configuration files need no migration.
The optional Enterprise Console integration adds authenticated review receipts.
Quorum review chains, fleet rollout and compliance reporting remain separate work.

## First-release contract

- The workflow owner supplies and freezes an acceptance suite independently of the
  candidate. The included evaluator checks exact answers and resource ceilings
  against completed, signed trial journals. Domain-specific evaluators implement
  the same Rust interface; evaluation does not run tools or inference.
- Candidates contain instructions, a rationale and evidence references. References
  are descriptive data, never URLs to fetch or commands to execute.
- Approval binds one candidate, suite and evaluation report. Activation requires
  that approval and an exact expected current version to prevent lost updates.
- The agent source and configured deployment files are fingerprinted. Changed
  dependencies require a new workflow and evaluation. An approved version is also
  bound to its evaluated provider, model and loop configuration.
- Disabled, missing, modified, failed or unapproved selections fail before model
  work. There is no fallback to an unapproved or previous candidate.
- Running invocations retain their selected version when an operator promotes,
  rolls back or disables a workflow. Disabling stops new selections; it does not
  cancel work already admitted.
- Runtime-owned state is private and signed. The trusted local operator can manage
  it; agents and sandboxed workers must not receive access to that state or signing
  keys. Local approvals identify the OS operator, not an independently authenticated
  enterprise reviewer. External verification needs a separately pinned public key.

Integration covers ordinary CLI ORGA runs, explicitly registered HTTP and scheduled
ORGA agents, and an explicit Rust runner API on Unix hosts. Managed CLI and chat
surfaces do not consume these workflows. Explicit unsupported combinations are refused. This is an opt-in
workflow instruction lifecycle, not a general counterfactual simulator or an
automatically self-modifying runtime.

## Evidence limits

Passing a configured suite establishes results for those cases and environment.
It does not prove that the instructions generalize or meet a regulatory standard.
Use independently maintained cases and domain review. Recorded policy denials are
reported separately; a successful final answer does not erase those attempts.
Signed records establish provenance, not the truth of an external tool's output.
An old journal cannot supply outcomes for actions that were never taken.

The normal execution policy, approvals, isolation and shared resource limits remain
authoritative during both trials and approved runs. A trial is real governed
execution: run it only against a deliberately configured evaluation environment.

## Validation

Release validation covers the disabled/default path, complete candidate-to-run
lifecycle, rejected evidence, exact approval binding, stale activation, rollback,
dependency drift, tampering, and an installed CLI with a local scripted provider.
No external inference service is required for the included checks.

For a real local-model pilot, use
[`scripts/test-intake-improvements.py`](https://github.com/ThirdKeyAI/Symbiont/blob/main/scripts/test-intake-improvements.py)
with the [document-intake contract](https://github.com/ThirdKeyAI/Symbiont/blob/main/examples/governed-improvements/document-intake/contract.md).
It compares the original agent with two fixed instruction candidates on 14
synthetic cases, then exercises approval, activation, rollback and disabling.
The model receives case inputs without the expected outputs. Every outcome and
signed journal is retained in a new private test project. Failed candidates stay
unpromoted; the driver never relaxes acceptance thresholds to finish the lifecycle.
This is an engineering pilot, not evidence of suitability for a regulated deployment.
See the [recorded local-model pilot results](governed-intake-pilot.md) for the
baseline comparison, resource use, lifecycle checks and remaining validation scope.

The [phase 2 follow-up](governed-intake-phase2.md) repeats a frozen candidate on
new cases against both general and information-matched ordinary controls. It also
exercises a separate approved Landlock receipt workflow through denial and crash
recovery. The routing candidate failed all three held-out evaluations and remained
unpromoted; successful lifecycle controls do not establish model reliability.

## CLI walkthrough

The files in `examples/governed-improvements/` demonstrate administrative intake
routing. They are fixtures, not an insurance decision system. Copy `claims.symbi`
into the evaluation project's `agents/` directory and place `suite.json` and
`proposal.json` beside it. Configure the project's normal model provider and
policies before initialization. No additional permissions are needed to return
text; tools still require the normal explicit grants.

```sh
symbi improvement init --workflow claims --agent agents/claims.symbi --suite suite.json
symbi improvement propose --workflow claims --file proposal.json
```

Save the returned `candidate` digest as `CANDIDATE`. The acceptance suite is copied
into signed, private operator state. Editing the original suite file afterward
cannot relax it. Initialization refuses an existing workflow; use a new workflow
name to change the acceptance contract.

Run each case explicitly in the evaluation environment:

```sh
symbi run claims --improvement claims --improvement-trial "$CANDIDATE" --input missing_document
symbi run claims --improvement claims --improvement-trial "$CANDIDATE" --input exception_case
```

For each run, retain its printed audit run ID, journal path and public key. Create
`trials.json` using those exact references (replace the illustrative values):

```json
[
  {"case_id":"missing", "audit":{"run_id":"UUID-1", "path":"/absolute/project/.symbiont/governed/JOURNAL-1.jsonl", "public_key":"HEX-PUBLIC-KEY"}},
  {"case_id":"exception", "audit":{"run_id":"UUID-2", "path":"/absolute/project/.symbiont/governed/JOURNAL-2.jsonl", "public_key":"HEX-PUBLIC-KEY"}}
]
```

The evaluator verifies against the project's own audit key, not a key trusted
merely because it appears in `trials.json`. Every case must have one distinct,
completed trial with matching input, candidate and environment. Incomplete or
uncertain evidence is refused. Evaluation performs no network or tool calls.

```sh
symbi improvement evaluate --workflow claims --candidate "$CANDIDATE" --trials trials.json
```

Exit 0 means the frozen criteria passed, exit 2 means the recorded evaluation
failed them, and exit 1 means invalid input, evidence or state. Save the returned
`evaluation` digest as `EVALUATION`. Review the candidate and report:

```sh
symbi improvement inspect --workflow claims --candidate "$CANDIDATE" --evaluation "$EVALUATION"
symbi improvement approve --workflow claims --candidate "$CANDIDATE" --evaluation "$EVALUATION" --rationale 'Reviewed the candidate and acceptance evidence'
```

Save the returned `approval` as `APPROVAL`, then activate:

```sh
symbi improvement promote --workflow claims --candidate "$CANDIDATE" --approval "$APPROVAL" --expected-active none
symbi run claims --improvement claims --input missing_document
```

Only that explicit selection uses the improvement. `symbi run claims --input ...`
continues to use the ordinary agent behavior. An approval does not activate a
candidate, and activation does not rewrite the `.symbi` source.

A replacement candidate records the currently active version as its parent. After
separate trials and approval, promote it with `--expected-active` set to that
parent's digest. A stale expected version fails without changing active state.
Rollback requires a previously activated candidate and its valid approval:

```sh
symbi improvement rollback --workflow claims --candidate "$PRIOR" --approval "$PRIOR_APPROVAL" --expected-active "$CURRENT"
symbi improvement disable --workflow claims
symbi improvement enable --workflow claims
```

Disabling causes explicitly selected runs to fail; it never silently falls back.
It does not revoke ordinary independent permission to run the original agent.
Use the existing execution policy if that ordinary capability must be restricted.

## Export and independent verification

`inspect` and `init` print the workflow signing public key. Pin it through a
trusted channel. The workflow key is distinct from the project's run-audit key.

```sh
symbi improvement export --workflow claims --kind candidate --id "$CANDIDATE" > candidate.signed.json
symbi improvement verify --kind candidate --file candidate.signed.json --public-key "$WORKFLOW_PUBLIC_KEY"
```

The same commands support `evaluation`, `approval` and `state` (omit `--id` for
state). Exported documents can contain sensitive instructions or case data; share
and retain them under the source data's access rules. Signature verification
requires no Enterprise service. It authenticates one document, not its current
activation status or suitability for a particular deployment.

## Runtime integration and current bounds

Rust integrations use `improvement::Store::pin` with the exact resolved agent name
and source, then explicitly call `ReasoningLoopRunner::run_with_improvement`.
Release the store before starting the run. The returned pin retains an immutable
candidate and approval. Include `PinnedImprovement::identity()` in any durable
invocation claim; CLI invocation identities already do so.

`Evaluator` receives verified `TrialEvidence`. The first-release exact-answer,
coverage and resource gates remain mandatory; a registered domain evaluator can
add stricter acceptance criteria. Candidates cannot register or execute evaluators.
The runner requires a protected journal and records the selected candidate,
approval, trial/approved mode, input hash and environment before inference.
Custom inference providers must implement `InferenceProvider::configuration_identity`
for improvement runs; ordinary execution does not require it. The shipping cloud
provider fingerprints its configured endpoint and region without exporting secrets.

State lives in `.symbiont/improvements/<workflow>/`. Inputs and signed documents
are bounded to 1 MiB; instructions to 32 KiB; suites to 128 distinct cases;
deployment fingerprints to 1,024 entries/16 MiB; workflows to 4,096 immutable
objects, 1,024 activations and 1,024 enable/disable records. Administrative access
is serialized by a lock. Control records retain the operator UID and time.
Atomic state replacement preserves the previous complete state if publication
fails before replacement. A crash after replacement can leave the caller uncertain;
inspect state before retrying, using the expected active version.
A failed state publication invalidates that open SDK store handle; reopen and
inspect it before attempting another selection or update.

Deployment fingerprints cover `symbiont.toml`, `toolclad.toml`, `mcp-config.toml`,
`scope/`, `policies/` and `tools/`, including additions and removals. The agent
source is bound separately. Provider name, configured endpoint/region, model
identifier, loop settings and base system messages are bound to the accepted
trial environment. Provider endpoint internals, remote model
weight changes and external service state are not attested. Operators must
revalidate when those conditions change. Dynamic knowledge/delegation state is
also outside this first evaluator's suitability claim.

Private state assumes a trusted operator and protected host storage. It does not
resist an administrator replacing both keys and records, or establish freshness
against restoration of an entire older signed store. Retain independent public-key
and audit references for external review. The capability does not automatically
collect experience, share data across tenants, schedule evaluation, or change
runtime behavior in installations that have not opted in.

## Operational agent selection

Set the operator-owned registration metadata `improvement_workflow` to the workflow
identifier. With `symbi up` or `symbi cron add`, declare it in the source file:

```symbi
metadata { improvement_workflow = "claims" }
agent claims {}
```

For an administrative API registration, use `metadata: {"improvement_workflow":
"claims"}`. Invocation input cannot select a workflow or supply a trial candidate.
The workflow must already exist, be enabled, and have a matching approved active
version. Ordinary CLI runs still require `--improvement claims`; metadata alone
does not change that CLI behavior. Custom scheduler executors must explicitly
support admitted improvement pins or the selection is refused.
Those executors retrieve the immutable selection through `ScheduledTask::improvement`
and pass it to `run_with_improvement_cancellable`, retaining normal cancellation
and cleanup ownership.

The CLI's default prompt and loop profile is shared with selected operational
runs: 10 iterations, 100,000 total tokens, and the source timeout capped by the
ordinary loop's default deadline. Collect trials with the same source, deployment,
provider and model, using the default CLI limits. An evaluation with different
limits cannot authorize this profile. Queue time and tighter registered resource
limits independently cancel a run; they never broaden the evaluated profile.
Selected HTTP runs refuse a caller `system_prompt` override.

A version is pinned before queue admission. Promotion, rollback or disablement
affects subsequent selections. HTTP and scheduler claims separately bind the
original caller request and runtime-selected instruction version. A retry with
the same identity returns its original saved result even after promotion or
disablement. Unknown outcomes retain the original claim and never grant replay.
The CLI's existing invocation identity contract continues to refuse reuse after
its selected active version changes. HTTP responses and scheduled completions
include `improvement` attribution; signed journals include both the claim binding
and the evaluated execution binding.

## Operator HTTP controls

Authenticated administrative keys can read
`GET /api/v1/improvements/:workflow`, optionally with `candidate`, `evaluation`
and `approval` digest query parameters. It returns the current version, enablement,
activation history and requested evidence. Agent-scoped keys cannot read or mutate
this control plane. An absent workflow is never created by a read.

`POST /api/v1/improvements/:workflow/actions` accepts:

- `{"action":"approve","receipt":{...}}`: import a signed Console decision.
- `{"action":"activate","candidate":"DIGEST","approval":"DIGEST","expected_active":null,"rollback":false}`:
  promote, using `null` only before the first activation. Later transitions require
  the exact currently active digest. Set `rollback:true` for a previously activated
  approved version.
- `{"action":"set_enabled","enabled":false}`: stop new selections.

The API cannot replace authenticated reviewer identity with a caller-supplied name.
Local OS-operator approval remains available through `symbi improvement approve`.
There is no requirement to operate Enterprise services for the OSS lifecycle.

## Trusting Console review receipts

Provision a private, operator-owned `.symbiont/console-review-trust.json` with mode
`0600`. Keep it outside all worker grants, alongside the existing protected state:

```json
{"workspace":"acme/prod","runtime":"intake","public_keys":["64_HEX_DIGITS_OF_THE_CONSOLE_PUBLIC_KEY"]}
```

Obtain and verify the Console's Ed25519 public key through your deployment's trusted
configuration channel. A key supplied in an API request or receipt is not trusted.
The runtime matches workspace, runtime, workflow, candidate and evaluation and
rechecks that the exact candidate passed the workflow's frozen acceptance suite.
Receipts also bind the actual authenticated reviewer, approval record, decision
time and rationale. Import is allowed for at most 15 minutes after receipt issue;
that transport expiry does not expire an already imported instruction approval.
An accepted receipt is retained inside the runtime's signed approval document.

For an offline transfer, use:

```sh
symbi improvement approve-receipt --workflow claims --file console-receipt.json
```

Import does not activate the candidate. Promotion still requires a separate
operator action and the expected current version. Removing a trusted key prevents
new imports from that key; disable the workflow to stop new runs using approvals
already accepted. Local administrators remain trusted and can still make local
approvals. This integration does not impose reviewer quorum or separation of duties.

Run the installed CLI and operational route check with:

```sh
python3 scripts/test-governed-improvements.py --binary /path/to/symbi --report /tmp/improvements.json --operational
```

The operational fixture requires OpenSSL 3+ to sign independent receipt fixtures.
It uses local synthetic inference and no effectful tools, checking CLI/API receipt
imports, version selection, attribution, transitions and retry behavior; it does not qualify
model reliability or expand the existing HTTP/scheduler sandbox support matrix.
