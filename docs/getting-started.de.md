---
layout: default
title: Erste Schritte
description: "Schnellstart-Anleitung für Symbiont"
---

# Erste Schritte
{: .no_toc }

## 🌐 Andere Sprachen
{: .no_toc}

[English](getting-started.md) | [中文简体](getting-started.zh-cn.md) | [Español](getting-started.es.md) | [Português](getting-started.pt.md) | [日本語](getting-started.ja.md) | **Deutsch**

---

Dieser Leitfaden führt Sie durch die Einrichtung von Symbi und die Erstellung Ihres ersten KI-Agenten.
{: .fs-6 .fw-300 }

## Inhaltsverzeichnis
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Voraussetzungen

Bevor Sie mit Symbi beginnen, stellen Sie sicher, dass Sie Folgendes installiert haben:

### Erforderliche Abhängigkeiten

- **Docker** (für containerisierte Entwicklung)
- **Rust 1.88+** (wenn Sie lokal kompilieren)
- **Git** (zum Klonen des Repositories)

### Optionale Abhängigkeiten

- **Qdrant** Vektordatenbank (für semantische Suchfunktionen)
- **SchemaPin Go CLI** (für Tool-Verifizierung)

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

# Entwicklungsumgebung ausführen
docker run --rm -it -v $(pwd):/workspace symbi:latest bash
```

### Option 2: Lokale Installation

Für lokale Entwicklung:

```bash
# Repository klonen
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont

# Rust-Abhängigkeiten installieren und kompilieren
cargo build --release

# Tests ausführen, um die Installation zu überprüfen
cargo test
```

### Installation überprüfen

Testen Sie, ob alles korrekt funktioniert:

```bash
# DSL-Parser testen
cd crates/dsl && cargo run && cargo test

# Laufzeitsystem testen
cd ../runtime && cargo test

# Beispiel-Agenten ausführen
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

### 2. Agent ausführen

```bash
# Agenten-Definition parsen und validieren
cargo run -- dsl parse my_agent.dsl

# Agent in der Laufzeitumgebung ausführen
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

Stellt wesentliche Informationen über Ihren Agenten für Dokumentation und Laufzeitverwaltung bereit.

### Agenten-Definition

```rust
agent agent_name(parameter: Type) -> ReturnType {
    capabilities = ["capability1", "capability2"]
    // Agenten-Implementierung
}
```

Definiert die Schnittstelle, Fähigkeiten und das Verhalten des Agenten.

### Richtlinien-Definitionen

```rust
policy policy_name {
    allow: action_list if condition
    deny: action_list if condition
    audit: operation_type with audit_method
}
```

Deklarative Sicherheitsrichtlinien, die zur Laufzeit durchgesetzt werden.

### Ausführungskontext

```rust
with memory = "persistent", privacy = "high" {
    // Agenten-Implementierung
}
```

Spezifiziert Laufzeitkonfiguration für Speicherverwaltung und Datenschutzanforderungen.

---

## Nächste Schritte

### Beispiele erkunden

Das Repository enthält mehrere Beispiel-Agenten:

```bash
# Grundlegender Agent-Beispiel
cd crates/runtime && cargo run --example basic_agent

# Vollständige Systemdemonstration
cd crates/runtime && cargo run --example full_system

# Kontext- und Speicher-Beispiel
cd crates/runtime && cargo run --example context_example

# RAG-verstärkter Agent
cd crates/runtime && cargo run --example rag_example
```

### Erweiterte Funktionen aktivieren

#### HTTP API (Optional)

```bash
# HTTP API-Funktion aktivieren
cd crates/runtime && cargo build --features http-api

# Mit API-Endpunkten ausführen
cd crates/runtime && cargo run --features http-api --example full_system
```

**Wichtige API-Endpunkte:**
- `GET /api/v1/health` - Gesundheitsprüfung und Systemstatus
- `GET /api/v1/agents` - Alle aktiven Agenten auflisten
- `POST /api/v1/workflows/execute` - Arbeitsabläufe ausführen

#### Vektordatenbank-Integration

Für semantische Suchfunktionen:

```bash
# Qdrant-Vektordatenbank starten
docker run -p 6333:6333 qdrant/qdrant

# Agent mit RAG-Funktionen ausführen
cd crates/runtime && cargo run --example rag_example
```

---

## Konfiguration

### Umgebungsvariablen

Richten Sie Ihre Umgebung für optimale Leistung ein:

```bash
# Grundkonfiguration
export SYMBI_LOG_LEVEL=info
export SYMBI_RUNTIME_MODE=development

# Vektordatenbank (optional)
export QDRANT_URL=http://localhost:6333

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
url = "http://localhost:6333"
collection_name = "symbi_knowledge"
```

---

## Häufige Probleme

### Docker-Probleme

**Problem**: Docker-Build schlägt mit Berechtigungsfehlern fehl
```bash
# Lösung: Sicherstellen, dass Docker-Daemon läuft und Benutzer Berechtigungen hat
sudo systemctl start docker
sudo usermod -aG docker $USER
```

**Problem**: Container beendet sich sofort
```bash
# Lösung: Docker-Logs überprüfen
docker logs <container_id>
```

### Rust-Build-Probleme

**Problem**: Cargo-Build schlägt mit Abhängigkeitsfehlern fehl
```bash
# Lösung: Rust aktualisieren und Build-Cache löschen
rustup update
cargo clean
cargo build
```

**Problem**: Fehlende Systemabhängigkeiten
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
# Agenten-Definitionssyntax überprüfen
cargo run -- dsl parse your_agent.dsl

# Debug-Protokollierung aktivieren
RUST_LOG=debug cd crates/runtime && cargo run --example basic_agent
```

---

## Hilfe erhalten

### Dokumentation

- **[DSL-Leitfaden](/dsl-guide)** - Vollständige DSL-Referenz
- **[Laufzeit-Architektur](/runtime-architecture)** - Details zur Systemarchitektur
- **[Sicherheitsmodell](/security-model)** - Sicherheits- und Richtliniendokumentation

### Community-Support

- **Issues**: [GitHub Issues](https://github.com/thirdkeyai/symbiont/issues)
- **Diskussionen**: [GitHub Discussions](https://github.com/thirdkeyai/symbiont/discussions)
- **Dokumentation**: [Vollständige API-Referenz](https://docs.symbiont.platform)

### Debug-Modus

Für die Fehlerbehebung detaillierte Protokollierung aktivieren:

```bash
# Debug-Protokollierung aktivieren
export RUST_LOG=symbi=debug

# Mit detaillierter Ausgabe ausführen
cd crates/runtime && cargo run --example basic_agent 2>&1 | tee debug.log
```

---

## Was kommt als nächstes?

Jetzt, da Sie Symbi zum Laufen gebracht haben, erkunden Sie diese fortgeschrittenen Themen:

1. **[DSL-Leitfaden](/dsl-guide)** - Erweiterte DSL-Funktionen lernen
2. **[Laufzeit-Architektur](/runtime-architecture)** - Systeminternas verstehen
3. **[Sicherheitsmodell](/security-model)** - Sicherheitsrichtlinien implementieren
4. **[Beitragen](/contributing)** - Zum Projekt beitragen

Bereit, etwas Großartiges zu bauen? Beginnen Sie mit unseren [Beispielprojekten](https://github.com/thirdkeyai/symbiont/tree/main/crates/runtime/examples) oder tauchen Sie in die [vollständige Spezifikation](/specification) ein.