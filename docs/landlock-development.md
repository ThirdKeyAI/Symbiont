# Governed development on a Linux desktop

Landlock supports local development without Docker or a VM. The kernel restricts
file access, IPC and signals; the independent supervisor controls worker CPU,
memory, process count and lifetime. Native command workspaces additionally use
unprivileged user and mount namespaces. Managed CLI workers also require an
unprivileged network namespace containing only private loopback.

Use Linux with Landlock ABI 6+, supported seccomp, cgroup v2 delegation and a
systemd user manager. The runtime and supervisor must have the same non-root UID.
See [supervision](landlock-supervision.md) for requirements and recovery. Namespace
creation must be permitted by the distribution's security policy. Missing
capabilities fail the operation; no unrestricted host fallback is attempted.

## Create the control project

For a first repository review, use the native development profile. Keep its new,
empty control directory separate from the source repository. On an interactive
terminal this command asks for the repository, installed Claude Code executable,
inference base URL, model and credential environment variable name:

```sh
symbi init --sandbox landlock --profile dev-agent --dir ./control
```

For scripts or noninteractive terminals, supply all five settings explicitly:

```sh
symbi init --sandbox landlock --profile dev-agent --dir ./control \
  --source /home/developer/project \
  --managed-executable /home/developer/.local/bin/claude \
  --inference-url http://127.0.0.1:8000 \
  --inference-model your-configured-model \
  --inference-key-env REVIEW_PROVIDER_KEY
cd control
symbi doctor
symbi run dev --target /home/developer/project \
  --input 'Review this repository using the source tools. Report findings without changing files.'
```

Set `REVIEW_PROVIDER_KEY` in your shell or the control project's private `.env`
before the review. The flag accepts its variable **name**, never the secret.
The endpoint must support the **Anthropic Messages API** used by managed Claude
Code; an OpenAI-only endpoint is insufficient. Initialization and `doctor` make
no provider requests. The review sends authorized tool results to the selected
provider and requires a working model and credential.

The generated project includes `read_file`, `list_files` and `grep_files`
manifests, matching managed CLI Cedar policies, an exact tool allowlist, a
read-only source ceiling and no output roots. It grants no writes, command tools
or shell policies. `DEVELOPMENT.md` and the final terminal output contain the
commands with your resolved source path. `.env.example` names the chosen
credential variable; no credential value is collected or generated for it.

Settings, root separation, executable permissions and basic kernel requirements
are validated before project files are written. Missing flags, overlapping
directories (including symlink aliases), ambiguous source paths and invalid
inference settings fail with guidance. The development profile requires an empty
control directory even with `--force`, so existing policies and tools cannot be
silently combined with its read-only setup. Catalog imports are separate from
this initialization. Other profiles keep their existing behavior.

For a custom configuration instead, begin with the general profile and add the
settings described below:

```sh
symbi init --sandbox landlock --profile assistant --dir ./control
cd control
symbi doctor
```

`doctor` starts the automatic delegated user service and confirms cleanup after
each check: the basic restricted launch, a writable private native workspace,
and a separate network namespace with working loopback and both inherited
connections. These diagnostics require Python 3 at `/usr/bin/python3`. No service
unit needs to be written by hand.

When `[managed_cli]` is present, `doctor` also resolves the same executable as
`symbi run` and runs only `--version` through the private adapters. Without that
section it explicitly reports CLI startup as unchecked. The startup check uses
no inherited caller environment, private scratch, the existing executable/metadata
grants, and diagnostic sockets with no tool dispatcher or inference provider.
It needs no provider credentials. Native probes have a 10-second worker limit
(or a shorter configured deadline) and at most 4 KiB per output stream. Errors
name the failed stage and suggest namespace, Python, or executable setup fixes.
A passing startup check does not validate model access, tool policies or every
CLI feature.

Add narrowly selected ceilings to `symbiont.toml`, replacing the absolute paths:

```toml
[sandbox.roots]
source_roots = ["/home/developer/project:/workspace:ro"]
output_roots = ["/home/developer/project:/workspace:rw"]
```

Source roots authorize reads; output roots separately authorize writes. Omit
output roots for review-only use. `/workspace` is the logical file-broker path,
not a host mount handed to every worker. Never expose the control project's
agents, policies, tools, journal keys or supervisor state through these roots.

## Read, edit, run and inspect

The registered `read_file`, `list_files` and `grep_files` tools return bounded
results from retained source handles. They refuse traversal, symlinks and private
control paths. In `symbi-shell`, fixed read/search/edit/save operations use the
same ceilings. An edit requires exact approval in the Gate panel; its contract
binds the destination, new content and previous content. `--yes` does not approve
file writes. Denial or cancellation leaves the file unchanged.

Arbitrary command tools receive `/tmp/symbi-workspace`, a private tmpfs with its
own HOME and TMPDIR. Its size is half the worker memory limit, clamped between
1 MiB and 128 MiB, with at most 4,096 inodes. Memory remains charged to the worker
cgroup. Commands receive no original source directories. Declare individual
inputs and new outputs in the tool manifest:

```toml
[filesystem]
read = ["{input}"]
create = ["{output}"]
max_file_bytes = 1048576
```

Only the input snapshots and reserved output files are bound into that workspace.
Inputs are read-only; bound files cannot be replaced by unlinking their paths. Backend IP access remains denied by default; an explicit `require_network = false` allows IP sockets for those backend workers. Managed CLI always keeps its private network namespace.
File-size limits apply to descendants. A successful output is published without
replacing an existing destination, after whole-cgroup cleanup is confirmed.
Failure, timeout or uncertain cleanup does not publish it. Custom parsers receive
only a fresh workspace and the captured tool result. See
[file grants](filesystem-grants.md) for manifest limits.

