---
layout: default
title: REPL-Leitfaden
nav_exclude: true
---

# Symbiont REPL-Leitfaden

## Andere Sprachen
{: .no_toc}

[English](repl-guide.md) | [中文简体](repl-guide.zh-cn.md) | [Español](repl-guide.es.md) | [Português](repl-guide.pt.md) | [日本語](repl-guide.ja.md) | **Deutsch**

---

Die Symbiont-REPL (Read-Eval-Print Loop) bietet eine interaktive Umgebung zum Entwickeln, Testen und Debuggen von Symbiont-Agenten und DSL-Code.

## Funktionen

- **Interaktive DSL-Auswertung**: Symbiont-DSL-Code in Echtzeit ausfuehren
- **Agenten-Lebenszyklus-Verwaltung**: Agenten erstellen, starten, stoppen, pausieren, fortsetzen und zerstoeren
- **Ausfuehrungsueberwachung**: Echtzeitueberwachung der Agentenausfuehrung mit Statistiken und Traces
- **Policy-Durchsetzung**: Integrierte Policy-Pruefung und Capability-Gating
- **Sitzungsverwaltung**: REPL-Sitzungen als Snapshot sichern und wiederherstellen
- **JSON-RPC-Protokoll**: Programmatischer Zugriff ueber JSON-RPC via stdio
- **LSP-Unterstuetzung**: Language Server Protocol fuer IDE-Integration

## Erste Schritte

### REPL starten

```bash
# Interaktiver REPL-Modus
symbi repl

# JSON-RPC-Servermodus (fuer IDE-Integration)
symbi repl --json-rpc

# Mit benutzerdefinierter Konfiguration
symbi repl --config custom-config.toml
```

### Grundlegende Verwendung

```rust
# Einen Agenten definieren
agent GreetingAgent {
  name: "Greeting Agent"
  version: "1.0.0"
  description: "A simple greeting agent"
}

# Ein Verhalten definieren
behavior Greet {
  input { name: string }
  output { greeting: string }
  steps {
    let greeting = format("Hello, {}!", name)
    return greeting
  }
}

# Ausdruecke ausfuehren
let message = "Welcome to Symbiont"
print(message)
```

## REPL-Befehle

### Agentenverwaltung

| Befehl | Beschreibung |
|--------|-------------|
| `:agents` | Alle Agenten auflisten |
| `:agent list` | Alle Agenten auflisten |
| `:agent start <id>` | Einen Agenten starten |
| `:agent stop <id>` | Einen Agenten stoppen |
| `:agent pause <id>` | Einen Agenten pausieren |
| `:agent resume <id>` | Einen pausierten Agenten fortsetzen |
| `:agent destroy <id>` | Einen Agenten zerstoeren |
| `:agent execute <id> <behavior> [args]` | Agentenverhalten ausfuehren |
| `:agent debug <id>` | Debug-Informationen fuer einen Agenten anzeigen |

### Ueberwachungsbefehle

| Befehl | Beschreibung |
|--------|-------------|
| `:monitor stats` | Ausfuehrungsstatistiken anzeigen |
| `:monitor traces [limit]` | Ausfuehrungs-Traces anzeigen |
| `:monitor report` | Detaillierten Ausfuehrungsbericht anzeigen |
| `:monitor clear` | Ueberwachungsdaten loeschen |

### Sitzungsbefehle

| Befehl | Beschreibung |
|--------|-------------|
| `:snapshot` | Sitzungs-Snapshot erstellen |
| `:clear` | Sitzung loeschen |
| `:help` oder `:h` | Hilfemeldung anzeigen |
| `:version` | Versionsinformationen anzeigen |

## DSL-Funktionen

### Agenten-Definitionen

```rust
agent DataAnalyzer {
  name: "Data Analysis Agent"
  version: "2.1.0"
  description: "Analyzes datasets with privacy protection"

  security {
    capabilities: ["data_read", "analysis"]
    sandbox: true
  }

  resources {
    memory: 512MB
    cpu: 2
    storage: 1GB
  }
}
```

### Verhaltensdefinitionen

