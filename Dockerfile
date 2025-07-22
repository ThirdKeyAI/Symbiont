# Multi-stage build for Symbiont Runtime
FROM rust:1.82-slim-bookworm as builder

# Install system dependencies
RUN apt-get update && apt-get install -y \
    build-essential \
    pkg-config \
    libssl-dev \
    git \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

WORKDIR /workspace

# Copy workspace Cargo files
COPY Cargo.toml Cargo.lock ./
COPY runtime/Cargo.toml ./runtime/
COPY dsl/Cargo.toml ./dsl/

# Create dummy source files to cache dependencies
RUN mkdir -p runtime/src/bin dsl/src && \
    echo "fn main() {}" > runtime/src/main.rs && \
    echo "fn main() {}" > runtime/src/bin/symbiont_mcp.rs && \
    echo "" > runtime/src/lib.rs && \
    echo "fn main() {}" > dsl/src/main.rs && \
    echo "" > dsl/src/lib.rs

# Build dependencies (this layer will be cached)
RUN cargo build --release --bin symbiont-mcp
RUN rm -rf runtime/src

# Copy actual source code
COPY runtime/ ./runtime/

# Build the actual application
RUN cargo build --release --bin symbiont-mcp

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Create non-root user
RUN groupadd -r symbiont && useradd -r -g symbiont -m -d /home/symbiont symbiont

# Copy binary from builder
COPY --from=builder /workspace/target/release/symbiont-mcp /usr/local/bin/symbiont-mcp

# Set ownership and permissions
RUN chown symbiont:symbiont /usr/local/bin/symbiont-mcp
RUN chmod +x /usr/local/bin/symbiont-mcp

# Switch to non-root user
USER symbiont
WORKDIR /home/symbiont

# Expose default MCP port
EXPOSE 8080

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD symbiont-mcp --version || exit 1

# Default command
CMD ["symbiont-mcp"]