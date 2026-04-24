# Docker-Container-Leitfaden

## Andere Sprachen


---

Symbi stellt einen einheitlichen Docker-Container mit allen Funktionen bereit, verfuegbar ueber die GitHub Container Registry.

## Verfuegbares Image

### Einheitlicher Symbi-Container
- **Image**: `ghcr.io/thirdkeyai/symbi:latest`
- **Zweck**: All-in-One-Container mit DSL-Parsing, Agenten-Laufzeit und MCP-Server
- **Groesse**: ~80MB (inkl. Vektordatenbank und HTTP-API-Unterstuetzung)
- **CLI**: Einheitlicher `symbi`-Befehl mit Unterbefehlen fuer verschiedene Operationen

## Schnellstart

### Projekt erstellen und starten (empfohlen)

`symbi init` funktioniert innerhalb des Containers und schreibt ein Projekt in Ihr Host-Verzeichnis, einschliesslich einer sofort lauffaehigen `docker-compose.yml` und einer `.env` mit einem frisch generierten `SYMBIONT_MASTER_KEY`:

```bash
# 1. Projektdateien auf dem Host erstellen
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest \
  init --profile assistant --no-interact --dir /workspace

# 2. Runtime starten (liest .env automatisch)
docker compose up
```

Das Flag `--dir /workspace` weist `symbi init` an, in das gemountete Volume zu schreiben anstatt in das WORKDIR des Images. Nach der Ausfuehrung haben Sie `symbiont.toml`, `agents/`, `policies/`, `.symbiont/audit/`, `AGENTS.md`, `docker-compose.yml`, `.env` und `.env.example` im aktuellen Verzeichnis.

Um die Generierung der Compose-Datei zu ueberspringen:

```bash
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest \
  init --profile minimal --no-interact --no-docker-compose --dir /workspace
```

### Vorgefertigtes Image verwenden (ad-hoc)

```bash
# Neuestes Image herunterladen
docker pull ghcr.io/thirdkeyai/symbi:latest

# Eine DSL-Datei parsen
docker run --rm -v $(pwd):/workspace \
  ghcr.io/thirdkeyai/symbi:latest \
  dsl --file /workspace/agent.dsl

# MCP-Server starten (stdio-basiert, kein Port erforderlich)
docker run --rm -i \
  ghcr.io/thirdkeyai/symbi:latest \
  mcp

# Runtime ohne Projekt starten (ephemer, kein Master Key)
docker run --rm -p 8080:8080 -p 8081:8081 \
  ghcr.io/thirdkeyai/symbi:latest \
  up --http-bind 0.0.0.0
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
  -p 8081:8081 \
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
docker run --rm -v $(pwd):/workspace symbi:latest dsl --help

# MCP-Server testen
docker run --rm symbi:latest mcp
```

## Multi-Architektur-Unterstuetzung

Images werden gebaut fuer:
- `linux/amd64` (x86_64)
- `linux/arm64` (ARM64/Apple Silicon)

Docker laedt automatisch die korrekte Architektur fuer Ihre Plattform herunter.

## Sicherheitsfunktionen

### Ausfuehrung als Nicht-Root
- Container laufen als Nicht-Root-Benutzer `symbi` (UID 1000)
- Minimale Angriffsflaeche mit sicherheitsgehaerteten Basis-Images

### Schwachstellenscanning
- Alle Images werden automatisch mit Trivy gescannt
- Sicherheitshinweise werden im GitHub-Security-Tab veroeffentlicht
- SARIF-Berichte fuer detaillierte Schwachstellenanalyse

## Konfiguration

### Umgebungsvariablen

**Symbi-Container:**
- `SYMBIONT_MASTER_KEY` - **Erforderlich fuer persistenten Zustand.** 32-Byte-Hex-Schluessel zur Verschluesselung des lokalen Speichers. Generieren mit `openssl rand -hex 32`. `symbi init` schreibt automatisch einen in `.env`.
- `RUST_LOG` - Logging-Level festlegen (debug, info, warn, error)
- `SYMBIONT_VECTOR_BACKEND` - Vektor-Backend: `lancedb` (Standard) oder `qdrant`
- `QDRANT_URL` - Qdrant-Vektordatenbank-URL (nur bei Verwendung des optionalen Qdrant-Backends)
- `OPENROUTER_API_KEY` / `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` - Optionale LLM-Zugangsdaten; schon eine davon aktiviert den Coordinator-Chat-Endpunkt.

### Volume-Mounts

Das Image laeuft als Benutzer `symbi` (UID 1000) mit `WORKDIR=/var/lib/symbi`. Projektdateien werden schreibgeschuetzt in dieses Verzeichnis gemountet; persistente Zustaende (der lokale SQLite-Speicher und Audit-Logs) liegen in benannten Volumes, damit sie Container-Neustarts ueberdauern.

```bash
# Projektdateien (schreibgeschuetzt)
-v $(pwd)/symbiont.toml:/var/lib/symbi/symbiont.toml:ro
-v $(pwd)/agents:/var/lib/symbi/agents:ro
-v $(pwd)/policies:/var/lib/symbi/policies:ro
-v $(pwd)/tools:/var/lib/symbi/tools:ro

# Persistenter Zustand
-v symbi-data:/var/lib/symbi/.symbi
-v symbi-audit:/var/lib/symbi/.symbiont
```

## Docker Compose Beispiel

`symbi init` erzeugt eine sofort lauffaehige `docker-compose.yml`, die mit dem Rest dieses Abschnitts uebereinstimmt — bevorzugen Sie das gegenueber einer handgeschriebenen Compose-Datei. Als Referenz oder wenn Sie ohne `init` starten:

Standardmaessig verwendet Symbiont **LanceDB** als eingebettete Vektordatenbank -- keine externen Dienste erforderlich. Wenn Sie ein verteiltes Vektor-Backend fuer skalierte Deployments benoetigen, koennen Sie optional Qdrant hinzufuegen.

### Minimal (LanceDB-Standard -- kein Qdrant erforderlich)

Kombinieren Sie dies mit einer `.env`-Datei, die `SYMBIONT_MASTER_KEY` setzt:

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

### Mit optionalem Qdrant-Backend

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
docker run --name symbi-test -d ghcr.io/thirdkeyai/symbi:latest up --http-bind 0.0.0.0:8080
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
