<img src="logo-hz.png" alt="Symbi">

[English](README.md) | [中文简体](README.zh-cn.md) | [Español](README.es.md) | [Português](README.pt.md) | [日本語](README.ja.md) | **Deutsch**

[![Build](https://img.shields.io/github/actions/workflow/status/thirdkeyai/symbiont/docker-build.yml?branch=main)](https://github.com/thirdkeyai/symbiont/actions)
[![Crates.io](https://img.shields.io/crates/v/symbi)](https://crates.io/crates/symbi)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Docs](https://img.shields.io/badge/docs-online-brightgreen)](https://docs.symbiont.dev)
[![YouTube](https://img.shields.io/badge/YouTube-%40ThirdKeyAI-FF0000?logo=youtube&logoColor=white)](https://www.youtube.com/@ThirdKeyAI)

[![OATS Reference Implementation](https://img.shields.io/badge/OATS-Reference%20Implementation-1f6feb)](https://openagenttruststack.org)
[![DOI Typestate Loops](https://zenodo.org/badge/DOI/10.5281/zenodo.19896446.svg)](https://doi.org/10.5281/zenodo.19896446)
[![DOI ToolClad](https://zenodo.org/badge/DOI/10.5281/zenodo.19957596.svg)](https://doi.org/10.5281/zenodo.19957596)
[![DOI Empirical Eval](https://zenodo.org/badge/DOI/10.5281/zenodo.20043247.svg)](https://doi.org/10.5281/zenodo.20043247)

---

**Richtliniengesteuerte Agenten-Laufzeitumgebung fuer den Produktionseinsatz.**
*Derselbe Agent. Sichere Laufzeitumgebung.*

![A Cedar policy denies a live agent's privileged tool call](https://raw.githubusercontent.com/ThirdKeyAI/Symbiont/main/docs/media/cedar-demo.gif)

> **Was Sie hier sehen:** ein echtes Modell (`claude-haiku-4.5`) fordert die Auflistung der Agentenflotte an. Eine Cedar-`forbid`-Regel verweigert den Aufruf bei **jedem erneuten Versuch** -- keine Code-Aenderung, nur policy. [Reproduzieren Sie es mit einem einzigen Befehl ↓](#das-policy-gate-verweigert-ein-tool----ein-befehl-kein-setup) · [▶ Vollstaendiger Walkthrough](https://www.youtube.com/watch?v=RPyKpqKz5ik)

Symbiont ist eine Rust-native Laufzeitumgebung fuer die Ausfuehrung von KI-Agenten und Tools unter expliziter Richtlinien-, Identitaets- und Audit-Kontrolle.

Die meisten Agenten-Frameworks konzentrieren sich auf Orchestrierung. Symbiont konzentriert sich darauf, was passiert, wenn Agenten in realen Umgebungen mit echten Risiken ausgefuehrt werden muessen: nicht vertrauenswuerdige Tools, sensible Daten, Genehmigungsgrenzen, Audit-Anforderungen und wiederholbare Durchsetzung.

---

## Warum Symbiont

KI-Agenten sind leicht zu demonstrieren und schwer zu vertrauen.

Sobald ein Agent Tools aufrufen, auf Dateien zugreifen, Nachrichten senden oder externe Dienste nutzen kann, braucht man mehr als Prompts und Glue-Code. Man braucht:

* **Richtliniendurchsetzung** fuer das, was ein Agent tun darf -- eingebaute DSL und [Cedar](https://www.cedarpolicy.com/)-Autorisierung
* **Tool-Verifikation**, damit die Ausfuehrung kein blindes Vertrauen ist -- [SchemaPin](https://github.com/ThirdKeyAI/SchemaPin) kryptografische Verifikation von MCP-Tools
* **Tool-Vertraege**, die regeln, wie Tools ausgefuehrt werden -- [ToolClad](https://github.com/ThirdKeyAI/ToolClad) deklarative Argumentvalidierung, Scope-Durchsetzung und Injection-Schutz
* **Agenten-Identitaet**, damit man weiss, wer handelt -- [AgentPin](https://github.com/ThirdKeyAI/AgentPin) domaingebundene ES256-Identitaet
* **Sandboxing** fuer riskante Workloads -- Docker-Isolation mit Ressourcenlimits
* **Audit-Trails** fuer das, was passiert ist und warum -- kryptografisch manipulationssichere Logs
* **Genehmigungsgates** fuer sensible Aktionen -- menschliche Ueberpruefung vor der Ausfuehrung, wenn die Richtlinie es erfordert

Symbiont ist fuer diese Schicht gebaut.

### Open Agent Trust Stack (OATS) -- Referenzimplementierung

Symbiont ist die **Referenzimplementierung des [Open Agent Trust Stack (OATS)](https://openagenttruststack.org)** -- einer offenen Spezifikation (CC BY 4.0) zur Absicherung der Ausfuehrung von KI-Agenten durch strukturelle Durchsetzung statt nachgelagerter Interception ("define what is permitted and make everything else structurally inexpressible" -- definiere, was erlaubt ist, und mache alles Uebrige strukturell unausdrueckbar). Die OATS-Spezifikation basiert auf den operativen Produktionserfahrungen von Symbiont, und das Design von Symbiont entspricht direkt den OATS-Schichten:

| OATS-Schicht | Symbiont-Zuordnung |
|---|---|
| **Layer 1 -- ORGA Loop** (typestate-erzwungenes Observe-Reason-Gate-Act) | `crates/runtime/src/reasoning/` -- typestate-erzwungene Phasen; das Policy-Gate ist zur Compile-Zeit nicht umgehbar. Siehe [Wanger 2026 / DOI 10.5281/zenodo.19896446](https://doi.org/10.5281/zenodo.19896446). |
| **Layer 2 -- Tool Contracts** | [ToolClad](https://github.com/ThirdKeyAI/ToolClad) deklarative `.clad.toml`-Manifeste + der `agent_summary`-Typestate-Zaun in `crates/runtime/src/toolclad/`. Siehe [Wanger 2026 / DOI 10.5281/zenodo.19957596](https://doi.org/10.5281/zenodo.19957596). |
| **Layer 3 -- Identity** | [SchemaPin](https://github.com/ThirdKeyAI/SchemaPin) fuer MCP-Tools + [AgentPin](https://github.com/ThirdKeyAI/AgentPin) ES256 domaingebundene Agenten-Identitaet. |
| **Layer 4 -- Policy Engine** | Cedar-Policy-Gate (`crates/runtime/src/reasoning/cedar_gate.rs`) + `CommunicationPolicyGate` fuer Inter-Agent-Aufrufe; beide seit v1.14.0 standardmaessig fail-closed. |
| **Layer 5 -- Audit Journal** | Hash-verkettetes, Ed25519-signiertes `BufferedJournal` in der Reasoning-Schleife; verschluesselte Modell-I/O-Logs in `crates/runtime/src/logging.rs`. |

Symbiont entspricht **OATS Extended** (C1-C7 + E1-E8). Der empirische Vergleich von Runtimes mit struktureller Durchsetzung, der der Spezifikation zugrunde liegt, ist [Wanger 2026 / DOI 10.5281/zenodo.20043247](https://doi.org/10.5281/zenodo.20043247).

---

## Schnellstart

### Das Policy-Gate verweigert ein Tool -- ein Befehl, kein Setup

Ein Cedar-`forbid` blockiert ein privilegiertes Tool, waehrend ein sicheres durchgelassen wird. Kopieren Sie dies und fuehren Sie es gegen das veroeffentlichte Image aus (kein Klon, kein Build):

```bash
docker run --rm --entrypoint sh ghcr.io/thirdkeyai/symbi:latest -c '
mkdir -p /tmp/p && cat > /tmp/p/policy.cedar <<EOF
forbid(principal, action == Symbi::Action::"tool_call::list_agents",   resource);
permit(principal, action == Symbi::Action::"tool_call::system_health", resource);
EOF
echo "{\"tool_name\":\"list_agents\"}"   | symbi policy evaluate --stdin --policies /tmp/p --json
echo "{\"tool_name\":\"system_health\"}" | symbi policy evaluate --stdin --policies /tmp/p --json'
```

```json
{"decision":"deny","reason":"deny policies matched: policy_0","tool":"list_agents", ...}
{"decision":"allow","reason":"allow policies matched: policy_1","tool":"system_health", ...}
```

Das ist dasselbe Cedar-Gate, das die Laufzeitumgebung in die Live-Reasoning-Schleife einbindet -- genau die Ablehnung, die in der Demo oben gezeigt wird.

### CLI installieren

```bash
# Linux / macOS — installs the `symbi` binary to /usr/local/bin
curl -fsSL https://symbiont.dev/install.sh | bash
symbi --help
```

Der Installer laedt das vorgefertigte Release-Binary fuer Ihre Plattform herunter. Fixieren Sie eine Version mit `bash -s -- --version v1.15.2` oder aendern Sie das Zielverzeichnis mit `--dir`. Bevorzugen Sie Docker oder das [Erstellen aus Quellcode](#aus-quellcode-erstellen)? Beides finden Sie weiter unten.

### Voraussetzungen

* Docker (empfohlen) oder Rust 1.82+

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

1. **Agenten schlagen** Aktionen durch die Reasoning-Schleife vor (Observe-Reason-Gate-Act)
2. **Die Laufzeitumgebung evaluiert** jede Aktion gegen Richtlinien-, Identitaets- und Vertrauenspruefungen
3. **Richtlinien entscheiden** -- erlaubte Aktionen werden ausgefuehrt; abgelehnte Aktionen werden blockiert oder zur Genehmigung weitergeleitet
4. **Alles wird protokolliert** -- manipulationssicherer Audit-Trail fuer jede Entscheidung

Modellausgaben werden niemals als Ausfuehrungsberechtigung behandelt. Die Laufzeitumgebung kontrolliert, was tatsaechlich passiert.

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
agent secure_analyst(input: DataSet) -> Result {
    policy access_control {
        allow: read(input) if input.verified == true
        deny: send_email without approval
        audit: all_operations
    }

    with memory = "persistent", requires = "approval" {
        result = analyze(input);
        return result;
    }
}
```

Den vollstaendigen DSL-Leitfaden mit `metadata`-, `schedule`-, `webhook`- und `channel`-Bloecken finden Sie im [DSL-Leitfaden](https://docs.symbiont.dev/dsl-guide).

---

## Kernfaehigkeiten

| Faehigkeit | Beschreibung |
|-----------|-------------|
| **Policy Engine** | Feingranulare [Cedar](https://www.cedarpolicy.com/)-Autorisierung fuer Agenten-Aktionen, Tool-Aufrufe und Ressourcenzugriff |
| **Tool-Verifikation** | [SchemaPin](https://github.com/ThirdKeyAI/SchemaPin) kryptografische Verifikation von MCP-Tool-Schemas vor der Ausfuehrung |
| **Tool-Vertraege** | [ToolClad](https://github.com/ThirdKeyAI/ToolClad) deklarative Vertraege mit Argumentvalidierung, Scope-Durchsetzung und Cedar-Policy-Generierung |
| **Agenten-Identitaet** | [AgentPin](https://github.com/ThirdKeyAI/AgentPin) domaingebundene ES256-Identitaet fuer Agenten und geplante Aufgaben |
| **Reasoning-Schleife** | Typestate-erzwungener Observe-Reason-Gate-Act-Zyklus mit Richtlinien-Gates und Circuit-Breakern |
| **Sandboxing** | Docker-basierte Isolation mit Ressourcenlimits fuer nicht vertrauenswuerdige Workloads |
| **Audit-Logging** | Manipulationssichere Logs mit strukturierten Datensaetzen fuer jede Richtlinienentscheidung |
| **Secrets Management** | Vault/OpenBao-Integration, AES-256-GCM-verschluesselter Speicher, pro Agent begrenzt |
| **MCP-Integration** | Native Model Context Protocol-Unterstuetzung mit gesteuertem Tool-Zugriff |

Weitere Faehigkeiten: Bedrohungsscanning fuer Tool-/Skill-Inhalte (40 Regeln, 10 Angriffskategorien), Cron-Scheduling, persistenter Agentenspeicher, hybride RAG-Suche (LanceDB/Qdrant), Webhook-Verifikation, Delivery-Routing, OTLP-Telemetrie, HTTP-Sicherheitshardening und Governance-Plugins fuer [Claude Code](https://github.com/thirdkeyai/symbi-claude-code) und [Gemini CLI](https://github.com/thirdkeyai/symbi-gemini-cli). Details finden Sie in der [vollstaendigen Dokumentation](https://docs.symbiont.dev).

Repraesentative Benchmarks sind im [Benchmark-Harness](crates/runtime/benches/performance_claims.rs) und in den [Schwellenwerttests](crates/runtime/tests/performance_claims.rs) verfuegbar.

---

## Sicherheitsmodell

Symbiont basiert auf einem einfachen Prinzip: **Modellausgaben sollten niemals als Ausfuehrungsberechtigung vertraut werden.**

Aktionen durchlaufen Laufzeitkontrollen:

* **Zero Trust** -- alle Agenten-Eingaben sind standardmaessig nicht vertrauenswuerdig
* **Richtlinienpruefungen** -- Cedar-Autorisierung vor jedem Tool-Aufruf und Ressourcenzugriff
* **Tool-Verifikation** -- SchemaPin kryptografische Verifikation von Tool-Schemas
* **Sandbox-Grenzen** -- Docker-Isolation fuer nicht vertrauenswuerdige Ausfuehrung
* **Operator-Genehmigung** -- menschliche Ueberpruefungsgates fuer sensible Aktionen
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
| `symbi-shell` | Interaktive TUI fuer Autorenerstellung, Orchestrierung und Remote-Attach (Beta) |
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

Wenn Sie Symbiont fuer den Produktionseinsatz evaluieren, beginnen Sie mit dem Sicherheitsmodell und der Erste-Schritte-Dokumentation.

---

## SDKs

Offizielle Client-SDKs zur Integration der Symbiont-Laufzeitumgebung in Ihre Anwendung:

| Sprache | Paket | Repository |
|---------|-------|------------|
| **JavaScript/TypeScript** | [symbiont-sdk-js](https://www.npmjs.com/package/symbiont-sdk-js) | [GitHub](https://github.com/ThirdKeyAI/symbiont-sdk-js) |
| **Python** | [symbiont-sdk](https://pypi.org/project/symbiont-sdk/) | [GitHub](https://github.com/ThirdKeyAI/symbiont-sdk-python) |

---

## Lizenz

* **Community Edition** (Apache 2.0): Kern-Laufzeitumgebung, DSL, Policy Engine, Tool-Verifikation, Sandboxing, Agentenspeicher, Scheduling, MCP-Integration, RAG, Audit-Logging und alle CLI/REPL-Werkzeuge.
* **Enterprise Edition** (kommerzielle Lizenz): Erweiterte Sandbox-Backends, Compliance-Audit-Exporte, KI-gestuetzte Tool-Ueberpruefung, verschluesselte Multi-Agenten-Kollaboration, Monitoring-Dashboards und dedizierter Support.

Kontaktieren Sie [ThirdKey](https://thirdkey.ai) fuer Enterprise-Lizenzierung.

---

<div align="right">
  <img src="symbi-trans.png" alt="Symbi-Logo" width="120">
</div>
