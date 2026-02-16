# Symbiont — Agent Instructions

Symbiont (Symbi) is a Rust-native, zero-trust agent framework for building autonomous, policy-aware AI agents. Part of the [ThirdKey](https://thirdkey.ai) trust stack: [SchemaPin](https://schemapin.org) → [AgentPin](https://agentpin.org) → **Symbiont**.

- **Docs**: https://docs.symbiont.dev
- **Repo**: https://github.com/ThirdKeyAI/Symbiont
- **Crate**: https://crates.io/crates/symbi

## Project Structure

```
crates/
├── dsl/              # Symbi DSL parser with Tree-sitter integration
├── runtime/          # Agent runtime (scheduling, routing, sandbox, AgentPin)
├── channel-adapter/  # Slack, Teams, Mattermost adapters
├── repl-core/        # Core REPL engine
├── repl-proto/       # JSON-RPC wire protocol types
├── repl-cli/         # Command-line REPL interface
├── repl-lsp/         # Language Server Protocol implementation
src/                  # Unified `symbi` CLI binary
```

## Build and Test

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace
cargo fmt --check
```

All four commands must pass before committing. Clippy must produce zero warnings.

## Code Style

- Rust edition 2021
- Run `cargo fmt` before committing
- Run `cargo clippy --workspace` and fix all warnings before committing
- Inline tests in source files using `#[cfg(test)] mod tests`
- ES256 (ECDSA P-256) only for AgentPin identity — reject all other algorithms

## Commit Guidelines

- Write concise commit messages focused on the "why"
- No mention of AI assistants or co-authoring in commit messages
- Use `date` command to determine the current date when adding dates to docs

## Security

- Zero-trust by default: all inputs are untrusted
- Cryptographic audit trails for agent actions
- Policy engine enforces runtime constraints via the Symbi DSL
- AgentPin integration for domain-anchored agent identity
- SchemaPin integration for tool schema verification
- Private keys (`*.private.pem`, `*.private.jwk.json`) must never be committed

## Docker

- Image: `ghcr.io/thirdkeyai/symbi:latest`
- Base: `rust:1.88-slim-bookworm` (builder), `debian:bookworm-slim` (runtime)
- The Dockerfile uses dependency caching with stub sources; cleanup globs must catch `libsymbi*` and `.fingerprint/symbi*`

## Releasing

See `.claude/RELEASE_RUNBOOK.md` for the full release process, including:

- How to determine which crates need version bumps
- Cross-crate version reference update checklist
- CI verification steps before tagging
- Docker build cache pitfalls
- crates.io publish order

## OSS Sync

Private repo is on Gitea. Public mirror is `github.com:ThirdKeyAI/Symbiont.git`.

```bash
bash scripts/sync_oss_to_github.sh --force
```

The script exits with code 1 during cleanup even on success — this is a known quirk.
