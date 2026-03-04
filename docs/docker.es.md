---
layout: default
title: Guia de Docker
nav_exclude: true
description: "Guia de contenedores Docker para ejecutar Symbiont"
---

# Guia de Contenedores Docker

## Otros idiomas
{: .no_toc}

[English](docker.md) | [中文简体](docker.zh-cn.md) | **Español** | [Português](docker.pt.md) | [日本語](docker.ja.md) | [Deutsch](docker.de.md)

---

Symbi proporciona un contenedor Docker unificado con toda la funcionalidad incluida, disponible a traves de GitHub Container Registry.

## Imagen Disponible

### Contenedor Unificado de Symbi
- **Imagen**: `ghcr.io/thirdkeyai/symbi:latest`
- **Proposito**: Contenedor todo en uno con analisis de DSL, runtime de agentes y servidor MCP
- **Tamano**: ~80MB (incluye base de datos vectorial y soporte de API HTTP)
- **CLI**: Comando unificado `symbi` con subcomandos para diferentes operaciones

## Inicio Rapido

### Usando la Imagen Pre-construida

```bash
# Pull latest image
docker pull ghcr.io/thirdkeyai/symbi:latest

# Parse a DSL file
docker run --rm -v $(pwd):/workspace \
  ghcr.io/thirdkeyai/symbi:latest \
  dsl parse /workspace/agent.dsl

# Run MCP server
docker run --rm -p 8080:8080 \
  ghcr.io/thirdkeyai/symbi:latest \
  mcp --port 8080

# Run with HTTP API
docker run --rm -p 8080:8080 \
  ghcr.io/thirdkeyai/symbi:latest \
  mcp --http-api --port 8080
```

### Flujo de Trabajo de Desarrollo

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

## Etiquetas Disponibles

- `latest` - Ultima version estable
- `main` - Ultima compilacion de desarrollo
- `v1.0.0` - Versiones especificas
- `sha-<commit>` - Compilaciones de commits especificos

## Compilacion Local

### Contenedor Unificado de Symbi

```bash
# From project root
docker build -t symbi:latest .

# Test the build
docker run --rm symbi:latest --version

# Test DSL parsing
docker run --rm -v $(pwd):/workspace symbi:latest dsl parse --help

# Test MCP server
docker run --rm symbi:latest mcp --help
```

## Soporte Multi-Arquitectura

Las imagenes se compilan para:
- `linux/amd64` (x86_64)
- `linux/arm64` (ARM64/Apple Silicon)

Docker descarga automaticamente la arquitectura correcta para su plataforma.

## Caracteristicas de Seguridad

### Ejecucion Sin Root
- Los contenedores se ejecutan como el usuario no root `symbiont` (UID 1000)
- Superficie de ataque minima con imagenes base reforzadas en seguridad

### Escaneo de Vulnerabilidades
- Todas las imagenes se escanean automaticamente con Trivy
- Los avisos de seguridad se publican en la pestana de Seguridad de GitHub
- Informes SARIF para analisis detallado de vulnerabilidades

## Configuracion

### Variables de Entorno

**Contenedor Symbi:**
- `SYMBI_LOG_LEVEL` - Establecer nivel de registro (debug, info, warn, error)
- `SYMBI_HTTP_PORT` - Puerto de API HTTP (predeterminado: 8080)
- `SYMBI_MCP_PORT` - Puerto del servidor MCP (predeterminado: 3000)
- `SYMBIONT_VECTOR_BACKEND` - Backend vectorial: `lancedb` (predeterminado) o `qdrant`
- `QDRANT_URL` - URL de la base de datos vectorial Qdrant (solo si se usa el backend opcional de Qdrant)
- `DSL_OUTPUT_FORMAT` - Formato de salida DSL (json, yaml, text)

### Montajes de Volumenes

```bash
# Mount agent definitions
-v $(pwd)/agents:/var/lib/symbi/agents

# Mount configuration
-v $(pwd)/config:/etc/symbi

# Mount data directory
-v symbi-data:/var/lib/symbi/data
```

## Ejemplo de Docker Compose

Por defecto, Symbiont usa **LanceDB** como base de datos vectorial integrada -- no se requieren servicios externos. Si necesita un backend vectorial distribuido para despliegues a escala, puede agregar opcionalmente Qdrant.

### Minimo (LanceDB predeterminado -- no se necesita Qdrant)

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
      - SYMBI_LOG_LEVEL=info
    command: ["mcp", "--http-api", "--port", "8080"]

volumes:
  symbi-data:
```

### Con Backend Opcional de Qdrant

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
      - SYMBI_LOG_LEVEL=info
      - SYMBIONT_VECTOR_BACKEND=qdrant
      - QDRANT_URL=http://qdrant:6334
    depends_on:
      - qdrant
    command: ["mcp", "--http-api", "--port", "8080"]

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

## Solucion de Problemas

### Problemas Comunes

**Permiso Denegado:**
```bash
# Ensure correct ownership
sudo chown -R 1000:1000 ./data

# Or use different user
docker run --user $(id -u):$(id -g) ...
```

**Conflictos de Puertos:**
```bash
# Use different ports
docker run -p 8081:8080 ghcr.io/thirdkeyai/symbi:latest
```

**Fallos de Compilacion:**
```bash
# Clear Docker cache
docker builder prune -a

# Rebuild without cache
docker build --no-cache -f runtime/Dockerfile .
```

### Verificaciones de Salud

```bash
# Check container health
docker run --name symbi-test -d ghcr.io/thirdkeyai/symbi:latest mcp --port 8080
docker exec symbi-test /usr/local/bin/symbi --version
docker rm -f symbi-test
```

## Optimizacion de Rendimiento

### Limites de Recursos

```bash
# Set memory and CPU limits
docker run --memory=512m --cpus=1.0 \
  ghcr.io/thirdkeyai/symbi:latest mcp
```

### Optimizacion de Compilacion

```bash
# Use BuildKit for faster builds
DOCKER_BUILDKIT=1 docker build .

# Multi-stage caching
docker build --target builder -t symbi-builder .
docker build --cache-from symbi-builder .
```

## Integracion CI/CD

GitHub Actions compila y publica automaticamente los contenedores cuando:
- Se hace push a la rama `main`
- Se crean nuevas etiquetas de version (`v*`)
- Se abren pull requests (solo compilacion)

Las imagenes incluyen metadatos:
- SHA del commit de Git
- Marca de tiempo de compilacion
- Resultados del escaneo de vulnerabilidades
- SBOM (Software Bill of Materials)
