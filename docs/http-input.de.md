# HTTP-Eingabe-Modul

## 🌐 Andere Sprachen
{: .no_toc}

[English](http-input.md) | [中文简体](http-input.zh-cn.md) | [Español](http-input.es.md) | [Português](http-input.pt.md) | [日本語](http-input.ja.md) | **Deutsch**

---

Das HTTP-Eingabe-Modul stellt einen Webhook-Server bereit, der es externen Systemen ermöglicht, Symbiont-Agenten über HTTP-Anfragen aufzurufen. Dieses Modul ermöglicht die Integration mit externen Diensten, Webhooks und APIs, indem es Agenten über HTTP-Endpunkte verfügbar macht.

## Überblick

Das HTTP-Eingabe-Modul besteht aus:

- **HTTP-Server**: Ein Axum-basierter Webserver, der auf eingehende HTTP-Anfragen lauscht
- **Authentifizierung**: Unterstützung für Bearer-Token- und JWT-basierte Authentifizierung
- **Anfrage-Routing**: Flexible Routing-Regeln zur Weiterleitung von Anfragen an spezifische Agenten
- **Antwort-Kontrolle**: Konfigurierbare Antwortformatierung und Statuscodes
- **Sicherheitsfeatures**: CORS-Unterstützung, Anfragegrößenlimits und Audit-Logging
- **Parallelitätsverwaltung**: Eingebaute Anfrage-Ratenbegrenzung und Parallelitätskontrolle

Das Modul wird bedingt mit dem `http-input` Feature-Flag kompiliert und integriert sich nahtlos in die Symbiont-Agenten-Laufzeitumgebung.

## Konfiguration

Das HTTP-Eingabe-Modul wird mit der [`HttpInputConfig`](../crates/runtime/src/http_input/config.rs) Struktur konfiguriert:

### Grundkonfiguration

```rust
use symbiont_runtime::http_input::HttpInputConfig;
use symbiont_runtime::types::AgentId;

let config = HttpInputConfig {
    bind_address: "0.0.0.0".to_string(),
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
| `bind_address` | `String` | `"0.0.0.0"` | IP-Adresse zum Binden des HTTP-Servers |
| `port` | `u16` | `8081` | Portnummer zum Lauschen |
| `path` | `String` | `"/webhook"` | HTTP-Pfad-Endpunkt |
| `agent` | `AgentId` | Neue ID | Standard-Agent für Anfragen aufzurufen |
| `auth_header` | `Option<String>` | `None` | Bearer-Token für Authentifizierung |
| `jwt_public_key_path` | `Option<String>` | `None` | Pfad zur JWT-Public-Key-Datei |
| `max_body_bytes` | `usize` | `65536` | Maximale Anfrage-Body-Größe (64 KB) |
| `concurrency` | `usize` | `10` | Maximale gleichzeitige Anfragen |
| `routing_rules` | `Option<Vec<AgentRoutingRule>>` | `None` | Anfrage-Routing-Regeln |
| `response_control` | `Option<ResponseControlConfig>` | `None` | Antwortformatierungskonfiguration |
| `forward_headers` | `Vec<String>` | `[]` | Header zur Weiterleitung an Agenten |
| `cors_enabled` | `bool` | `false` | CORS-Unterstützung aktivieren |
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

Das HTTP-Eingabe-Modul unterstützt mehrere Authentifizierungsmethoden:

#### Bearer-Token-Authentifizierung

Statischen Bearer-Token konfigurieren:

```rust
let config = HttpInputConfig {
    auth_header: Some("Bearer your-secret-token".to_string()),
    ..Default::default()
};
```

#### Secret-Store-Integration

Secret-Referenzen für erweiterte Sicherheit verwenden:

```rust
let config = HttpInputConfig {
    auth_header: Some("vault://webhook/auth_token".to_string()),
    ..Default::default()
};
```

#### JWT-Authentifizierung

JWT-basierte Authentifizierung konfigurieren:

```rust
let config = HttpInputConfig {
    jwt_public_key_path: Some("/path/to/jwt/public.key".to_string()),
    ..Default::default()
};
```

### Sicherheitskontrollen

- **Anfragegrößenlimits**: Konfigurierbare maximale Body-Größe verhindert Ressourcenerschöpfung
- **Parallelitätslimits**: Eingebauter Semaphor kontrolliert gleichzeitige Anfragebearbeitung
- **CORS-Unterstützung**: Optionale CORS-Header für browserbasierte Anwendungen
- **Audit-Logging**: Strukturiertes Logging aller eingehenden Anfragen bei Aktivierung
- **Secret-Auflösung**: Integration mit Vault und dateibasierten Secret-Stores

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
    cors_enabled: true,
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

Webhook-Anfrage senden, um den Agenten auszulösen:

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

Der Server gibt eine JSON-Antwort mit der Ausgabe des Agenten zurück:

```json
{
  "status": "invoked",
  "agent_id": "webhook_handler",
  "timestamp": "2024-01-15T10:30:00Z"
}
```

## Integrationsmuster

### Webhook-Endpunkte

Verschiedene Agenten für verschiedene Webhook-Quellen konfigurieren:

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
    cors_enabled: true,
    forward_headers: vec![
        "X-Forwarded-For".to_string(),
        "X-Request-ID".to_string(),
    ],
    ..Default::default()
};
```

### Health-Check-Endpunkt

Der Server stellt automatisch Health-Check-Funktionen für Load Balancer und Überwachungssysteme bereit.

## Fehlerbehandlung

Das HTTP-Eingabe-Modul bietet umfassende Fehlerbehandlung:

- **Authentifizierungsfehler**: Gibt `401 Unauthorized` für ungültige Token zurück
- **Ratenbegrenzung**: Gibt `429 Too Many Requests` zurück, wenn Parallelitätslimits überschritten werden
- **Payload-Fehler**: Gibt `400 Bad Request` für fehlerhaftes JSON zurück
- **Agenten-Fehler**: Gibt konfigurierbaren Fehlerstatus mit Fehlerdetails zurück
- **Server-Fehler**: Gibt `500 Internal Server Error` für Laufzeitfehler zurück

## Überwachung und Observability

### Audit-Logging

Wenn `audit_enabled` true ist, protokolliert das Modul strukturierte Informationen über alle Anfragen:

```log
INFO HTTP Input: Received request with 5 headers
INFO Would invoke agent webhook_handler with input data
```

### Metriken-Integration

Das Modul integriert sich in das Metriken-System der Symbiont-Laufzeitumgebung und bietet:

- Anfragezahl und -rate
- Antwortzeit-Verteilungen
- Fehlerrate nach Typ
- Aktive Verbindungszahlen
- Parallelitätsauslastung

## Best Practices

1. **Sicherheit**: In Produktionsumgebungen immer Authentifizierung verwenden
2. **Ratenbegrenzung**: Angemessene Parallelitätslimits basierend auf Ihrer Infrastruktur konfigurieren
3. **Überwachung**: Audit-Logging aktivieren und in Ihren Monitoring-Stack integrieren
4. **Fehlerbehandlung**: Angemessene Fehlerantworten für Ihren Anwendungsfall konfigurieren
5. **Agenten-Design**: Agenten für webhook-spezifische Eingabeformate entwerfen
6. **Ressourcenlimits**: Vernünftige Body-Größenlimits setzen, um Ressourcenerschöpfung zu verhindern

## Siehe auch

- [Erste Schritte](getting-started.de.md)
- [DSL-Leitfaden](dsl-guide.de.md)
- [API-Referenz](api-reference.de.md)
- [Agenten-Laufzeitumgebung-Dokumentation](../crates/runtime/README.md)