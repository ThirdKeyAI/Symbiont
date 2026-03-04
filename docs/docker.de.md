---
layout: default
title: Docker-Leitfaden
nav_exclude: true
description: "Docker-Container-Leitfaden fuer den Betrieb von Symbiont"
---

# Docker-Container-Leitfaden

## Andere Sprachen
{: .no_toc}

[English](docker.md) | [中文简体](docker.zh-cn.md) | [Español](docker.es.md) | [Português](docker.pt.md) | [日本語](docker.ja.md) | **Deutsch**

---

Symbi stellt einen einheitlichen Docker-Container mit allen Funktionen bereit, verfuegbar ueber die GitHub Container Registry.

## Verfuegbares Image

### Einheitlicher Symbi-Container
- **Image**: `ghcr.io/thirdkeyai/symbi:latest`
- **Zweck**: All-in-One-Container mit DSL-Parsing, Agenten-Laufzeit und MCP-Server
- **Groesse**: ~80MB (inkl. Vektordatenbank und HTTP-API-Unterstuetzung)
- **CLI**: Einheitlicher `symbi`-Befehl mit Unterbefehlen fuer verschiedene Operationen

## Schnellstart

### Vorgefertigtes Image verwenden

```bash
# Neuestes Image herunterladen
docker pull ghcr.io/thirdkeyai/symbi:latest

# Eine DSL-Datei parsen
docker run --rm -v $(pwd):/workspace \
  ghcr.io/thirdkeyai/symbi:latest \
  dsl parse /workspace/agent.dsl

# MCP-Server starten
docker run --rm -p 8080:8080 \
  ghcr.io/thirdkeyai/symbi:latest \
  mcp --port 8080

# Mit HTTP-API starten
docker run --rm -p 8080:8080 \
  ghcr.io/thirdkeyai/symbi:latest \
  mcp --http-api --port 8080
```

### Entwicklungs-Workflow

```bash
# Interaktive Entwicklung
docker run --rm -it -v $(pwd):/workspace \
  ghcr.io/thirdkeyai/symbi:latest bash

# Entwicklung mit Volume-Mounts und Ports
docker run --rm -it \
  -v $(pwd):/workspace \
  -p 8080:8080 \
  -p 3000:3000 \
  ghcr.io/thirdkeyai/symbi:latest bash
```

## Verfuegbare Tags

- `latest` - Neuestes stabiles Release
- `main` - Neuester Entwicklungs-Build
- `v1.0.0` - Spezifische Versionsreleases
- `sha-<commit>` - Spezifische Commit-Builds

## Lokal bauen

### Einheitlicher Symbi-Container

```bash
# Vom Projektstammverzeichnis
docker build -t symbi:latest .

# Build testen
docker run --rm symbi:latest --version

# DSL-Parsing testen
docker run --rm -v $(pwd):/workspace symbi:latest dsl parse --help

# MCP-Server testen
docker run --rm symbi:latest mcp --help
```

## Multi-Architektur-Unterstuetzung

Images werden gebaut fuer:
- `linux/amd64` (x86_64)
- `linux/arm64` (ARM64/Apple Silicon)

Docker laedt automatisch die korrekte Architektur fuer Ihre Plattform herunter.

## Sicherheitsfunktionen

### Ausfuehrung als Nicht-Root
- Container laufen als Nicht-Root-Benutzer `symbiont` (UID 1000)
- Minimale Angriffsflaeche mit sicherheitsgehaerteten Basis-Images

### Schwachstellenscanning
- Alle Images werden automatisch mit Trivy gescannt
- Sicherheitshinweise werden im GitHub-Security-Tab veroeffentlicht
- SARIF-Berichte fuer detaillierte Schwachstellenanalyse

## Konfiguration

### Umgebungsvariablen

**Symbi-Container:**
- `SYMBI_LOG_LEVEL` - Logging-Level festlegen (debug, info, warn, error)
- `SYMBI_HTTP_PORT` - HTTP-API-Port (Standard: 8080)
- `SYMBI_MCP_PORT` - MCP-Server-Port (Standard: 3000)
- `SYMBIONT_VECTOR_BACKEND` - Vektor-Backend: `lancedb` (Standard) oder `qdrant`
- `QDRANT_URL` - Qdrant-Vektordatenbank-URL (nur bei Verwendung des optionalen Qdrant-Backends)
- `DSL_OUTPUT_FORMAT` - DSL-Ausgabeformat (json, yaml, text)

### Volume-Mounts

```bash
# Agentendefinitionen mounten
-v $(pwd)/agents:/var/lib/symbi/agents

# Konfiguration mounten
-v $(pwd)/config:/etc/symbi

# Datenverzeichnis mounten
-v symbi-data:/var/lib/symbi/data
```

## Docker Compose Beispiel

Standardmaessig verwendet Symbiont **LanceDB** als eingebettete Vektordatenbank -- keine externen Dienste erforderlich. Wenn Sie ein verteiltes Vektor-Backend fuer skalierte Deployments benoetigen, koennen Sie optional Qdrant hinzufuegen.

### Minimal (LanceDB-Standard -- kein Qdrant erforderlich)

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

### Mit optionalem Qdrant-Backend

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

## Fehlerbehebung

### Haeufige Probleme

**Zugriff verweigert:**
```bash
# Korrektes Eigentum sicherstellen
sudo chown -R 1000:1000 ./data

# Oder anderen Benutzer verwenden
docker run --user $(id -u):$(id -g) ...
```

**Port-Konflikte:**
```bash
# Andere Ports verwenden
docker run -p 8081:8080 ghcr.io/thirdkeyai/symbi:latest
```

**Build-Fehler:**
```bash
# Docker-Cache leeren
docker builder prune -a

# Ohne Cache neu bauen
docker build --no-cache -f runtime/Dockerfile .
```

### Gesundheitspruefungen

```bash
# Container-Gesundheit pruefen
docker run --name symbi-test -d ghcr.io/thirdkeyai/symbi:latest mcp --port 8080
docker exec symbi-test /usr/local/bin/symbi --version
docker rm -f symbi-test
```

## Performance-Optimierung

### Ressourcenlimits

```bash
# Speicher- und CPU-Limits setzen
docker run --memory=512m --cpus=1.0 \
  ghcr.io/thirdkeyai/symbi:latest mcp
```

### Build-Optimierung

```bash
# BuildKit fuer schnellere Builds verwenden
DOCKER_BUILDKIT=1 docker build .

# Multi-Stage-Caching
docker build --target builder -t symbi-builder .
docker build --cache-from symbi-builder .
```

## CI/CD-Integration

GitHub Actions baut und veroeffentlicht Container automatisch bei:
- Push auf den `main`-Branch
- Neue Versions-Tags (`v*`)
- Pull Requests (nur Build)

Images enthalten Metadaten:
- Git-Commit-SHA
- Build-Zeitstempel
- Schwachstellen-Scan-Ergebnisse
- SBOM (Software Bill of Materials)
