# symbi-e2e

End-to-end tests for the Symbiont runtime. Boots real HTTP servers on
loopback ports and drives them with `reqwest`. Not part of the default
test run; gate on the `e2e` feature.

## Running

```
# Just the E2E suite
cargo test -p symbi-e2e --features e2e

# A single test file
cargo test -p symbi-e2e --features e2e --test api_auth_scope

# Default cargo test on the workspace does NOT run these:
cargo test --workspace   # e2e tests are behind the feature gate
```

## Coverage

| Test file | Exercises |
|---|---|
| `api_auth_scope.rs` | `/api/v1/*` authentication + per-agent scope enforcement (H-1 + L-1 end to end) |
| `messaging_ingress.rs` | `/api/v1/agents/:id/messages` body cap, TTL clamp, bad-UUID status |
| `cross_runtime_bus.rs` | Two live runtimes, `RemoteCommunicationBus` A→B, C-1 cross-bus refusal |
| `agentpin_messaging.rs` | AgentPin `agentpin_jwt` gate on `/messages` (AP-1 wire-level) |
| `webhook_signature.rs` | `HttpInputServer` GitHub-style HMAC verify — valid / invalid / missing / tampered |
| `rate_limit.rs` | Per-IP rate limiter trips 429 under a burst |
| `docker_volumes.rs` | `DockerConfig::with_volume` / `validate` refuses dangerous host-path mounts |

Tests that are already covered at the unit level in `crates/runtime`
(ToolClad parser allowlist, SchemaPin SSRF, DSL parallel fan-out cap)
are intentionally NOT duplicated here — see the respective unit tests.
