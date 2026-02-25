# Unified Symbi Container - DSL and Runtime
# Multi-stage build with cargo-chef for deterministic dependency caching
FROM rust:1.88-slim-bookworm AS chef

# Install build dependencies
RUN apt-get update && apt-get install -y \
    build-essential \
    pkg-config \
    libssl-dev \
    git \
    ca-certificates \
    clang \
    mold \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

RUN cargo install cargo-chef
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
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
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

# Expose ports: 8080 (gRPC), 8081 (HTTP API/webhooks)
EXPOSE 8080 8081

# Health check: verify HTTP port 8081 is listening (no curl/wget in image)
HEALTHCHECK --interval=30s --timeout=10s --start-period=10s --retries=3 \
    CMD grep -q ':1F91 ' /proc/net/tcp 2>/dev/null || grep -q ':1F91 ' /proc/net/tcp6 2>/dev/null || exit 1

# Default entrypoint is the unified symbi binary
ENTRYPOINT ["/usr/local/bin/symbi"]
CMD ["--help"]

# Labels for metadata
LABEL org.opencontainers.image.title="Symbi" \
      org.opencontainers.image.description="Unified DSL and Runtime for AI-native programming" \
      org.opencontainers.image.vendor="ThirdKey.ai" \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.source="https://github.com/thirdkeyai/symbiont"
