# Production DSL Container - Symbiont OSS
# Multi-stage build for minimal runtime image
FROM rust:1.80-slim-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    git \
    clang \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

WORKDIR /app

# Copy DSL source files
COPY . .

# Build the DSL binary in release mode
RUN cargo build --release --bin dsl

# Runtime stage - minimal security-hardened image
FROM debian:bookworm-slim

# Install minimal runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Create non-root user for security
RUN groupadd -r symbiont && useradd -r -g symbiont -u 1000 symbiont

# Copy the DSL binary
COPY --from=builder /app/target/release/dsl /usr/local/bin/symbiont-dsl

# Set proper ownership and permissions
RUN chown symbiont:symbiont /usr/local/bin/symbiont-dsl && \
    chmod +x /usr/local/bin/symbiont-dsl

# Switch to non-root user
USER symbiont

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD /usr/local/bin/symbiont-dsl --version || exit 1

# Set entrypoint
ENTRYPOINT ["/usr/local/bin/symbiont-dsl"]
CMD ["--help"]