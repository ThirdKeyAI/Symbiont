# HTTP-Eingabe-Modul

Das HTTP-Eingabe-Modul stellt einen Webhook-Server bereit, der es externen Systemen ermoeglicht, Symbiont-Agenten ueber HTTP-Anfragen aufzurufen. Dieses Modul ermoeglicht die Integration mit externen Diensten, Webhooks und APIs, indem es Agenten ueber HTTP-Endpunkte verfuegbar macht.

## Ueberblick

In diesem Branch wird jede HTTP-Reasoning-Anfrage unabhaengig ausgefuehrt, auch
wenn ihr registrierter Agent bereits aktiv ist. Registrierte Quelle und
Sicherheitsstufe waehlen vor der Inferenz einen eingefrorenen Tool-Executor aus.
CPU-, Speicher- und Ausfuehrungszeitlimits begrenzen diesen Aufruf. Kontrollierte
Worker teilen sich zudem den vom Supervisor konfigurierten CPU-, Speicher- und
Worker-Pool mit Scheduler- und CLI-Starts, die dasselbe private
Zustandsverzeichnis verwenden; siehe [geteilte Budgets](/shared-budgets).
Erfolgreiche Antworten enthalten `audit` mit `run_id`, `path` und `public_key`.
Fehler beim erforderlichen Speichern stoppen weitere Effekte, und verworfene
Anfragen behalten die Verantwortung fuer die Bereinigung.
Siehe [Lauf-Audit](/run-audit) und den [Branch-Leitfaden](/containment-branch-guide).

Das HTTP-Eingabe-Modul besteht aus:

- **HTTP-Server**: Ein Axum-basierter Webserver, der auf eingehende HTTP-Anfragen lauscht
- **Authentifizierung**: Unterstuetzung fuer Bearer-Token- und JWT-basierte Authentifizierung
- **Anfrage-Routing**: Flexible Routing-Regeln zur Weiterleitung von Anfragen an spezifische Agenten
- **Antwort-Kontrolle**: Konfigurierbare Antwortformatierung und Statuscodes
- **Sicherheitsfeatures**: CORS-Unterstuetzung, Anfragengroessenlimits und Audit-Logging
- **Parallelitaetsverwaltung**: Eingebaute Anfrage-Ratenbegrenzung und Parallelitaetskontrolle
- **LLM-Aufruf mit ToolClad**: Jede Anfrage ruft den registrierten Agenten unabhaengig ueber den konfigurierten LLM-Anbieter und die kontrollierte ORGA-Tool-Calling-Schleife auf, auch wenn ein anderer Aufruf aktiv ist

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

Der Key-Loader akzeptiert fuer die EdDSA-Verifizierung Ed25519 im PEM-Format oder
rohe Public-Key-Bytes. JWTs muessen ein gueltiges `exp` und ein nicht leeres `sub`
(hoechstens 512 Bytes) besitzen. Die Ablaufpruefung erlaubt fuenf Sekunden
Zeitabweichung. Falls angegeben, muss `iss` nicht leer und hoechstens 2.048 Bytes
gross sein. Die Erneuerung eines Tokens mit demselben signierten Subject, demselben
Issuer und demselben konfigurierten Key erhaelt seine Aufrufer-Identitaet.

Dieser HTTP-Eingabe-Verifizierer erzwingt **keine** Audience- oder
Issuer-Allowlist. Der konfigurierte Key ist seine Vertrauensinstanz; verwenden Sie
einen Key, der dieser Instanz vorbehalten ist. Ein signierter Issuer traegt zur
Wiederholungs-Identitaet bei, begruendet aber keine Issuer-Allowlist.
Bearer-Authentifizierung ist auch dann erforderlich, wenn die Verifizierung der
Webhook-Signatur konfiguriert ist; die Webhook-Signatur ist eine zusaetzliche
Pruefung.

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
- **CORS standardmaessig deaktiviert**: `cors_origins` ist standardmaessig eine leere Liste, was bedeutet, dass CORS deaktiviert ist; fuegen Sie spezifische Urspruenge hinzu, um Cross-Origin-Zugriff zu ermoeglichen. Ein literales `"*"` in `cors_origins` wird **beim Start abgelehnt** -- der HTTP-Eingabe-Server verweigert den Start mit einem Wildcard-Origin. (Im Audit nach v1.13.0 hinzugefuegt; siehe `SECURITY_AUDIT.md` M1.)
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

