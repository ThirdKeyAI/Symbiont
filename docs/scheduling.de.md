---
layout: default
title: Scheduling-Leitfaden
description: "Produktionsreifes Cron-basiertes Aufgaben-Scheduling für Symbiont AI-Agenten"
nav_exclude: true
---

# Scheduling-Leitfaden

## 🌐 Andere Sprachen
{: .no_toc}

[English](scheduling.md) | [中文简体](scheduling.zh-cn.md) | [Español](scheduling.es.md) | [Português](scheduling.pt.md) | [日本語](scheduling.ja.md) | **Deutsch**

---

## Überblick

Das Scheduling-System von Symbiont bietet produktionsreife Cron-basierte Aufgabenausführung für KI-Agenten. Das System unterstützt:

- **Cron-Zeitpläne**: Traditionelle Cron-Syntax für wiederkehrende Aufgaben
- **Einmalige Aufträge**: Einmalige Ausführung zu einem bestimmten Zeitpunkt
- **Heartbeat-Muster**: Kontinuierliche Bewertung-Aktion-Schlaf-Zyklen für Überwachungsagenten
- **Sitzungsisolation**: Kurzlebige, gemeinsame oder vollständig isolierte Agentenkontexte
- **Zustellungsrouting**: Mehrere Ausgabekanäle (Stdout, LogFile, Webhook, Slack, Email, Custom)
- **Richtliniendurchsetzung**: Sicherheits- und Compliance-Prüfungen vor der Ausführung
- **Produktionshärtung**: Jitter, Nebenläufigkeitsgrenzen, Dead-Letter-Warteschlangen und AgentPin-Verifizierung

## Architektur

Das Scheduling-System basiert auf drei Kernkomponenten:

```
┌─────────────────────┐
│   CronScheduler     │  Hintergrund-Tick-Schleife (1-Sekunden-Intervalle)
│   (Tick Loop)       │  Job-Auswahl und Ausführungsorchestrierung
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│   SqliteJobStore    │  Persistente Job-Speicherung
│   (Job Storage)     │  Transaktionsunterstützung, Zustandsverwaltung
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│DefaultAgentScheduler│  Agenten-Ausführungslaufzeit
│ (Execution Engine)  │  AgentContext-Lebenszyklusverwaltung
└─────────────────────┘
```

### CronScheduler

Der `CronScheduler` ist der primäre Einstiegspunkt. Er verwaltet:

- Hintergrund-Tick-Schleife mit 1-Sekunden-Intervallen
- Job-Auswahl basierend auf der nächsten Ausführungszeit
- Nebenläufigkeitssteuerung und Jitter-Injektion
- Metrik-Erfassung und Gesundheitsüberwachung
- Ordnungsgemäßes Herunterfahren mit Nachverfolgung laufender Jobs

### SqliteJobStore

Der `SqliteJobStore` bietet dauerhafte Job-Persistenz mit:

- ACID-Transaktionen für Job-Zustandsaktualisierungen
- Job-Lebenszyklus-Tracking (Active, Paused, Completed, Failed, DeadLetter)
- Ausführungshistorie mit Prüfprotokoll
- Abfragefunktionen zum Filtern nach Status, Agenten-ID usw.

### DefaultAgentScheduler

Der `DefaultAgentScheduler` führt geplante Agenten aus:

- Erstellt isolierte oder gemeinsame `AgentContext`-Instanzen
- Verwaltet den Sitzungslebenszyklus (Erstellen, Ausführen, Zerstören)
- Leitet Zustellungen an konfigurierte Kanäle weiter
- Erzwingt Richtlinien-Gates vor der Ausführung

## DSL-Syntax

### Zeitplanblock-Struktur

Zeitplanblöcke werden in Symbiont-DSL-Dateien definiert:

```symbiont
schedule {
  name: "daily-report"
  agent: "reporter-agent"
  cron: "0 9 * * *"

  session_mode: "ephemeral_with_summary"
  delivery: ["stdout", "log_file"]

  policy {
    require_approval: false
    max_runtime: "5m"
  }
}
```

