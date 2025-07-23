# Docker Containers Guide

Symbiont provides pre-built Docker containers for both DSL and Runtime components, available through GitHub Container Registry.

## Available Images

### DSL Parser Container
- **Image**: `ghcr.io/thirdkeyai/symbiont-dsl`
- **Purpose**: Parse and validate Symbiont DSL files
- **Size**: ~50MB (minimal Debian-based image)

### Runtime Container  
- **Image**: `ghcr.io/thirdkeyai/symbiont-runtime`
- **Purpose**: Execute agents and run MCP server
- **Size**: ~80MB (includes vector DB and HTTP API support)

## Quick Start

### Using Pre-built Images

```bash
# Pull latest images
docker pull ghcr.io/thirdkeyai/symbiont-dsl:latest
docker pull ghcr.io/thirdkeyai/symbiont-runtime:latest

# Parse a DSL file
docker run --rm -v $(pwd):/workspace \
  ghcr.io/thirdkeyai/symbiont-dsl:latest \
  parse /workspace/agent.dsl

# Run the runtime with HTTP API
docker run --rm -p 8080:8080 \
  ghcr.io/thirdkeyai/symbiont-runtime:latest \
  --http-api --port 8080
```

### Development Workflow

```bash
# Interactive DSL development
docker run --rm -it -v $(pwd):/workspace \
  ghcr.io/thirdkeyai/symbiont-dsl:latest bash

# Runtime development with volume mounts
docker run --rm -it \
  -v $(pwd):/workspace \
  -p 8080:8080 \
  -p 3000:3000 \
  ghcr.io/thirdkeyai/symbiont-runtime:latest bash
```

## Available Tags

- `latest` - Latest stable release
- `main` - Latest development build
- `v1.0.0` - Specific version releases
- `sha-<commit>` - Specific commit builds

## Building Locally

### DSL Container

```bash
# From project root
docker build -f dsl/Dockerfile -t symbiont-dsl .

# Test the build
docker run --rm symbiont-dsl --version
```

### Runtime Container

```bash
# From project root  
docker build -f runtime/Dockerfile -t symbiont-runtime .

# Test the build
docker run --rm symbiont-runtime --version
```

## Multi-Architecture Support

Images are built for:
- `linux/amd64` (x86_64)
- `linux/arm64` (ARM64/Apple Silicon)

Docker automatically pulls the correct architecture for your platform.

## Security Features

### Non-Root Execution
- Containers run as non-root user `symbiont` (UID 1000)
- Minimal attack surface with security-hardened base images

### Vulnerability Scanning
- All images automatically scanned with Trivy
- Security advisories published to GitHub Security tab
- SARIF reports for detailed vulnerability analysis

## Configuration

### Environment Variables

**DSL Container:**
- `DSL_LOG_LEVEL` - Set logging level (debug, info, warn, error)
- `DSL_OUTPUT_FORMAT` - Output format (json, yaml, text)

**Runtime Container:**
- `SYMBIONT_HTTP_PORT` - HTTP API port (default: 8080)
- `SYMBIONT_MCP_PORT` - MCP server port (default: 3000)
- `SYMBIONT_LOG_LEVEL` - Logging level
- `QDRANT_URL` - Vector database URL

### Volume Mounts

```bash
# Mount agent definitions
-v $(pwd)/agents:/var/lib/symbiont/agents

# Mount configuration
-v $(pwd)/config:/etc/symbiont

# Mount data directory
-v symbiont-data:/var/lib/symbiont/data
```

## Docker Compose Example

```yaml
version: '3.8'

services:
  symbiont-runtime:
    image: ghcr.io/thirdkeyai/symbiont-runtime:latest
    ports:
      - "8080:8080"
      - "3000:3000"
    volumes:
      - ./agents:/var/lib/symbiont/agents
      - ./config:/etc/symbiont
      - symbiont-data:/var/lib/symbiont/data
    environment:
      - SYMBIONT_LOG_LEVEL=info
      - QDRANT_URL=http://qdrant:6334
    depends_on:
      - qdrant

  qdrant:
    image: qdrant/qdrant:latest
    ports:
      - "6333:6333"
      - "6334:6334"
    volumes:
      - qdrant-data:/qdrant/storage

volumes:
  symbiont-data:
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
docker run -p 8081:8080 ghcr.io/thirdkeyai/symbiont-runtime:latest
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
docker run --name symbiont-test -d ghcr.io/thirdkeyai/symbiont-runtime:latest
docker exec symbiont-test /usr/local/bin/symbiont-mcp --version
docker rm -f symbiont-test
```

## Performance Optimization

### Resource Limits

```bash
# Set memory and CPU limits
docker run --memory=512m --cpus=1.0 \
  ghcr.io/thirdkeyai/symbiont-runtime:latest
```

### Build Optimization

```bash
# Use BuildKit for faster builds
DOCKER_BUILDKIT=1 docker build -f runtime/Dockerfile .

# Multi-stage caching
docker build --target builder -t symbiont-builder .
docker build --cache-from symbiont-builder .
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