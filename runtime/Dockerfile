# Production Runtime Container - Symbiont OSS
# Multi-stage build for minimal runtime image
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

# Copy runtime-specific files (OSS only)
COPY . ./runtime/

# Create a minimal workspace that only includes runtime
RUN echo '[workspace]' > Cargo.toml && \
    echo 'members = ["runtime"]' >> Cargo.toml && \
    echo 'resolver = "2"' >> Cargo.toml

# Build the runtime binaries in release mode
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