### Cron-Syntax

Erweiterte Cron-Syntax mit sechs Feldern (Sekunden zuerst, optionales siebtes Feld für Jahr):

```
┌─────────────── Sekunde (0-59)
│ ┌───────────── Minute (0-59)
│ │ ┌─────────── Stunde (0-23)
│ │ │ ┌───────── Tag des Monats (1-31)
│ │ │ │ ┌─────── Monat (1-12)
│ │ │ │ │ ┌───── Wochentag (0-6, Sonntag = 0)
│ │ │ │ │ │
* * * * * *
```

**Beispiele:**

```symbiont
# Jeden Tag um 9 Uhr
cron: "0 0 9 * * *"

# Jeden Montag um 18 Uhr
cron: "0 0 18 * * 1"

# Alle 15 Minuten
cron: "0 */15 * * * *"

# Am ersten Tag jedes Monats um Mitternacht
cron: "0 0 0 1 * *"
```

### Einmalige Aufträge (At-Syntax)

Für Aufträge, die einmalig zu einem bestimmten Zeitpunkt ausgeführt werden:

```symbiont
schedule {
  name: "deployment-check"
  agent: "health-checker"
  at: "2026-02-15T14:30:00Z"  # ISO-8601-Zeitstempel

  delivery: ["webhook"]
  webhook_url: "https://ops.example.com/hooks/deployment"
}
```

### Heartbeat-Muster

Für kontinuierliche Überwachungsagenten, die bewerten, handeln und schlafen:

```symbiont
schedule {
  name: "system-monitor"
  agent: "heartbeat-agent"
  cron: "*/5 * * * *"  # Alle 5 Minuten aufwachen

  heartbeat: {
    enabled: true
    context_mode: "ephemeral_with_summary"
    max_iterations: 100  # Sicherheitslimit
  }
}
```

Der Heartbeat-Agent folgt diesem Zyklus:

1. **Bewertung**: Systemzustand auswerten (z. B. Metriken, Logs prüfen)
2. **Aktion**: Bei Bedarf Korrekturmaßnahmen ergreifen (z. B. Dienst neustarten, Ops benachrichtigen)
3. **Schlaf**: Bis zum nächsten geplanten Tick warten

## CLI-Befehle

Der Befehl `symbi cron` bietet vollständige Lebenszyklus-Verwaltung:

### Aufträge auflisten

```bash
# Alle Aufträge auflisten
symbi cron list

# Nach Status filtern
symbi cron list --status active
symbi cron list --status paused

# Nach Agent filtern
symbi cron list --agent "reporter-agent"

# JSON-Ausgabe
symbi cron list --format json
```

### Auftrag hinzufügen

```bash
# Aus DSL-Datei
symbi cron add --file agent.symbi --schedule "daily-report"

# Inline-Definition (JSON)
symbi cron add --json '{
  "name": "quick-task",
  "agent_id": "agent-123",
  "cron_expr": "0 * * * *"
}'
```

### Auftrag entfernen

```bash
# Nach Job-ID
symbi cron remove <job-id>

# Nach Name
symbi cron remove --name "daily-report"

# Erzwungenes Entfernen (Bestätigung überspringen)
symbi cron remove <job-id> --force
```

### Pausieren/Fortsetzen

```bash
# Auftrag pausieren (stoppt Planung, behält Zustand)
symbi cron pause <job-id>

# Pausierten Auftrag fortsetzen
symbi cron resume <job-id>
```

### Status

```bash
# Auftragsdetails mit nächster Ausführungszeit
symbi cron status <job-id>

# Die letzten 10 Ausführungsdatensätze einbeziehen
symbi cron status <job-id> --history 10

# Überwachungsmodus (automatische Aktualisierung alle 5 Sekunden)
symbi cron status <job-id> --watch
```

### Sofort ausführen

