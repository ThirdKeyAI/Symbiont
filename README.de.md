<img src="logo-hz.png" alt="Symbi">

[English](README.md) | [中文简体](README.zh-cn.md) | [Español](README.es.md) | [Português](README.pt.md) | [日本語](README.ja.md) | **Deutsch**

[![Build](https://img.shields.io/github/actions/workflow/status/thirdkeyai/symbiont/docker-build.yml?branch=main)](https://github.com/thirdkeyai/symbiont/actions)
[![Crates.io](https://img.shields.io/crates/v/symbi)](https://crates.io/crates/symbi)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Docs](https://img.shields.io/badge/docs-online-brightgreen)](https://docs.symbiont.dev)

---

**Richtliniengesteuerte Agenten-Laufzeitumgebung fuer den Produktionseinsatz.**

Symbiont ist eine Rust-native Laufzeitumgebung fuer die Ausfuehrung von KI-Agenten, Tools und Workflows unter expliziter Richtlinien-, Identitaets- und Audit-Kontrolle.

Die meisten Agenten-Frameworks konzentrieren sich auf Orchestrierung. Symbiont konzentriert sich darauf, was passiert, wenn Agenten in realen Umgebungen mit echten Risiken ausgefuehrt werden muessen: nicht vertrauenswuerdige Tools, sensible Daten, Genehmigungsgrenzen, Audit-Anforderungen und wiederholbare Durchsetzung.

---

## Warum Symbiont

KI-Agenten sind leicht zu demonstrieren und schwer zu vertrauen.

Sobald ein Agent Tools aufrufen, auf Dateien zugreifen, Nachrichten senden oder externe Dienste nutzen kann, braucht man mehr als Prompts und Glue-Code. Man braucht:

* **Richtliniendurchsetzung** fuer das, was ein Agent tun darf -- eingebaute DSL und [Cedar](https://www.cedarpolicy.com/)-Autorisierung
* **Tool-Verifikation**, damit die Ausfuehrung kein blindes Vertrauen ist -- [SchemaPin](https://github.com/ThirdKeyAI/SchemaPin) kryptografische Verifikation von MCP-Tools
* **Agenten-Identitaet**, damit man weiss, wer handelt -- [AgentPin](https://github.com/ThirdKeyAI/AgentPin) domaingebundene ES256-Identitaet
* **Sandboxing** fuer riskante Workloads -- Docker-Isolation mit Ressourcenlimits
* **Audit-Trails** fuer das, was passiert ist und warum -- kryptografisch manipulationssichere Logs
* **Review-Workflows** fuer Aktionen, die eine Genehmigung erfordern -- Human-in-the-Loop-Gates in der Reasoning-Schleife

Symbiont ist fuer diese Schicht gebaut.

---

## Schnellstart

### Voraussetzungen

* Docker (empfohlen) oder Rust 1.82+
* Keine externe Vektordatenbank erforderlich (LanceDB eingebettet; Qdrant optional fuer skalierte Deployments)

### Ausfuehrung mit Docker

```bash
# Laufzeitumgebung starten (API auf :8080, HTTP-Eingabe auf :8081)
docker run --rm -p 8080:8080 -p 8081:8081 ghcr.io/thirdkeyai/symbi:latest up

# Nur MCP-Server ausfuehren
docker run --rm -p 8080:8080 ghcr.io/thirdkeyai/symbi:latest mcp

# Agent-DSL-Datei parsen
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest dsl parse /workspace/agent.dsl
```

### Aus Quellcode erstellen

```bash
cargo build --release
./target/release/symbi --help

# Laufzeitumgebung starten
cargo run -- up

# Interaktive REPL
cargo run -- repl
```

> Fuer Produktionsdeployments lesen Sie `SECURITY.md` und den [Deployment-Leitfaden](https://docs.symbiont.dev/getting-started), bevor Sie nicht vertrauenswuerdige Tool-Ausfuehrung aktivieren.

---

## Funktionsweise

Symbiont trennt die Absicht des Agenten von der Ausfuehrungsberechtigung:

1. **Agenten schlagen** Aktionen durch die ORGA-Reasoning-Schleife vor (Observe-Reason-Gate-Act)
2. **Die Laufzeitumgebung evaluiert** jede Aktion gegen Richtlinien-, Identitaets- und Vertrauenspruefungen
3. **Richtlinien entscheiden** -- erlaubte Aktionen werden ausgefuehrt; abgelehnte Aktionen werden blockiert oder zur Genehmigung weitergeleitet
4. **Alles wird protokolliert** -- manipulationssicherer Audit-Trail fuer jede Entscheidung

Das bedeutet, dass Modellausgaben niemals als Ausfuehrungsberechtigung behandelt werden. Die Laufzeitumgebung kontrolliert, was tatsaechlich passiert.

### Beispiel: nicht vertrauenswuerdiges Tool durch Richtlinie blockiert

Ein Agent versucht, ein nicht verifiziertes MCP-Tool aufzurufen. Die Laufzeitumgebung:

1. Prueft den SchemaPin-Verifikationsstatus -- Tool-Signatur fehlt oder ist ungueltig
2. Evaluiert die Cedar-Richtlinie -- `forbid(action == Action::"tool_call") when { !resource.verified }`
3. Blockiert die Ausfuehrung und protokolliert die Ablehnung mit vollstaendigem Kontext
4. Leitet optional an einen Operator zur manuellen Genehmigung weiter

Keine Code-Aenderung erforderlich. Die Richtlinie steuert die Ausfuehrung.

---

## DSL-Beispiel

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

---

## Kernfaehigkeiten

| Faehigkeit | Beschreibung |
|-----------|-------------|
| **Cedar Policy Engine** | Feingranulare Autorisierung fuer Agenten-Aktionen, Tool-Aufrufe und Ressourcenzugriff |
| **SchemaPin-Verifikation** | Kryptografische Verifikation von MCP-Tool-Schemas vor der Ausfuehrung |
| **AgentPin-Identitaet** | Domaingebundene ES256-Identitaet fuer Agenten und geplante Aufgaben |
| **ORGA-Reasoning-Schleife** | Typestate-erzwungener Observe-Reason-Gate-Act-Zyklus mit Richtlinien-Gates und Circuit-Breakern |
| **Sandboxing** | Docker-basierte Isolation mit Ressourcenlimits fuer nicht vertrauenswuerdige Workloads |
| **Audit-Logging** | Manipulationssichere Logs mit strukturierten Datensaetzen fuer jede Richtlinienentscheidung |
| **ClawHavoc-Scanning** | 40 Regeln in 10 Angriffskategorien fuer die Analyse von Skill-/Tool-Inhalten |
| **Secrets Management** | Vault/OpenBao-Integration, AES-256-GCM-verschluesselter Speicher, pro Agent begrenzt |
| **Cron-Scheduling** | SQLite-gestuetzter Scheduler mit Jitter, Parallelitaetsschutz und Dead-Letter-Queues |
| **Persistenter Speicher** | Markdown-basierter Agentenspeicher mit Faktenextraktion, Prozeduren und Kompaktierung |
| **RAG Engine** | Hybride semantische + Keyword-Suche ueber LanceDB (eingebettet) oder Qdrant (skaliert) |
| **MCP-Integration** | Native Model Context Protocol-Unterstuetzung mit gesteuertem Tool-Zugriff |
| **Webhook-Verifikation** | HMAC-SHA256- und JWT-Verifikation mit GitHub-, Stripe- und Slack-Presets |
| **Delivery-Routing** | Agentenausgabe an Webhooks, Slack, E-Mail oder benutzerdefinierte Kanaele weiterleiten |
| **Metriken & Telemetrie** | OTLP-Export mit OpenTelemetry Tracing Spans fuer die Reasoning-Schleife |
| **HTTP-Sicherheit** | Loopback-only-Bindung, CORS-Allow-Lists, JWT EdDSA-Validierung, agentenbezogene API-Keys |
| **KI-Assistenten-Plugins** | Governance-Plugins fuer [Claude Code](https://github.com/thirdkeyai/symbi-claude-code) und [Gemini CLI](https://github.com/thirdkeyai/symbi-gemini-cli) |

Leistung: Richtlinienevaluierung <1ms, ECDSA P-256-Verifikation <5ms, 10k Agenten-Scheduling mit <2% CPU-Overhead. Siehe [Benchmarks](crates/runtime/benches/performance_claims.rs) und [Schwellenwerttests](crates/runtime/tests/performance_claims.rs).

---

## Sicherheitsmodell

Symbiont basiert auf einem einfachen Prinzip: **Modellausgaben sollten niemals als Ausfuehrungsberechtigung vertraut werden.**

Aktionen durchlaufen Laufzeitkontrollen:

* **Zero Trust** -- alle Agenten-Eingaben sind standardmaessig nicht vertrauenswuerdig
* **Richtlinienpruefungen** -- Cedar-Autorisierung vor jedem Tool-Aufruf und Ressourcenzugriff
* **Tool-Verifikation** -- SchemaPin kryptografische Verifikation von Tool-Schemas
* **Sandbox-Grenzen** -- Docker-Isolation fuer nicht vertrauenswuerdige Ausfuehrung
* **Operator-Genehmigung** -- Human-in-the-Loop-Gates fuer sensible Aktionen
* **Secrets-Kontrolle** -- Vault/OpenBao-Backends, verschluesselter lokaler Speicher, Agenten-Namespaces
* **Audit-Logging** -- kryptografisch manipulationssichere Aufzeichnungen jeder Entscheidung

Wenn Sie nicht vertrauenswuerdigen Code oder riskante Tools ausfuehren, verlassen Sie sich nicht auf ein schwaches lokales Ausfuehrungsmodell als einzige Grenze. Siehe [`SECURITY.md`](SECURITY.md) und die [Sicherheitsmodell-Dokumentation](https://docs.symbiont.dev/security-model).

---

## Workspace

| Crate | Beschreibung |
|-------|-------------|
| `symbi` | Einheitliches CLI-Binary |
| `symbi-runtime` | Kern-Agenten-Laufzeitumgebung und Ausfuehrungsengine |
| `symbi-dsl` | DSL-Parser und -Evaluator |
| `symbi-channel-adapter` | Slack/Teams/Mattermost-Adapter |
| `repl-core` / `repl-proto` / `repl-cli` | Interaktive REPL und JSON-RPC-Server |
| `repl-lsp` | Language Server Protocol-Unterstuetzung |
| `symbi-a2ui` | Admin-Dashboard (Lit/TypeScript, Alpha) |

Governance-Plugins: [`symbi-claude-code`](https://github.com/thirdkeyai/symbi-claude-code) | [`symbi-gemini-cli`](https://github.com/thirdkeyai/symbi-gemini-cli)

---

## Dokumentation

* [Erste Schritte](https://docs.symbiont.dev/getting-started)
* [Sicherheitsmodell](https://docs.symbiont.dev/security-model)
* [Runtime-Architektur](https://docs.symbiont.dev/runtime-architecture)
* [Reasoning-Loop-Leitfaden](https://docs.symbiont.dev/reasoning-loop)
* [DSL-Leitfaden](https://docs.symbiont.dev/dsl-guide)
* [API-Referenz](https://docs.symbiont.dev/api-reference)
* [Erweiterte Reasoning-Primitive](https://docs.symbiont.dev/orga-adaptive)

Wenn Sie Symbiont fuer den Produktionseinsatz evaluieren, beginnen Sie mit dem Sicherheitsmodell und der Erste-Schritte-Dokumentation.

---

## Lizenz

* **Community Edition** (Apache 2.0): Kern-Laufzeitumgebung, DSL, ORGA-Reasoning-Schleife, Cedar Policy Engine, SchemaPin/AgentPin-Verifikation, Docker-Sandboxing, persistenter Speicher, Cron-Scheduling, MCP-Integration, RAG (LanceDB), Audit-Logging, Webhook-Verifikation, ClawHavoc Skill-Scanning und alle CLI/REPL-Werkzeuge.
* **Enterprise Edition** (kommerzielle Lizenz): Multi-Tier-Sandboxing (gVisor, Firecracker, E2B), kryptografische Audit-Trails mit Compliance-Exporten (HIPAA, SOX, PCI-DSS), KI-gestuetzte Tool-Ueberpruefung und Bedrohungserkennung, verschluesselte Multi-Agenten-Kollaboration, Echtzeit-Monitoring-Dashboards und dedizierter Support. Siehe [`enterprise/README.md`](enterprise/README.md) fuer Details.

Kontaktieren Sie [ThirdKey](https://thirdkey.ai) fuer Enterprise-Lizenzierung.

---

*Derselbe Agent. Sichere Laufzeitumgebung.*

<div align="right">
  <img src="symbi-trans.png" alt="Symbi-Logo" width="120">
</div>
