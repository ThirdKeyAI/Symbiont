# HTTP-Eingabe-Modul

Das HTTP-Eingabe-Modul stellt einen Webhook-Server bereit, der es externen Systemen ermoeglicht, Symbiont-Agenten ueber HTTP-Anfragen aufzurufen. Dieses Modul ermoeglicht die Integration mit externen Diensten, Webhooks und APIs, indem es Agenten ueber HTTP-Endpunkte verfuegbar macht.

## Ueberblick

Das HTTP-Eingabe-Modul besteht aus:

- **HTTP-Server**: Ein Axum-basierter Webserver, der auf eingehende HTTP-Anfragen lauscht
- **Authentifizierung**: Unterstuetzung fuer Bearer-Token- und JWT-basierte Authentifizierung
- **Anfrage-Routing**: Flexible Routing-Regeln zur Weiterleitung von Anfragen an spezifische Agenten
- **Antwort-Kontrolle**: Konfigurierbare Antwortformatierung und Statuscodes
- **Sicherheitsfeatures**: CORS-Unterstuetzung, Anfragengroessenlimits und Audit-Logging
- **Parallelitaetsverwaltung**: Eingebaute Anfrage-Ratenbegrenzung und Parallelitaetskontrolle
- **LLM-Aufruf mit ToolClad**: Wenn der Ziel-Agent nicht aktiv auf dem Laufzeit-Kommunikationsbus laeuft, kann der Webhook den Agenten bei Bedarf ueber einen konfigurierten LLM-Anbieter aufrufen und dabei eine ORGA-artige Tool-Calling-Schleife verwenden, die durch ToolClad-Manifeste gestuetzt wird

Das Modul wird bedingt mit dem `http-input` Feature-Flag kompiliert und integriert sich nahtlos in die Symbiont-Agenten-Laufzeitumgebung.

## Konfiguration

Das HTTP-Eingabe-Modul wird mit der [`HttpInputConfig`](../crates/runtime/src/http_input/config.rs) Struktur konfiguriert:

### Grundkonfiguration

```rust
use symbiont_runtime::http_input::HttpInputConfig;
use symbiont_runtime::types::AgentId;

let config = HttpInputConfig {
    bind_address: "127.0.0.1".to_string(),
    port: 8081,
    path: "/webhook".to_string(),
    agent: AgentId::from_str("webhook_handler")?,
    // ... other fields
    ..Default::default()
};
```

### Konfigurationsfelder

| Feld | Typ | Standard | Beschreibung |
|-------|------|---------|-------------|
| `bind_address` | `String` | `"127.0.0.1"` | IP-Adresse zum Binden des HTTP-Servers |
| `port` | `u16` | `8081` | Portnummer zum Lauschen |
| `path` | `String` | `"/webhook"` | HTTP-Pfad-Endpunkt |
| `agent` | `AgentId` | Neue ID | Standard-Agent fuer Anfragen aufzurufen |
| `auth_header` | `Option<String>` | `None` | Bearer-Token fuer Authentifizierung |
| `jwt_public_key_path` | `Option<String>` | `None` | Pfad zur JWT-Public-Key-Datei |
| `max_body_bytes` | `usize` | `65536` | Maximale Anfrage-Body-Groesse (64 KB) |
| `concurrency` | `usize` | `10` | Maximale gleichzeitige Anfragen |
| `routing_rules` | `Option<Vec<AgentRoutingRule>>` | `None` | Anfrage-Routing-Regeln |
| `response_control` | `Option<ResponseControlConfig>` | `None` | Antwortformatierungskonfiguration |
| `forward_headers` | `Vec<String>` | `[]` | Header zur Weiterleitung an Agenten |
| `cors_origins` | `Vec<String>` | `[]` | Erlaubte CORS-Urspruenge (leer = CORS deaktiviert) |
| `audit_enabled` | `bool` | `true` | Anfrage-Audit-Logging aktivieren |

### Agenten-Routing-Regeln

Anfragen basierend auf Anfrageeigenschaften an verschiedene Agenten weiterleiten:

```rust
use symbiont_runtime::http_input::{AgentRoutingRule, RouteMatch};

let routing_rules = vec![
    AgentRoutingRule {
        condition: RouteMatch::PathPrefix("/api/github".to_string()),
        agent: AgentId::from_str("github_handler")?,
    },
    AgentRoutingRule {
        condition: RouteMatch::HeaderEquals("X-Source".to_string(), "slack".to_string()),
        agent: AgentId::from_str("slack_handler")?,
    },
    AgentRoutingRule {
        condition: RouteMatch::JsonFieldEquals("source".to_string(), "twilio".to_string()),
        agent: AgentId::from_str("sms_handler")?,
    },
];
```

### Antwort-Kontrolle

HTTP-Antworten mit [`ResponseControlConfig`](../crates/runtime/src/http_input/config.rs) anpassen:

```rust
use symbiont_runtime::http_input::ResponseControlConfig;

let response_control = ResponseControlConfig {
    default_status: 200,
    agent_output_to_json: true,
    error_status: 500,
    echo_input_on_error: false,
};
```

## Sicherheitsfeatures

### Authentifizierung

Das HTTP-Eingabe-Modul unterstuetzt mehrere Authentifizierungsmethoden:

#### Bearer-Token-Authentifizierung

Statischen Bearer-Token konfigurieren:

```rust
let config = HttpInputConfig {
    auth_header: Some("Bearer your-secret-token".to_string()),
    ..Default::default()
};
```

#### Secret-Store-Integration

Secret-Referenzen fuer erweiterte Sicherheit verwenden:

```rust
let config = HttpInputConfig {
    auth_header: Some("vault://webhook/auth_token".to_string()),
    ..Default::default()
};
```

#### JWT-Authentifizierung (EdDSA)

JWT-basierte Authentifizierung mit Ed25519-Public-Keys konfigurieren:

```rust
let config = HttpInputConfig {
    jwt_public_key_path: Some("/path/to/jwt/ed25519-public.pem".to_string()),
    ..Default::default()
};
```

Der JWT-Verifizierer laedt einen Ed25519-Public-Key aus der angegebenen PEM-Datei und validiert eingehende `Authorization: Bearer <jwt>`-Token. Nur der **EdDSA**-Algorithmus wird akzeptiert -- HS256, RS256 und andere Algorithmen werden abgelehnt.

#### Health-Endpunkt

Das HTTP-Eingabe-Modul stellt keinen eigenen `/health`-Endpunkt bereit. Gesundheitspruefungen sind ueber die Haupt-HTTP-API unter `/api/v1/health` verfuegbar, wenn `symbi up` ausgefuehrt wird, das die vollstaendige Laufzeitumgebung einschliesslich des API-Servers startet:

```bash
# Gesundheitspruefung ueber den Haupt-API-Server (Standard-Port 8080)
curl http://127.0.0.1:8080/api/v1/health
# => {"status": "ok"}
```

Wenn Sie Gesundheitstests speziell fuer den HTTP-Eingabe-Server benoetigen, leiten Sie Ihren Load Balancer stattdessen an den Haupt-API-Gesundheitsendpunkt weiter.

### Sicherheitskontrollen

- **Nur-Loopback-Standard**: `bind_address` ist standardmaessig `127.0.0.1` -- der Server akzeptiert nur lokale Verbindungen, sofern nicht explizit anders konfiguriert
- **CORS standardmaessig deaktiviert**: `cors_origins` ist standardmaessig eine leere Liste, was bedeutet, dass CORS deaktiviert ist; fuegen Sie spezifische Urspruenge hinzu, um Cross-Origin-Zugriff zu ermoeglichen
- **Anfragengroessenlimits**: Konfigurierbare maximale Body-Groesse verhindert Ressourcenerschoepfung
- **Parallelitaetslimits**: Eingebauter Semaphor kontrolliert gleichzeitige Anfragebearbeitung
- **Audit-Logging**: Strukturiertes Logging aller eingehenden Anfragen bei Aktivierung
- **Secret-Aufloesung**: Integration mit Vault und dateibasierten Secret-Stores

## Verwendungsbeispiel

### HTTP-Eingabe-Server starten