```bash
# Sofortige Ausführung auslösen (umgeht den Zeitplan)
symbi cron run <job-id>

# Mit benutzerdefinierter Eingabe
symbi cron run <job-id> --input "Check production database"
```

### Verlauf

```bash
# Ausführungsverlauf eines Auftrags anzeigen
symbi cron history <job-id>

# Letzte 20 Ausführungen
symbi cron history <job-id> --limit 20

# Nach Status filtern
symbi cron history <job-id> --status failed

# Als CSV exportieren
symbi cron history <job-id> --format csv > runs.csv
```

## Heartbeat-Muster

### HeartbeatContextMode

Steuert, wie der Kontext über Heartbeat-Iterationen hinweg erhalten bleibt:

```rust
pub enum HeartbeatContextMode {
    /// Frischer Kontext pro Iteration, Zusammenfassung an Ausführungshistorie anhängen
    EphemeralWithSummary,

    /// Gemeinsamer Kontext über alle Iterationen (Speicher akkumuliert)
    SharedPersistent,

    /// Frischer Kontext pro Iteration, keine Zusammenfassung (zustandslos)
    FullyEphemeral,
}
```

**EphemeralWithSummary (Standard)**:
- Neuer `AgentContext` pro Iteration
- Zusammenfassung der vorherigen Iteration wird dem Kontext hinzugefügt
- Verhindert unbegrenztes Speicherwachstum
- Erhält Kontinuität für zusammenhängende Aktionen

**SharedPersistent**:
- Einzelner `AgentContext` wird über alle Iterationen wiederverwendet
- Vollständiger Gesprächsverlauf bleibt erhalten
- Höherer Speicherverbrauch
- Am besten für Agenten mit tiefem Kontextbedarf (z. B. Debugging-Sitzungen)

**FullyEphemeral**:
- Neuer `AgentContext` pro Iteration, keine Übernahme
- Geringstes Speicherprofil
- Am besten für unabhängige Prüfungen (z. B. API-Gesundheitstests)

### Heartbeat-Agent-Beispiel

```symbiont
agent heartbeat_monitor {
  model: "claude-sonnet-4.5"
  system_prompt: """
  Du bist ein Systemüberwachungsagent. Bei jedem Heartbeat:
  1. Systemmetriken prüfen (CPU, Arbeitsspeicher, Festplatte)
  2. Aktuelle Fehlerprotokolle überprüfen
  3. Bei erkannten Problemen handeln:
     - Dienste bei Sicherheit neu starten
     - Ops-Team über Slack benachrichtigen
     - Vorfalldetails protokollieren
  4. Ergebnisse zusammenfassen
  5. 'sleep' zurückgeben, wenn fertig
  """
}

schedule {
  name: "heartbeat-monitor"
  agent: "heartbeat_monitor"
  cron: "*/10 * * * *"  # Alle 10 Minuten

  heartbeat: {
    enabled: true
    context_mode: "ephemeral_with_summary"
    max_iterations: 50
  }

  delivery: ["log_file", "slack"]
  slack_channel: "#ops-alerts"
}
```

## Sitzungsisolation

### Sitzungsmodi

```rust
pub enum SessionIsolationMode {
    /// Kurzlebiger Kontext mit Zusammenfassungsübernahme (Standard)
    EphemeralWithSummary,

    /// Gemeinsamer persistenter Kontext über alle Ausführungen
    SharedPersistent,

    /// Vollständig kurzlebig, keine Zustandsübernahme
    FullyEphemeral,
}
```

**Konfiguration:**

```symbiont
schedule {
  name: "data-pipeline"
  agent: "etl-agent"
  cron: "0 2 * * *"

  # Frischer Kontext pro Ausführung, Zusammenfassung der vorherigen Ausführung enthalten
  session_mode: "ephemeral_with_summary"
}
```

### Sitzungslebenszyklus

Für jede geplante Ausführung:

