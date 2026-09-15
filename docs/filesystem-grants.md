# Per-operation filesystem grants

ToolClad commands, MCP calls and persistent terminals on Linux Docker/gVisor
and Firecracker accept individual file grants. Configured host roots define
the maximum available access. A mount in `symbiont.toml` no longer automatically
appears in every one-shot worker. Declare the files needed by each invocation:

```toml
[args.input]
position = 1
required = true
type = "path"

[args.output]
position = 2
required = true
type = "path"

[filesystem]
read = ["{input}"]
create = ["{output}"]
max_file_bytes = 8388608
```

Paths are relative to the configured worker working directory. Each entry is a
literal path such as `data/input.csv`, or an entire argument reference such as
`{input}`. Concatenated templates, rooted paths, traversal, empty components,
backslashes and mount delimiters are refused. Inputs must be regular files; links,
directories and special files are refused. Existing parent directories are
required. A read-only mount ceiling cannot authorize output creation.

There may be at most 32 inputs, 32 MiB of input snapshots in total and one new
output per operation. The default per-file bound is 8 MiB, configurable from one
byte to 16 MiB. Larger file sets and directory grants are not supported by this
broker. Tools without a filesystem declaration receive no host mounts on these
one-shot routes; they retain their image and private bounded scratch space.
Persistent terminals use the same bounded file sets across their session and
require explicit finalization to publish output; see below.

## Execution and authorization

Preparation reads bounded input snapshots through file handles, rejecting links
at every path component. It binds the input hashes, byte counts, relative paths
and output creation path into the immutable prepared call. Cedar and exact-call
approval see these at `context.invocation.resolved.command_boundary.filesystem`.
Changing an input after preparation does not replace the authorized snapshot.
Input bytes are not included in this descriptor.
The descriptor also binds the device/inode identity of the held output parent
directory, so replacing a directory cannot silently reuse its earlier grant.

After authorization, the runtime stages private copies. Docker/gVisor mounts only the
declared files. Firecracker transfers their bounded bytes into guest scratch. Inputs are read-only. The output is a separate writable
staging file; the worker does not receive its host destination directory. An OS
file-size hard limit bounds writes to that file. General-purpose programs can
read their declared inputs and image contents, but cannot inspect adjacent host
files through the former directory mount. Files they create in worker scratch
space are not published to the host.

After a successful operation, required output parsing and confirmed worker cleanup, the
runtime publishes the output through the held destination-directory handle. The
rename is atomic and refuses any existing file, symlink or directory. Concurrent
calls targeting the same name cannot overwrite each other. Failed commands and
parsers, MCP errors and failed schema verification do not publish output. Successful envelopes include `created_files`
receipts with relative paths, byte counts and SHA-256 hashes.

Before the rename, the broker syncs a private candidate and a required signed
`FilePublicationPrepared` record binding its inode, parent identity, size, hash
and exact destination. A signed `FilePublicationFinished` follows confirmed
publication. Lost acknowledgement or a crash retains the candidate or published
file for [operator recovery](file-publication-recovery.md). Workers cannot write
these records or access the private candidate directory.

Custom output parsers receive the bounded raw output in a fresh worker without
host mounts. Provision their executable in the selected image/rootfs and retain
the explicit parser allowlist. A parser cannot reuse the main command's file
grants or a host-mounted parser script.

## MCP workers

Use the same `[filesystem]` block alongside `[mcp]`. Argument references use local
manifest names, before `[mcp.field_map]` translates them to upstream names. Each
invocation receives its own snapshots and staging file, including during server
startup, discovery and SchemaPin verification. Those steps share one worker and
one connection with the tool call. Startup writes remain private staging data;
an unsigned schema, protocol failure, `isError` response, cancellation or timeout
cannot publish them. A successful MCP result is returned with `created_files`
after confirmed worker cleanup and output publication.

Standalone SDK discovery and `verified_invoke_with_boundary` receive no host
mounts. SDK integrations requiring files prepare a `FileAccessPlan` against their
trusted boundary and call `verified_invoke_with_files` for declared inputs.
Publishing outputs requires the live audit authority supplied by governed
ToolClad dispatch; a standalone SDK output plan without it is refused before
starting the worker. A
successful governed result contains `content` and `created_files`, using the plan
frozen before authorization. Discovery cannot load a server executable from a host
mount: provision executable dependencies in the image, or pass explicitly
configured code as launch arguments. Files granted to a call are available to
all code in that worker, including its startup code; schema verification does
not confine access to one function inside the server.

## Fixed source queries

The bundled `read_file`, `list_files` and `grep_files` tools now use an explicit
`[source]` backend. The trusted runtime returns bounded snapshots or metadata;
no general-purpose program receives the source directory. See
[source queries](source-queries.md) for arguments, evidence and limits, and
[Git source queries](git-source-queries.md) for isolated repository snapshots
on Docker, gVisor and Firecracker.

## Firecracker file transfer

Configure read and new-output ceilings separately:

```toml
[sandbox.firecracker]
working_dir = "/tmp"
source_roots = ["/srv/project/input:/tmp/input:ro"]
output_roots = ["/srv/project/results:/tmp/results:rw"]
```

