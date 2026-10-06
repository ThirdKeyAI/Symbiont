# Isolated Git source queries

Linux Landlock also supports this workflow through explicit `[sandbox.roots]` ceilings. Native commands use private bounded workspaces and retained staging; see [Linux development](landlock-development.md).

The bundled `git_diff`, `git_staged_diff`, `git_log` and `git_status` tools now use
`[source]` operations. A fixed Git driver runs in the selected Linux Docker,
gVisor or Firecracker worker with bounded private copies of the repository. The original source
directory is never mounted into that worker. Configure the repository root as
the source working directory and retain its operator-owned mount ceiling; see
[source queries](source-queries.md).

```toml
[tool]
name = "git_diff"
version = "1.1.0"
description = "Read the unstaged source diff"
timeout_seconds = 15
[source]
operation = "git_diff"
[output]
format = "json"
```

These operations accept no arguments, command template, binary override or custom
parser. Provision Git and Python 3 in the selected image. The Python driver uses
isolated import mode, so repository files cannot replace its standard libraries.

## Inputs and execution

| Query | Snapshot contents | Output |
| --- | --- | --- |
| `git_log` | Git object store, refs, HEAD, repository format configuration and relevant metadata; no index or worktree | Up to 20 recent commits, one line each |
| `git_staged_diff` | Git metadata plus index and shared-index files; no worktree | Staged diff |
| `git_diff` | Git metadata, index and worktree | Unstaged diff |
| `git_status` | Git metadata, index and worktree | Porcelain v1 status, normal untracked-file reporting |

The metadata grant includes the bounded object database, including historical
blobs and unreachable objects. It is not restricted to the text of the requested
commits. The worktree excludes `.git` and `.symbiont` directories. Symlink targets
are copied as link text, never read on the host. Hard-linked regular files,
special files, non-UTF-8 paths and symlinked metadata are refused. Overlapping
mounts honor the more specific ceiling, including virtual child mounts.

Preparation streams copies through retained file handles into a private directory.
It records each input's path, kind, byte count and SHA-256 hash before policy and
approval. A subsequent source change cannot replace those copies. This is not an
atomic snapshot of a changing repository: inconsistent metadata can produce a Git
error. Nothing is truncated silently to manufacture a clean status or empty diff.

One snapshot permits at most 128 MiB in total, 64 MiB per regular file, 10,000
files, 20,000 enumerated entries, 64 directory levels and 1 MiB of source path bytes, including directory names.
Exceeding a bound refuses preparation. Host I/O deadlines are cooperative; use
appropriate local storage. Private staging consumes bounded disk space, while
copying uses a bounded buffer rather than retaining the whole repository in RAM.
Snapshot preparation also reserves from the shared host staging pool before
copying. Concurrent processes and retained copies compete for that capacity;
worker admission remains a separate gate. See [staging capacity](staging-capacity.md).

After authorization, the independent supervisor admits and owns the worker. Its
profile keeps the selected image, tier and CPU/memory limits, forces networking
to `none`, and mounts only the private Git input and worktree copies read-only at
`/symbi-git-input` and `/symbi-source`. Git's temporary configuration and top-level
metadata files live in private guest scratch. Object/reference directories point
to the read-only snapshot. Success is returned after confirmed worker cleanup.

The fixed driver parses repository configuration inside the worker with includes
disabled. It preserves only supported repository/object format information and
creates a new configuration. External diff drivers, text conversion, filters,
hooks, fsmonitor, optional index writes, replacement objects and network protocols
cannot be enabled by the source configuration. Repository/global customizations
such as aliases, autocrlf, external excludes and diff drivers are intentionally
not reproduced. Results describe the fixed query contract. Submodule changes are
ignored; this is not a recursive submodule review.