The registered Git tools use bounded private repository snapshots, fixed Git
configuration and denied networking. Hooks, includes, external object stores and
source-supplied helpers cannot expand their authority. Snapshots consume the
shared staging budget and remain charged while a durable worker lease references
them, including after runtime process loss. See [Git queries](git-source-queries.md).

Inspect the printed signed journal with `symbi audit inspect`. An incomplete or
uncertain run requires reconciliation; cleanup alone does not authorize replay.

## Managed CLI

Install Python 3 and the CLI toolchain on the host. For a standalone user-local
executable, configure its absolute path:

```toml
[managed_cli]
executable = "/home/developer/.local/bin/claude"

[managed_cli.inference]
base_url = "http://127.0.0.1:8000"
model = "your-configured-model"
api_key_env = "REVIEW_PROVIDER_KEY"
max_requests = 32
max_output_tokens_per_request = 4096
request_timeout_seconds = 60
```

The launcher resolves executable symlinks and grants read/execute access only to
the canonical regular file. It does not grant its containing home directory.
The executable must be outside `/tmp`, which the workspace shadows. Without an
explicit setting, the launcher looks for `claude` under `/usr/local/bin`,
`/usr/bin` or `/bin`. System runtime libraries remain readable; packages requiring
additional files in a user home are not implicitly granted those directories.
The selected path is recorded in the admission contract. Operator-installed
executables and system libraries remain trusted deployment inputs.

Choose a conversational agent with `executor = "claude_code"` and an exact
`allowed_tools` list. Configure Cedar admission and tool policy as described in
[managed CLI containment](managed-cli-containment.md), then run:

```sh
symbi run reviewer --target /home/developer/project --input 'Review the changes'
```

The CLI sees private scratch space and receives only governed tool results. It
cannot directly read the selected source repository. Built-in tools and automatic
plugin discovery remain disabled. Two inherited connected sockets carry tool and
inference requests to the runtime. The adapter keeps those descriptors and
provides loopback endpoints inside the worker's network namespace; subprocesses
do not receive the host sockets. Direct host TCP, internet and Unix-socket access
remain denied. Provider credentials, policy and approval state stay in the runtime.
The private MCP relay accepts one bridge connection per managed session.

For standalone runtime compatibility, the launcher grants read-only access to
exactly the primary process's `/proc/self/maps` and `/proc/self/stat` files.
These describe its loaded image and resource usage. The rules pin those process
inodes before exec; they do not grant access to host processes or create new
metadata grants when the worker forks. The CLI replaces the launch process,
while a private adapter child owns the broker connections and watches a pidfd
for CLI exit. The supervisor still owns whole-worker cleanup. Cgroup files,
other `/proc` files, `/sys` and writable kernel controls remain denied.
The admission contract records these metadata paths and boundary version 5.

A read-only tool allowlist supports review. Generating files requires separately
registered commands with declared new outputs; existing-file editing through the
shell uses its exact approval workflow. Selecting Landlock does not implicitly
register additional tools or grant writes.

## Focused verification

The shipping drivers use synthetic sources, local providers and disposable user
services. They require the actual kernel/service features and fail if absent:

```sh
python3 scripts/test-landlock-onboarding.py --binary /absolute/path/symbi --report /absolute/path/onboarding.json
python3 scripts/test-filesystem-grants.py --landlock --binary /absolute/path/symbi --report /absolute/path/files.json
python3 scripts/test-source-broker.py --landlock --binary /absolute/path/symbi --report /absolute/path/source.json
python3 scripts/test-git-source.py --landlock --case log --case diff --case snapshot --case capacity --case crash --binary /absolute/path/symbi --report /absolute/path/git.json
python3 scripts/test-landlock-managed.py --binary /absolute/path/symbi --report /absolute/path/managed.json
```

Add `--development --install-dir /absolute/new/install-directory` to the onboarding driver to copy the executable into a fresh installation, scaffold a project, test a declared Python source file and publish its result after cleanup. Use new report paths for each run. The managed driver uses a synthetic CLI to
verify the shipping launch, adapters, brokers and policy contracts; it does not
claim compatibility with every third-party CLI release. A separate real-toolchain
check is needed for the installed version.

Claude Code 2.1.274 has also passed the shipping managed route with a synthetic
local provider: MCP discovery, an authorized source read, a Cedar-denied read,
streamed inference, verified signed journals and confirmed cleanup. Reproduce
that check without account credentials or external model calls:

```sh
python3 scripts/test-landlock-managed.py --binary /absolute/path/symbi --executable /absolute/path/claude --report /absolute/new/real-managed.json
```

Add `--managed-executable /absolute/path/claude` to the onboarding driver to also
exercise the configured CLI startup diagnostic on an installed toolchain. The
onboarding cases cover blocked workspace/network namespaces, startup errors and
hung-CLI descendant cleanup.

The managed-run report records both executable hashes. This verifies the installed CLI and
broker protocol; it does not assess model quality or external provider availability.
Other CLI releases and subprocess toolchains can have additional requirements.

Test the generated development profile, including prompts, flags, source scope,
tool denials, signed journals and complete cleanup, with:

```sh
python3 scripts/test-landlock-managed.py --onboarding --binary /absolute/path/symbi --report /absolute/new/developer.json
```

The default synthetic CLI also attempts direct source writes, unregistered write
and command tools, traversal and symlink escapes. Add `--executable /absolute/path/claude`
to verify the printed review command with the installed CLI and a synthetic local
provider. Neither mode uses account credentials or an external model service.

Interactive PTY tools are not yet available on the native workspace route. This implementation targets Linux;
macOS and Windows native backends remain outside this workflow.
