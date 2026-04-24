# Docker Container Guide


Symbi provides a unified Docker container with all functionality included, available through GitHub Container Registry.

## Available Image

### Unified Symbi Container
- **Image**: `ghcr.io/thirdkeyai/symbi:latest`
- **Purpose**: All-in-one container with DSL parsing, agent runtime, and MCP server
- **Size**: ~80MB (includes vector DB and HTTP API support)
- **CLI**: Unified `symbi` command with subcommands for different operations

## Quick Start

### Scaffold and run a project (recommended)

`symbi init` works inside the container and writes a project into your host directory, including a ready-to-run `docker-compose.yml` and a `.env` with a freshly generated `SYMBIONT_MASTER_KEY`:

```bash
# 1. Create the project files on the host
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest \
  init --profile assistant --no-interact --dir /workspace

# 2. Start the runtime (reads .env automatically)
docker compose up
```

The `--dir /workspace` flag tells `symbi init` to write into the mounted volume rather than the image's WORKDIR. After this runs you'll have `symbiont.toml`, `agents/`, `policies/`, `.symbiont/audit/`, `AGENTS.md`, `docker-compose.yml`, `.env`, and `.env.example` in the current directory.

To skip the compose file generation:

```bash
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest \
  init --profile minimal --no-interact --no-docker-compose --dir /workspace
```

### Using Pre-built Image (ad-hoc)

```bash
# Pull latest image
docker pull ghcr.io/thirdkeyai/symbi:latest

# Parse a DSL file
docker run --rm -v $(pwd):/workspace \
  ghcr.io/thirdkeyai/symbi:latest \
  dsl --file /workspace/agent.dsl

# Run MCP server (stdio-based, no port needed)
docker run --rm -i \
  ghcr.io/thirdkeyai/symbi:latest \
  mcp

# Run the runtime without a project (ephemeral, no master key)
docker run --rm -p 8080:8080 -p 8081:8081 \
  ghcr.io/thirdkeyai/symbi:latest \
  up --http-bind 0.0.0.0
```

### Development Workflow

```bash
# Interactive development
docker run --rm -it -v $(pwd):/workspace \
  ghcr.io/thirdkeyai/symbi:latest bash

# Development with volume mounts and ports
docker run --rm -it \
  -v $(pwd):/workspace \
  -p 8080:8080 \
  -p 8081:8081 \
  ghcr.io/thirdkeyai/symbi:latest bash
```

## Available Tags

- `latest` - Latest stable release
- `main` - Latest development build
- `v1.0.0` - Specific version releases
- `sha-<commit>` - Specific commit builds

## Building Locally

### Unified Symbi Container

```bash
# From project root
docker build -t symbi:latest .

# Test the build
docker run --rm symbi:latest --version

# Test DSL parsing
docker run --rm -v $(pwd):/workspace symbi:latest dsl --help

# Test MCP server
docker run --rm symbi:latest mcp
```

## Multi-Architecture Support

Images are built for:
- `linux/amd64` (x86_64)
- `linux/arm64` (ARM64/Apple Silicon)

Docker automatically pulls the correct architecture for your platform.

## Security Features

### Non-Root Execution
- Containers run as non-root user `symbi` (UID 1000)
- Minimal attack surface with security-hardened base images

### Vulnerability Scanning
- All images automatically scanned with Trivy
- Security advisories published to GitHub Security tab
- SARIF reports for detailed vulnerability analysis

## Configuration

### Environment Variables

**Symbi Container:**
- `SYMBIONT_MASTER_KEY` - **Required for persistent state.** 32-byte hex key used to encrypt the local store. Generate with `openssl rand -hex 32`. `symbi init` writes one into `.env` automatically.
- `RUST_LOG` - Set logging level (debug, info, warn, error)
- `SYMBIONT_VECTOR_BACKEND` - Vector backend: `lancedb` (default) or `qdrant`
- `QDRANT_URL` - Qdrant vector database URL (only if using optional Qdrant backend)
- `OPENROUTER_API_KEY` / `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` - Optional LLM credentials; any one enables the Coordinator Chat endpoint.

### Volume Mounts

The image runs as user `symbi` (UID 1000) with `WORKDIR=/var/lib/symbi`. Project files mount read-only into that directory; persistent state (the local SQLite store and audit logs) lives in named volumes so it survives container restarts.