```rust
use symbiont_runtime::http_input::{HttpInputConfig, start_http_input};
use symbiont_runtime::secrets::SecretsConfig;
use std::sync::Arc;

// HTTP-Eingabe-Server konfigurieren
let config = HttpInputConfig {
    bind_address: "127.0.0.1".to_string(),
    port: 8081,
    path: "/webhook".to_string(),
    agent: AgentId::from_str("webhook_handler")?,
    auth_header: Some("Bearer secret-token".to_string()),
    audit_enabled: true,
    cors_origins: vec!["https://example.com".to_string()],
    ..Default::default()
};

// Optional: Secrets konfigurieren
let secrets_config = SecretsConfig::default();

// Server starten
start_http_input(config, Some(runtime), Some(secrets_config)).await?;
```

### Beispiel-Agenten-Definition

Webhook-Handler-Agent in [`webhook_handler.dsl`](../agents/webhook_handler.dsl) erstellen:

```dsl
agent webhook_handler(body: JSON) -> Maybe<Alert> {
    capabilities = ["http_input", "event_processing", "alerting"]
    memory = "ephemeral"
    privacy = "strict"

    policy webhook_guard {
        allow: use("llm") if body.source == "slack" || body.user.ends_with("@company.com")
        allow: publish("topic://alerts") if body.type == "security_alert"
        audit: all_operations
    }

    with context = {} {
        if body.type == "security_alert" {
            alert = {
                "summary": body.message,
                "source": body.source,
                "level": body.severity,
                "user": body.user
            }
            publish("topic://alerts", alert)
            return alert
        }

        return None
    }
}
```

### Beispiel-HTTP-Anfrage

Webhook-Anfrage senden, um den Agenten auszuloesen:

```bash
curl -X POST http://localhost:8081/webhook \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer secret-token" \
  -d '{
    "type": "security_alert",
    "message": "Suspicious login detected",
    "source": "slack",
    "severity": "high",
    "user": "admin@company.com"
  }'
```

### Erwartete Antwort

Die Antwortstruktur haengt davon ab, wie der Agent aufgerufen wurde.

**Laufzeit-Dispatch** -- der Ziel-Agent ist `Running` auf dem Kommunikationsbus und die Nachricht wurde zur asynchronen Verarbeitung uebergeben:

```json
{
  "status": "execution_started",
  "agent_id": "webhook_handler",
  "message_id": "01H...",
  "latency_ms": 3,
  "timestamp": "2024-01-15T10:30:00Z"
}
```

