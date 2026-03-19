# Scheduling-Leitfaden

## Andere Sprachen

[English](scheduling.md) | [中文简体](scheduling.zh-cn.md) | [Español](scheduling.es.md) | [Português](scheduling.pt.md) | [日本語](scheduling.ja.md) | **Deutsch**

---

## Ueberblick

Das Scheduling-System von Symbiont bietet produktionsreife Cron-basierte Aufgabenausfuehrung fuer KI-Agenten. Das System unterstuetzt:

- **Cron-Zeitplaene**: Traditionelle Cron-Syntax fuer wiederkehrende Aufgaben
- **Einmalige Auftraege**: Einmalige Ausfuehrung zu einem bestimmten Zeitpunkt
- **Heartbeat-Muster**: Kontinuierliche Bewertung-Aktion-Schlaf-Zyklen fuer Ueberwachungsagenten
- **Sitzungsisolation**: Kurzlebige, gemeinsame oder vollstaendig isolierte Agentenkontexte
- **Zustellungsrouting**: Mehrere Ausgabekanaele (Stdout, LogFile, Webhook, Slack, Email, Custom)
- **Richtliniendurchsetzung**: Sicherheits- und Compliance-Pruefungen vor der Ausfuehrung
- **Produktionshaertung**: Jitter, Nebenlaeufigkeitsgrenzen, Dead-Letter-Warteschlangen und AgentPin-Verifizierung

## Architektur

Das Scheduling-System basiert auf drei Kernkomponenten:

```
┌─────────────────────┐
│   CronScheduler     │  Hintergrund-Tick-Schleife (1-Sekunden-Intervalle)
│   (Tick Loop)       │  Job-Auswahl und Ausfuehrungsorchestrierung
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│   SqliteJobStore    │  Persistente Job-Speicherung
│   (Job Storage)     │  Transaktionsunterstuetzung, Zustandsverwaltung
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│DefaultAgentScheduler│  Agenten-Ausfuehrungslaufzeit
│ (Execution Engine)  │  AgentContext-Lebenszyklusverwaltung
└─────────────────────┘
```

### CronScheduler

Der `CronScheduler` ist der primaere Einstiegspunkt. Er verwaltet:

- Hintergrund-Tick-Schleife mit 1-Sekunden-Intervallen
- Job-Auswahl basierend auf der naechsten Ausfuehrungszeit
- Nebenlaeufigkeitssteuerung und Jitter-Injektion
- Metrik-Erfassung und Gesundheitsueberwachung
- Ordnungsgemaesses Herunterfahren mit Nachverfolgung laufender Jobs

### SqliteJobStore

Der `SqliteJobStore` bietet dauerhafte Job-Persistenz mit:

- ACID-Transaktionen fuer Job-Zustandsaktualisierungen
- Job-Lebenszyklus-Tracking (Active, Paused, Completed, Failed, DeadLetter)
- Ausfuehrungshistorie mit Pruefprotokoll
- Abfragefunktionen zum Filtern nach Status, Agenten-ID usw.

### DefaultAgentScheduler

Der `DefaultAgentScheduler` fuehrt geplante Agenten aus:

- Erstellt isolierte oder gemeinsame `AgentContext`-Instanzen
- Verwaltet den Sitzungslebenszyklus (Erstellen, Ausfuehren, Zerstoeren)
- Leitet Zustellungen an konfigurierte Kanaele weiter
- Erzwingt Richtlinien-Gates vor der Ausfuehrung

## DSL-Syntax

### Zeitplanblock-Struktur

Zeitplanboecke werden in Symbiont-DSL-Dateien definiert:

```symbiont
schedule {
  name: "daily-report"
  agent: "reporter-agent"
  cron: "0 0 9 * * *"

  session_mode: "ephemeral_with_summary"
  delivery: ["stdout", "log_file"]

  policy {
    require_approval: false
    max_runtime: "5m"
  }
}
```

### Cron-Syntax

