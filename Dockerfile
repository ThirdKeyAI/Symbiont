# Unified Symbi Container - DSL and Runtime
# Multi-stage build with cargo-chef for deterministic dependency caching
# Base images are pinned by digest (verified against Docker Hub on 2026-05-21).
# Update the digest alongside any tag bump.
# Bumped 1.88 -> 1.89: cedar-policy (default since v1.14.2) transitively
# requires smol_str 0.3.5 which mandates rustc 1.89+.
FROM rust:1.89-slim-bookworm@sha256:d7fc7de78bb8c1469933aeecbf801314d30d7d6e9f0578bba4cfa285bfa37fe6 AS chef

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    pkg-config \
    libssl-dev \
    git \
    ca-certificates \
    clang \
    mold \
    protobuf-compiler \
    libprotobuf-dev \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

RUN cargo install cargo-chef --locked
WORKDIR /app

# --- Planner: generate dependency recipe ---
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# --- Builder: cook deps then build app ---
FROM chef AS builder

# Build profile: "ci" (fast) or "release" (optimized)
ARG BUILD_PROFILE=release

# Common env
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true \
    CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse \
    CARGO_INCREMENTAL=0 \
    RUSTC_WRAPPER="" \
    CARGO_PROFILE_RELEASE_STRIP=true

# Use mold linker for faster linking
ENV RUSTFLAGS="-C link-arg=-fuse-ld=mold"

# Profile-specific env written to file (sourced in build RUNs)
RUN if [ "$BUILD_PROFILE" = "ci" ]; then \
      echo "--- CI profile: no LTO, codegen-units=16, opt-level=2 ---"; \
      printf 'export CARGO_PROFILE_RELEASE_LTO=false\nexport CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16\nexport CARGO_PROFILE_RELEASE_OPT_LEVEL=2\n' > /tmp/build-profile.env; \
    else \
      echo "--- Release profile: full LTO, codegen-units=1, opt-level=3 ---"; \
      printf 'export CARGO_PROFILE_RELEASE_LTO=true\nexport CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1\nexport CARGO_PROFILE_RELEASE_OPT_LEVEL=3\n' > /tmp/build-profile.env; \
    fi

# Auto-detect parallelism
RUN mkdir -p .cargo && printf '[net]\ngit-fetch-with-cli = true\n[registries.crates-io]\nprotocol = "sparse"\n[build]\njobs = %d\n' "$(nproc)" > .cargo/config.toml

# Cook dependencies (cached when only source changes)
COPY --from=planner /app/recipe.json recipe.json
# cargo-chef doesn't create stubs for [[example]] entries
RUN mkdir -p examples && echo "fn main() {}" > examples/native-execution-example.rs
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    . /tmp/build-profile.env && \
    cargo chef cook --release --recipe-path recipe.json

# Copy source and build
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    . /tmp/build-profile.env && \
    cargo build --release && \
    cp target/release/symbi /tmp/symbi

# --- Runtime stage - minimal security-hardened image ---
# Base image pinned by digest (verified against Docker Hub on 2026-05-17).
# Update the digest alongside any tag bump.
FROM debian:bookworm-slim@sha256:67b30a61dc87758f0caf819646104f29ecbda97d920aaf5edc834128ac8493d3

# curl is required by the application-level HEALTHCHECK below; ca-certificates
# and libssl3 are required by the runtime binary for TLS.
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Create non-root user with home directory at /var/lib/symbi
# The runtime defaults to $HOME/.symbi/ for storage; containers have no /home/symbi
RUN groupadd -r symbi && useradd -r -g symbi -u 1000 -d /var/lib/symbi symbi

# Create directories for agent data and configuration
RUN mkdir -p /var/lib/symbi /etc/symbi && \
    chown -R symbi:symbi /var/lib/symbi /etc/symbi

# Copy the unified binary from builder
COPY --from=builder /tmp/symbi /usr/local/bin/symbi

# Set proper ownership and permissions
RUN chown symbi:symbi /usr/local/bin/symbi && \
    chmod +x /usr/local/bin/symbi

# Switch to non-root user
USER symbi

# Set HOME so the runtime finds $HOME/.symbi/ at /var/lib/symbi/.symbi/
# Containers have no D-Bus keychain — set SYMBIONT_MASTER_KEY env var for encryption
ENV HOME=/var/lib/symbi

# Set working directory for operations (symbi auto-discovers symbi.toml from CWD)
WORKDIR /var/lib/symbi

# Expose ports: 8080 (Runtime API / MCP), 8081 (HTTP Input / webhooks)
EXPOSE 8080 8081

# Health check: probe the runtime API health endpoint on port 8080.
# Falls back to a socket-listen check on the HTTP Input port (8081, hex 1F91)
# so the probe still passes for deployments that only expose the webhook
# server. The HTTP endpoint catches application-level hangs that a raw
# socket-listen check would miss.
HEALTHCHECK --interval=30s --timeout=10s --start-period=10s --retries=3 \
    CMD curl -fsS --max-time 5 http://127.0.0.1:8080/api/v1/health >/dev/null 2>&1 \
        || grep -q ':1F91 ' /proc/net/tcp 2>/dev/null \
        || grep -q ':1F91 ' /proc/net/tcp6 2>/dev/null \
        || exit 1

# Default entrypoint is the unified symbi binary
ENTRYPOINT ["/usr/local/bin/symbi"]
CMD ["--help"]

# Labels for metadata
LABEL org.opencontainers.image.title="Symbi" \
      org.opencontainers.image.description="Unified DSL and Runtime for AI-native programming" \
      org.opencontainers.image.vendor="ThirdKey.ai" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.source="https://github.com/thirdkeyai/symbiont"