SHA-1 and SHA-256 object formats are supported. External gitdir files, common
metadata directories, alternate object stores, partial/promisor stores and other
repository extensions are refused. A worktree using an external `.git` pointer
needs a future explicit metadata-grant design; the broker does not follow it or
run host Git to resolve it. These structures are described in Git's
[repository layout](https://git-scm.com/docs/gitrepository-layout) and
[configuration documentation](https://git-scm.com/docs/git-config).

## Results, evidence and interruption

Successful envelopes contain `results.raw_output`, `results.stderr`,
`results.exit_code`, the stdout `output_hash`, and `snapshot_bytes`. The prepared
call binds `selected_git_snapshot_boundary`, the driver implementation, effective
worker profile, input hashes, query operation and mount destinations. Operator
inspection exposes the descriptor under
`context.invocation.resolved.command_boundary.filesystem`.

A nonzero Git exit or cleanup failure is an error, not an empty successful result.
The existing governed dispatcher retains uncertain outcomes and prevents further
inference or silent replay. A crash can leave an incomplete signed journal and
private snapshot directories; same-ID retries remain subject to invocation
recovery. The staging pool automatically reconciles copies after caller locks
and durable worker references are gone. Uncertain worker cleanup retains the
copy and its charge. Older temporary directories are not automatically adopted;
establish their ownership and worker cleanup before removing them.

Firecracker uses explicit read-only source roots and the bounded snapshot
transfer described below. Fixed read/list/search queries continue using the
host broker without launching a worker. Standalone ToolClad runners refuse the
runtime-owned source extension and cannot execute these Git operations directly.

## Firecracker snapshots

Select `tier = "firecracker"`, provide the matching kernel/rootfs/VMM artifacts,
and configure a source ceiling. The virtual working directory resolves the
repository; it does not mount that host directory into the guest:

```toml
[sandbox.firecracker]
working_dir = "/tmp"
source_roots = ["/srv/project/source:/tmp:ro"]
```

Provision Git and Python 3 in the rootfs. The runtime retains the configured
vCPU and guest-memory allowance and reserves shared host staging before copying.
It streams the prepared tree to `/tmp/symbi-git-snapshot` inside a fresh guest.
This separate protocol preserves the existing 128 MiB repository limit and
64 MiB file limit; ordinary `[filesystem]` grants retain their smaller bounds.

Each entry has a bounded header with a relative path, kind, length, content hash
and executable bit. The prepared call includes a hash of all ordered headers.
The transfer allows up to 20,008 entries, 10,001 non-directory entries and 2 MiB
of virtual path bytes, including fixed scaffolding and an optional empty config.
Per-entry headers are capped at 16 KiB. Input metadata cannot contain symlinks;
worktree symlinks carry only bounded link text. No host archive extractor or
source directory mount is used.

Guest PID 1 creates the tree through directory handles beneath the fixed root,
checks all hashes and aggregate counts, then remounts the snapshot tmpfs read-only
before launching the fixed driver as UID 65534. The snapshot mount is capped at
384 MiB, alongside the existing 128 MiB scratch mount. These are filesystem
ceilings, not additional allocated host memory: the selected guest RAM limit and
shared worker reservation still apply. Large trees or Git processing can exhaust
an undersized guest; failure cannot return a successful empty query.

A guest receipt binds the admitted manifest to the observed read-only filesystem
flag and size limit. The host requires that receipt and confirms VM cleanup
before exposing it as `snapshot_transfer` in the signed result. An interrupted
transfer or a missing/mismatched receipt cannot report completion. Host copies
remain retained by the detached runtime owner during transfer and cleanup; a
killed runtime leaves copies safe for the staging reaper because the VM has no
access to those host paths.

Deploy matching protocol-5 runtime and guest builds. Older fingerprints are
refused. External worktree metadata grants and access across a root-managed
supervisor identity remain separate work; those cases continue to fail closed.

## Validation

The focused tests verify operation-specific input selection, exact snapshots,
external metadata/hard-link refusal, literal names, symlink handling, overlapping
mounts, empty configuration and size limits. The shipping fixture uses synthetic
repositories, a cached Git/Python image, a local scripted provider and an observer
outside every worker mount. It independently verifies signed journals and the
actual pre-start snapshot hashes, useful query results, hostile configuration,
SHA-256 repositories, changed source and crash cleanup:

```bash
python3 scripts/test-git-source.py --binary target/debug/symbi \
  --image symbi-managed-real-e2e:local --report /tmp/git-source.json
```

The same fixture accepts `--firecracker-binary PATH --kernel PATH --rootfs PATH`.
A test-only host launcher hashes the actual private tree before guest payload
release. The observer then records the real VMM PID, executable and process
start time, requires a single read-only root image and no network interface, and
compares the prepared manifest with the guest receipt and independently verified
signed result. Snapshot mutation, shared staging exhaustion and hard-kill cases
retain their protected host observations. Use repeated `--case NAME` options for
focused regressions.