Erweiterte Cron-Syntax mit sechs Feldern (Sekunden zuerst, optionales siebtes Feld fuer Jahr):

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

### Einmalige Auftraege (At-Syntax)

Fuer Auftraege, die einmalig zu einem bestimmten Zeitpunkt ausgefuehrt werden:

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

Fuer kontinuierliche Ueberwachungsagenten, die bewerten, handeln und schlafen:

```symbiont
schedule {
  name: "system-monitor"
  agent: "heartbeat-agent"
  cron: "0 */5 * * * *"  # Alle 5 Minuten aufwachen

  heartbeat: {
    enabled: true
    context_mode: "ephemeral_with_summary"
    max_iterations: 100  # Sicherheitslimit
  }
}
```

Der Heartbeat-Agent folgt diesem Zyklus:

1. **Bewertung**: Systemzustand auswerten (z. B. Metriken, Logs pruefen)
2. **Aktion**: Bei Bedarf Korrekturmassnahmen ergreifen (z. B. Dienst neustarten, Ops benachrichtigen)
3. **Schlaf**: Bis zum naechsten geplanten Tick warten

## CLI-Befehle

Der Befehl `symbi cron` bietet vollstaendige Lebenszyklus-Verwaltung:

### Auftraege auflisten

```bash
# Alle Auftraege auflisten
symbi cron list

# Nach Status filtern
symbi cron list --status active
symbi cron list --status paused

# Nach Agent filtern
symbi cron list --agent "reporter-agent"

# JSON-Ausgabe
symbi cron list --format json
```

### Auftrag hinzufuegen

```bash
# Aus DSL-Datei
symbi cron add --file agent.symbi --schedule "daily-report"

# Inline-Definition (JSON)
symbi cron add --json '{
  "name": "quick-task",
  "agent_id": "agent-123",
  "cron_expr": "0 0 * * * *"
}'
```

### Auftrag entfernen

```bash
# Nach Job-ID
symbi cron remove <job-id>

# Nach Name
symbi cron remove --name "daily-report"

# Erzwungenes Entfernen (Bestaetigung ueberspringen)
symbi cron remove <job-id> --force
```

### Pausieren/Fortsetzen

```bash
# Auftrag pausieren (stoppt Planung, behaelt Zustand)
symbi cron pause <job-id>

# Pausierten Auftrag fortsetzen
symbi cron resume <job-id>
```

### Status

```bash
# Auftragsdetails mit naechster Ausfuehrungszeit
symbi cron status <job-id>

# Die letzten 10 Ausfuehrungsdatensaetze einbeziehen
symbi cron status <job-id> --history 10

# Ueberwachungsmodus (automatische Aktualisierung alle 5 Sekunden)
symbi cron status <job-id> --watch
```

### Sofort ausfuehren

```bash
# Sofortige Ausfuehrung ausloesen (umgeht den Zeitplan)
symbi cron run <job-id>

# Mit benutzerdefinierter Eingabe
symbi cron run <job-id> --input "Check production database"
```

### Verlauf

```bash
# Ausfuehrungsverlauf eines Auftrags anzeigen
symbi cron history <job-id>

# Letzte 20 Ausfuehrungen
symbi cron history <job-id> --limit 20

# Nach Status filtern
symbi cron history <job-id> --status failed

# Als CSV exportieren
symbi cron history <job-id> --format csv > runs.csv
```

## Heartbeat-Muster

### HeartbeatContextMode

Steuert, wie der Kontext ueber Heartbeat-Iterationen hinweg erhalten bleibt:

```rust
pub enum HeartbeatContextMode {
    /// Fresh context each iteration, append summary to run history
    EphemeralWithSummary,

    /// Shared context across all iterations (memory accumulates)
    SharedPersistent,

    /// Fresh context each iteration, no summary (stateless)
    FullyEphemeral,
}
```