1. **Vor-Ausführung**: Nebenläufigkeitsgrenzen prüfen, Jitter anwenden
2. **Sitzungserstellung**: `AgentContext` basierend auf `session_mode` erstellen
3. **Richtlinien-Gate**: Richtlinienbedingungen auswerten
4. **Ausführung**: Agent mit Eingabe und Kontext ausführen
5. **Zustellung**: Ausgabe an konfigurierte Kanäle weiterleiten
6. **Sitzungsbereinigung**: Kontext je nach Modus zerstören oder beibehalten
7. **Nach-Ausführung**: Ausführungsdatensatz aktualisieren, Metriken erfassen

## Zustellungsrouting

### Unterstützte Kanäle

```rust
pub enum DeliveryChannel {
    Stdout,           // Auf Konsole ausgeben
    LogFile,          // An auftragsspezifische Protokolldatei anhängen
    Webhook,          // HTTP POST an URL
    Slack,            // Slack-Webhook oder API
    Email,            // SMTP-E-Mail
    Custom(String),   // Benutzerdefinierter Kanal
}
```

### Konfigurationsbeispiele

**Einzelner Kanal:**

```symbiont
schedule {
  name: "backup"
  agent: "backup-agent"
  cron: "0 3 * * *"
  delivery: ["log_file"]
}
```

**Mehrere Kanäle:**

```symbiont
schedule {
  name: "security-scan"
  agent: "scanner"
  cron: "0 1 * * *"

  delivery: ["log_file", "slack", "email"]

  slack_channel: "#security"
  email_recipients: ["ops@example.com", "security@example.com"]
}
```

**Webhook-Zustellung:**

```symbiont
schedule {
  name: "metrics-report"
  agent: "metrics-agent"
  cron: "*/30 * * * *"

  delivery: ["webhook"]
  webhook_url: "https://metrics.example.com/ingest"
  webhook_headers: {
    "Authorization": "Bearer ${METRICS_API_KEY}"
    "Content-Type": "application/json"
  }
}
```

### DeliveryRouter-Trait

Benutzerdefinierte Zustellungskanäle implementieren:

```rust
#[async_trait]
pub trait DeliveryRouter: Send + Sync {
    async fn route(
        &self,
        channel: &DeliveryChannel,
        job: &CronJobDefinition,
        run: &JobRunRecord,
        output: &str,
    ) -> Result<(), SchedulerError>;
}
```

## Richtliniendurchsetzung

### PolicyGate

Das `PolicyGate` wertet zeitplanspezifische Richtlinien vor der Ausführung aus:

```rust
pub struct PolicyGate {
    policy_engine: Arc<RealPolicyParser>,
}

impl PolicyGate {
    pub async fn evaluate(
        &self,
        job: &CronJobDefinition,
        context: &AgentContext,
    ) -> Result<SchedulePolicyDecision, SchedulerError>;
}
```

### Richtlinienbedingungen

```symbiont
schedule {
  name: "production-deploy"
  agent: "deploy-agent"
  cron: "0 0 * * 0"  # Sonntag Mitternacht

  policy {
    # Menschliche Genehmigung vor Ausführung erforderlich
    require_approval: true

    # Maximale Laufzeit vor erzwungenem Abbruch
    max_runtime: "30m"

    # Bestimmte Fähigkeiten erforderlich
    require_capabilities: ["deployment", "production_write"]

    # Zeitfenster-Durchsetzung (UTC)
    allowed_hours: {
      start: "00:00"
      end: "04:00"
    }

    # Umgebungsbeschränkungen
    allowed_environments: ["staging", "production"]

    # AgentPin-Verifizierung erforderlich
    require_agent_pin: true
  }
}
```

### SchedulePolicyDecision

```rust
pub enum SchedulePolicyDecision {
    Allow,
    Deny { reason: String },
    RequireApproval { approvers: Vec<String> },
}
```

## Produktionshärtung

### Jitter

Verhindert den Thundering-Herd-Effekt, wenn mehrere Aufträge denselben Zeitplan teilen:

```rust
pub struct CronSchedulerConfig {
    pub max_jitter_seconds: u64,  // Zufällige Verzögerung 0-N Sekunden
    // ...
}
```