**LLM-Aufruf** -- der Agent laeuft nicht und wurde bei Bedarf ueber den konfigurierten LLM-Anbieter ausgefuehrt (siehe [LLM-Aufruf mit ToolClad-Tools](#llm-aufruf-mit-toolclad-tools) unten). Die Antwort enthaelt den endgueltigen Text und eine Zusammenfassung aller ausgefuehrten Tool-Aufrufe:

```json
{
  "status": "completed",
  "agent_id": "webhook_handler",
  "response": "Scanned target and found 3 open ports …",
  "tool_runs": [
    {
      "tool": "nmap_scan",
      "input": {"target": "example.com"},
      "output_preview": "{\"scan_id\": \"…\", \"ports\": [ … ]}"
    }
  ],
  "model": "claude-sonnet-4-20250514",
  "provider": "Anthropic",
  "latency_ms": 4821,
  "timestamp": "2024-01-15T10:30:00Z"
}
```

## LLM-Aufruf mit ToolClad-Tools

Wenn die Laufzeitumgebung angebunden ist, der geroutete Agent sich aber **nicht im `Running`-Zustand befindet**, faellt der Webhook-Handler auf einen bedarfsgesteuerten LLM-Aufrufpfad zurueck. Dies ist nuetzlich fuer Agenten, die pro Anfrage ausgefuehrt werden, anstatt als dauerhaft laufende Listener.

### Funktionsweise

1. Der Webhook-Handler ruft `scheduler.get_agent_status()` auf, um zu pruefen, ob der Agent aktiv laeuft. Nachrichten an nicht laufende Agenten werden nicht ueber den Kommunikationsbus zugestellt, da `send_message` sie stillschweigend verwerfen wuerde.
2. Wenn der Agent nicht laeuft, erstellt der Handler einen System-Prompt aus allen `.dsl`-Dateien, die im Verzeichnis `agents/` gefunden werden, haengt einen optionalen vom Aufrufer bereitgestellten `system_prompt` an (laengenbegrenzt und protokolliert) und konstruiert eine Benutzernachricht aus dem Anfrage-Payload.
3. ToolClad-Manifeste im Verzeichnis `tools/` werden geladen und dem LLM als Function-Calling-Tools bereitgestellt. Benutzerdefinierte Typen aus `toolclad.toml` werden angewendet.
4. Der Handler fuehrt eine **ORGA**-Schleife (Observe-Reason-Gate-Act) fuer Tool-Aufrufe mit bis zu 15 Iterationen aus:
   - Das LLM schlaegt null oder mehr `tool_use`-Aufrufe vor.
   - Jeder Tool-Aufruf wird von ToolClad validiert und in einem Blocking-Threadpool mit einem **Timeout von 120 Sekunden pro Tool** ausgefuehrt.
   - Doppelte `(tool_name, input)`-Paare innerhalb einer einzelnen Iteration werden dedupliziert, um die redundante Ausfuehrung nicht-idempotenter Tools zu vermeiden.
   - Tool-Ergebnisse werden dem LLM als `tool_result`-Nachrichten zurueckgespielt.
   - Die Schleife endet, wenn das LLM eine endgueltige Textantwort erzeugt oder die Iterationsobergrenze erreicht wird.
5. Die endgueltige Antwort, die Liste der ausgefuehrten Tool-Aufrufe sowie Anbieter-/Modell-Metadaten werden an den Aufrufer zurueckgegeben.

### Anbieter-Auto-Erkennung

Der LLM-Client wird beim Serverstart aus Umgebungsvariablen initialisiert. Der erste Anbieter, dessen API-Schluessel gesetzt ist, gewinnt, in dieser Reihenfolge:

| Env-Variable | Anbieter | Modell-Override | Base-URL-Override |
|---------|----------|----------------|-------------------|
| `OPENROUTER_API_KEY` | OpenRouter | `OPENROUTER_MODEL` (Standard: `anthropic/claude-sonnet-4`) | `OPENROUTER_BASE_URL` |
| `OPENAI_API_KEY` | OpenAI | `CHAT_MODEL` (Standard: `gpt-4o`) | `OPENAI_BASE_URL` |
| `ANTHROPIC_API_KEY` | Anthropic | `ANTHROPIC_MODEL` (Standard: `claude-sonnet-4-20250514`) | `ANTHROPIC_BASE_URL` |

Wenn kein API-Schluessel gesetzt ist, ist der LLM-Aufrufpfad deaktiviert und Anfragen fuer nicht laufende Agenten geben einen Fehler zurueck.

### Eingabefelder

Der Webhook-JSON-Body wird wie folgt interpretiert, wenn der LLM-Pfad verwendet wird:

- `prompt` oder `message` -- wird als Benutzernachricht verwendet. Wenn keines von beiden vorhanden ist, wird das gesamte Payload formatiert ausgegeben und als Aufgabenbeschreibung uebergeben.
- `system_prompt` -- optionaler vom Aufrufer bereitgestellter System-Prompt, der an den aus dem DSL abgeleiteten System-Prompt angehaengt wird. Begrenzt auf 4096 Bytes und protokolliert. Als Prompt-Injection-Flaeche behandeln: Authentifizierung stets erzwingen, wenn dieser Endpunkt nicht vertrauenswuerdigen Aufrufern zugaenglich gemacht wird.

### Normalisiertes Tool-Call-Format

Der LLM-Client normalisiert OpenAI/OpenRouter-Function-Calling in dieselbe Content-Block-Struktur, die von der Anthropic Messages API verwendet wird. Unabhaengig vom Anbieter ist jeder Antwort-Content-Block entweder `{"type": "text", "text": "..."}` oder `{"type": "tool_use", "id": "...", "name": "...", "input": {...}}`, und `stop_reason` ist `"end_turn"` oder `"tool_use"`.

## Integrationsmuster

### Webhook-Endpunkte

Verschiedene Agenten fuer verschiedene Webhook-Quellen konfigurieren:

```rust
let routing_rules = vec![
    AgentRoutingRule {
        condition: RouteMatch::HeaderEquals("X-GitHub-Event".to_string(), "push".to_string()),
        agent: AgentId::from_str("github_push_handler")?,
    },
    AgentRoutingRule {
        condition: RouteMatch::JsonFieldEquals("source".to_string(), "stripe".to_string()),
        agent: AgentId::from_str("payment_processor")?,
    },
];
```

### API-Gateway-Integration

Als Backend-Service hinter einem API-Gateway verwenden:

```rust
let config = HttpInputConfig {
    bind_address: "0.0.0.0".to_string(),
    port: 8081,
    path: "/api/webhook".to_string(),
    cors_origins: vec!["https://example.com".to_string()],
    forward_headers: vec![
        "X-Forwarded-For".to_string(),
        "X-Request-ID".to_string(),
    ],
    ..Default::default()
};
```

### Health-Check-Integration

Das HTTP-Eingabe-Modul bietet keinen dedizierten Health-Endpunkt. Verwenden Sie den Haupt-API-Gesundheitsendpunkt (`/api/v1/health`) fuer die Integration von Load Balancern und Ueberwachungssystemen. Siehe den Abschnitt [Health-Endpunkt](#health-endpunkt) oben fuer Details.

## Fehlerbehandlung

Das HTTP-Eingabe-Modul bietet umfassende Fehlerbehandlung:

- **Authentifizierungsfehler**: Gibt `401 Unauthorized` fuer ungueltige Token zurueck
- **Ratenbegrenzung**: Gibt `429 Too Many Requests` zurueck, wenn Parallelitaetslimits ueberschritten werden
- **Payload-Fehler**: Gibt `400 Bad Request` fuer fehlerhaftes JSON zurueck
- **Agenten-Fehler**: Gibt konfigurierbaren Fehlerstatus mit Fehlerdetails zurueck
- **Server-Fehler**: Gibt `500 Internal Server Error` fuer Laufzeitfehler zurueck

## Ueberwachung und Observability

### Audit-Logging

Wenn `audit_enabled` true ist, protokolliert das Modul strukturierte Informationen ueber alle Anfragen:

```log
INFO HTTP Input: Received request with 5 headers
INFO Agent webhook_handler is running, dispatching via communication bus
INFO Runtime execution dispatched for agent webhook_handler: message_id=… latency=3ms
```

Wenn der LLM-Aufrufpfad verwendet wird, verfolgen zusaetzliche Zeilen die ORGA-Schleife:

```log
INFO Agent webhook_handler is not running, using LLM invocation path
INFO Invoking LLM for agent webhook_handler: provider=Anthropic model=… tools=4 …
INFO ORGA ACT: executing tool 'nmap_scan' (id=…) for agent webhook_handler
INFO Tool 'nmap_scan' executed successfully
INFO ORGA loop iteration 1 for agent webhook_handler: executed 1 tool(s), continuing
INFO LLM invocation completed for agent webhook_handler: latency=4821ms tool_runs=1 response_len=…
```

### Metriken-Integration

Das Modul integriert sich in das Metriken-System der Symbiont-Laufzeitumgebung und bietet:

- Anfragezahl und -rate
- Antwortzeit-Verteilungen
- Fehlerrate nach Typ
- Aktive Verbindungszahlen
- Parallelitaetsauslastung

## Best Practices

1. **Sicherheit**: In Produktionsumgebungen immer Authentifizierung verwenden
2. **Ratenbegrenzung**: Angemessene Parallelitaetslimits basierend auf Ihrer Infrastruktur konfigurieren
3. **Ueberwachung**: Audit-Logging aktivieren und in Ihren Monitoring-Stack integrieren
4. **Fehlerbehandlung**: Angemessene Fehlerantworten fuer Ihren Anwendungsfall konfigurieren
5. **Agenten-Design**: Agenten fuer webhook-spezifische Eingabeformate entwerfen
6. **Ressourcenlimits**: Vernuenftige Body-Groessenlimits setzen, um Ressourcenerschoepfung zu verhindern

## Siehe auch

- [Erste Schritte](getting-started.md)
- [DSL-Leitfaden](dsl-guide.md)
- [API-Referenz](api-reference.md)
- [Reasoning-Schleife (ORGA)](reasoning-loop.md)
- [ToolClad-Tool-Contracts](toolclad.md)
- [Agenten-Laufzeitumgebung-Dokumentation](../crates/runtime/README.md)
