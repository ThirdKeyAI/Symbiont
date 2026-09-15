# Recover an interrupted file publication

Linux Docker/gVisor ToolClad command, MCP and persistent-terminal output grants
publish one new file after successful work, required parsing and confirmed worker
cleanup. The broker first syncs a private candidate and records a signed
`FilePublicationPrepared` intent. The intent binds the output path, parent
device/inode, candidate device/inode, byte count and SHA-256 hash. Publication is
a non-replacing atomic rename, followed by directory sync and a signed
`FilePublicationFinished` record.

A crash between these steps can leave a complete candidate or a published output
whose completion was never recorded. Recovery uses those exact identities and
bytes; it does not rerun the worker or ask a model to reconstruct the file.

## Inspect the original invocation

Run as the runtime service user against its original project. Retain the original
audit public key through a trusted channel and compare it with the inspection.
Use the scope from the original claim: ordinary CLI runs use `cli:orga`, runtime
API and scheduler runs use `scheduler:v1`, and HTTP Input uses `http:input:v1`.

```bash
symbi invocation inspect --project /srv/project --scope cli:orga \
  --id 72d6a833-b825-4b22-b50c-206337d77f7c
```

In `recovery.effects`, find the `identity.kind: "file_publication"` entry and its
`publication_id`. Inspect that publication:

```bash
symbi invocation file-inspect --project /srv/project --scope cli:orga \
  --id 72d6a833-b825-4b22-b50c-206337d77f7c \
  --publication 1a475f5a-1d66-435e-bb5e-98d39e40b8b8
```

| State | Meaning | Exit code |
|---|---|---|
| `ready_to_publish` | The exact candidate remains and the destination is absent. | 2 |
| `published` | The destination contains the exact recorded inode and bytes; the candidate is absent. | 0 |
| `missing` | Neither recorded candidate nor destination exists. | 2 |
| `conflict` | The entries do not match the permitted publication state. | 2 |

Unsafe paths, links, changed parent identity, invalid evidence and an active
invocation owner produce exit 1. Inspection reports the original snapshot hash,
intent and any separately signed recovery receipt. `published` describes this
file's present state; it does not establish completion of the whole invocation.

## Recover the exact file

Review the original effects and cleanup evidence. With the `snapshot_hash` from
inspection, explicitly recover this publication:

```bash
symbi invocation file-recover --project /srv/project --scope cli:orga \
  --id 72d6a833-b825-4b22-b50c-206337d77f7c \
  --publication 1a475f5a-1d66-435e-bb5e-98d39e40b8b8 \
  --snapshot-hash 'sha256:<hash from inspection>'
```

For `ready_to_publish`, recovery rechecks the exact candidate and atomically
publishes it without overwriting any destination. For `published`, it syncs and
acknowledges the existing file without copying or replacing it. Both return a
signed recovery receipt with the original snapshot, intent, file receipt, local
service UID and timestamp. The UID identifies the account, which can be shared
by several operators.

The receipt is stored separately in `.symbiont/invocations/*.publication.json`.
Its Ed25519 signature covers the serialized payload prefixed by
`symbi-file-publication-recovery:v1` and a newline, using the original audit key.
Receipt storage is bounded, synced and published without replacement. Original
claim and journal bytes remain unchanged, including an incomplete final fragment.

Repeating recovery verifies the retained receipt and current output identity,
then returns the original receipt. If the process dies after publication but
before saving the receipt, repetition recognizes the exact published inode and
finishes recording it. Changed evidence, missing signing keys, changed file bytes
or identities, links, competing destinations and previously resolved invocations
are refused. Nothing grants another execution under the original invocation ID.

Recovery returns `tool_repeated: false` and `invocation_completed: false`. The
invocation remains unresolved until the operator separately records an
[invocation-wide assessment](invocation-reconciliation.md), considering all its
effects. Perform file recovery before that final assessment.

## Limits and validation

This operator command is not exposed as a model tool. It only handles retained
claims with authenticated broker publication intents. A candidate created before
the intent becomes durable has no authenticated recovery authority. Older runs,
missing candidates, changed destination directories, damaged evidence and
arbitrary external effects require separate investigation. Preserve uncertain
artifacts rather than deleting them to make a retry appear fresh.

The focused `file_publication_recovery` integration tests check identity/content
changes, hard links, symlinks, competing targets, stale snapshots, active owners,
forged receipts, repeated recovery and original-evidence preservation.
`scripts/test-file-publication.py` uses the shipping CLI, actual Docker workers,
a local scripted provider and host `strace` to inject `SIGKILL` immediately before
and after the rename. It checks useful output, independent journal/receipt
verification and same-ID retries without another provider request. This is a
process-crash test; power-loss durability and privileged VM host outage behavior
require their own deployment tests.