**EphemeralWithSummary (Standard)**:
- Neuer `AgentContext` pro Iteration
- Zusammenfassung der vorherigen Iteration wird dem Kontext hinzugefuegt
- Verhindert unbegrenztes Speicherwachstum
- Erhaelt Kontinuitaet fuer zusammenhaengende Aktionen

**SharedPersistent**:
- Einzelner `AgentContext` wird ueber alle Iterationen wiederverwendet
- Vollstaendiger Gespraechsverlauf bleibt erhalten
- Hoeherer Speicherverbrauch
- Am besten fuer Agenten mit tiefem Kontextbedarf (z. B. Debugging-Sitzungen)

**FullyEphemeral**:
- Neuer `AgentContext` pro Iteration, keine Uebernahme
- Geringstes Speicherprofil
- Am besten fuer unabhaengige Pruefungen (z. B. API-Gesundheitstests)

### Heartbeat-Agent-Beispiel

```symbiont
agent heartbeat_monitor {
  model: "claude-sonnet-4.5"
  system_prompt: """
  You are a system monitoring agent. On each heartbeat:
  1. Check system metrics (CPU, memory, disk)
  2. Review recent error logs
  3. If issues detected, take action:
     - Restart services if safe
     - Alert ops team via Slack
     - Log incident details
  4. Summarize findings
  5. Return 'sleep' when done
  """
}

schedule {
  name: "heartbeat-monitor"
  agent: "heartbeat_monitor"
  cron: "0 */10 * * * *"  # Alle 10 Minuten

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
pub enum HeartbeatContextMode {
    /// Ephemeral context with summary carryover (default)
    EphemeralWithSummary,

    /// Shared persistent context across all runs
    SharedPersistent,

    /// Fully ephemeral, no state carryover
    FullyEphemeral,
}
```

**Konfiguration:**

```symbiont
schedule {
  name: "data-pipeline"
  agent: "etl-agent"
  cron: "0 0 2 * * *"

  # Frischer Kontext pro Ausfuehrung, Zusammenfassung der vorherigen Ausfuehrung enthalten
  session_mode: "ephemeral_with_summary"
}
```

### Sitzungslebenszyklus

Fuer jede geplante Ausfuehrung:

1. **Vor-Ausfuehrung**: Nebenlaeufigkeitsgrenzen pruefen, Jitter anwenden
2. **Sitzungserstellung**: `AgentContext` basierend auf `session_mode` erstellen
3. **Richtlinien-Gate**: Richtlinienbedingungen auswerten
4. **Ausfuehrung**: Agent mit Eingabe und Kontext ausfuehren
5. **Zustellung**: Ausgabe an konfigurierte Kanaele weiterleiten
6. **Sitzungsbereinigung**: Kontext je nach Modus zerstoeren oder beibehalten
7. **Nach-Ausfuehrung**: Ausfuehrungsdatensatz aktualisieren, Metriken erfassen

## Zustellungsrouting

### Unterstuetzte Kanaele

```rust
pub enum DeliveryChannel {
    Stdout,           // Print to console
    LogFile,          // Append to job-specific log file
    Webhook,          // HTTP POST to URL
    Slack,            // Slack webhook or API
    Email,            // SMTP email
    Custom(String),   // User-defined channel
}
```

### Konfigurationsbeispiele

**Einzelner Kanal:**

```symbiont
schedule {
  name: "backup"
  agent: "backup-agent"
  cron: "0 0 3 * * *"
  delivery: ["log_file"]
}
```

**Mehrere Kanaele:**

```symbiont
schedule {
  name: "security-scan"
  agent: "scanner"
  cron: "0 0 1 * * *"

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
  cron: "0 */30 * * * *"

  delivery: ["webhook"]
  webhook_url: "https://metrics.example.com/ingest"
  webhook_headers: {
    "Authorization": "Bearer ${METRICS_API_KEY}"
    "Content-Type": "application/json"
  }
}
```

### DeliveryRouter-Trait

Benutzerdefinierte Zustellungskanaele implementieren:

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

