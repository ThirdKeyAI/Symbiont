default:
    @just --list

# Pre-release health sweep — must pass before tagging or publishing.
# `geiger` is intentionally not in this list: cargo-geiger 0.13 panics
# against cargo ≥0.86 (upstream incompat). Run `just geiger` manually
# when you want the unsafe-surface report; re-add to `check` once the
# upstream fix lands.
check: fmt clippy test machete audit deny

fmt:
    cargo fmt --check

clippy:
    cargo clippy --all-targets --all-features -- -D warnings

test:
    # --workspace matters: without it this runs only the root `symbi`
    # package's tests, so every crate under crates/ goes unverified
    # while the sweep still reports success.
    #
    # Not --all-features: that pulls the embedding-model and vector-backend
    # suites, which download large model/data files and link test binaries fat
    # enough to exhaust RAM during `ld`. CI skips them for the same reason and
    # leans on `just clippy` (--all-targets --all-features) to compile every
    # feature-gated path, plus its own targeted per-feature test steps.
    cargo test --workspace

machete:
    cargo machete

audit:
    # RUSTSEC-2023-0071 (rsa Marvin Attack via jsonwebtoken) is runtime-mitigated:
    # the JWT verifier enforces an ES256/EdDSA/HS256 algorithm allowlist and refuses
    # RS/PS algorithms before the rsa crate's timing-side-channel path is reachable.
    # See crates/runtime/src/http_input/webhook_verify.rs and deny.toml.
    cargo audit --ignore RUSTSEC-2023-0071

deny:
    cargo deny check

geiger:
    cargo geiger