**Beispiel:**

```toml
[scheduler]
max_jitter_seconds = 30  # Job-Starts über ein 30-Sekunden-Fenster verteilen
```

### Nebenläufigkeit pro Auftrag

Begrenzt gleichzeitige Ausführungen pro Auftrag, um Ressourcenerschöpfung zu verhindern:

```symbiont
schedule {
  name: "data-sync"
  agent: "sync-agent"
  cron: "*/5 * * * *"

  max_concurrent: 2  # Maximal 2 gleichzeitige Ausführungen erlauben
}
```

Wenn ein Auftrag bereits mit maximaler Nebenläufigkeit läuft, überspringt der Scheduler den Tick.

### Dead-Letter-Warteschlange

Aufträge, die `max_retries` überschreiten, werden in den Status `DeadLetter` verschoben und müssen manuell überprüft werden:

```symbiont
schedule {
  name: "flaky-job"
  agent: "unreliable-agent"
  cron: "0 * * * *"

  max_retries: 3  # Nach 3 Fehlschlägen in die Dead-Letter-Warteschlange verschieben
}
```

**Wiederherstellung:**

```bash
# Dead-Letter-Aufträge auflisten
symbi cron list --status dead_letter

# Fehlerursachen überprüfen
symbi cron history <job-id> --status failed

# Auftrag nach Behebung zurücksetzen
symbi cron reset <job-id>
```

### AgentPin-Verifizierung

Kryptographische Verifizierung der Agentenidentität vor der Ausführung:

```symbiont
schedule {
  name: "secure-task"
  agent: "trusted-agent"
  cron: "0 * * * *"

  agent_pin_jwt: "${AGENT_PIN_JWT}"  # ES256-JWT von agentpin-cli

  policy {
    require_agent_pin: true
  }
}
```

Der Scheduler verifiziert:
1. JWT-Signatur mittels ES256 (ECDSA P-256)
2. Agenten-ID stimmt mit dem `iss`-Claim überein
3. Domänenanker stimmt mit dem erwarteten Ursprung überein
4. Ablaufzeit (`exp`) ist gültig

Fehlschläge lösen ein `SecurityEventType::AgentPinVerificationFailed`-Audit-Ereignis aus.

## HTTP-API-Endpunkte

### Zeitplanverwaltung

**POST /api/v1/schedule**
Einen neuen geplanten Auftrag erstellen.

```bash
curl -X POST http://localhost:8080/api/v1/schedule \
  -H "Content-Type: application/json" \
  -d '{
    "name": "hourly-report",
    "agent_id": "reporter",
    "cron_expr": "0 * * * *",
    "session_mode": "ephemeral_with_summary",
    "delivery": ["stdout"]
  }'
```

**GET /api/v1/schedule**
Alle Aufträge auflisten (filterbar nach Status, Agenten-ID).

```bash
curl "http://localhost:8080/api/v1/schedule?status=active&agent_id=reporter"
```

**GET /api/v1/schedule/{job_id}**
Auftragsdetails abrufen.

```bash
curl http://localhost:8080/api/v1/schedule/job-123
```

**PUT /api/v1/schedule/{job_id}**
Auftrag aktualisieren (Cron-Ausdruck, Zustellung usw.).

```bash
curl -X PUT http://localhost:8080/api/v1/schedule/job-123 \
  -H "Content-Type: application/json" \
  -d '{"cron_expr": "0 */2 * * *"}'
```

**DELETE /api/v1/schedule/{job_id}**
Auftrag löschen.

```bash
curl -X DELETE http://localhost:8080/api/v1/schedule/job-123
```

**POST /api/v1/schedule/{job_id}/pause**
Auftrag pausieren.

```bash
curl -X POST http://localhost:8080/api/v1/schedule/job-123/pause
```

**POST /api/v1/schedule/{job_id}/resume**
Pausierten Auftrag fortsetzen.