Das `PolicyGate` wertet zeitplanspezifische Richtlinien vor der Ausfuehrung aus:

```rust
pub struct PolicyGate {
    policy_engine: Arc<RealPolicyParser>,
}

impl PolicyGate {
    pub fn evaluate(
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
  cron: "0 0 0 * * 0"  # Sonntag Mitternacht

  policy {
    # Menschliche Genehmigung vor Ausfuehrung erforderlich
    require_approval: true

    # Maximale Laufzeit vor erzwungenem Abbruch
    max_runtime: "30m"

    # Bestimmte Faehigkeiten erforderlich
    require_capabilities: ["deployment", "production_write"]

    # Zeitfenster-Durchsetzung (UTC)
    allowed_hours: {
      start: "00:00"
      end: "04:00"
    }

    # Umgebungsbeschraenkungen
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
    RequiresApproval { approver: String, reason: String, policy_id: String },
}
```

## Produktionshaertung

### Jitter

Verhindert den Thundering-Herd-Effekt, wenn mehrere Auftraege denselben Zeitplan teilen:

```rust
pub struct CronSchedulerConfig {
    pub max_jitter_seconds: u64,  // Random delay 0-N seconds
    // ...
}
```

**Beispiel:**

```toml
[scheduler]
max_jitter_seconds = 30  # Job-Starts ueber ein 30-Sekunden-Fenster verteilen
```

### Nebenlaeufigkeit pro Auftrag

Begrenzt gleichzeitige Ausfuehrungen pro Auftrag, um Ressourcenerschoepfung zu verhindern:

```symbiont
schedule {
  name: "data-sync"
  agent: "sync-agent"
  cron: "0 */5 * * * *"

  max_concurrent: 2  # Maximal 2 gleichzeitige Ausfuehrungen erlauben
}
```

Wenn ein Auftrag bereits mit maximaler Nebenlaeufigkeit laeuft, ueberspringt der Scheduler den Tick.

### Dead-Letter-Warteschlange

Auftraege, die `max_retries` ueberschreiten, werden in den Status `DeadLetter` verschoben und muessen manuell ueberprueft werden:

```symbiont
schedule {
  name: "flaky-job"
  agent: "unreliable-agent"
  cron: "0 0 * * * *"

  max_retries: 3  # Nach 3 Fehlschlaegen in die Dead-Letter-Warteschlange verschieben
}
```

**Wiederherstellung:**

```bash
# Dead-Letter-Auftraege auflisten
symbi cron list --status dead_letter

# Fehlerursachen ueberpruefen
symbi cron history <job-id> --status failed

# Auftrag nach Behebung zuruecksetzen
symbi cron reset <job-id>
```

### AgentPin-Verifizierung

Kryptographische Verifizierung der Agentenidentitaet vor der Ausfuehrung:

```symbiont
schedule {
  name: "secure-task"
  agent: "trusted-agent"
  cron: "0 0 * * * *"

  agent_pin_jwt: "${AGENT_PIN_JWT}"  # ES256-JWT von agentpin-cli

  policy {
    require_agent_pin: true
  }
}
```

Der Scheduler verifiziert:
1. JWT-Signatur mittels ES256 (ECDSA P-256)
2. Agenten-ID stimmt mit dem `iss`-Claim ueberein
3. Domaenenanker stimmt mit dem erwarteten Ursprung ueberein
4. Ablaufzeit (`exp`) ist gueltig

Fehlschlaege loesen ein `SecurityEventType::AgentPinVerificationFailed`-Audit-Ereignis aus.

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
    "cron_expr": "0 0 * * * *",
    "session_mode": "ephemeral_with_summary",
    "delivery": ["stdout"]
  }'
```

**GET /api/v1/schedule**
Alle Auftraege auflisten (filterbar nach Status, Agenten-ID).

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
  -d '{"cron_expr": "0 0 */2 * * *"}'
```

