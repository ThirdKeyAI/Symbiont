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

Der schnellste Weg zum Einstieg ist die Verwendung von Docker:

```bash
# Repository klonen
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont

# Einheitlichen symbi-Container erstellen
docker build -t symbi:latest .

# Oder vorgefertigten Container verwenden
docker pull ghcr.io/thirdkeyai/symbi:latest

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
- **Sandbox-Stufe**: `tier0` (keine), `tier1` (Docker) oder `tier2` (gVisor)

### Nicht-interaktiver Modus

Fuer CI/CD oder skriptbasierte Setups:

```bash
symbi init --profile assistant --schemapin tofu --sandbox tier1 --no-interact
```

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
symbi dsl -f agents/assistant.dsl   # Ihren Agenten validieren
symbi run assistant -i '{"query": "hello"}'  # Einen einzelnen Agenten testen
symbi up                             # Die Laufzeitumgebung starten
```

### Einen einzelnen Agenten ausfuehren

Verwenden Sie `symbi run`, um einen Agenten auszufuehren, ohne den vollstaendigen Laufzeit-Server zu starten:

```bash
symbi run <agent-name-oder-datei> --input <json>
```

Der Befehl loest Agentennamen auf, indem er zuerst den direkten Pfad und dann das `agents/`-Verzeichnis durchsucht. Er richtet Cloud-Inferenz ueber Umgebungsvariablen (`OPENROUTER_API_KEY`, `OPENAI_API_KEY` oder `ANTHROPIC_API_KEY`) ein, fuehrt die ORGA-Reasoning-Schleife aus und beendet sich.

```bash
symbi run assistant -i 'Summarize this document'
symbi run agents/recon.dsl -i '{"target": "10.0.1.5"}' --max-iterations 5
```

---

## Ihr erster Agent

Lassen Sie uns einen einfachen Datenanalyse-Agenten erstellen, um die Grundlagen von Symbi zu verstehen.

### 1. Agenten-Definition erstellen

Erstellen Sie eine neue Datei `my_agent.dsl`:

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
cargo run -- dsl parse my_agent.dsl

# Agent in der Laufzeitumgebung ausfuehren
cd crates/runtime && cargo run --example basic_agent -- --agent ../../my_agent.dsl
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

Einzeiler fuer Cloud-native Agenten mit LLM-Inferenz und Composio-Tool-Zugriff:

```bash
cargo build --features standalone-agent
# Aktiviert: cloud-llm + composio
```

#### Erweiterte Reasoning-Primitiven

Tool-Kuratierung, Stuck-Loop-Erkennung, Kontext-Pre-Fetch und verzeichnisspezifische Konventionen aktivieren:

```bash
cargo build --features orga-adaptive
```

Siehe den [orga-adaptive-Leitfaden](/orga-adaptive) fuer die vollstaendige Dokumentation.

#### Cedar Policy Engine

Formale Autorisierung mit der Cedar-Richtliniensprache:

```bash
cargo build --features cedar
```

#### Vektordatenbank (integriert)

Symbi enthaelt LanceDB als konfigurationsfreie eingebettete Vektordatenbank. Semantische Suche und RAG funktionieren sofort -- kein separater Dienst erforderlich:

```bash
# Agent mit RAG-Funktionen ausfuehren (Vektorsuche funktioniert sofort)
cd crates/runtime && cargo run --example rag_example

# Kontextverwaltung mit erweiterter Suche testen
cd crates/runtime && cargo run --example context_example
```

> **Enterprise-Option:** Fuer Teams, die eine dedizierte Vektordatenbank benoetigen, ist Qdrant als optionales Feature-gated Backend verfuegbar. Setzen Sie `SYMBIONT_VECTOR_BACKEND=qdrant` und `QDRANT_URL`, um es zu verwenden.

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
| `composio` | Composio MCP-Tool-Integration | Nein |
| `standalone-agent` | Cloud LLM + Composio Kombination | Nein |
| `cedar` | Cedar Policy Engine | Nein |
| `orga-adaptive` | Erweiterte Reasoning-Primitiven | Nein |
| `cron` | Persistentes Cron-Scheduling | Nein |
| `native-sandbox` | Native Prozess-Sandbox | Nein |
| `metrics` | OpenTelemetry Metriken/Tracing | Nein |
| `interactive` | Interaktive Eingabeaufforderungen fuer `symbi init` (dialoguer) | Standard |
| `full` | Alle Features ausser Enterprise | Nein |

```bash
# Mit bestimmten Features kompilieren
cargo build --features "cloud-llm,orga-adaptive,cedar"

# Mit allem kompilieren
cargo build --features full
```

---

## Konfiguration

### Umgebungsvariablen

Richten Sie Ihre Umgebung fuer optimale Leistung ein:

```bash
# Grundkonfiguration
export SYMBI_LOG_LEVEL=info
export SYMBI_RUNTIME_MODE=development

# Vektorsuche funktioniert sofort mit dem integrierten LanceDB-Backend.
# Um stattdessen Qdrant zu verwenden (optional, Enterprise):
# export SYMBIONT_VECTOR_BACKEND=qdrant
# export QDRANT_URL=http://localhost:6333

# MCP-Integration (optional)
export MCP_SERVER_URLS="http://localhost:8080"
```

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
cargo run -- dsl parse your_agent.dsl

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

Bereit, etwas Grossartiges zu bauen? Beginnen Sie mit unseren [Beispielprojekten](https://github.com/thirdkeyai/symbiont/tree/main/crates/runtime/examples) oder tauchen Sie in die [vollstaendige Spezifikation](/specification) ein.