```bash
curl -X POST http://localhost:8080/api/v1/schedule/job-123/resume
```

**POST /api/v1/schedule/{job_id}/run**
Sofortige Ausführung auslösen.

```bash
curl -X POST http://localhost:8080/api/v1/schedule/job-123/run \
  -H "Content-Type: application/json" \
  -d '{"input": "Run with custom input"}'
```

**GET /api/v1/schedule/{job_id}/history**
Ausführungsverlauf abrufen.

```bash
curl "http://localhost:8080/api/v1/schedule/job-123/history?limit=20&status=failed"
```

**GET /api/v1/schedule/{job_id}/next_run**
Nächste geplante Ausführungszeit abrufen.

```bash
curl http://localhost:8080/api/v1/schedule/job-123/next_run
```

### Gesundheitsüberwachung

**GET /api/v1/health/scheduler**
Scheduler-Gesundheit und Metriken.

```bash
curl http://localhost:8080/api/v1/health/scheduler
```

**Antwort:**

```json
{
  "status": "healthy",
  "active_jobs": 15,
  "paused_jobs": 3,
  "in_flight_jobs": 2,
  "metrics": {
    "runs_total": 1234,
    "runs_succeeded": 1180,
    "runs_failed": 54,
    "avg_execution_time_ms": 850
  }
}
```

## SDK-Beispiele

### JavaScript-SDK

```javascript
import { SymbiontClient } from '@symbiont/sdk-js';

const client = new SymbiontClient({
  baseUrl: 'http://localhost:8080',
  apiKey: process.env.SYMBI_API_KEY
});

// Geplanten Auftrag erstellen
const job = await client.schedule.create({
  name: 'daily-backup',
  agentId: 'backup-agent',
  cronExpr: '0 2 * * *',
  sessionMode: 'ephemeral_with_summary',
  delivery: ['webhook'],
  webhookUrl: 'https://backup.example.com/notify'
});

console.log(`Auftrag erstellt: ${job.id}`);

// Aktive Aufträge auflisten
const activeJobs = await client.schedule.list({ status: 'active' });
console.log(`Aktive Aufträge: ${activeJobs.length}`);

// Auftragsstatus abrufen
const status = await client.schedule.getStatus(job.id);
console.log(`Nächste Ausführung: ${status.next_run}`);

// Sofortige Ausführung auslösen
await client.schedule.runNow(job.id, { input: 'Backup database' });

// Auftrag pausieren
await client.schedule.pause(job.id);

// Verlauf anzeigen
const history = await client.schedule.getHistory(job.id, { limit: 10 });
history.forEach(run => {
  console.log(`Ausführung ${run.id}: ${run.status} (${run.execution_time_ms}ms)`);
});

// Auftrag fortsetzen
await client.schedule.resume(job.id);

// Auftrag löschen
await client.schedule.delete(job.id);
```

### Python-SDK

```python
from symbiont import SymbiontClient

client = SymbiontClient(
    base_url='http://localhost:8080',
    api_key=os.environ['SYMBI_API_KEY']
)

# Geplanten Auftrag erstellen
job = client.schedule.create(
    name='hourly-metrics',
    agent_id='metrics-agent',
    cron_expr='0 * * * *',
    session_mode='ephemeral_with_summary',
    delivery=['slack', 'log_file'],
    slack_channel='#metrics'
)

print(f"Auftrag erstellt: {job.id}")

# Aufträge für bestimmten Agenten auflisten
jobs = client.schedule.list(agent_id='metrics-agent')
print(f"{len(jobs)} Aufträge für metrics-agent gefunden")

# Auftragsdetails abrufen
details = client.schedule.get(job.id)
print(f"Cron: {details.cron_expr}")
print(f"Nächste Ausführung: {details.next_run}")

# Cron-Ausdruck aktualisieren
client.schedule.update(job.id, cron_expr='*/30 * * * *')

# Sofortige Ausführung auslösen
run = client.schedule.run_now(job.id, input='Generate metrics report')
print(f"Ausführungs-ID: {run.id}")

# Während Wartung pausieren
client.schedule.pause(job.id)
print("Auftrag für Wartung pausiert")

# Letzte Fehlschläge anzeigen
history = client.schedule.get_history(
    job.id,
    status='failed',
    limit=5
)
for run in history:
    print(f"Fehlgeschlagene Ausführung {run.id}: {run.error_message}")

# Nach Wartung fortsetzen
client.schedule.resume(job.id)

# Scheduler-Gesundheit prüfen
health = client.schedule.health()
print(f"Scheduler-Status: {health.status}")
print(f"Aktive Aufträge: {health.active_jobs}")
print(f"Laufende Aufträge: {health.in_flight_jobs}")
```

