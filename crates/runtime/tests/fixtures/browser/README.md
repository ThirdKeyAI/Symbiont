# Contained Chromium transport fixtures

These ignored runtime tests exercise the private CDP pipe through the
existing `CommandBoundary` stdio transport and independent sandbox supervisor.
They are prerequisites for a browser backend; the public ToolClad browser
executor still returns an unavailable error. They do not establish policy,
approval or production run ownership for browser commands.

Provision a local Linux x86_64 Chromium **headless-shell** directory as `chrome/`
in a separate Docker build context, retaining its adjacent libraries and data.
Use an operator-selected, trusted distribution. The Dockerfile installs system
libraries during provisioning; the tests never pull images or download browsers.

From the repository root, with that build context at `/tmp/symbi-browser-image`:

```sh
docker build --pull=false -t symbi-browser-e2e:local \
  -f crates/runtime/tests/fixtures/browser/Dockerfile /tmp/symbi-browser-image
export SYMBI_BROWSER_TEST_IMAGE="$(docker image inspect symbi-browser-e2e:local --format '{{.Id}}')"
cargo build -p symbi-sandbox-supervisor
export SYMBIONT_SANDBOX_SUPERVISOR="$PWD/target/debug/symbi-sandbox-supervisor"
cargo test -p symbi-runtime --features mcp-client --lib \
  toolclad::browser_executor::transport_tests:: -- --ignored --test-threads=1 --nocapture
```

Adjust the supervisor path if using a separate Cargo target directory. The tests
require an exact local `sha256:` image ID; a missing browser, image, Docker daemon
or supervisor is a failure, not a skipped success. Retain the image ID, source and
test-binary hashes, and test logs with the reported browser version for each run.

The navigation fixture creates one network-disabled, non-root container with a
read-only rootfs, dropped capabilities and private temporary storage. It checks
the actual Docker configuration and communicates with Chromium through inherited
descriptors; no TCP debugging listener or host browser is used. Chromium's own
namespace sandbox is disabled inside this outer worker. Its private profile is
removed with the worker.

The fixture fulfills one synthetic HTML response through CDP, fails an unapproved
image request, clicks a page button and reads the resulting DOM state. It bounds
CDP frames, total bytes, pending commands, page requests and execution time.
The second test retains a live browser handle until the independent supervisor
deadline removes the worker. Both checks require acknowledged cleanup and verify
that their uniquely labeled containers are gone. Other containers are untouched.

The additional `broker_tests::contained_browser_brokers_audited_http_and_blocks_redirect_egress`
case uses a real local HTTP server through the reusable browser request policy
and shared audited HTTP transport. Its fixture executor obtains call-bound journal
authority from `GovernedToolDispatcher`. The server verifies the signed request
checkpoint before reading each connection. Chromium loads a page, submits exact
UTF-8 POST bytes, and reads the response into the DOM. A DELETE, an image on a
different local port and a redirect to that port are refused. The denied listener
receives no connection. Request/outcome records match the authorized call and
cleanup is acknowledged. The container itself retains `network=none`.

For only that request-policy integration check, replace the filter above with:

```text
toolclad::browser_executor::transport_tests::broker_tests::
```

The fixture uses a permissive development gate and a trusted local executor;
it does not exercise shipping browser command authorization or approval. It
handles one explicitly attached page. Production popup, iframe and worker target
coverage, persistent browser ownership and the full command surface remain
pending. These fixtures also do not exercise Firecracker or gVisor browser images,
a hostile guest observer or host filesystem canaries.
