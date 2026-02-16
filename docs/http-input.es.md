---
layout: default
title: Módulo de Entrada HTTP
description: "Módulo de entrada HTTP para integración de webhook con agentes Symbiont"
nav_exclude: true
---

# Módulo de Entrada HTTP

## 🌐 Otros idiomas
{: .no_toc}

[English](http-input.md) | [中文简体](http-input.zh-cn.md) | **Español** | [Português](http-input.pt.md) | [日本語](http-input.ja.md) | [Deutsch](http-input.de.md)

---

El módulo de Entrada HTTP proporciona un servidor webhook que permite a sistemas externos invocar agentes Symbiont a través de peticiones HTTP. Este módulo habilita la integración con servicios externos, webhooks y APIs exponiendo agentes a través de endpoints HTTP.

## Descripción General

El módulo de Entrada HTTP consiste en:

- **Servidor HTTP**: Un servidor web basado en Axum que escucha peticiones HTTP entrantes
- **Autenticación**: Soporte para autenticación basada en Bearer token y JWT
- **Enrutamiento de Peticiones**: Reglas de enrutamiento flexibles para dirigir peticiones a agentes específicos
- **Control de Respuestas**: Formato de respuesta configurable y códigos de estado
- **Características de Seguridad**: Soporte CORS, límites de tamaño de petición y registro de auditoría
- **Gestión de Concurrencia**: Limitación de tasa de peticiones integrada y control de concurrencia

El módulo se compila condicionalmente con el flag de característica `http-input` y se integra sin problemas con el runtime de agentes Symbiont.

## Configuración

El módulo de Entrada HTTP se configura usando la estructura [`HttpInputConfig`](../crates/runtime/src/http_input/config.rs):

### Configuración Básica

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

### Campos de Configuración

| Campo | Tipo | Por Defecto | Descripción |
|-------|------|---------|-------------|
| `bind_address` | `String` | `"127.0.0.1"` | Dirección IP para vincular el servidor HTTP |
| `port` | `u16` | `8081` | Número de puerto en el que escuchar |
| `path` | `String` | `"/webhook"` | Endpoint de ruta HTTP |
| `agent` | `AgentId` | Nuevo ID | Agente por defecto a invocar para peticiones |
| `auth_header` | `Option<String>` | `None` | Bearer token para autenticación |
| `jwt_public_key_path` | `Option<String>` | `None` | Ruta al archivo de clave pública JWT |
| `max_body_bytes` | `usize` | `65536` | Tamaño máximo del cuerpo de petición (64 KB) |
| `concurrency` | `usize` | `10` | Máximo número de peticiones concurrentes |
| `routing_rules` | `Option<Vec<AgentRoutingRule>>` | `None` | Reglas de enrutamiento de peticiones |
| `response_control` | `Option<ResponseControlConfig>` | `None` | Configuración de formato de respuesta |
| `forward_headers` | `Vec<String>` | `[]` | Cabeceras a reenviar a los agentes |
| `cors_origins` | `Vec<String>` | `[]` | Orígenes CORS permitidos (vacío = CORS deshabilitado) |
| `audit_enabled` | `bool` | `true` | Habilitar registro de auditoría de peticiones |

### Reglas de Enrutamiento de Agentes

Enrutar peticiones a diferentes agentes basándose en características de la petición:

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

### Control de Respuestas

Personalizar respuestas HTTP con [`ResponseControlConfig`](../crates/runtime/src/http_input/config.rs):

```rust
use symbiont_runtime::http_input::ResponseControlConfig;

let response_control = ResponseControlConfig {
    default_status: 200,
    agent_output_to_json: true,
    error_status: 500,
    echo_input_on_error: false,
};
```

## Características de Seguridad

### Autenticación

El módulo de Entrada HTTP soporta múltiples métodos de autenticación:

#### Autenticación con Bearer Token

Configurar un bearer token estático:

```rust
let config = HttpInputConfig {
    auth_header: Some("Bearer your-secret-token".to_string()),
    ..Default::default()
};
```

#### Integración con Almacén de Secretos

Usar referencias de secretos para seguridad mejorada:

```rust
let config = HttpInputConfig {
    auth_header: Some("vault://webhook/auth_token".to_string()),
    ..Default::default()
};
```

#### Autenticación JWT

Configurar autenticación basada en JWT:

```rust
let config = HttpInputConfig {
    jwt_public_key_path: Some("/path/to/jwt/public.key".to_string()),
    ..Default::default()
};
```

### Controles de Seguridad

- **Límites de Tamaño de Petición**: El tamaño máximo configurable del cuerpo previene el agotamiento de recursos
- **Límites de Concurrencia**: Semáforo integrado controla el procesamiento de peticiones concurrentes
- **Soporte CORS**: Cabeceras CORS opcionales para aplicaciones basadas en navegador
- **Registro de Auditoría**: Registro estructurado de todas las peticiones entrantes cuando está habilitado
- **Resolución de Secretos**: Integración con Vault y almacenes de secretos basados en archivos

## Ejemplo de Uso

### Iniciar el Servidor de Entrada HTTP

```rust
use symbiont_runtime::http_input::{HttpInputConfig, start_http_input};
use symbiont_runtime::secrets::SecretsConfig;
use std::sync::Arc;

// Configurar el servidor de entrada HTTP
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

// Opcional: Configurar secretos
let secrets_config = SecretsConfig::default();

// Iniciar el servidor
start_http_input(config, Some(runtime), Some(secrets_config)).await?;
```

### Ejemplo de Definición de Agente

Crear un agente manejador de webhook en [`webhook_handler.dsl`](../agents/webhook_handler.dsl):

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

### Ejemplo de Petición HTTP

Enviar una petición webhook para activar el agente:

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

### Respuesta Esperada

El servidor devuelve una respuesta JSON con la salida del agente:

```json
{
  "status": "execution_started",
  "agent_id": "webhook_handler",
  "timestamp": "2024-01-15T10:30:00Z"
}
```

## Patrones de Integración

### Endpoints de Webhook

Configurar diferentes agentes para diferentes fuentes de webhook:

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

### Integración con API Gateway

Usar como servicio backend detrás de un API gateway:

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

### Endpoint de Verificación de Salud

El servidor proporciona automáticamente capacidades de verificación de salud para balanceadores de carga y sistemas de monitoreo.

## Manejo de Errores

El módulo de Entrada HTTP proporciona manejo de errores integral:

- **Errores de Autenticación**: Devuelve `401 Unauthorized` para tokens inválidos
- **Limitación de Tasa**: Devuelve `429 Too Many Requests` cuando se exceden los límites de concurrencia
- **Errores de Carga Útil**: Devuelve `400 Bad Request` para JSON mal formado
- **Errores de Agente**: Devuelve estado de error configurable con detalles del error
- **Errores del Servidor**: Devuelve `500 Internal Server Error` para fallos en tiempo de ejecución

## Monitoreo y Observabilidad

### Registro de Auditoría

Cuando `audit_enabled` es true, el módulo registra información estructurada sobre todas las peticiones:

```log
INFO HTTP Input: Received request with 5 headers
INFO Would invoke agent webhook_handler with input data
```

### Integración de Métricas

El módulo se integra con el sistema de métricas del runtime Symbiont para proporcionar:

- Conteo y tasa de peticiones
- Distribuciones de tiempo de respuesta
- Tasas de error por tipo
- Conteos de conexiones activas
- Utilización de concurrencia

## Mejores Prácticas

1. **Seguridad**: Siempre usar autenticación en entornos de producción
2. **Limitación de Tasa**: Configurar límites de concurrencia apropiados basados en su infraestructura
3. **Monitoreo**: Habilitar registro de auditoría e integrar con su stack de monitoreo
4. **Manejo de Errores**: Configurar respuestas de error apropiadas para su caso de uso
5. **Diseño de Agentes**: Diseñar agentes para manejar formatos de entrada específicos de webhook
6. **Límites de Recursos**: Establecer límites razonables de tamaño de cuerpo para prevenir agotamiento de recursos

## Ver También

- [Guía de Inicio](getting-started.es.md)
- [Guía DSL](dsl-guide.es.md)
- [Referencia de API](api-reference.es.md)
- [Documentación del Runtime de Agentes](../crates/runtime/README.md)