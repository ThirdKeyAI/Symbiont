# Optimized Production Runtime Container - Symbiont OSS
FROM rust:1.82-slim-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    git \
    clang \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

WORKDIR /app

# Copy dependency files first for better caching
COPY Cargo.toml Cargo.lock ./

# Create dummy source files to cache dependencies
RUN mkdir -p src/bin && \
    echo "fn main() {}" > src/main.rs && \
    echo "fn main() {}" > src/bin/symbiont_mcp.rs && \
    echo "" > src/lib.rs

# Build dependencies only (cached layer)
RUN cargo build --release --bin symbiont-mcp && \
    rm -rf target/release/deps/symbiont* target/release/symbiont-mcp*

# Copy source code (invalidates cache only when source changes)
COPY . ./

# Build actual application (fast incremental build)
RUN cargo build --release --bin symbiont-mcp

# Runtime stage - minimal security-hardened image
FROM debian:bookworm-slim

# Install minimal runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Create non-root user for security
RUN groupadd -r symbiont && useradd -r -g symbiont -u 1000 symbiont

# Create directories for agent data and configuration
RUN mkdir -p /var/lib/symbiont /etc/symbiont && \
    chown -R symbiont:symbiont /var/lib/symbiont /etc/symbiont

# Copy the runtime binaries
COPY --from=builder /app/target/release/symbiont-mcp /usr/local/bin/symbiont-mcp

# Set proper ownership and permissions
RUN chown symbiont:symbiont /usr/local/bin/symbiont-mcp && \
    chmod +x /usr/local/bin/symbiont-mcp

# Switch to non-root user
USER symbiont

# Set working directory for agent operations
WORKDIR /var/lib/symbiont

# Expose ports for HTTP API and MCP server
EXPOSE 8080 3000

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD /usr/local/bin/symbiont-mcp --version || exit 1

# Set entrypoint to MCP server
ENTRYPOINT ["/usr/local/bin/symbiont-mcp"]
CMD ["--help"]