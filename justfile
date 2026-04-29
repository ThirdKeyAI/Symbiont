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
    cargo audit

deny:
    cargo deny check

geiger:
    cargo geiger