Webhook-Handler-Agent in [`webhook_handler.symbi`](../agents/webhook_handler.symbi) erstellen:

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
  -H "Idempotency-Key: 72d6a833-b825-4b22-b50c-206337d77f7c" \
  -d '{
    "type": "security_alert",
    "message": "Suspicious login detected",
    "source": "slack",
    "severity": "high",
    "user": "admin@company.com"
  }'
```

Waehlen Sie fuer jede beabsichtigte Aufgabe eine neue UUID und behalten Sie diese
bei. Jede HTTP-Einreichung erfordert genau einen `Idempotency-Key`-Header;
wiederholen Sie mit derselben ID, derselben URI und demselben JSON-Payload. Die
Wiederverwendung dieser Beispiel-ID fuer andere Arbeit wird abgelehnt.
Webhook-Absender muessen eine stabile UUID pro Zustellung vorhalten oder einen
Adapter verwenden, der ihre Zustellungsidentitaet vor der Einreichung auf eine
stabile UUID abbildet. Der Server leitet die Identitaet nicht aus der
Modellausgabe ab und erzeugt keinen Ersatz, wenn der Header fehlt.

### Wiederholungszustaende

IDs belegen eine HTTP-Domaene pro kanonischem Projekt. Der dauerhafte Anspruch
bindet den verifizierten Aufrufer, die Anfrage-URI, die JSON-Eingabe sowie die
vertrauenswuerdige Zielquelle und deren Einstellungen. Registrierte Agenten-IDs
koennen sich beim Neustart aendern, ohne dass daraus eine andere Anfrage wird. Ein
eigenstaendiger SDK-Server muss seine konfigurierte `AgentId` ueber Neustarts
hinweg beibehalten. Das Aendern von Quelle, Ziel, Aufrufer oder Payload unter
einer bestehenden ID verweigert die Arbeit; es werden weder Cache-Inhalte noch
eine Audit-Referenz an einen anderen Aufrufer zurueckgegeben.

| HTTP-Status | Body-`status` | Bedeutung |
|---|---|---|
| 200 standardmaessig | `completed` | Das urspruengliche Ergebnis wurde gespeichert; `replayed` kennzeichnet eine gespeicherte Antwort. |
| 422 | `failed` | Ein endgueltiger Fehlschlag mit vollstaendigen nachverfolgten Nachweisen wurde gespeichert; Wiederholungen geben ihn zurueck. |
| 409 | `in_progress` | Ein anderer Eigentuemer haelt die ID; diese Anfrage startet keine Arbeit. |
| 409 | `unresolved` | Der urspruengliche Lauf benoetigt einen Abgleich; enthaelt seine Audit-Referenz, sofern verfuegbar. |
| 409 | `reconciled` | Gibt eine separate signierte Betreiber-Bewertung zurueck; die urspruengliche ID kann nicht erneut ausgefuehrt werden. |
| 409 | `conflict` | Die ID ist an einen anderen Aufrufer oder eine andere Anfrage gebunden. |
| 400 | `invalid_invocation_id` | Fehlender, mehrfach vorhandener oder ungueltiger UUID-Header. |
| 503 | `unavailable` | Der erforderliche Aufruf-Speicher konnte die Ausfuehrung nicht autorisieren. |

Antworten zum Aufrufzustand enthalten die Header `Idempotency-Key`,
`Idempotency-Replayed` und `Cache-Control: no-store`. Die konfigurierte
Erfolgsformatierung gilt weiterhin fuer abgeschlossene Ergebnisse; sie kann
ungeklaerte oder widerspruechliche Ausgaenge nicht in erfolgreiche Antworten
verwandeln. Konfigurierte CORS-Urspruenge erlauben die Aufruf-Header und geben sie
frei.

Ein beibehaltener Eigentuemer haelt den Anspruch ueber Einrichtung, Ausfuehrung,
Bereinigung und Speichern des Ergebnisses hinweg. Eine Client-Trennung bricht die
Arbeit dieses Eigentuemers ab, gibt die ID aber nicht zur erneuten Ausfuehrung
frei. Ein Prozessverlust vor dem Speichern des Ergebnisses hinterlaesst einen
ungeklaerten Anspruch. Gespeicherte Ergebnisse werden vor der Rueckgabe gegen ihr
urspruengliches signiertes Audit geprueft; der Abruf fuehrt weder den Anbieter
noch den Executor erneut aus.

Statische geteilte Zugangsdaten repraesentieren einen Aufrufer. Ein JWT-Aufrufer
bindet den konfigurierten Key, den signierten Issuer und das Subject;
Ablauf- und Erneuerungsfelder aendern ihn nicht. Das Rotieren von Zugangsdaten
oder Schluesselmaterial fuehrt dazu, dass eine bestehende ID in Konflikt geraet,
statt stillschweigend eine zweite Aufgabe zu erzeugen. Ansprueche gelten
projektweit ueber HTTP-Listener hinweg; verwenden Sie daher neue zufaellige UUIDs
und bewahren Sie den Speicher mitsamt seinen Audit-Nachweisen auf. Siehe
[persistente Aufruf-Identitaeten](/invocation-idempotency) fuer die
Speichergrenzen.

### Erwartete Antwort

Jede Reasoning-Anfrage gibt ihr eigenes Ergebnis zurueck, auch wenn ein anderer
Aufruf desselben Agenten aktiv ist. Die alte
`execution_started`/`message_id`-Uebergabeantwort wird auf dieser Route nicht mehr
verwendet. Erfolgreiche Antworten enthalten die oeffentliche Audit-Referenz des
Laufs, `invocation_id`, `replayed`, `total_usage` sowie den geteilten
`budget`-Schnappschuss. Beispielhafte Werte:

```json
{
  "status": "completed",
  "agent_id": "11111111-1111-4111-8111-111111111111",
  "response": "Task complete.",
  "tool_runs": [],
  "termination_reason": "Completed",
  "iterations": 1,
  "audit": {
    "run_id": "22222222-2222-4222-8222-222222222222",
    "path": "/srv/control/.symbiont/governed/11111111-1111-4111-8111-111111111111.22222222-2222-4222-8222-222222222222.jsonl",
    "public_key": "<hex-encoded-public-key>"
  },
  "model": "<configured-model>",
  "provider": "<configured-provider>",
  "latency_ms": 4821,
  "timestamp": "2024-01-15T10:30:00Z"
}
```

`tool_runs` fasst korrelierte Tool-Beobachtungen zusammen, einschliesslich
Ablehnungen und Validierungsfehlern. Sein Vorhandensein belegt nicht, dass ein
Effekt ausgefuehrt wurde, und `status: completed` kann eine
Richtlinien-Ablehnungsantwort begleiten. Verwenden Sie zur Verifizierung die
exakten normalisierten Argumente, Entscheidungen und Effekt-Eintraege des
geschuetzten Journals. Fehler beim erforderlichen Audit oder bei der Bereinigung
geben einen Fehler zurueck, selbst wenn zuvor bereits ein Effekt eingetreten ist.
Der `audit.path` ist ein Pfad auf dem Laufzeit-Host, keine Download-URL.

## LLM-Aufruf mit ToolClad-Tools

Jede HTTP-Reasoning-Anfrage startet einen unabhaengigen kontrollierten Aufruf,
auch wenn der registrierte Agent bereits einen anderen aktiven Aufruf hat.

### Funktionsweise

1. Bei angebundener Laufzeitumgebung wird der Agent aus der vertrauenswuerdigen
   Registry aufgeloest. Seine ausgewaehlte Quelle, Sandbox und
   Ressourceneinstellungen werden eingefroren; fehlende Agenten, mehrdeutige
   Auswahlen und nicht uebereinstimmende Sicherheitsstufen werden vor der Inferenz
   abgelehnt. Ein eigenstaendiger SDK-Server verwendet seinen explizit
   konfigurierten generischen Agenten/Executor.
2. Der System-Prompt wird ausschliesslich aus der ausgewaehlten Agentenquelle
   erstellt. Ein optionaler vom Aufrufer bereitgestellter `system_prompt` bleibt
   laengenbegrenzt und protokolliert; er verleiht keine Richtlinien-, Prinzipal-
   oder Sandbox-Autoritaet. Die Benutzernachricht wird aus dem Anfrage-Payload
   erstellt.
3. ToolClad-Tools werden im eingefrorenen Projekt ermittelt und ein
   erforderliches privates signiertes Journal wird geoeffnet. Die ORGA-Schleife
   erlaubt bis zu 15 Iterationen. Registrierte und vom Agenten gewaehlte Fristen
   verschaerfen die Schleifen- und Tool-Limits; das Standardlimit pro Tool betraegt
   120 Sekunden.
4. Vorgeschlagene Aufrufe werden vor Cedar vorbereitet und normalisiert.
   Verpflichtende exakte Genehmigungen, erforderliches Audit und
   Einmal-Autorisierung gehen den Effekten voraus. Doppelte oder leere Aufruf-IDs
   werden abgelehnt; Ergebnisse muessen mit den tatsaechlich vorbereiteten Aufrufen
   korrelieren.
5. Es wird auf die Worker-Bereinigung und das abschliessende Journaling gewartet.
   Erfolgreiche Antworten enthalten die endgueltige Antwort, die Tool-Ergebnisse,
   Anbieter-/Modell-Metadaten und die `audit`-Referenz. Ein Abbruch behaelt die
   Verantwortung fuer die Bereinigung; Fehler beim erforderlichen Speichern oder
   bei der Bereinigung koennen nicht stillschweigend ein erfolgreiches Ergebnis
   erzeugen.

Siehe [vorbereitete Aufrufe](/prepared-calls) und [Lauf-Audit](/run-audit).
Ressourcenlimits pro Aufruf begruenden keine aggregierte Anfragezulassung.

### Anbieter-Auto-Erkennung

Der LLM-Client wird beim Serverstart aus Umgebungsvariablen initialisiert. Der erste Anbieter, dessen API-Schluessel gesetzt ist, gewinnt, in dieser Reihenfolge:

| Env-Variable | Anbieter | Modell-Override | Base-URL-Override |
|---------|----------|----------------|-------------------|
| `OPENROUTER_API_KEY` | OpenRouter | `OPENROUTER_MODEL` (Standard: `anthropic/claude-sonnet-4`) | `OPENROUTER_BASE_URL` |
| `OPENAI_API_KEY` | OpenAI | `CHAT_MODEL` (Standard: `gpt-4o`) | `OPENAI_BASE_URL` |
| `ANTHROPIC_API_KEY` | Anthropic | `ANTHROPIC_MODEL` (Standard: `claude-sonnet-4-20250514`) | `ANTHROPIC_BASE_URL` |

Ohne konfigurierten Inferenz-Anbieter geben Reasoning-Anfragen einen Fehler
zurueck. Vom Betreiber konfigurierte lokale Endpunkte werden weiterhin
unterstuetzt.

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
- **Aufruf-Ausgaenge**: Gibt die oben genannten expliziten Wiederholungszustaende zurueck; ungeklaerte Arbeit wird niemals als abgeschlossen gemeldet.
- **Server-Fehler**: Nicht klassifizierte Laufzeitfehler geben einen konfigurierbaren Status mit einer generischen oeffentlichen Meldung zurueck.

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

Ein beibehaltener Aufruf mit einer Betreiber-Aufloesung gibt HTTP 409 und
`status: "reconciled"`, seine urspruengliche Audit-Referenz sowie eine separate
signierte `resolution`-Quittung zurueck. Er gibt kein erfundenes erfolgreiches
Ergebnis zurueck und wird nicht erneut ausgefuehrt. Siehe
[Betreiber-Abgleich](/invocation-reconciliation).
