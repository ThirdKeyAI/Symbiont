default:
    @just --list

# Pre-release health sweep — must pass before tagging or publishing.
check: fmt clippy test machete audit deny geiger

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
