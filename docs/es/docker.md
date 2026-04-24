# Guia de Contenedores Docker

## Otros idiomas


## Imagen Disponible

### Contenedor Unificado de Symbi
- **Imagen**: `ghcr.io/thirdkeyai/symbi:latest`
- **Proposito**: Contenedor todo en uno con analisis de DSL, runtime de agentes y servidor MCP
- **Tamano**: ~80MB (incluye base de datos vectorial y soporte de API HTTP)
- **CLI**: Comando unificado `symbi` con subcomandos para diferentes operaciones

## Inicio Rapido

### Crear y ejecutar un proyecto (recomendado)

`symbi init` funciona dentro del contenedor y escribe un proyecto en su directorio del host, incluyendo un `docker-compose.yml` listo para ejecutar y un `.env` con un `SYMBIONT_MASTER_KEY` recien generado:

```bash
# 1. Create the project files on the host
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest \
  init --profile assistant --no-interact --dir /workspace

# 2. Start the runtime (reads .env automatically)
docker compose up
```

El flag `--dir /workspace` le indica a `symbi init` que escriba en el volumen montado en lugar del WORKDIR de la imagen. Despues de esto tendra `symbiont.toml`, `agents/`, `policies/`, `.symbiont/audit/`, `AGENTS.md`, `docker-compose.yml`, `.env` y `.env.example` en el directorio actual.

Para omitir la generacion del archivo compose:

```bash
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest \
  init --profile minimal --no-interact --no-docker-compose --dir /workspace
```

### Usando la Imagen Pre-construida (ad-hoc)

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

### Flujo de Trabajo de Desarrollo

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
docker run --rm -v $(pwd):/workspace symbi:latest dsl --help

# Test MCP server
docker run --rm symbi:latest mcp
```

## Soporte Multi-Arquitectura

Las imagenes se compilan para:
- `linux/amd64` (x86_64)
- `linux/arm64` (ARM64/Apple Silicon)

Docker descarga automaticamente la arquitectura correcta para su plataforma.

## Caracteristicas de Seguridad

### Ejecucion Sin Root
- Los contenedores se ejecutan como el usuario no root `symbi` (UID 1000)
- Superficie de ataque minima con imagenes base reforzadas en seguridad

### Escaneo de Vulnerabilidades
- Todas las imagenes se escanean automaticamente con Trivy
- Los avisos de seguridad se publican en la pestana de Seguridad de GitHub
- Informes SARIF para analisis detallado de vulnerabilidades

## Configuracion

### Variables de Entorno

**Contenedor Symbi:**
- `SYMBIONT_MASTER_KEY` - **Requerido para estado persistente.** Clave hexadecimal de 32 bytes usada para cifrar el almacen local. Genere con `openssl rand -hex 32`. `symbi init` escribe una en `.env` automaticamente.
- `RUST_LOG` - Establecer nivel de registro (debug, info, warn, error)
- `SYMBIONT_VECTOR_BACKEND` - Backend vectorial: `lancedb` (predeterminado) o `qdrant`
- `QDRANT_URL` - URL de la base de datos vectorial Qdrant (solo si se usa el backend opcional de Qdrant)
- `OPENROUTER_API_KEY` / `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` - Credenciales LLM opcionales; cualquiera habilita el endpoint Coordinator Chat.

### Montajes de Volumenes

La imagen se ejecuta como usuario `symbi` (UID 1000) con `WORKDIR=/var/lib/symbi`. Los archivos del proyecto se montan en modo solo lectura en ese directorio; el estado persistente (el almacen SQLite local y los registros de auditoria) vive en volumenes con nombre para que sobreviva a reinicios del contenedor.

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

## Ejemplo de Docker Compose

`symbi init` genera un `docker-compose.yml` listo para ejecutar que coincide con el resto de esta seccion — preferirlo a escribir un archivo compose a mano. Como referencia, o al empezar sin `init`:

Por defecto, Symbiont usa **LanceDB** como base de datos vectorial integrada -- no se requieren servicios externos. Si necesita un backend vectorial distribuido para despliegues a escala, puede agregar opcionalmente Qdrant.

### Minimo (LanceDB predeterminado -- no se necesita Qdrant)

Combinelo con un archivo `.env` que defina `SYMBIONT_MASTER_KEY`:

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

### Con Backend Opcional de Qdrant

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
docker build --no-cache .
```

### Verificaciones de Salud

```bash
# Check container health
docker run --name symbi-test -d ghcr.io/thirdkeyai/symbi:latest up --http-bind 0.0.0.0:8080
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
