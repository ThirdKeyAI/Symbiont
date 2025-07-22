# Multi-stage build for Symbiont Runtime
FROM rust:1.88-slim-bookworm as builder

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

# Copy workspace Cargo files for dependency caching
COPY Cargo.toml Cargo.lock ./
COPY runtime/Cargo.toml ./runtime/

# Create dummy source files to cache dependencies
RUN mkdir -p runtime/src && echo "fn main() {}" > runtime/src/main.rs
RUN echo "fn main() {}" > runtime/src/lib.rs

# Build dependencies (this layer will be cached)
RUN cd runtime && cargo build --release --bin symbiont_mcp
RUN rm -rf runtime/src

# Copy actual source code
COPY runtime/src ./runtime/src
COPY runtime/examples ./runtime/examples
COPY runtime/tests ./runtime/tests

# Build the actual application
RUN cd runtime && cargo build --release --bin symbiont_mcp

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
COPY --from=builder /workspace/runtime/target/release/symbiont_mcp /usr/local/bin/symbiont_mcp

# Set ownership and permissions
RUN chown symbiont:symbiont /usr/local/bin/symbiont_mcp
RUN chmod +x /usr/local/bin/symbiont_mcp

# Switch to non-root user
USER symbiont
WORKDIR /home/symbiont

# Expose default MCP port
EXPOSE 8080

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD symbiont_mcp --version || exit 1

# Default command
CMD ["symbiont_mcp"]