```bash
# Project files (read-only)
-v $(pwd)/symbiont.toml:/var/lib/symbi/symbiont.toml:ro
-v $(pwd)/agents:/var/lib/symbi/agents:ro
-v $(pwd)/policies:/var/lib/symbi/policies:ro
-v $(pwd)/tools:/var/lib/symbi/tools:ro

# Persistent state
-v symbi-data:/var/lib/symbi/.symbi
-v symbi-audit:/var/lib/symbi/.symbiont
```

## Docker Compose Example

`symbi init` generates a ready-to-run `docker-compose.yml` that matches the rest of this section — prefer that to hand-writing a compose file. For reference, or when starting without `init`:

By default, Symbiont uses **LanceDB** as an embedded vector database -- no external services required. If you need a distributed vector backend for scaled deployments, you can optionally add Qdrant.

### Minimal (LanceDB default -- no Qdrant needed)

Pair this with a `.env` file that sets `SYMBIONT_MASTER_KEY`:

```yaml
services:
  symbi:
    image: ghcr.io/thirdkeyai/symbi:latest
    command: ["up", "--http-bind", "0.0.0.0"]
    ports:
      - "8080:8080"
      - "8081:8081"
    volumes:
      - ./symbiont.toml:/var/lib/symbi/symbiont.toml:ro
      - ./agents:/var/lib/symbi/agents:ro
      - ./policies:/var/lib/symbi/policies:ro
      - ./tools:/var/lib/symbi/tools:ro
      - symbi-data:/var/lib/symbi/.symbi
      - symbi-audit:/var/lib/symbi/.symbiont
    environment:
      SYMBIONT_MASTER_KEY: ${SYMBIONT_MASTER_KEY:?set SYMBIONT_MASTER_KEY in .env}
      RUST_LOG: ${RUST_LOG:-info}
    restart: unless-stopped

volumes:
  symbi-data:
  symbi-audit:
```

### With Optional Qdrant Backend

```yaml
services:
  symbi:
    image: ghcr.io/thirdkeyai/symbi:latest
    command: ["up", "--http-bind", "0.0.0.0"]
    ports:
      - "8080:8080"
      - "8081:8081"
    volumes:
      - ./symbiont.toml:/var/lib/symbi/symbiont.toml:ro
      - ./agents:/var/lib/symbi/agents:ro
      - ./policies:/var/lib/symbi/policies:ro
      - symbi-data:/var/lib/symbi/.symbi
      - symbi-audit:/var/lib/symbi/.symbiont
    environment:
      SYMBIONT_MASTER_KEY: ${SYMBIONT_MASTER_KEY:?set SYMBIONT_MASTER_KEY in .env}
      RUST_LOG: ${RUST_LOG:-info}
      SYMBIONT_VECTOR_BACKEND: qdrant
      QDRANT_URL: http://qdrant:6334
    depends_on:
      - qdrant
    restart: unless-stopped

  qdrant:
    image: qdrant/qdrant:latest
    ports:
      - "6333:6333"
      - "6334:6334"
    volumes:
      - qdrant-data:/qdrant/storage

volumes:
  symbi-data:
  symbi-audit:
  qdrant-data:
```

## Troubleshooting

### Common Issues

**Permission Denied:**
```bash
# Ensure correct ownership
sudo chown -R 1000:1000 ./data

# Or use different user
docker run --user $(id -u):$(id -g) ...
```

**Port Conflicts:**
```bash
# Use different ports
docker run -p 8081:8080 ghcr.io/thirdkeyai/symbi:latest
```

**Build Failures:**
```bash
# Clear Docker cache
docker builder prune -a

# Rebuild without cache
docker build --no-cache -f runtime/Dockerfile .
```

### Health Checks

```bash
# Check container health
docker run --name symbi-test -d ghcr.io/thirdkeyai/symbi:latest up --http-bind 0.0.0.0:8080
docker exec symbi-test /usr/local/bin/symbi --version
docker rm -f symbi-test
```

## Performance Optimization

### Resource Limits

```bash
# Set memory and CPU limits
docker run --memory=512m --cpus=1.0 \
  ghcr.io/thirdkeyai/symbi:latest mcp
```

### Build Optimization

```bash
# Use BuildKit for faster builds
DOCKER_BUILDKIT=1 docker build .

# Multi-stage caching
docker build --target builder -t symbi-builder .
docker build --cache-from symbi-builder .
```

## CI/CD Integration

GitHub Actions automatically builds and publishes containers on:
- Push to `main` branch
- New version tags (`v*`)
- Pull requests (build only)

Images include metadata:
- Git commit SHA
- Build timestamp  
- Vulnerability scan results
- SBOM (Software Bill of Materials)