Declare relative files such as `input/data.csv` and `results/report.json` in the
manifest. The host output parent must already exist. A write ceiling permits
creation only, never reading or overwriting existing host files. Each group
accepts at most 32 roots with distinct virtual destinations. Neither group is a
guest mount or an automatic import. Fixed source queries continue using only the
read ceilings.

Protocol 5 carries lengths and SHA-256 hashes, then bounded raw bytes separately
from command arguments. Guest paths must be canonical nonreserved `/tmp/...`
paths. PID 1 creates immutable inputs and retains the exact writable output inode.
Root-owned parent directories and sticky `/tmp` prevent the workload from replacing
these files. The workload runs as UID 65534 with the declared file-size limit.
Before export, PID 1 kills and reaps descendants, checks the retained inode and
path, and hashes the bounded output. The runtime verifies those bytes into private
staging. Publication additionally requires the operation's semantic success,
required parsing, confirmed VM removal and the existing signed publication intent.

Persistent MCP servers and terminals receive a separate finalization control
frame when the successful operation closes its guard. This permits stopping the
server and exporting its declared file without treating SIGKILL as a natural
successful exit. The host allows up to two seconds within the remaining lifetime
for that acknowledgement and transfer. Lost acknowledgement, cancellation, guard
drop, nonzero natural exit or failed cleanup cannot publish. Rebuild the guest
service and rootfs with the runtime; older versions or fingerprints are refused.

Current staging requires access to the selected supervisor's private state.
Cross-identity root-managed staging is not implemented and fails closed; use
this file-transfer path only where the runtime legitimately owns that state.
Git repository queries use a separate
[bounded snapshot transfer](git-source-queries.md#firecracker-snapshots), which
does not increase ordinary file-grant limits.

## Migration and scope

Move required host input and output paths into `[filesystem]` and keep the
operator mount configuration as their ceiling. Replace in-place modification
with an explicitly named new output. Provision executables in the image; do not
rely on mounting a host script into a one-shot worker. Observe active workers via
the supervisor or container state rather than requiring a host marker during
execution: outputs are published only after successful cleanup.

Persistent PTY tools also use these file grants. Their manifest must declare a
command with `finalize = true` when output is requested. That command returns its
ready prompt, closes the worker and publishes the new output after confirmed
cleanup. Other commands report pending publication and retain private session
state. Closing without finalization fails explicitly and discards staging.
Changed input snapshots or output parent identities require a new run. See
[terminal file grants](interactive-terminal-boundary.md#session-file-grants-and-finalization).

Explicit `[filesystem]` declarations on HTTP, browser and
development-host routes are refused. The shell's fixed file tools use a separate
[bounded file broker](shell-containment.md), including exact approved updates to
existing files. Its arbitrary commands receive no host mounts. Other runtime
file capabilities still need review.
Development host execution provides no filesystem isolation.

An interrupted publication can leave an uncertain outcome or a private temporary
file. For an inactive invocation with a signed publication intent, the operator
can inspect and finish that exact publication without replaying the tool. Missing
or conflicting candidates and files created before a durable intent still require
investigation. This does not establish exactly-once effects for arbitrary services.

The ToolClad Rust, Python, JavaScript and Go reference runners retain filesystem
declarations for previews and refuse actual execution because they do not provide
the embedding file broker. A preview is not enforcement or host-path validation.

## Verification

`scripts/test-filesystem-grants.py` runs the shipping CLI with real Docker workers,
a synthetic provider and protected host observations. All three file fixtures
accept `--firecracker-binary PATH --kernel PATH --rootfs PATH` to exercise real
VMs using the same cases. VM observations bind the actual process identity and
configuration, require one read-only root drive and no network interface, and
verify worker, lease and staging cleanup. The rootfs must contain Python 3. It checks useful file work,
adjacent-file isolation, absent declarations, parser isolation, OS file-size
limits, existing-output refusal and input-symlink refusal. It inspects actual
mounts and verifies signed journals. Focused broker tests additionally exercise
snapshot stability, hard links, FIFOs and competing output publication.

`scripts/test-mcp-filesystem.py` exercises the shipping CLI with a signed MCP
schema, actual Docker mounts, protected journals and a local scripted provider.
It checks useful file work, snapshot stability, absent declarations, failed
verification after startup writes, errors, deadlines, file-size limits and
competing output creation. SDK Docker tests additionally cover discovery without
mounts and cancellation of detached descendants.

`scripts/test-file-publication.py` kills the shipping CLI before and after the
atomic rename, verifies original and recovery signatures independently, checks
same-ID retry refusal and confirms competing outputs cannot be overwritten.

## Shared private staging

Command, MCP and persistent terminal file copies use the selected supervisor's
shared staging pool. Inputs and maximum output size are reserved before copying;
capacity refusal starts no worker. Live callers and durable container-worker references
retain the reservation until private data removal is confirmed. A VM receives
byte copies, so its detached runtime owner retains the host staging lease through
transfer and cleanup; after runtime death those copies can be reaped without
waiting for a VM that cannot access them. Guest RAM remains separately charged. Publication
candidates keep their separate signed recovery lifecycle. See
[staging capacity](staging-capacity.md) for limits and crash cleanup.
