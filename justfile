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
    cargo test --all-features

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
