# Symbi DSL

A secure, containerized development environment for the Symbi Domain-Specific Language (DSL) - an AI-native programming language built with Tree-sitter for precise AST-based code manipulation.

## Overview

The Symbi DSL is designed for building autonomous, policy-aware agents that can safely collaborate with humans, other agents, and large language models while enforcing zero-trust security, data privacy, and provable behavior.

### Key Features

- **Tree-sitter Integration**: AST-based parsing for Rust, Python, JavaScript, and TypeScript
- **Security-First Design**: Multi-tiered sandboxing with cryptographic auditability
- **Policy-Aware Programming**: Declarative security policies with runtime enforcement
- **Agent Framework**: Foundation for autonomous code generation and manipulation

## Prerequisites

- Docker Engine 20.10+ (for containerized development)
- Git (for version control)
- 4GB+ available RAM
- 10GB+ available disk space

## Quick Start

### 1. Build the Docker Development Environment

```bash
# Build the Docker image (from the repository root)
docker build -t symbi:latest .

# Verify the build
docker images | grep symbi
```

### 2. Run the Development Container

```bash
# Start an interactive development session
docker run -it --rm \
  --name symbi-dev \
  -v "$(pwd)":/workspace \
  -w /workspace \
  symbi:latest

# Alternative: Run with port forwarding for development servers
docker run -it --rm \
  --name symbi-dev \
  -v "$(pwd)":/workspace \
  -w /workspace \
  -p 3000:3000 -p 8000:8000 -p 8080:8080 \
  symbi:latest
```

### 3. Build and Run the DSL Project

Inside the container:

```bash
# Navigate to the DSL project
cd crates/dsl

# Build the project
cargo build

# Run the DSL binary
cargo run

# Run tests
cargo test

# Run with release optimizations
cargo build --release
cargo run --release

# Use via the unified symbi CLI (from project root)
cd ../.. && cargo run -- dsl parse my_agent.dsl
```

## Development Workflow

### Container-Based Development

The Docker environment provides a consistent, secure development experience:

```bash
# Start development container with volume mounting
docker run -it --rm \
  --name symbi-dev \
  -v "$(pwd)":/workspace \
  -w /workspace/crates/dsl \
  symbi:latest bash

# Inside the container, you have access to:
# - Rust toolchain (rustc, cargo, rustfmt, clippy)
# - Tree-sitter CLI
# - Development tools (cargo-watch, cargo-edit, cargo-audit)
```

### Hot Reloading During Development

```bash
# Watch for changes and rebuild automatically
cargo watch -x build

# Watch and run tests
cargo watch -x test

# Watch and run the application
cargo watch -x run
```

### Code Quality and Security

```bash
# Format code
cargo fmt

# Run linter
cargo clippy

# Security audit
cargo audit

# Check for outdated dependencies
cargo outdated
```

## Docker Environment Details

### Security Features

The Docker environment implements security best practices:

- **Non-root user**: All operations run as `rustdev` user (UID 1000)
- **Minimal attack surface**: Only essential packages installed
- **Resource limits**: Configurable memory and CPU constraints
- **Health checks**: Container monitoring for reliability

### Environment Variables

```bash
# Rust configuration
CARGO_HOME=/usr/local/cargo
RUSTUP_HOME=/usr/local/rustup
PATH=/usr/local/cargo/bin:$PATH

# Development optimizations
RUST_BACKTRACE=1
RUST_LOG=debug
```

### Installed Tools

- **Rust 1.75**: Latest stable Rust compiler
- **Tree-sitter CLI**: For grammar development and testing
- **Development Tools**: cargo-watch, cargo-edit, cargo-audit
- **System Tools**: git, curl, clang, llvm

## Project Structure

```
crates/dsl/
├── Cargo.toml          # Project configuration and dependencies
├── src/
│   ├── main.rs         # Application entry point
│   ├── lib.rs          # Library root
│   ├── parser/         # Tree-sitter integration
│   ├── ast/            # AST manipulation
│   ├── agent/          # Agent framework
│   └── policy/         # Policy engine
├── tests/              # Integration tests
├── benches/            # Performance benchmarks
└── examples/           # Usage examples
```

## Building Without Docker

If you prefer local development:

### Prerequisites

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# Install Tree-sitter CLI
npm install -g tree-sitter-cli

# Install development tools
cargo install cargo-watch cargo-edit cargo-audit
```

### Build Commands

```bash
# Standard build
cargo build

# Release build with optimizations
cargo build --release

# Run tests
cargo test

# Generate documentation
cargo doc --open
```

## Performance Considerations

### Build Optimization

The project is configured for optimal performance:

- **Release Profile**: LTO enabled, single codegen unit
- **Security**: Panic abort, symbol stripping
- **Development**: Fast compilation, debug symbols

### Resource Requirements

- **Memory**: 512MB minimum, 2GB recommended
- **CPU**: 2 cores minimum, 4 cores recommended
- **Storage**: 1GB for dependencies, 5GB for full build cache

## Troubleshooting

### Common Issues

1. **Permission Errors**
   ```bash
   # Ensure proper ownership
   sudo chown -R $(id -u):$(id -g) target/
   ```

2. **Out of Memory**
   ```bash
   # Increase Docker memory limit or use fewer parallel jobs
   cargo build -j 2
   ```

3. **Tree-sitter Compilation Issues**
   ```bash
   # Verify Tree-sitter CLI installation
   tree-sitter --version
   
   # Rebuild Tree-sitter parsers
   cargo clean
   cargo build
   ```

### Debug Mode

```bash
# Enable verbose logging
RUST_LOG=debug cargo run

# Enable backtraces
RUST_BACKTRACE=full cargo run

# Profile compilation
cargo build --timings
```

## Security Considerations

### Container Security

- Runs as non-privileged user
- No network access by default
- Read-only root filesystem
- Minimal package installation
- Regular security updates

### Development Security

- Dependency auditing with `cargo audit`
- Static analysis with `clippy`
- Memory safety guaranteed by Rust
- Cryptographic operations via `ring` crate

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make changes in the containerized environment
4. Run tests and security checks
5. Submit a pull request

### Code Standards

- Follow Rust formatting (`cargo fmt`)
- Pass all lints (`cargo clippy`)
- Maintain test coverage
- Update documentation

## License

This project is licensed under MIT OR Apache-2.0.

## Support

For issues and questions:
- Create an issue in the repository
- Contact: jascha@thirdkey.ai
- Documentation: See project wiki