# Unified Symbi Container - DSL and Runtime
# Multi-stage build for optimal performance and security
FROM rust:1.88-slim-bookworm AS builder

# Install build dependencies with parallel processing
RUN apt-get update && apt-get install -y \
    build-essential \
    pkg-config \
    libssl-dev \
    git \
    ca-certificates \
    clang \
    mold \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Set environment variables for faster compilation
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true \
    CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse \
    CARGO_INCREMENTAL=0 \
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 \
    CARGO_PROFILE_RELEASE_LTO=true \
    CARGO_PROFILE_RELEASE_OPT_LEVEL=3 \
    RUSTC_WRAPPER="" \
    CARGO_PROFILE_RELEASE_STRIP=true

# Use mold linker for faster linking
ENV RUSTFLAGS="-C link-arg=-fuse-ld=mold"

WORKDIR /app

# Create cargo configuration for optimized builds
RUN mkdir -p .cargo && echo '[net]\ngit-fetch-with-cli = true\n[registries.crates-io]\nprotocol = "sparse"\n[build]\njobs = 4' > .cargo/config.toml

# Copy workspace configuration files first for better dependency caching
COPY Cargo.toml Cargo.lock ./
COPY crates/dsl/Cargo.toml ./crates/dsl/
COPY crates/runtime/Cargo.toml ./crates/runtime/
COPY crates/channel-adapter/Cargo.toml ./crates/channel-adapter/
COPY crates/repl-core/Cargo.toml ./crates/repl-core/
COPY crates/repl-proto/Cargo.toml ./crates/repl-proto/
COPY crates/repl-cli/Cargo.toml ./crates/repl-cli/
COPY crates/repl-lsp/Cargo.toml ./crates/repl-lsp/

# Create dummy source files to cache dependencies
RUN mkdir -p src crates/dsl/src crates/runtime/src/bin \
    crates/channel-adapter/src \
    crates/repl-core/src crates/repl-proto/src \
    crates/repl-cli/src crates/repl-lsp/src \
    examples && \
    echo "fn main() {}" > src/main.rs && \
    echo "fn main() {}" > examples/native-execution-example.rs && \
    echo "fn main() {}" > crates/dsl/src/main.rs && \
    echo "" > crates/dsl/src/lib.rs && \
    echo "fn main() {}" > crates/runtime/src/bin/symbiont_mcp.rs && \
    echo "" > crates/runtime/src/lib.rs && \
    echo "" > crates/channel-adapter/src/lib.rs && \
    echo "" > crates/repl-core/src/lib.rs && \
    echo "" > crates/repl-proto/src/lib.rs && \
    echo "fn main() {}" > crates/repl-cli/src/main.rs && \
    echo "fn main() {}" > crates/repl-lsp/src/main.rs

# Build dependencies only with optimized settings (cached layer)
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release && \
    rm -rf target/release/deps/symbi* target/release/deps/libsymbi* \
           target/release/.fingerprint/symbi* \
           target/release/symbi*

# Copy actual source code (invalidates cache only when source changes)
COPY src/ ./src/
COPY crates/ ./crates/

# Build the unified symbi binary with cached dependencies
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release && \
    cp target/release/symbi /tmp/symbi

# Runtime stage - minimal security-hardened image
FROM debian:bookworm-slim

# Install minimal runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Create non-root user for security
RUN groupadd -r symbi && useradd -r -g symbi -u 1000 symbi

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

# Set working directory for operations
WORKDIR /var/lib/symbi

# Expose ports for HTTP API and MCP server
EXPOSE 8080 3000

# Health check using the unified binary
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD /usr/local/bin/symbi --version || exit 1

# Default entrypoint is the unified symbi binary
ENTRYPOINT ["/usr/local/bin/symbi"]
CMD ["--help"]

# Labels for metadata
LABEL org.opencontainers.image.title="Symbi" \
      org.opencontainers.image.description="Unified DSL and Runtime for AI-native programming" \
      org.opencontainers.image.vendor="ThirdKey.ai" \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.source="https://github.com/thirdkeyai/symbiont"