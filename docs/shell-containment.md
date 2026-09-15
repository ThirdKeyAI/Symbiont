# Governed shell workspace

Local orchestrator turns and `@agent` fleet conversations use protected per-turn
journals. Fixed file tools execute through checked file handles in the trusted
runtime; arbitrary commands execute in the project-selected Docker or gVisor
worker. Both use prepared contracts, Cedar and the same approval queue. Invalid configuration and unavailable
transports fail before inference. Malformed `.symbi/constraints.toml` stops startup
before terminal raw mode. There is no implicit host project mount.

## Configure useful file access

Create an operator-owned data directory and declare its access ceiling in `symbiont.toml`:

```toml
[sandbox]
tier = "docker"

[sandbox.docker]
image = "python:3.12-slim"
user = "1000:1000"
volumes = ["/absolute/project/data:/workspace:rw"]
```

File operations use the runtime service user's permissions within these ceilings;
the configured worker UID/GID applies to arbitrary commands. Pin the image digest
in a controlled deployment and provide `sh` for commands. The fixed file broker
does not need Python in the image. Use `ro` for read-only data and declare only the needed directories. Project policy,
agent and tool definitions, runtime configuration, credentials, `.git` and audit
storage are control data and cannot be exposed by a project data mount.

Paths in `read_file`, `search`, `edit_file` and `save_artifact` are relative to
`/workspace`. For this example, `result.txt` refers to `/absolute/project/data/result.txt`.
Absolute paths, parent traversal, `.git` and `.symbiont` paths are rejected before
policy evaluation. The broker uses directory descriptors and rejects symlinks,
multiple hard links and nonregular files. Search reports skipped entries and limits.
Reads, write content and commands are bounded to 32 KiB; search also limits files,
inspected bytes and traversal depth. No arbitrary program receives a workspace
directory through these tools. Reads and searches return the bounded snapshots
captured during preparation; authorization includes their paths and hashes.
Search follows the most specific ceiling when declarations overlap, inspects at
most 512 files and 1 MiB, and reports truncation or other limits explicitly.

`save_artifact` validates the supplied DSL, Cedar or ToolClad content before an
approval request, then writes it under a writable data ceiling. It can create parent
directories there. Install reviewed artifacts into trusted project configuration
through the operator's normal workflow. The broker cannot rewrite project policy
or registered tool definitions.

Writes bind the destination, exact content hash, held parent directory and any
prior file identity/content hash into approval. New files publish atomically
without replacing a competing entry. Existing files are updated through their
retained inode after checking that the path and content still match. They preserve
the inode and permissions. Broker edits use a nonblocking file lock; independent
host writers must honor `flock` to serialize their changes with the broker.
An existing-file update is not an atomic replacement: a process or storage failure
can leave partial content and an unresolved action. Inspect the journal and file
before retrying. A changed file or destination refuses the held edit; prepare and
review a new call. Missing artifact directories are created only after approval.

`shell` commands receive private scratch space and no host mounts. Use the fixed
file tools for workspace I/O, or a ToolClad manifest with explicit filesystem
grants on a route exposing that manifest, when a program needs input/output files.

## Review an exact action

Cedar sees normalized arguments and the immutable executor, contract and selected
sandbox metadata. Each `edit_file`, `save_artifact` and enabled `shell` call requires
its own expiring, single-use approval. `--yes` skips conversational confirmation;
it does not bypass this runtime requirement. `--allow-shell` advertises `shell`
and enables its policy path; the command still needs approval and runs inside the
selected worker.

Press `Ctrl+G`, select the request, press Enter to review the complete request,
then press `a` to approve or `d` to deny. The review includes normalized paths,
exact content or command, contract hash and selected boundary. File calls also
show `execution_transport: fixed_workspace_file_broker` and the effective
`file_access` descriptor. The retained `worker_program_hash` field identifies the
compiled broker implementation. Changing project configuration after startup does not silently
replace the approved executor's profile. Restart the shell to select new configuration.

