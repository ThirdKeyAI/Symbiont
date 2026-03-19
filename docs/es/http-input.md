layout: default
title: Modulo de Entrada HTTP
description: "Modulo de entrada HTTP para integracion de webhook con agentes Symbiont"
nav_exclude: true
---

# Modulo de Entrada HTTP

El modulo de Entrada HTTP proporciona un servidor webhook que permite a sistemas externos invocar agentes Symbiont a traves de peticiones HTTP. Este modulo habilita la integracion con servicios externos, webhooks y APIs exponiendo agentes a traves de endpoints HTTP.

## Descripcion General

El modulo de Entrada HTTP consiste en:

- **Servidor HTTP**: Un servidor web basado en Axum que escucha peticiones HTTP entrantes
- **Autenticacion**: Soporte para autenticacion basada en Bearer token y JWT
- **Enrutamiento de Peticiones**: Reglas de enrutamiento flexibles para dirigir peticiones a agentes especificos
- **Control de Respuestas**: Formato de respuesta configurable y codigos de estado
- **Caracteristicas de Seguridad**: Soporte CORS, limites de tamano de peticion y registro de auditoria
- **Gestion de Concurrencia**: Limitacion de tasa de peticiones integrada y control de concurrencia

El modulo se compila condicionalmente con el flag de caracteristica `http-input` y se integra sin problemas con el runtime de agentes Symbiont.

## Configuracion

El modulo de Entrada HTTP se configura usando la estructura [`HttpInputConfig`](../crates/runtime/src/http_input/config.rs):

### Configuracion Basica

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

### Campos de Configuracion

| Campo | Tipo | Por Defecto | Descripcion |
|-------|------|---------|-------------|
| `bind_address` | `String` | `"127.0.0.1"` | Direccion IP para vincular el servidor HTTP |
| `port` | `u16` | `8081` | Numero de puerto en el que escuchar |
| `path` | `String` | `"/webhook"` | Endpoint de ruta HTTP |
| `agent` | `AgentId` | Nuevo ID | Agente por defecto a invocar para peticiones |
| `auth_header` | `Option<String>` | `None` | Bearer token para autenticacion |
| `jwt_public_key_path` | `Option<String>` | `None` | Ruta al archivo de clave publica JWT |
| `max_body_bytes` | `usize` | `65536` | Tamano maximo del cuerpo de peticion (64 KB) |
| `concurrency` | `usize` | `10` | Maximo numero de peticiones concurrentes |
| `routing_rules` | `Option<Vec<AgentRoutingRule>>` | `None` | Reglas de enrutamiento de peticiones |
| `response_control` | `Option<ResponseControlConfig>` | `None` | Configuracion de formato de respuesta |
| `forward_headers` | `Vec<String>` | `[]` | Cabeceras a reenviar a los agentes |
| `cors_origins` | `Vec<String>` | `[]` | Origenes CORS permitidos (vacio = CORS deshabilitado) |
| `audit_enabled` | `bool` | `true` | Habilitar registro de auditoria de peticiones |

### Reglas de Enrutamiento de Agentes

Enrutar peticiones a diferentes agentes basandose en caracteristicas de la peticion:

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

## Caracteristicas de Seguridad

### Autenticacion

El modulo de Entrada HTTP soporta multiples metodos de autenticacion:

#### Autenticacion con Bearer Token

Configurar un bearer token estatico:

```rust
let config = HttpInputConfig {
    auth_header: Some("Bearer your-secret-token".to_string()),
    ..Default::default()
};
```

#### Integracion con Almacen de Secretos

Usar referencias de secretos para seguridad mejorada:

```rust
let config = HttpInputConfig {
    auth_header: Some("vault://webhook/auth_token".to_string()),
    ..Default::default()
};
```

#### Autenticacion JWT (EdDSA)

Configurar autenticacion basada en JWT con claves publicas Ed25519:

```rust
let config = HttpInputConfig {
    jwt_public_key_path: Some("/path/to/jwt/ed25519-public.pem".to_string()),
    ..Default::default()
};
```

