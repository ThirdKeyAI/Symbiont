# Bounded source queries

The bundled `read_file`, `list_files` and `grep_files` tools use a fixed runtime
file broker. Configure a Docker or gVisor mount ceiling for the source tree and
its working directory, separately from the runtime control project:

```toml
[sandbox]
tier = "docker"
[sandbox.docker]
working_dir = "/source"
volumes = ["/srv/source:/source:ro"]
```

Firecracker selects its own explicit, read-only broker roots:

```toml
[sandbox]
tier = "firecracker"
[sandbox.firecracker]
kernel_image_path = "/srv/vm/vmlinux"
rootfs_path = "/srv/vm/rootfs.ext4"
firecracker_binary = "/usr/local/bin/firecracker"
working_dir = "/tmp"
source_roots = ["/srv/source:/tmp:ro"]
```

These roots use `host:virtual:ro` syntax and map relative query paths under the
configured working directory. They are host broker access ceilings; the guest
receives neither a mount nor an automatic copy. Ordinary guest commands still
use image contents and private scratch. An inactive Docker/gVisor configuration
cannot grant a Firecracker source path. At most 32 roots are accepted; writable
roots, duplicate destinations, protected host paths, supervisor storage and
runtime project control directories are refused. Nested roots retain the same
most-specific-path semantics as container ceilings.

Firecracker supports the three fixed operations below. Git snapshots and general
command/MCP/PTY file imports or output publication still require a separate guest
file-transfer contract and remain unsupported. VM artifacts and the selected
configuration are validated before source preparation. Fixed broker operations
do not launch a VM or establish that the deployment can boot one; validate an
actual guest command separately. See [Firecracker setup](firecracker-setup.md).

A source tool declares one operation instead of an executable backend:

```toml
[tool]
name = "list_files"
version = "1.1.0"
description = "List source files below the configured working directory"
timeout_seconds = 10
[source]
operation = "list_files"
[output]
format = "json"
```

The source backend requires `oneshot` mode and JSON output. It cannot be combined
with a binary, command, HTTP, MCP, session, browser, custom parser or `[filesystem]`
declaration. This is an embedding-runtime extension: the four standalone ToolClad
reference implementations explicitly refuse source manifests, including dry runs,
instead of ignoring the declaration or executing an accompanying command.

## Available operations

| Operation | Arguments | Result |
| --- | --- | --- |
| `read_file` | Required relative `path`; optional byte `offset` and `limit` | Path, offset, byte count, observed file size and UTF-8 text |
| `list_files` | None | Regular file paths, `truncated`, `skipped` |
| `grep_files` | Required literal UTF-8 `needle`, 1–1,024 bytes | Matching paths, one-based line numbers and excerpts; `truncated`, `skipped`, `files_scanned` |

The current ToolClad model schema represents argument values as strings; for
example, send `"offset": "6"` and `"limit": "23"`. Numeric declarations still
validate and bound those values.

Reads default to 32,768 bytes, allow at most 65,536 bytes per call, and permit
an offset up to 10,000,000 bytes. Listing reads names and metadata only. Search
reads at most 1 MiB per file and 8 MiB in total, returns at most 200 matches and
limits each excerpt to 512 input bytes. Enumeration stops at 10,000 entries,
2,000 regular files, 32 levels or 65,536 path bytes. Invalid UTF-8 content is
replaced when rendered; the recorded hash covers the actual input bytes.

Links, special files, parent traversal and `.git`/`.symbiont` paths are unavailable.
Search and listing report skipped entries; unreadable, linked or excessive input
makes the result explicitly partial. The two excluded control directories are
outside the query scope. Missing mount ceilings or unavailable declared roots
produce errors rather than successful empty results. More specific roots hide
the underlying parent contents, including during recursive enumeration.

These limits bound regular-file work and retained data. Deadline checks are
cooperative; an operator-mounted filesystem with blocking kernel I/O can still
stall a read. Deploy source ceilings on appropriate local storage. This broker
does not provide a separate process or OS resource quota for filesystem I/O.

## Authorization and evidence

Preparation captures the bounded result before policy evaluation. It retains
that result until authorized dispatch; later file changes do not change the
returned snapshot. Preparation reads are performed by trusted runtime code and
are not rolled back when policy denies the call. The result is released to the
caller only after authorization. A directory scan is not an atomic snapshot of
an entire changing tree.

The prepared call binds `fixed_source_broker`, the broker implementation hash,
operation, working directory, limits, input byte ranges/hashes and canonical
result digest. Cedar and exact approval see the file descriptor at
`context.invocation.resolved.command_boundary.filesystem`. Listing descriptors
contain no content reads. Signed dispatch records bind the prepared call and
returned observation. The result digest covers canonical JSON, not arbitrary
serialization of the outer evidence envelope.

No user program runs in the broker. No source mount is handed to a worker, and
these operations do not consume a worker slot. General-purpose command tools
continue to require explicit individual `[filesystem]` grants. Runtime SDK calls
remain trusted embedding interfaces; applications must use governed dispatch
when policy, approvals and protected audit records are required.

## Git queries and remaining work

The bundled `git_diff`, `git_staged_diff`, `git_log` and `git_status` tools use
[isolated repository snapshots](git-source-queries.md). They run a fixed driver
in the selected worker and consume shared worker capacity. Log and staged diff
omit the worktree; log also omits the index. The worker receives private copies,
not the original source directory. Results and supported repository layouts are
described separately from the in-process read/list/search operations above.

The fixed broker supports Linux Docker/gVisor ceilings and explicit Firecracker
source roots. Declared command/MCP/PTY files use a separate
[bounded transfer](filesystem-grants.md#firecracker-file-transfer). Guest Git
uses its own [snapshot stream](git-source-queries.md#firecracker-snapshots).
Existing configured guest commands retain their guest-image contract. The broker
does not copy host source into a VM or add filesystem access to a CLI worker.
Other filesystem capabilities retain their documented scopes.

## Validation

Focused tests cover exact range snapshots, changed files, overlapping and nested
ceilings, traversal, links/FIFOs, partial search, expired authority and conflicting
backends. The shipping CLI fixture runs local scripted inference, verifies signed
journals independently with a pinned Ed25519 key, checks policy denial and useful
source results, and observes a real Docker command with no source mounts:

```bash
python3 scripts/test-source-broker.py --binary target/debug/symbi --report /tmp/source-broker.json
```

The fixture uses synthetic source and observer files plus an already cached
`python:3.12-slim` image. It contacts no external inference provider.

Run the source fixture against actual Firecracker artifacts to verify the fixed
queries, refusal cases, signed evidence and an independent guest scratch control:

```bash
python3 scripts/test-source-broker.py --binary target/debug/symbi \
  --firecracker-binary /srv/vm/firecracker --kernel /srv/vm/vmlinux \
  --rootfs /srv/vm/rootfs.ext4 --report /tmp/vm-source-e2e.json
```

The fixture records artifact hashes, actual VMM process identity and VM device
configuration. The guest control must return useful output as UID 65534 without
seeing host source or observer files; confirmed process removal is required. It
uses a synthetic local provider and does not provision a privileged host service.
