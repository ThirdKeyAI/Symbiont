<img src="logo-hz.png" alt="Symbi">

**Symbi** ist ein KI-natives Agentenframework zum Aufbau autonomer und richtlinienbewusster Agenten, die sicher mit Menschen, anderen Agenten und großen Sprachmodellen zusammenarbeiten können. Die Community Edition bietet Kernfunktionalität mit optionalen Enterprise-Features für erweiterte Sicherheit, Überwachung und Zusammenarbeit.

## 🚀 Schnellstart

### Voraussetzungen
- Docker (empfohlen) oder Rust 1.88+
- Qdrant Vektordatenbank (für semantische Suche)

### Ausführung mit vorgefertigten Containern

**GitHub Container Registry verwenden (empfohlen):**

```bash
# symbi unified CLI ausführen
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest dsl parse /workspace/agent.dsl

# MCP Server ausführen
docker run --rm -p 8080:8080 ghcr.io/thirdkeyai/symbi:latest mcp

# Interaktive Entwicklung
docker run --rm -it -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest bash
```

### Aus Quellcode erstellen

```bash
# Entwicklungsumgebung erstellen
docker build -t symbi:latest .
docker run --rm -it -v $(pwd):/workspace symbi:latest bash

# symbi unified binary erstellen
cargo build --release

# Komponenten testen
cargo test

# Beispielagenten ausführen (von crates/runtime)
cd crates/runtime && cargo run --example basic_agent
cd crates/runtime && cargo run --example full_system
cd crates/runtime && cargo run --example rag_example

# symbi unified CLI verwenden
cargo run -- dsl parse my_agent.dsl
cargo run -- mcp --port 8080

# HTTP API aktivieren (optional)
cd crates/runtime && cargo run --features http-api --example full_system
```

### Optionale HTTP API

RESTful HTTP API für externe Integration aktivieren:

```bash
# Mit HTTP API Feature erstellen
cargo build --features http-api

# Oder zu Cargo.toml hinzufügen
[dependencies]
symbi-runtime = { version = "0.1.2", features = ["http-api"] }
```

**Hauptendpunkte:**
- `GET /api/v1/health` - Gesundheitsprüfung und Systemstatus
- `GET /api/v1/agents` - Alle aktiven Agenten auflisten
- `POST /api/v1/workflows/execute` - Workflows ausführen
- `GET /api/v1/metrics` - Systemmetriken

## 📁 Projektstruktur

```
symbi/
├── src/                   # symbi unified CLI binary
├── crates/                # Workspace crates
│   ├── dsl/              # Symbi DSL Implementation
│   │   ├── src/          # Parser- und Bibliothekscode
│   │   ├── tests/        # DSL-Testsuite
│   │   └── tree-sitter-symbiont/ # Grammatikdefinition
│   └── runtime/          # Agent Runtime System (Community)
│       ├── src/          # Kern-Runtime-Komponenten
│       ├── examples/     # Verwendungsbeispiele
│       └── tests/        # Integrationstests
├── docs/                 # Dokumentation
└── Cargo.toml           # Workspace-Konfiguration
```

## 🔧 Features

### ✅ Community Features (OSS)
- **DSL-Grammatik**: Vollständige Tree-sitter-Grammatik für Agentendefinitionen
- **Agenten-Runtime**: Aufgabenplanung, Ressourcenverwaltung, Lebenszykluskontrolle
- **Tier 1 Isolation**: Docker-Container-Isolation für Agentenoperationen
- **MCP Integration**: Model Context Protocol Client für externe Tools
- **SchemaPin Security**: Grundlegende kryptografische Tool-Verifikation
- **RAG Engine**: Retrieval-Augmented Generation mit Vektorsuche
- **Kontextverwaltung**: Persistenter Agentenspeicher und Wissenserhaltung
- **Vektordatenbank**: Qdrant-Integration für semantische Suche
- **Umfassendes Secrets Management**: HashiCorp Vault Integration mit mehreren Authentifizierungsmethoden
- **Verschlüsseltes Datei-Backend**: AES-256-GCM Verschlüsselung mit OS-Keyring-Integration
- **Secrets CLI Tools**: Vollständige Verschlüsseln/Entschlüsseln/Bearbeiten-Operationen mit Audit-Trails
- **HTTP API**: Optionale RESTful-Schnittstelle (Feature-gesteuert)

### 🏢 Enterprise Features (Lizenz erforderlich)
- **Erweiterte Isolation**: gVisor und Firecracker Isolation **(Enterprise)**
- **KI-Tool-Review**: Automatisierter Sicherheitsanalyse-Workflow **(Enterprise)**
- **Kryptografische Auditierung**: Vollständige Audit-Trails mit Ed25519-Signaturen **(Enterprise)**
- **Multi-Agent-Kommunikation**: Verschlüsseltes Messaging zwischen Agenten **(Enterprise)**
- **Echtzeit-Monitoring**: SLA-Metriken und Performance-Dashboards **(Enterprise)**
- **Professional Services & Support**: Kundenspezifische Entwicklung und Support **(Enterprise)**

## 📐 Symbiont DSL

Intelligente Agenten mit eingebauten Richtlinien und Fähigkeiten definieren:

```symbiont
metadata {
    version = "1.0.0"
    author = "Your Name"
    description = "Data analysis agent"
}

agent analyze_data(input: DataSet) -> Result {
    capabilities = ["data_analysis", "visualization"]
    
    policy data_privacy {
        allow: read(input) if input.anonymized == true
        deny: store(input) if input.contains_pii == true
        audit: all_operations
    }
    
    with memory = "persistent", requires = "approval" {
        if (llm_check_safety(input)) {
            result = analyze(input);
            return result;
        } else {
            return reject("Safety check failed");
        }
    }
}
```