El verificador JWT carga una clave publica Ed25519 del archivo PEM especificado y valida los tokens entrantes `Authorization: Bearer <jwt>`. Solo se acepta el algoritmo **EdDSA** — HS256, RS256 y otros algoritmos son rechazados.

#### Endpoint de Salud

El modulo de Entrada HTTP no expone su propio endpoint `/health`. Las verificaciones de salud estan disponibles a traves de la API HTTP principal en `/api/v1/health` cuando se ejecuta `symbi up`, que inicia el runtime completo incluyendo el servidor de API:

```bash
# Health check via the main API server (default port 8080)
curl http://127.0.0.1:8080/api/v1/health
# => {"status": "ok"}
```

Si necesita sondas de salud para el servidor de Entrada HTTP especificamente, dirija su balanceador de carga al endpoint de salud de la API principal.

### Controles de Seguridad

- **Solo Loopback por Defecto**: `bind_address` por defecto es `127.0.0.1` — el servidor solo acepta conexiones locales a menos que se configure explicitamente de otra manera
- **CORS Deshabilitado por Defecto**: `cors_origins` por defecto es una lista vacia, lo que significa que CORS esta deshabilitado; agregue origenes especificos para habilitar el acceso entre origenes
- **Limites de Tamano de Peticion**: El tamano maximo configurable del cuerpo previene el agotamiento de recursos
- **Limites de Concurrencia**: Semaforo integrado controla el procesamiento de peticiones concurrentes
- **Registro de Auditoria**: Registro estructurado de todas las peticiones entrantes cuando esta habilitado
- **Resolucion de Secretos**: Integracion con Vault y almacenes de secretos basados en archivos

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

### Ejemplo de Definicion de Agente

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

### Ejemplo de Peticion HTTP

Enviar una peticion webhook para activar el agente:

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

## Patrones de Integracion

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

### Integracion con API Gateway

Usar como servicio backend detras de un API gateway:

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

### Integracion de Verificacion de Salud

El modulo de Entrada HTTP no incluye un endpoint de salud dedicado. Use el endpoint de salud de la API principal (`/api/v1/health`) para la integracion con balanceadores de carga y monitoreo. Consulte la seccion [Endpoint de Salud](#endpoint-de-salud) para mas detalles.

## Manejo de Errores

El modulo de Entrada HTTP proporciona manejo de errores integral:

- **Errores de Autenticacion**: Devuelve `401 Unauthorized` para tokens invalidos
- **Limitacion de Tasa**: Devuelve `429 Too Many Requests` cuando se exceden los limites de concurrencia
- **Errores de Carga Util**: Devuelve `400 Bad Request` para JSON mal formado
- **Errores de Agente**: Devuelve estado de error configurable con detalles del error
- **Errores del Servidor**: Devuelve `500 Internal Server Error` para fallos en tiempo de ejecucion

## Monitoreo y Observabilidad

### Registro de Auditoria

Cuando `audit_enabled` es true, el modulo registra informacion estructurada sobre todas las peticiones:

```log
INFO HTTP Input: Received request with 5 headers
INFO Would invoke agent webhook_handler with input data
```

### Integracion de Metricas

El modulo se integra con el sistema de metricas del runtime Symbiont para proporcionar:

- Conteo y tasa de peticiones
- Distribuciones de tiempo de respuesta
- Tasas de error por tipo
- Conteos de conexiones activas
- Utilizacion de concurrencia

## Mejores Practicas

1. **Seguridad**: Siempre usar autenticacion en entornos de produccion
2. **Limitacion de Tasa**: Configurar limites de concurrencia apropiados basados en su infraestructura
3. **Monitoreo**: Habilitar registro de auditoria e integrar con su stack de monitoreo
4. **Manejo de Errores**: Configurar respuestas de error apropiadas para su caso de uso
5. **Diseno de Agentes**: Disenar agentes para manejar formatos de entrada especificos de webhook
6. **Limites de Recursos**: Establecer limites razonables de tamano de cuerpo para prevenir agotamiento de recursos

## Ver Tambien

- [Guia de Inicio](getting-started.md)
- [Guia DSL](dsl-guide.md)
- [Referencia de API](api-reference.md)
- [Documentacion del Runtime de Agentes](../crates/runtime/README.md)