```rust
behavior AnalyzeData {
  input {
    data: DataSet
    options: AnalysisOptions
  }
  output {
    results: AnalysisResults
  }

  steps {
    # Datenschutzanforderungen pruefen
    require capability("data_read")

    if (data.contains_pii) {
      return error("Cannot process data with PII")
    }

    # Analyse durchfuehren
    let results = analyze(data, options)
    emit analysis_completed { results: results }

    return results
  }
}
```

### Eingebaute Funktionen

| Funktion | Beschreibung | Beispiel |
|----------|-------------|---------|
| `print(...)` | Werte in die Ausgabe schreiben | `print("Hello", name)` |
| `len(value)` | Laenge von String, Liste oder Map ermitteln | `len("hello")` -> `5` |
| `upper(string)` | String in Grossbuchstaben umwandeln | `upper("hello")` -> `"HELLO"` |
| `lower(string)` | String in Kleinbuchstaben umwandeln | `lower("HELLO")` -> `"hello"` |
| `format(template, ...)` | String mit Argumenten formatieren | `format("Hello, {}!", name)` |

### Datentypen

```rust
# Grundtypen
let name = "Alice"          # String
let age = 30               # Integer
let height = 5.8           # Number
let active = true          # Boolean
let empty = null           # Null

# Sammlungen
let items = [1, 2, 3]      # Liste
let config = {             # Map
  "host": "localhost",
  "port": 8080
}

# Zeit- und Groesseneinheiten
let timeout = 30s          # Dauer
let max_size = 100MB       # Groesse
```

## Architektur

### Komponenten

```
symbi repl
├── repl-cli/          # CLI-Schnittstelle und JSON-RPC-Server
├── repl-core/         # Kern-REPL-Engine und Evaluator
├── repl-proto/        # JSON-RPC-Protokolldefinitionen
└── repl-lsp/          # Language Server Protocol Implementierung
```

### Kernkomponenten

- **DslEvaluator**: Fuehrt DSL-Programme mit Laufzeitintegration aus
- **ReplEngine**: Koordiniert Auswertung und Befehlsverarbeitung
- **ExecutionMonitor**: Verfolgt Ausfuehrungsstatistiken und Traces
- **RuntimeBridge**: Integriert sich mit der Symbiont-Laufzeit fuer Policy-Durchsetzung
- **SessionManager**: Verwaltet Snapshots und Sitzungszustand

### JSON-RPC-Protokoll

Die REPL unterstuetzt JSON-RPC 2.0 fuer programmatischen Zugriff:

```json
// DSL-Code auswerten
{
  "jsonrpc": "2.0",
  "method": "evaluate",
  "params": {"input": "let x = 42"},
  "id": 1
}

// Antwort
{
  "jsonrpc": "2.0",
  "result": {"value": "42", "type": "integer"},
  "id": 1
}
```

## Sicherheit & Policy-Durchsetzung

### Capability-Pruefung

Die REPL erzwingt Capability-Anforderungen, die in Agenten-Sicherheitsbloecken definiert sind:

```rust
agent SecureAgent {
  name: "Secure Agent"
  security {
    capabilities: ["filesystem", "network"]
    sandbox: true
  }
}

behavior ReadFile {
  input { path: string }
  output { content: string }
  steps {
    # Dies prueft, ob der Agent die Capability "filesystem" hat
    require capability("filesystem")
    let content = read_file(path)
    return content
  }
}
```

### Policy-Integration

Die REPL integriert sich mit der Symbiont-Policy-Engine, um Zugriffskontrollen und Audit-Anforderungen durchzusetzen.

## Debugging & Ueberwachung

### Ausfuehrungs-Traces

```
:monitor traces 10

Recent Execution Traces:
  14:32:15.123 - AgentCreated [Agent: abc-123] (2ms)
  14:32:15.125 - AgentStarted [Agent: abc-123] (1ms)
  14:32:15.130 - BehaviorExecuted [Agent: abc-123] (5ms)
  14:32:15.135 - AgentPaused [Agent: abc-123]
```

### Statistiken

```
:monitor stats

Execution Monitor Statistics:
  Total Executions: 42
  Successful: 38
  Failed: 4
  Success Rate: 90.5%
  Average Duration: 12.3ms
  Total Duration: 516ms
  Active Executions: 2
```

### Agenten-Debugging