## 🔐 Secrets Management

Symbi bietet Enterprise-Grade Secrets Management mit mehreren Backend-Optionen:

### Backend-Optionen
- **HashiCorp Vault**: Produktionsreifes Secrets Management mit mehreren Authentifizierungsmethoden
  - Token-basierte Authentifizierung
  - Kubernetes Service Account Authentifizierung
- **Verschlüsselte Dateien**: AES-256-GCM lokaler verschlüsselter Speicher mit OS-Keyring-Integration
- **Agent-Namespaces**: Agent-spezifischer Secrets-Zugriff für Isolation

### CLI-Operationen
```bash
# Secrets-Datei verschlüsseln
symbi secrets encrypt config.json --output config.enc

# Secrets-Datei entschlüsseln
symbi secrets decrypt config.enc --output config.json

# Verschlüsselte Secrets direkt bearbeiten
symbi secrets edit config.enc

# Vault-Backend konfigurieren
symbi secrets configure vault --endpoint https://vault.company.com
```

### Auditierung und Compliance
- Vollständige Audit-Trails für alle Secrets-Operationen
- Kryptografische Integritätsverifikation
- Agent-spezifische Zugriffskontrolle
- Manipulationssichere Protokollierung

## 🔒 Sicherheitsmodell

### Grundlegende Sicherheit (Community)
- **Tier 1 Isolation**: Docker-Container-Agentenausführung
- **Schema-Verifikation**: Kryptografische Tool-Validierung mit SchemaPin
- **Richtlinien-Engine**: Grundlegende Ressourcenzugriffskontrolle
- **Secrets Management**: Vault und verschlüsselte Dateispeicher-Integration
- **Audit-Protokollierung**: Operationsverfolgung und Compliance

### Erweiterte Sicherheit (Enterprise)
- **Verstärkte Isolation**: gVisor (Tier2) und Firecracker (Tier3) Isolation **(Enterprise)**
- **KI-Sicherheitsreview**: Automatisierte Tool-Analyse und -Genehmigung **(Enterprise)**
- **Verschlüsselte Kommunikation**: Sichere Agent-zu-Agent-Nachrichten **(Enterprise)**
- **Umfassende Audits**: Kryptografische Integritätsgarantien **(Enterprise)**

## 🧪 Tests

```bash
# Alle Tests ausführen
cargo test

# Spezifische Komponenten ausführen
cd crates/dsl && cargo test          # DSL Parser
cd crates/runtime && cargo test     # Runtime System

# Integrationstests
cd crates/runtime && cargo test --test integration_tests
cd crates/runtime && cargo test --test rag_integration_tests
cd crates/runtime && cargo test --test mcp_client_tests
```

## 📚 Dokumentation

- **[Erste Schritte](https://docs.symbiont.dev/getting-started)** - Installation und erste Schritte
- **[DSL-Leitfaden](https://docs.symbiont.dev/dsl-guide)** - Vollständige Sprachreferenz
- **[Runtime-Architektur](https://docs.symbiont.dev/runtime-architecture)** - Systemdesign
- **[Sicherheitsmodell](https://docs.symbiont.dev/security-model)** - Sicherheitsimplementierung
- **[API-Referenz](https://docs.symbiont.dev/api-reference)** - Vollständige API-Dokumentation
- **[Beitragen](https://docs.symbiont.dev/contributing)** - Entwicklungsrichtlinien

### Technische Referenzen
- [`crates/runtime/README.md`](crates/runtime/README.md) - Runtime-spezifische Dokumentation
- [`crates/runtime/API_REFERENCE.md`](crates/runtime/API_REFERENCE.md) - Vollständige API-Referenz
- [`crates/dsl/README.md`](crates/dsl/README.md) - DSL-Implementierungsdetails

## 🤝 Beitragen

Beiträge sind willkommen! Bitte konsultieren Sie [`docs/contributing.md`](docs/contributing.md) für Richtlinien.

**Entwicklungsprinzipien:**
- Security First - alle Features müssen Sicherheitsüberprüfung bestehen
- Zero Trust - alle Eingaben als potenziell böswillig annehmen
- Umfassende Tests - hohe Testabdeckung beibehalten
- Klare Dokumentation - alle Features und APIs dokumentieren

## 🎯 Anwendungsfälle

### Entwicklung und Automatisierung
- Sichere Codegenerierung und Refactoring
- Automatisierte Tests mit Richtlinien-Compliance
- KI-Agenten-Deployment mit Tool-Verifikation
- Wissensmanagement mit semantischer Suche

### Unternehmen und regulierte Branchen
- HIPAA-konforme Gesundheitsdatenverarbeitung **(Enterprise)**
- Finanzdienstleistungen mit Audit-Anforderungen **(Enterprise)**
- Regierungssysteme mit Sicherheitsfreigaben **(Enterprise)**
- Rechtsdokumentanalyse mit Vertraulichkeit **(Enterprise)**

## 📄 Lizenz

**Community Edition**: MIT-Lizenz  
**Enterprise Edition**: Kommerzielle Lizenz erforderlich

Kontaktieren Sie [ThirdKey](https://thirdkey.ai) für Enterprise-Lizenzierung.

## 🔗 Links

- [ThirdKey Website](https://thirdkey.ai)
- [Runtime API-Referenz](crates/runtime/API_REFERENCE.md)

---

*Symbi ermöglicht sichere Zusammenarbeit zwischen KI-Agenten und Menschen durch intelligente Richtliniendurchsetzung, kryptografische Verifikation und umfassende Audit-Trails.*

<div align="right">
  <img src="symbi-trans.png" alt="Symbi Transparentes Logo" width="120">
</div>