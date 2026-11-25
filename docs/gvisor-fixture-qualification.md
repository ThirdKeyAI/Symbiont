# Container fixture selection

The filesystem-grant, source-broker and Git-source shipping fixture drivers accept
`--gvisor` to select the actual gVisor wrapper. The default remains the Docker
profile. `--gvisor`, `--landlock` and `--firecracker-binary` are mutually exclusive;
VM kernel/rootfs arguments must accompany the complete Firecracker selection.

Stage an independently authenticated runtime binary, runsc binary and cached
fixture image first. The image needs Python and Git. The scripts do not pull an
image or configure the daemon. Keep the Docker socket with the trusted test
operator; it conveys host-level authority and never belongs in a worker.

Authenticate the complete installed gVisor distribution. Current releases also
execute sidecar files from the adjacent `gvisor-bin/` directory; see the
[upstream installation guide](https://gvisor.dev/docs/user_guide/install/).
The fixture accepts regular files and directories in that tree, and refuses
links and special files. A legacy single-binary layout is recorded explicitly
when no sidecar directory is present; this does not select or approve a version.

The gVisor path writes `[sandbox.gvisor]` with an explicit `runsc` runtime name
and canonical binary path, and puts all container settings under
`[sandbox.gvisor.docker]`. It does not inherit the Docker profile's mounts.
The local daemon must register that same absolute runsc binary path. Its
configuration and observed workers must agree; changing only Docker's default
runtime is not equivalent to selecting Symbiont's gVisor wrapper.

In a disposable qualification guest, run each complete driver with a new report:

```sh
python3 scripts/test-filesystem-grants.py --gvisor \
  --runsc-binary "$VERIFIED_RUNSC_BINARY" --binary "$VERIFIED_SYMBI_BINARY" \
  --image "$VERIFIED_FIXTURE_REFERENCE_WITH_DIGEST" --report "$NEW_FILE_REPORT"
python3 scripts/test-source-broker.py --gvisor \
  --runsc-binary "$VERIFIED_RUNSC_BINARY" --binary "$VERIFIED_SYMBI_BINARY" \
  --image "$VERIFIED_FIXTURE_REFERENCE_WITH_DIGEST" --report "$NEW_SOURCE_REPORT"
python3 scripts/test-git-source.py --gvisor \
  --runsc-binary "$VERIFIED_RUNSC_BINARY" --binary "$VERIFIED_SYMBI_BINARY" \
  --image "$VERIFIED_FIXTURE_REFERENCE_WITH_DIGEST" --report "$NEW_GIT_REPORT"
```

Retained reports identify the selected backend and helper hash. gVisor reports
also bind the selected runsc file hash and registered path. Actual container
observations record their runtime and reject a gVisor worker using any runtime
other than `runsc`. The artifact record also includes file permissions and the
complete sidecar inventory with relative paths and file hashes. Checkpoints
between cases and before a passing result reject changed, added or removed
sidecars, changed permissions, and changed registered paths. These snapshots
detect changes at the checkpoints; they do not authenticate the installation
or establish security against a compromised host operator.
The filesystem driver verifies artifacts in each case's cleanup checkpoint,
before marking that case passed. Its regression changes artifacts at this
checkpoint and restores them before completion, so completion-only verification
cannot satisfy the test.
The completion checkpoint rereads Docker's current runsc registration as well as
the selected files, so a removed or changed registration cannot retain a passing
result merely because the previously selected executable is still on disk.
Existing checks for file grants, protected observations, journal verification,
worker origins and cleanup still apply. A shortened `--case` run is diagnostic
only and cannot replace a complete qualification run.

`python3 scripts/test_container_fixture_profile.py -v` checks configuration,
selector conflicts, runtime bundle changes and rejection of a mismatched observed
runtime using temporary fixtures. It does not execute or qualify Docker/runsc.

These drivers cover CLI tool-worker filesystem, source and Git routes. They do
not establish managed-CLI tool/inference socket compatibility, a complete escape
suite, Fedora deployment qualification or resistance after daemon/host compromise.
Those gates require execution and retained evidence on the actual platform.
Do not publish a whole fixture directory: it contains synthetic private signing
keys. Retain only the reviewed public reports and the necessary sanitized logs.