```
:agent debug abc-123

Agent Debug Information:
  ID: abc-123-def-456
  Name: Data Analyzer
  Version: 2.1.0
  State: Running
  Created: 2024-01-15 14:30:00 UTC
  Description: Analyzes datasets with privacy protection
  Author: data-team@company.com
  Available Functions/Behaviors: 5
  Required Capabilities: 2
    - data_read
    - analysis
  Resource Configuration:
    Memory: 512MB
    CPU: 2
    Storage: 1GB
```

## IDE-Integration

### Language Server Protocol

Die REPL bietet LSP-Unterstuetzung fuer IDE-Integration:

```bash
# LSP-Server starten
symbi repl --lsp --port 9257
```

### Unterstuetzte Funktionen

- Syntaxhervorhebung
- Code-Vervollstaendigung
- Fehlerdiagnose
- Hover-Informationen
- Gehe zu Definition
- Symbolsuche

## Bewaeehrte Praktiken

### Entwicklungsworkflow

1. **Mit einfachen Ausdruecken beginnen**: Grundlegende DSL-Konstrukte testen
2. **Agenten schrittweise definieren**: Mit minimalen Agentendefinitionen starten
3. **Verhalten einzeln testen**: Verhalten vor der Integration definieren und testen
4. **Ueberwachung nutzen**: Ausfuehrungsueberwachung zum Debuggen einsetzen
5. **Snapshots erstellen**: Wichtige Sitzungszustaende sichern

### Performance-Tipps

- `:monitor clear` regelmaessig verwenden, um Ueberwachungsdaten zurueckzusetzen
- Trace-Verlauf mit `:monitor traces <limit>` begrenzen
- Ungenutzte Agenten zerstoeren, um Ressourcen freizugeben
- Snapshots fuer komplexe Sitzungszustaende verwenden

### Sicherheitsueberlegungen

- Immer geeignete Capabilities fuer Agenten definieren
- Policy-Durchsetzung in der Entwicklung testen
- Sandbox-Modus fuer nicht vertrauenswuerdigen Code verwenden
- Ausfuehrungs-Traces auf Sicherheitsereignisse ueberwachen

## Fehlerbehebung

### Haeufige Probleme

**Agentenerstellung schlaegt fehl**
```
Error: Missing capability: filesystem
```
*Loesung*: Erforderliche Capabilities zum Agenten-Sicherheitsblock hinzufuegen

**Ausfuehrungs-Timeout**
```
Error: Maximum execution depth exceeded
```
*Loesung*: Auf unendliche Rekursion in der Verhaltenslogik pruefen

**Policy-Verletzung**
```
Error: Policy violation: data access denied
```
*Loesung*: Sicherstellen, dass der Agent ueber die entsprechenden Berechtigungen verfuegt

### Debug-Befehle

```rust
# Agentenstatus pruefen
:agent debug <agent-id>

# Ausfuehrungs-Traces anzeigen
:monitor traces 50

# Systemstatistiken pruefen
:monitor stats

# Debug-Snapshot erstellen
:snapshot
```

## Beispiele

### Einfacher Agent

```rust
agent Calculator {
  name: "Basic Calculator"
  version: "1.0.0"
}

behavior Add {
  input { a: number, b: number }
  output { result: number }
  steps {
    return a + b
  }
}

# Das Verhalten testen
let result = Add(5, 3)
print("5 + 3 =", result)
```

### Datenverarbeitungs-Agent

```rust
agent DataProcessor {
  name: "Data Processing Agent"
  version: "1.0.0"

  security {
    capabilities: ["data_read", "data_write"]
    sandbox: true
  }

  resources {
    memory: 256MB
    cpu: 1
  }
}

behavior ProcessCsv {
  input { file_path: string }
  output { summary: ProcessingSummary }

  steps {
    require capability("data_read")

    let data = read_csv(file_path)
    let processed = transform_data(data)

    require capability("data_write")
    write_results(processed)

    return {
      "rows_processed": len(data),
      "status": "completed"
    }
  }
}
```

## Siehe auch

- [DSL-Leitfaden](dsl-guide.md) - Vollstaendige DSL-Sprachreferenz
- [Laufzeit-Architektur](runtime-architecture.md) - Systemarchitektur-Uebersicht
- [Sicherheitsmodell](security-model.md) - Sicherheitsimplementierungsdetails
- [API-Referenz](api-reference.md) - Vollstaendige API-Dokumentation