Fleet runners retain the registered principal and take the intersection of
registered tools and their selected tool list. `delegate` remains unavailable to
fleet runners. Canonical `.symbi` and legacy-extension `.dsl` files can declare
ORGA conversational agents with literal capabilities and empty `with` blocks:

```symbi
metadata { description = "Review workspace files", executor = "orga" }
agent reviewer() {
    capabilities = ["read"]
    with sandbox = "docker", timeout = 20.seconds {}
}
```

The loader retains the complete source bytes and selects each declaration by its
exact name. Capabilities and execution settings come from that declaration alone.
A declared sandbox overrides the project default before backend validation, so an
unused unavailable default does not prevent an explicitly selected Docker worker.
The timeout bounds both the turn and worker. The runner freezes its selected
profile before inference and approval; changing a file during a held action does
not replace the approved source or boundary. Startup audit and each prepared call
include source and declaration hashes, execution mode and selected settings.

This route executes ORGA conversations with external Cedar policies and the
supported [inline effect rules](inline-policies.md). Inline allows cannot waive
Cedar or mandatory approval. File-wide and selected-agent rules use actual
normalized calls; unsupported rule expressions, `require` and `audit` are refused.
Functions, parameters, return types, nonempty `with` bodies, unknown `with`
requirements and non-ORGA executors remain unsupported. Metadata is limited to
literal `description`, `version`, `author` and `executor` strings. Complete
canonical behavior and broader policy execution remain pending.

Duplicate names across files are refused rather than selected by directory order.
Reloading removes missing, ambiguous or refused definitions from the fleet and
clears cached runners. Existing admitted runs retain their original contract.
Dynamic DSL `spawn_agent` cannot replace a canonical registration. Direct `ask`,
`send_to`, composition and registry prompt-only inference cannot enforce canonical
requirements and reject these recipients; use the governed fleet `@name` route.
A prompt-only TOML agent continues to use the project profile.

## Inspect execution evidence

Response metadata includes the audit run ID. Orchestrator and fleet turns share
the display and public reference history while retaining separate signed journals.
`/audit` lists public references
(`run_id`, `path`, `public_key`) and the recent live display. Each turn writes a
separate signed journal under `.symbiont/governed/`; the live buffer is only a
display. Up to 256 references are displayed with an omitted count. Journal files
remain available independently of that display limit. Preserve the public key
through a trusted channel and verify the expected run and terminal record.

Ctrl+C requests cancellation; the UI reports that cleanup is pending. A retained
owner handles caller cancellation, and normal shell exit waits for these owners. One-shot command owners retain
cleanup acknowledgements, including after a call deadline drops its dispatch
future. Run termination waits for these acknowledgements; missing or failed
cleanup is an explicit run failure. Runtime process loss still depends on the
independent supervisor and its recovery contract. Failed storage can leave an
incomplete signed prefix. See [protected audit](run-audit.md) and
[Docker containment](docker-containment.md).

Provider failures and incomplete terminations, including timeouts, return an error with the audit reference. The shell does not
retry a whole turn automatically after tool effects. Inspect recorded outcomes
before explicitly retrying. These changes do not establish complete containment:
remaining transport coverage, durable effect reconciliation, full canonical DSL
behavior/policy execution and the outer evaluation lab remain in progress.

## Shipping regression test

```bash
python3 scripts/test-shell-governance.py \
  --binary target/debug/symbi-shell \
  --supervisor target/debug/symbi-sandbox-supervisor \
  --report /tmp/shell-governance.json
```

The test uses a local provider fixture, an actual terminal, the local Gate queue,
real brokered file effects and an independently pinned ephemeral audit key. It also
checks cancellation, wrapped transcript visibility, startup refusal and canonical fleet approval, denial and unavailable selection. It requires
tmux, OpenSSL and Docker for cleanup observations. Missing prerequisites
are failures, not passing containment evidence. The Rust shell regression suite
also exercises normalized Cedar input, exact content, fleet identity, rejected
links and delayed removal of a worker with a detached descendant.
