# Multi-stage build for security and size optimization
# Base: Rust 1.88 on Debian Bookworm (latest stable, security-focused)
FROM rust:1.88-slim-bookworm as builder

# Install system dependencies required for Tree-sitter and Rust development
# Following CIS Docker Benchmark recommendations
RUN apt-get update && apt-get install -y \
    build-essential \
    pkg-config \
    libssl-dev \
    git \
    curl \
    nodejs \
    npm \
    python3 \
    python3-pip \
    clang \
    llvm \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Install Tree-sitter CLI globally for DSL development
RUN npm install -g tree-sitter-cli

# Create non-root user for security (CIS Docker Benchmark 4.1)
RUN groupadd -r rustdev && useradd -r -g rustdev -m -d /home/rustdev rustdev

# Set working directory
WORKDIR /workspace

# Copy ownership to rustdev user
RUN chown -R rustdev:rustdev /workspace

# Switch to non-root user
USER rustdev

# Install Rust development tools
RUN rustup component add rustfmt clippy rust-src rust-analyzer
RUN cargo install cargo-watch cargo-edit cargo-audit

# Development stage - optimized for security and development workflow
FROM rust:1.88-slim-bookworm as development

# Install runtime dependencies with minimal attack surface
RUN apt-get update && apt-get install -y \
    git \
    curl \
    nodejs \
    npm \
    python3 \
    clang \
    llvm \
    pkg-config \
    libssl-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Install Tree-sitter CLI
RUN npm install -g tree-sitter-cli

# Create non-root user with consistent UID/GID for volume mounting
RUN groupadd -g 1000 rustdev && useradd -u 1000 -g 1000 -m -d /home/rustdev rustdev

# Set working directory
WORKDIR /workspace

# Copy Rust toolchain from builder
COPY --from=builder /usr/local/cargo /usr/local/cargo
COPY --from=builder /usr/local/rustup /usr/local/rustup

# Set environment variables for Rust
ENV CARGO_HOME=/usr/local/cargo
ENV RUSTUP_HOME=/usr/local/rustup
ENV PATH=/usr/local/cargo/bin:$PATH

# Fix ownership of copied Rust toolchain and workspace for rustdev user
RUN chown -R rustdev:rustdev /usr/local/cargo /usr/local/rustup /workspace

# Switch to non-root user for security
USER rustdev

# Install development tools as non-root user
RUN rustup component add rustfmt clippy rust-src rust-analyzer
RUN cargo install cargo-watch cargo-edit cargo-audit

# Create .cargo/config.toml for optimized builds
RUN mkdir -p /home/rustdev/.cargo && \
    echo '[build]' > /home/rustdev/.cargo/config.toml && \
    echo 'jobs = 4' >> /home/rustdev/.cargo/config.toml && \
    echo '[target.x86_64-unknown-linux-gnu]' >> /home/rustdev/.cargo/config.toml && \
    echo 'linker = "clang"' >> /home/rustdev/.cargo/config.toml

# Expose common development ports (non-privileged)
EXPOSE 3000 8000 8080

# Health check for container monitoring
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD rustc --version || exit 1

# Set default command to bash for interactive development
CMD ["/bin/bash"]