**DELETE /api/v1/schedule/{job_id}**
Auftrag loeschen.

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
Sofortige Ausfuehrung ausloesen.

```bash
curl -X POST http://localhost:8080/api/v1/schedule/job-123/run \
  -H "Content-Type: application/json" \
  -d '{"input": "Run with custom input"}'
```

**GET /api/v1/schedule/{job_id}/history**
Ausfuehrungsverlauf abrufen.

```bash
curl "http://localhost:8080/api/v1/schedule/job-123/history?limit=20&status=failed"
```

**GET /api/v1/schedule/{job_id}/next_run**
Naechste geplante Ausfuehrungszeit abrufen.

```bash
curl http://localhost:8080/api/v1/schedule/job-123/next_run
```

### Gesundheitsueberwachung

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
  cronExpr: '0 0 2 * * *',
  sessionMode: 'ephemeral_with_summary',
  delivery: ['webhook'],
  webhookUrl: 'https://backup.example.com/notify'
});

console.log(`Created job: ${job.id}`);

// Aktive Auftraege auflisten
const activeJobs = await client.schedule.list({ status: 'active' });
console.log(`Active jobs: ${activeJobs.length}`);

// Auftragsstatus abrufen
const status = await client.schedule.getStatus(job.id);
console.log(`Next run: ${status.next_run}`);

// Sofortige Ausfuehrung ausloesen
await client.schedule.runNow(job.id, { input: 'Backup database' });

// Auftrag pausieren
await client.schedule.pause(job.id);

// Verlauf anzeigen
const history = await client.schedule.getHistory(job.id, { limit: 10 });
history.forEach(run => {
  console.log(`Run ${run.id}: ${run.status} (${run.execution_time_ms}ms)`);
});

// Auftrag fortsetzen
await client.schedule.resume(job.id);

// Auftrag loeschen
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
    cron_expr='0 0 * * * *',
    session_mode='ephemeral_with_summary',
    delivery=['slack', 'log_file'],
    slack_channel='#metrics'
)

print(f"Created job: {job.id}")

# Auftraege fuer bestimmten Agenten auflisten
jobs = client.schedule.list(agent_id='metrics-agent')
print(f"Found {len(jobs)} jobs for metrics-agent")

# Auftragsdetails abrufen
details = client.schedule.get(job.id)
print(f"Cron: {details.cron_expr}")
print(f"Next run: {details.next_run}")

# Cron-Ausdruck aktualisieren
client.schedule.update(job.id, cron_expr='0 */30 * * * *')

# Sofortige Ausfuehrung ausloesen
run = client.schedule.run_now(job.id, input='Generate metrics report')
print(f"Run ID: {run.id}")

# Waehrend Wartung pausieren
client.schedule.pause(job.id)
print("Job paused for maintenance")

# Letzte Fehlschlaege anzeigen
history = client.schedule.get_history(
    job.id,
    status='failed',
    limit=5
)
for run in history:
    print(f"Failed run {run.id}: {run.error_message}")

# Nach Wartung fortsetzen
client.schedule.resume(job.id)

# Scheduler-Gesundheit pruefen
health = client.schedule.health()
print(f"Scheduler status: {health.status}")
print(f"Active jobs: {health.active_jobs}")
print(f"In-flight jobs: {health.in_flight_jobs}")
```

## Konfiguration

### CronSchedulerConfig

```rust
pub struct CronSchedulerConfig {
    /// Tick interval (default: 1 second)
    pub tick_interval: Duration,

    /// Global concurrency limit (default: 100)
    pub max_concurrent_cron_jobs: usize,

    /// Persistent job store path (default: None)
    pub job_store_path: Option<PathBuf>,

