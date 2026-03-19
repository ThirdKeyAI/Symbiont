# Docker Container Guide


Symbi provides a unified Docker container with all functionality included, available through GitHub Container Registry.

## Available Image

### Unified Symbi Container
- **Image**: `ghcr.io/thirdkeyai/symbi:latest`
- **Purpose**: All-in-one container with DSL parsing, agent runtime, and MCP server
- **Size**: ~80MB (includes vector DB and HTTP API support)
- **CLI**: Unified `symbi` command with subcommands for different operations

## Quick Start

### Using Pre-built Image

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

# Run with HTTP API
docker run --rm -p 8080:8080 \
  ghcr.io/thirdkeyai/symbi:latest \
  up --http-bind 0.0.0.0:8080
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
  -p 3000:3000 \
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
- `RUST_LOG` - Set logging level (debug, info, warn, error)
- `SYMBIONT_VECTOR_BACKEND` - Vector backend: `lancedb` (default) or `qdrant`
- `QDRANT_URL` - Qdrant vector database URL (only if using optional Qdrant backend)

### Volume Mounts

```bash
# Mount agent definitions
-v $(pwd)/agents:/var/lib/symbi/agents

# Mount configuration
-v $(pwd)/config:/etc/symbi

# Mount data directory
-v symbi-data:/var/lib/symbi/data
```

## Docker Compose Example

By default, Symbiont uses **LanceDB** as an embedded vector database -- no external services required. If you need a distributed vector backend for scaled deployments, you can optionally add Qdrant.

### Minimal (LanceDB default -- no Qdrant needed)

```yaml
version: '3.8'

services:
  symbi:
    image: ghcr.io/thirdkeyai/symbi:latest
    ports:
      - "8080:8080"
      - "3000:3000"
    volumes:
      - ./agents:/var/lib/symbi/agents
      - ./config:/etc/symbi
      - symbi-data:/var/lib/symbi/data
    environment:
      - RUST_LOG=info
    command: ["up", "--http-bind", "0.0.0.0:8080"]

volumes:
  symbi-data:
```

### With Optional Qdrant Backend

```yaml
version: '3.8'

services:
  symbi:
    image: ghcr.io/thirdkeyai/symbi:latest
    ports:
      - "8080:8080"
      - "3000:3000"
    volumes:
      - ./agents:/var/lib/symbi/agents
      - ./config:/etc/symbi
      - symbi-data:/var/lib/symbi/data
    environment:
      - RUST_LOG=info
      - SYMBIONT_VECTOR_BACKEND=qdrant
      - QDRANT_URL=http://qdrant:6334
    depends_on:
      - qdrant
    command: ["up", "--http-bind", "0.0.0.0:8080"]

  qdrant:
    image: qdrant/qdrant:latest
    ports:
      - "6333:6333"
      - "6334:6334"
    volumes:
      - qdrant-data:/qdrant/storage

volumes:
  symbi-data:
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