## Konfiguration

### CronSchedulerConfig

```rust
pub struct CronSchedulerConfig {
    /// Tick-Intervall in Sekunden (Standard: 1)
    pub tick_interval_seconds: u64,

    /// Maximaler Jitter zur Vermeidung des Thundering-Herd-Effekts (Standard: 0)
    pub max_jitter_seconds: u64,

    /// Globale Nebenläufigkeitsgrenze (Standard: 10)
    pub max_concurrent_jobs: usize,

    /// Metrik-Erfassung aktivieren (Standard: true)
    pub enable_metrics: bool,

    /// Dead-Letter-Wiederholungsschwelle (Standard: 3)
    pub default_max_retries: u32,

    /// Timeout für ordnungsgemäßes Herunterfahren (Standard: 30s)
    pub shutdown_timeout_seconds: u64,
}
```

### TOML-Konfiguration

```toml
[scheduler]
tick_interval_seconds = 1
max_jitter_seconds = 30
max_concurrent_jobs = 20
enable_metrics = true
default_max_retries = 3
shutdown_timeout_seconds = 60

[scheduler.delivery]
# Webhook-Einstellungen
webhook_timeout_seconds = 30
webhook_retry_attempts = 3

# Slack-Einstellungen
slack_api_token = "${SLACK_API_TOKEN}"
slack_default_channel = "#ops"

# E-Mail-Einstellungen
smtp_host = "smtp.example.com"
smtp_port = 587
smtp_username = "${SMTP_USER}"
smtp_password = "${SMTP_PASS}"
email_from = "symbiont@example.com"
```

### Umgebungsvariablen

```bash
# Scheduler-Einstellungen
SYMBI_SCHEDULER_MAX_JITTER=30
SYMBI_SCHEDULER_MAX_CONCURRENT=20

# Zustellungseinstellungen
SYMBI_SLACK_TOKEN=xoxb-...
SYMBI_WEBHOOK_AUTH_HEADER="Bearer secret-token"

# AgentPin-Verifizierung
SYMBI_AGENTPIN_REQUIRED=true
SYMBI_AGENTPIN_DOMAIN=agent.example.com
```

## Beobachtbarkeit

### Metriken (Prometheus-kompatibel)

```
# Gesamtausführungen
symbiont_cron_runs_total{job_name="daily-report",status="succeeded"} 450

# Fehlgeschlagene Ausführungen
symbiont_cron_runs_total{job_name="daily-report",status="failed"} 5

# Ausführungszeit-Histogramm
symbiont_cron_execution_duration_seconds{job_name="daily-report"} 1.234

# Laufende Aufträge (Gauge)
symbiont_cron_in_flight_jobs 3

# Dead-Letter-Aufträge
symbiont_cron_dead_letter_total{job_name="flaky-job"} 2
```

### Audit-Ereignisse

Alle Scheduler-Aktionen erzeugen Sicherheitsereignisse:

```rust
pub enum SecurityEventType {
    CronJobCreated,
    CronJobUpdated,
    CronJobDeleted,
    CronJobPaused,
    CronJobResumed,
    CronJobExecuted,
    CronJobFailed,
    CronJobDeadLettered,
    AgentPinVerificationFailed,
}
```

Audit-Protokoll abfragen:

```bash
symbi audit query --type CronJobFailed --since "2026-02-01" --limit 50
```

## Best Practices

1. **Jitter für gemeinsame Zeitpläne verwenden**: Verhindert den gleichzeitigen Start mehrerer Aufträge
2. **Nebenläufigkeitsgrenzen setzen**: Schutz vor Ressourcenerschöpfung
3. **Dead-Letter-Warteschlange überwachen**: Fehlgeschlagene Aufträge regelmäßig überprüfen und beheben
4. **EphemeralWithSummary verwenden**: Verhindert unbegrenztes Speicherwachstum bei langlebigen Heartbeats
5. **AgentPin-Verifizierung aktivieren**: Kryptographische Verifizierung der Agentenidentität
6. **Zustellungsrouting konfigurieren**: Geeignete Kanäle für verschiedene Auftragstypen verwenden
7. **Richtlinien-Gates setzen**: Zeitfenster, Genehmigungen und Fähigkeitsprüfungen erzwingen
8. **Heartbeat-Muster für Überwachung verwenden**: Kontinuierliche Bewertung-Aktion-Schlaf-Zyklen
9. **Zeitpläne im Staging testen**: Cron-Ausdrücke und Auftragslogik vor der Produktion validieren
10. **Metriken exportieren**: Integration mit Prometheus/Grafana für betriebliche Sichtbarkeit

## Fehlerbehebung

### Auftrag wird nicht ausgeführt

1. Auftragsstatus prüfen: `symbi cron status <job-id>`
2. Cron-Ausdruck verifizieren: [crontab.guru](https://crontab.guru/) verwenden
3. Scheduler-Gesundheit prüfen: `curl http://localhost:8080/api/v1/health/scheduler`
4. Logs überprüfen: `symbi logs --filter scheduler --level debug`

### Auftrag schlägt wiederholt fehl

1. Verlauf anzeigen: `symbi cron history <job-id> --status failed`
2. Fehlermeldungen in Ausführungsdatensätzen prüfen
3. Agentenkonfiguration und -fähigkeiten verifizieren
4. Agent außerhalb des Schedulers testen: `symbi run <agent-id> --input "test"`
5. Richtlinien-Gates prüfen: Zeitfenster und Fähigkeiten überprüfen

### Dead-Letter-Auftrag

1. Dead-Letter-Aufträge auflisten: `symbi cron list --status dead_letter`
2. Fehlermuster überprüfen: `symbi cron history <job-id>`
3. Grundursache beheben (Agentencode, Berechtigungen, externe Abhängigkeiten)
4. Auftrag zurücksetzen: `symbi cron reset <job-id>`

### Hoher Speicherverbrauch

1. Sitzungsmodus prüfen: Auf `ephemeral_with_summary` oder `fully_ephemeral` umstellen
2. Heartbeat-Iterationen reduzieren: `max_iterations` verringern
3. Kontextgröße überwachen: Ausführlichkeit der Agentenausgabe überprüfen
4. Kontextarchivierung aktivieren: Aufbewahrungsrichtlinien konfigurieren

## Migration von v0.9.0

Das Release v1.0.0 fügt Produktionshärtungsfunktionen hinzu. Aktualisieren Sie Ihre Auftragsdefinitionen:

```diff
 schedule {
   name: "my-job"
   agent: "my-agent"
   cron: "0 * * * *"
+
+  # Nebenläufigkeitsgrenze hinzufügen
+  max_concurrent: 2
+
+  # AgentPin für Identitätsverifizierung hinzufügen
+  agent_pin_jwt: "${AGENT_PIN_JWT}"
+
+  policy {
+    require_agent_pin: true
+  }
 }
```

Konfiguration aktualisieren:

```diff
 [scheduler]
 tick_interval_seconds = 1
+ max_jitter_seconds = 30
+ default_max_retries = 3
+ shutdown_timeout_seconds = 60
```

Keine API-Breaking-Changes. Alle v0.9.0-Aufträge funktionieren weiterhin.