    /// Catch up missed runs on startup (default: true)
    pub enable_missed_run_catchup: bool,
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
# Gesamtausfuehrungen
symbiont_cron_runs_total{job_name="daily-report",status="succeeded"} 450

# Fehlgeschlagene Ausfuehrungen
symbiont_cron_runs_total{job_name="daily-report",status="failed"} 5

# Ausfuehrungszeit-Histogramm
symbiont_cron_execution_duration_seconds{job_name="daily-report"} 1.234

# Laufende Auftraege (Gauge)
symbiont_cron_in_flight_jobs 3

# Dead-Letter-Auftraege
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

1. **Jitter fuer gemeinsame Zeitplaene verwenden**: Verhindert den gleichzeitigen Start mehrerer Auftraege
2. **Nebenlaeufigkeitsgrenzen setzen**: Schutz vor Ressourcenerschoepfung
3. **Dead-Letter-Warteschlange ueberwachen**: Fehlgeschlagene Auftraege regelmaessig ueberpruefen und beheben
4. **EphemeralWithSummary verwenden**: Verhindert unbegrenztes Speicherwachstum bei langlebigen Heartbeats
5. **AgentPin-Verifizierung aktivieren**: Kryptographische Verifizierung der Agentenidentitaet
6. **Zustellungsrouting konfigurieren**: Geeignete Kanaele fuer verschiedene Auftragstypen verwenden
7. **Richtlinien-Gates setzen**: Zeitfenster, Genehmigungen und Faehigkeitspruefungen erzwingen
8. **Heartbeat-Muster fuer Ueberwachung verwenden**: Kontinuierliche Bewertung-Aktion-Schlaf-Zyklen
9. **Zeitplaene im Staging testen**: Cron-Ausdruecke und Auftragslogik vor der Produktion validieren
10. **Metriken exportieren**: Integration mit Prometheus/Grafana fuer betriebliche Sichtbarkeit

## Fehlerbehebung

### Auftrag wird nicht ausgefuehrt

1. Auftragsstatus pruefen: `symbi cron status <job-id>`
2. Cron-Ausdruck verifizieren: [crontab.guru](https://crontab.guru/) verwenden
3. Scheduler-Gesundheit pruefen: `curl http://localhost:8080/api/v1/health/scheduler`
4. Logs ueberpruefen: `symbi logs --filter scheduler --level debug`

### Auftrag schlaegt wiederholt fehl

1. Verlauf anzeigen: `symbi cron history <job-id> --status failed`
2. Fehlermeldungen in Ausfuehrungsdatensaetzen pruefen
3. Agentenkonfiguration und -faehigkeiten verifizieren
4. Agent ausserhalb des Schedulers testen: `symbi run <agent-id> --input "test"`
5. Richtlinien-Gates pruefen: Zeitfenster und Faehigkeiten ueberpruefen

### Dead-Letter-Auftrag

1. Dead-Letter-Auftraege auflisten: `symbi cron list --status dead_letter`
2. Fehlermuster ueberpruefen: `symbi cron history <job-id>`
3. Grundursache beheben (Agentencode, Berechtigungen, externe Abhaengigkeiten)
4. Auftrag zuruecksetzen: `symbi cron reset <job-id>`

### Hoher Speicherverbrauch

1. Sitzungsmodus pruefen: Auf `ephemeral_with_summary` oder `fully_ephemeral` umstellen
2. Heartbeat-Iterationen reduzieren: `max_iterations` verringern
3. Kontextgroesse ueberwachen: Ausfuehrlichkeit der Agentenausgabe ueberpruefen
4. Kontextarchivierung aktivieren: Aufbewahrungsrichtlinien konfigurieren

## Migration von v0.9.0

Das Release v1.0.0 fuegt Produktionshaertungsfunktionen hinzu. Aktualisieren Sie Ihre Auftragsdefinitionen:

```diff
 schedule {
   name: "my-job"
   agent: "my-agent"
   cron: "0 0 * * * *"
+
+  # Nebenlaeufigkeitsgrenze hinzufuegen
+  max_concurrent: 2
+
+  # AgentPin fuer Identitaetsverifizierung hinzufuegen
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

Keine API-Breaking-Changes. Alle v0.9.0-Auftraege funktionieren weiterhin.
