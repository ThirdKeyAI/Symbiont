# Erste Schritte

Dieser Leitfaden fuehrt Sie durch die Einrichtung von Symbi und die Erstellung Ihres ersten KI-Agenten.

## Inhaltsverzeichnis


---

## Voraussetzungen

Bevor Sie mit Symbi beginnen, stellen Sie sicher, dass Sie Folgendes installiert haben:

### Erforderliche Abhaengigkeiten

- **Docker** (fuer containerisierte Entwicklung)
- **Rust 1.82+** (wenn Sie lokal kompilieren)
- **Git** (zum Klonen des Repositories)

> **Hinweis:** Vektorsuche ist integriert. Symbi wird mit [LanceDB](https://lancedb.com/) als eingebetteter Vektordatenbank ausgeliefert -- kein externer Dienst erforderlich.

---

## Installation

### Option 1: Docker (Empfohlen)

Der schnellste Weg zu einem lauffaehigen Runtime ist, den Container das Projekt fuer Sie erstellen zu lassen:

```bash
# 1. symbiont.toml, agents/, policies/, docker-compose.yml und
#    eine .env mit einem frisch generierten SYMBIONT_MASTER_KEY erzeugen.
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest \
  init --profile assistant --no-interact --dir /workspace

# 2. Runtime starten. Liest .env automatisch.
docker compose up
```

Die Runtime-API ist jetzt unter `http://localhost:8080` und HTTP Input unter `http://localhost:8081` erreichbar.

Wenn Sie lieber aus einem Klon arbeiten moechten (um das Image selbst zu bauen oder Tests auszufuehren):

```bash
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont

# Einheitlichen symbi-Container erstellen
docker build -t symbi:latest .

# Entwicklungsumgebung ausfuehren
docker run --rm -it -v $(pwd):/workspace symbi:latest bash
```

### Option 2: Lokale Installation

Fuer lokale Entwicklung:

```bash
# Repository klonen
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont

# Rust-Abhaengigkeiten installieren und kompilieren
cargo build --release

# Tests ausfuehren, um die Installation zu ueberpruefen
cargo test
```

### Installation ueberpruefen

Testen Sie, ob alles korrekt funktioniert:

```bash
# DSL-Parser testen
cd crates/dsl && cargo run && cargo test

# Laufzeitsystem testen
cd ../runtime && cargo test

# Beispiel-Agenten ausfuehren
cargo run --example basic_agent
cargo run --example full_system

# Einheitliche symbi CLI testen
cd ../.. && cargo run -- dsl --help
cargo run -- mcp --help

# Mit Docker-Container testen
docker run --rm symbi:latest --version
docker run --rm -v $(pwd):/workspace symbi:latest dsl parse --help
docker run --rm symbi:latest mcp --help
```

---

## Projektinitialisierung

Der schnellste Weg, ein neues Symbiont-Projekt zu starten, ist `symbi init`:

```bash
symbi init
```

Dies startet einen interaktiven Assistenten, der Sie durch Folgendes fuehrt:
- **Profilauswahl**: `minimal`, `assistant`, `dev-agent` oder `multi-agent`
- **SchemaPin-Modus**: `tofu` (Trust-On-First-Use), `strict` oder `disabled`
- **Sandbox-Stufe**: `tier0` (keine, nur Entwicklung), `tier1` (Docker), `tier2` (gVisor / `runsc`) oder `tier3` (Firecracker microVM)

### Was `init` erzeugt

Jeder Durchlauf schreibt:

| Datei | Zweck |
|-------|-------|
| `symbiont.toml` | Runtime- und Richtlinienkonfiguration |
| `policies/default.cedar` | Cedar-Richtlinie nach dem Deny-by-default-Prinzip |
| `agents/*.symbi` | Profilspezifische Agentendefinitionen (das Legacy-Format `.dsl` wird ebenfalls erkannt; ausser bei `minimal`) |
| `AGENTS.md` | Automatisch generierter Index der deklarierten Agenten |
| `.symbiont/audit/` | Verzeichnis fuer manipulationssichere Audit-Logs |
| `.gitignore` | Ergaenzt um Symbiont-spezifische Eintraege, einschliesslich `.env` |
| `.env` | `SYMBIONT_MASTER_KEY` aus `/dev/urandom` generiert (0600-Berechtigungen) |
| `.env.example` | Zum Commit geeignete Vorlage mit den benoetigten Umgebungsvariablen |
| `docker-compose.yml` | Sofort lauffaehige Compose-Datei mit korrekten Volume-Mounts und Env-Verdrahtung |

Mit `--no-docker-compose` ueberspringen Sie die Compose-Datei, und mit `--dir <PATH>` schreiben Sie in ein anderes als das aktuelle Verzeichnis (unverzichtbar innerhalb eines Docker-Containers — siehe unten).

### Nicht-interaktiver Modus

Fuer CI/CD oder skriptbasierte Setups:

```bash
symbi init --profile assistant --schemapin tofu --sandbox tier1 --no-interact
```

### `init` innerhalb von Docker ausfuehren

Da das WORKDIR des Images `/var/lib/symbi` ist, verwenden Sie `--dir`, um in Ihr gemountetes Volume zu schreiben:

```bash
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest \
  init --profile assistant --no-interact --dir /workspace
```

Das fuellt das aktuelle Verzeichnis des Hosts mit dem vollstaendigen Projektbaum.

### Profile

| Profil | Was erstellt wird |
|--------|------------------|
| `minimal` | `symbiont.toml` + Standard-Cedar-Richtlinie |
| `assistant` | + einzelner verwalteter Assistenten-Agent |
| `dev-agent` | + CliExecutor-Agent mit Sicherheitsrichtlinien |
| `multi-agent` | + Koordinator-/Worker-Agenten mit Inter-Agent-Richtlinien |

### Aus dem Katalog importieren

Vorgefertigte Agenten neben jedem Profil importieren:

```bash
symbi init --profile minimal --no-interact
symbi init --catalog assistant,dev
```

Verfuegbare Katalog-Agenten auflisten:

```bash
symbi init --catalog list
```

Nach der Initialisierung validieren und starten:

```bash
symbi dsl -f agents/assistant.symbi   # Ihren Agenten validieren
symbi run assistant -i '{"query": "hello"}'  # Einen einzelnen Agenten testen
symbi up                             # Runtime lokal starten
docker compose up                    # ...oder in Docker starten (liest .env)
```

### Einen einzelnen Agenten ausfuehren

Verwenden Sie `symbi run`, um einen Agenten auszufuehren, ohne den vollstaendigen Laufzeit-Server zu starten:

```bash
symbi run <agent-name-oder-datei> --input <json>
```

Der Befehl loest Agentennamen auf, indem er zuerst den direkten Pfad und dann das `agents/`-Verzeichnis durchsucht. Er richtet Cloud-Inferenz ueber Umgebungsvariablen (`OPENROUTER_API_KEY`, `OPENAI_API_KEY` oder `ANTHROPIC_API_KEY`) ein, fuehrt die ORGA-Reasoning-Schleife aus und beendet sich.

```bash
symbi run assistant -i 'Summarize this document'
symbi run agents/recon.symbi -i '{"target": "10.0.1.5"}' --max-iterations 5
```

### Aus einer Vorlage starten (`symbi new`)

`symbi init` erstellt ein generisches Projekt; `symbi new` erstellt ein Projekt rund um eine von mehreren aufgabenorientierten Vorlagen. Nuetzlich, wenn Sie die Art des benoetigten Agenten kennen, bevor Sie wissen, welche Agenten Sie brauchen.

```bash
symbi new --list                     # verfuegbare Vorlagen anzeigen
symbi new <template> <project-name>  # neues Projekt aus einer Vorlage erstellen
```

Mitgelieferte Vorlagen:

| Vorlage | Was Sie erhalten |
|---------|------------------|
| `webhook-min` | Minimaler Webhook-gesteuerter Agent -- HTTP-Input-Konfiguration + Handler-DSL |
| `webscraper-agent` | Scraping-Agent mit Cedar-Zugriffsrichtlinien und einem ToolClad-Scraper-Tool |
| `slm-first` | Router + SLM-Allowlist + Confidence-Fallback-Muster |
| `rag-lite` | Qdrant-gestuetzte Ingestion-Skripte plus ein Such-Agent |

`symbi new` und `symbi init` ergaenzen sich: `new` liefert einen aufgabenspezifischen Ausgangspunkt, `init` (+ `--catalog`) einen governance-spezifischen. Sie koennen beides kombinieren -- mit `new` einsteigen und anschliessend `symbi init --catalog ...` verwenden, um zusaetzliche vorgefertigte Agenten aus dem Katalog einzubinden.

---

## Ihr erster Agent

Lassen Sie uns einen einfachen Datenanalyse-Agenten erstellen, um die Grundlagen von Symbi zu verstehen.

### 1. Agenten-Definition erstellen

Erstellen Sie eine neue Datei `my_agent.symbi`:

```rust
metadata {
    version = "1.0.0"
    author = "your-name"
    description = "My first Symbi agent"
}

agent greet_user(name: String) -> String {
    capabilities = ["greeting", "text_processing"]

    policy safe_greeting {
        allow: read(name) if name.length <= 100
        deny: store(name) if name.contains_sensitive_data
        audit: all_operations with signature
    }

    with memory = "ephemeral", privacy = "low" {
        if (validate_name(name)) {
            greeting = format_greeting(name);
            audit_log("greeting_generated", greeting.metadata);
            return greeting;
        } else {
            return "Hello, anonymous user!";
        }
    }
}
```

### 2. Agent ausfuehren

```bash
# Agenten-Definition parsen und validieren
cargo run -- dsl parse my_agent.symbi

# Agent in der Laufzeitumgebung ausfuehren
cd crates/runtime && cargo run --example basic_agent -- --agent ../../my_agent.symbi
```

---

## Die DSL verstehen

Die Symbi DSL hat mehrere Hauptkomponenten:

### Metadaten-Block

```rust
metadata {
    version = "1.0.0"
    author = "developer"
    description = "Agent description"
}
```

Stellt wesentliche Informationen ueber Ihren Agenten fuer Dokumentation und Laufzeitverwaltung bereit.

### Agenten-Definition

```rust
agent agent_name(parameter: Type) -> ReturnType {
    capabilities = ["capability1", "capability2"]
    // Agenten-Implementierung
}
```

Definiert die Schnittstelle, Faehigkeiten und das Verhalten des Agenten.

### Richtlinien-Definitionen

```rust
policy policy_name {
    allow: action_list if condition
    deny: action_list if condition
    audit: operation_type with audit_method
}
```

Deklarative Sicherheitsrichtlinien, die zur Laufzeit durchgesetzt werden.

### Ausfuehrungskontext

```rust
with memory = "persistent", privacy = "high" {
    // Agenten-Implementierung
}
```

Spezifiziert Laufzeitkonfiguration fuer Speicherverwaltung und Datenschutzanforderungen.

---

## Naechste Schritte

### Beispiele erkunden

Das Repository enthaelt mehrere Beispiel-Agenten:

```bash
# Grundlegender Agent-Beispiel
cd crates/runtime && cargo run --example basic_agent

# Vollstaendige Systemdemonstration
cd crates/runtime && cargo run --example full_system

# Kontext- und Speicher-Beispiel
cd crates/runtime && cargo run --example context_example

# RAG-verstaerkter Agent
cd crates/runtime && cargo run --example rag_example
```

### Erweiterte Funktionen aktivieren

#### HTTP API (Optional)

```bash
# HTTP API-Funktion aktivieren
cd crates/runtime && cargo build --features http-api

# Mit API-Endpunkten ausfuehren
cd crates/runtime && cargo run --features http-api --example full_system
```

**Wichtige API-Endpunkte:**
- `GET /api/v1/health` - Gesundheitspruefung und Systemstatus
- `GET /api/v1/agents` - Alle aktiven Agenten mit Echtzeit-Ausfuehrungsstatus auflisten
- `GET /api/v1/agents/{id}/status` - Detaillierte Agent-Ausfuehrungsmetriken abrufen
- `POST /api/v1/workflows/execute` - Workflows ausfuehren

**Neue Agent-Management-Funktionen:**
- Echtzeit-Prozessueberwachung und Gesundheitspruefungen
- Ordnungsgemaesse Shutdown-Faehigkeiten fuer laufende Agenten
- Umfassende Ausfuehrungsmetriken und Ressourcennutzungsverfolgung
- Unterstuetzung verschiedener Ausfuehrungsmodi (ephemer, persistent, geplant, ereignisgesteuert)

#### Cloud-LLM-Inferenz

Verbindung zu Cloud-LLM-Anbietern ueber OpenRouter:

```bash
# Cloud-Inferenz aktivieren
cargo build --features cloud-llm

# API-Schluessel und Modell festlegen
export OPENROUTER_API_KEY="sk-or-..."
export OPENROUTER_MODEL="google/gemini-2.0-flash-001"  # optional
```

#### Standalone Agent-Modus

Einzeiler fuer Cloud-native Agenten mit LLM-Inferenz:

```bash
cargo build --features standalone-agent
# Aktiviert: cloud-llm
```

> **Hinweis:** Die Composio-MCP- und SymbiBot-Integration wurden in dieser Version aus Sicherheitsgruenden entfernt -- siehe SECURITY_AUDIT.md C3 fuer den Hintergrund.

#### Erweiterte Reasoning-Primitiven

Tool-Kuratierung, Stuck-Loop-Erkennung, Kontext-Pre-Fetch und verzeichnisspezifische Konventionen aktivieren:

```bash
cargo build --features orga-adaptive
```

Siehe den [orga-adaptive-Leitfaden](/orga-adaptive) fuer die vollstaendige Dokumentation.

#### Cedar Policy Engine

Formale Autorisierung mit der Cedar-Richtliniensprache. **Standardmaessig aktiv seit v1.14.x**: Veroeffentlichte `symbi`-Binaries (crates.io, Docker, GitHub-Release-Tarballs) enthalten Cedar, und `symbi up` / `symbi run` verdrahten `CedarPolicyGate` beim Start automatisch aus `policies/*.cedar`-Dateien; sind keine vorhanden, faellt die Runtime auf den fail-closed `DefaultPolicyGate` zurueck. Um ohne Cedar zu bauen (z. B. wenn Sie stattdessen `OpaPolicyGateBridge` oder ein eigenes `ReasoningPolicyGate` anbinden moechten), verwenden Sie:

```bash
cargo build --no-default-features --features "keychain,vector-lancedb"  # cedar weglassen
```

#### Vektordatenbank (integriert)

Symbi enthaelt LanceDB als konfigurationsfreie eingebettete Vektordatenbank. Semantische Suche und RAG funktionieren sofort -- kein separater Dienst erforderlich:

```bash
# Agent mit RAG-Funktionen ausfuehren (Vektorsuche funktioniert sofort)
cd crates/runtime && cargo run --example rag_example

# Kontextverwaltung mit erweiterter Suche testen
cd crates/runtime && cargo run --example context_example
```

> **Minimaler Build:** LanceDB ist standardmaessig enthalten, kann aber fuer schlankere Binaries ausgeschlossen werden: `cargo build --no-default-features`. Die Runtime faellt sauber auf ein No-op-Vektor-Backend zurueck.
>
> **Skalierte Deployments:** Qdrant ist als optionales Backend verfuegbar. Bauen Sie mit `--features vector-qdrant` und setzen Sie `SYMBIONT_VECTOR_BACKEND=qdrant`.

**Kontextverwaltungsfunktionen:**
- **Multi-modale Suche**: Schluesselwort-, zeitliche, Aehnlichkeits- und Hybrid-Suchmodi
- **Wichtigkeitsberechnung**: Ausgefeilter Bewertungsalgorithmus unter Beruecksichtigung von Zugriffsmustern, Aktualitaet und Benutzerfeedback
- **Zugriffskontrolle**: Integration der Richtlinien-Engine mit agentenspezifischen Zugriffskontrollen
- **Automatische Archivierung**: Aufbewahrungsrichtlinien mit komprimiertem Speicher und Bereinigung
- **Wissensaustausch**: Sicherer agentenuebergreifender Wissensaustausch mit Vertrauensbewertungen

#### Feature Flags Referenz

| Feature | Beschreibung | Standard |
|---------|-------------|---------|
| `keychain` | OS-Keychain-Integration fuer Geheimnisse | Ja |
| `vector-lancedb` | LanceDB eingebettetes Vektor-Backend | Ja |
| `vector-qdrant` | Qdrant verteiltes Vektor-Backend | Nein |
| `embedding-models` | Lokale Embedding-Modelle ueber Candle | Nein |
| `http-api` | REST API mit Swagger UI | Nein |
| `http-input` | Webhook-Server mit JWT-Authentifizierung | Nein |
| `cloud-llm` | Cloud-LLM-Inferenz (OpenRouter) | Nein |
| `standalone-agent` | Cloud LLM Meta-Feature | Nein |
| `cedar` | Cedar Policy Engine — verdrahtet sich beim Start automatisch aus `policies/*.cedar` | **Yes** |
| `orga-adaptive` | Erweiterte Reasoning-Primitiven | Nein |
| `cron` | Persistentes Cron-Scheduling | Nein |
| `cli-executor` | Verwaltete KI-CLI-Subprozesse (Claude Code etc.) — Mode B | **Yes** |
| `native-sandbox` | Native Prozess-Sandbox | Nein |
| `metrics` | OpenTelemetry Metriken/Tracing | Nein |
| `interactive` | Interaktive Eingabeaufforderungen fuer `symbi init` (dialoguer) | Standard |
| `full` | Alle optionalen Runtime-, Vektor- und Policy-Features | Nein |

```bash
# Mit bestimmten Features kompilieren
cargo build --features "cloud-llm,orga-adaptive,cedar"

# Mit allem kompilieren
cargo build --features full
```

---

## KI-Assistenten-Plugins

Symbiont bietet First-Party-Governance-Plugins fuer gaengige KI-Coding-Assistenten mit drei progressiven Schutzstufen:

1. **Awareness** (Standard) — beratende Protokollierung aller zustandsaendernden Tool-Aufrufe
2. **Protection** — ein blockierender Hook setzt eine lokale Deny-Liste durch (`.symbiont/local-policy.toml`)
3. **Governance** — Cedar-Richtlinienauswertung, wenn `symbi` im PATH liegt

Die Deny-Listen-Konfiguration ist tool-unabhaengig — dieselbe `.symbiont/local-policy.toml` funktioniert mit beiden Plugins:

```toml
[deny]
paths = [".env", ".ssh/", ".aws/"]
commands = ["rm -rf", "git push --force"]
branches = ["main", "master", "production"]
```

### Claude Code

```bash
# Install from marketplace
/plugin marketplace add https://github.com/thirdkeyai/symbi-claude-code

# Available skills: /symbi-init, /symbi-policy, /symbi-verify, /symbi-audit, /symbi-dsl
```

Siehe [symbi-claude-code](https://github.com/thirdkeyai/symbi-claude-code) fuer Details.

#### Mode B: verwalteter Claude-Code-Subprozess

Ueber die In-Editor-Hooks hinaus kann Symbiont Claude Code als *verwalteten
Subprozess* ausfuehren — den "Mode B"-Pfad (ORGA-verwaltet). Ein Agent, dessen
Metadaten `executor = "claude_code"` deklarieren, laeuft, indem Claude Code unter
dem `CliExecutor` der Runtime gestartet wird, anstatt die LLM-Reasoning-Schleife
zu verwenden. Der mitgelieferte `code_reviewer`-Agent ist das Referenzbeispiel:

```bash
# Review a working tree with a governed Claude Code subprocess
symbi run code_reviewer --target /path/to/repo

# Bounds: --max-turns is the primary (cooperative) limit; --budget-timeout is a
# hard wall-clock backstop (graceful SIGTERM -> SIGKILL).
symbi run code_reviewer --target . --max-turns 12 --budget-timeout 15m
```

Bei jedem Durchlauf macht Symbiont Folgendes:

- es wertet den Start ueber das Policy-**Gate** aus (fail-closed — erlauben Sie ihn
  ueber eine Cedar-Richtlinie oder mit `SYMBI_INSECURE_ALLOW_ALL=1` fuer die lokale
  Entwicklung);
- es setzt den Env-Handshake (`SYMBIONT_MANAGED=true`, `SYMBIONT_SESSION_ID`,
  `SYMBIONT_BUDGET_TOKENS`, `SYMBIONT_BUDGET_TIMEOUT`, `CLAUDE_PROJECT_DIR`), damit das
  symbi-claude-code-Plugin seine Hooks an das aeussere Gate **delegiert**;
- es laedt das Plugin ueber `--plugin-dir` und verdrahtet den stdio-`symbi mcp`-Rueckkanal
  ueber `--mcp-config --strict-mcp-config`;
- es fuehrt Claude Code headless aus (`--print --output-format json --permission-mode dontAsk`).

| Variable / flag | Zweck | Standard |
|---|---|---|
| `SYMBIONT_CLAUDE_PLUGIN_DIR` | Pfad zum symbi-claude-code-Plugin | autodetect sibling repo |
| `--plugin-dir` | Plugin-Pfad fuer einen Durchlauf ueberschreiben | — |
| `--target` | Arbeitsverzeichnis, auf dem operiert wird | current dir |
| `--max-turns` | Primaere kooperative Schranke (agentische Turns) | 12 |
| `--budget-timeout` | Wall-clock-Backstop, z. B. `15m` / `900s` | 15m |
| `--budget-tokens` | An den Subprozess uebergebener Token-Budget-Hinweis (zur Information) | 100000 |

> **Auth:** Der Subprozess verwendet die eigene Authentifizierung von Claude Code — eine
> angemeldete Sitzung (`claude /login`) oder `ANTHROPIC_API_KEY`. Das `cli-executor`-Feature
> ist standardmaessig aktiviert.

### Gemini CLI

```bash
# Install extension
gemini extensions install https://github.com/thirdkeyai/symbi-gemini-cli
```

Die Gemini-CLI-Erweiterung bietet zusaetzliche Defense-in-Depth durch `excludeTools`-Manifest-Blockierung und native `policies/*.toml`-Durchsetzung auf Plattformebene.

Siehe [symbi-gemini-cli](https://github.com/thirdkeyai/symbi-gemini-cli) fuer Details.

---

## Konfiguration

### Umgebungsvariablen

Richten Sie Ihre Umgebung fuer optimale Leistung ein:

```bash
# Erforderlich: 32-Byte-Hex-Schluessel zum Verschluesseln persistenter Zustaende.
# Generieren mit: openssl rand -hex 32
# `symbi init` schreibt automatisch einen in .env.
export SYMBIONT_MASTER_KEY="..."

# Grundkonfiguration
export SYMBI_LOG_LEVEL=info
export SYMBI_RUNTIME_MODE=development

# Vektorsuche funktioniert sofort mit dem integrierten LanceDB-Backend.
# Um stattdessen Qdrant zu verwenden (optional, aktiviert das `vector-qdrant`-Feature):
# export SYMBIONT_VECTOR_BACKEND=qdrant
# export QDRANT_URL=http://localhost:6333

# MCP-Integration (optional)
export MCP_SERVER_URLS="http://localhost:8080"
```

#### Sicherheitsrelevante Umgebungsvariablen (nach dem v1.13.0-Audit)

| Variable | Standard | Wirkung |
|---|---|---|
| `SYMBI_INSECURE_ALLOW_ALL` | nicht gesetzt | Wenn auf `1` gesetzt, verwenden `symbi up` / `symbi run` das permissive Policy-Gate (jeder Tool-Aufruf und jede Delegation ist erlaubt). Entspricht dem Flag `--insecure-allow-all`. Ein deutlicher stderr-Banner wird ausgegeben. **Nur fuer lokale Entwicklung.** Ohne diese Variable ist die Reasoning-Schleife fail-closed und lehnt Tool-Aufrufe sowie Delegationen ab, bis ein explizites Policy-Backend angebunden ist. |
| `SYMBI_REJECT_LEGACY_API_KEYS` | nicht gesetzt | Wenn auf `1` gesetzt, ueberspringt der API-Key-Validator den veralteten O(n)-Argon2-Scan fuer Schluessel ohne Praefix. Setzen Sie diese Variable unmittelbar nach der Neuausgabe aller Schluessel im Format `keyid.secret`. Der Legacy-Pfad wird im naechsten Minor-Release ohnehin entfernt. |
| `SYMBI_UNSAFE_NATIVE_SANDBOX` | nicht gesetzt | Erforderlich (zusaetzlich dazu, dass `SYMBI_ENV=production` nicht gesetzt ist), um den `native`-Sandbox-Runner zu konstruieren. Das Cargo-Feature `native-sandbox` schlaegt in Release-Builds zudem beim Kompilieren fehl. Der native Runner bietet keinerlei Isolation und ist ausschliesslich fuer lokales Debugging gedacht. |
| `SYMBI_TRUSTED_PROXIES` | nicht gesetzt | CIDR-Allowlist fuer vertrauenswuerdige Reverse-Proxies; `X-Forwarded-For` wird nur von diesen Adressen akzeptiert. |

Folgende Umgebungsvariablen wurden **entfernt**:

- `SYMBIONT_ALLOW_NO_JWT_AUDIENCE` -- der JWT-Verifizierer verlangt jetzt immer `aud`. (Im Audit nach v1.13.0 entfernt; war ein unsicherer Notausgang.)
- `COMPOSIO_API_KEY`, `COMPOSIO_MCP_URL` -- die Composio-MCP-Integration wurde vollstaendig entfernt. Siehe `SECURITY_AUDIT.md` C3.

### Laufzeitkonfiguration

Erstellen Sie eine `symbi.toml`-Konfigurationsdatei:

```toml
[runtime]
max_agents = 1000
memory_limit_mb = 512
execution_timeout_seconds = 300

[security]
default_sandbox_tier = "docker"
audit_enabled = true
policy_enforcement = "strict"

[vector_db]
enabled = true
backend = "lancedb"              # Standard; unterstuetzt auch "qdrant"
collection_name = "symbi_knowledge"
# url = "http://localhost:6333"  # nur erforderlich bei backend = "qdrant"
```

---

## Haeufige Probleme

### Docker-Probleme

**Problem**: Docker-Build schlaegt mit Berechtigungsfehlern fehl
```bash
# Loesung: Sicherstellen, dass Docker-Daemon laeuft und Benutzer Berechtigungen hat
sudo systemctl start docker
sudo usermod -aG docker $USER
```

**Problem**: Container beendet sich sofort
```bash
# Loesung: Docker-Logs ueberpruefen
docker logs <container_id>
```

### Rust-Build-Probleme

**Problem**: Cargo-Build schlaegt mit Abhaengigkeitsfehlern fehl
```bash
# Loesung: Rust aktualisieren und Build-Cache loeschen
rustup update
cargo clean
cargo build
```

**Problem**: Fehlende Systemabhaengigkeiten
```bash
# Ubuntu/Debian
sudo apt-get update
sudo apt-get install build-essential pkg-config libssl-dev

# macOS
brew install pkg-config openssl
```

### Laufzeit-Probleme

**Problem**: Agent startet nicht
```bash
# Agenten-Definitionssyntax ueberpruefen
cargo run -- dsl parse your_agent.symbi

# Debug-Protokollierung aktivieren
RUST_LOG=debug cd crates/runtime && cargo run --example basic_agent
```

---

## Hilfe erhalten

### Dokumentation

- **[DSL-Leitfaden](/dsl-guide)** - Vollstaendige DSL-Referenz
- **[Laufzeit-Architektur](/runtime-architecture)** - Details zur Systemarchitektur
- **[Sicherheitsmodell](/security-model)** - Sicherheits- und Richtliniendokumentation

### Community-Support

- **Issues**: [GitHub Issues](https://github.com/thirdkeyai/symbiont/issues)
- **Diskussionen**: [GitHub Discussions](https://github.com/thirdkeyai/symbiont/discussions)
- **Dokumentation**: [Vollstaendige API-Referenz](https://docs.symbiont.dev/api-reference)

### Debug-Modus

Fuer die Fehlerbehebung detaillierte Protokollierung aktivieren:

```bash
# Debug-Protokollierung aktivieren
export RUST_LOG=symbi=debug

# Mit detaillierter Ausgabe ausfuehren
cd crates/runtime && cargo run --example basic_agent 2>&1 | tee debug.log
```

---

## Was kommt als naechstes?

Jetzt, da Sie Symbi zum Laufen gebracht haben, erkunden Sie diese fortgeschrittenen Themen:

1. **[DSL-Leitfaden](/dsl-guide)** - Erweiterte DSL-Funktionen lernen
2. **[Reasoning-Loop-Leitfaden](/reasoning-loop)** - Den ORGA-Zyklus verstehen
3. **[Erweitertes Reasoning (orga-adaptive)](/orga-adaptive)** - Tool-Kuratierung, Stuck-Loop-Erkennung, Pre-Hydration
4. **[Laufzeit-Architektur](/runtime-architecture)** - Systeminternas verstehen
5. **[Sicherheitsmodell](/security-model)** - Sicherheitsrichtlinien implementieren
6. **[Beitragen](/contributing)** - Zum Projekt beitragen

Bereit, etwas Grossartiges zu bauen? Beginnen Sie mit unseren [Beispielprojekten](https://github.com/thirdkeyai/symbiont/tree/main/crates/runtime/examples) oder tauchen Sie in die [vollstaendige Spezifikation](/dsl-specification) ein.
