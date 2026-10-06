# Modulo de Entrada HTTP

El modulo de Entrada HTTP proporciona un servidor webhook que permite a sistemas externos invocar agentes Symbiont a traves de peticiones HTTP. Este modulo habilita la integracion con servicios externos, webhooks y APIs exponiendo agentes a traves de endpoints HTTP.

## Descripcion General

En esta rama, cada peticion HTTP de razonamiento se ejecuta de forma independiente
aunque su agente registrado ya este activo. El codigo fuente registrado y el nivel
de seguridad seleccionan un ejecutor de herramientas congelado antes de la
inferencia. Los limites de CPU, memoria y tiempo de ejecucion acotan esa
invocacion. Los workers gobernados tambien comparten la CPU, la memoria y el pool
de workers configurados del supervisor con los lanzamientos del planificador y de
la CLI que usan el mismo directorio de estado privado; consulta
[presupuestos compartidos](/shared-budgets). Las respuestas correctas incluyen
`audit` con `run_id`, `path` y `public_key`. Los fallos del almacenamiento
obligatorio detienen cualquier efecto posterior, y las peticiones descartadas
conservan la propiedad de la limpieza. Consulta
[auditoria de ejecuciones](/run-audit) y la [guia de la rama](/containment-branch-guide).

El modulo de Entrada HTTP consiste en:

- **Servidor HTTP**: Un servidor web basado en Axum que escucha peticiones HTTP entrantes
- **Autenticacion**: Soporte para autenticacion basada en Bearer token y JWT
- **Enrutamiento de Peticiones**: Reglas de enrutamiento flexibles para dirigir peticiones a agentes especificos
- **Control de Respuestas**: Formato de respuesta configurable y codigos de estado
- **Caracteristicas de Seguridad**: Soporte CORS, limites de tamano de peticion y registro de auditoria
- **Gestion de Concurrencia**: Limitacion de tasa de peticiones integrada y control de concurrencia
- **Invocacion LLM con ToolClad**: Cada peticion invoca al agente registrado de forma independiente a traves del proveedor LLM configurado y del bucle gobernado de llamada a herramientas ORGA, incluso cuando hay otra invocacion activa

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

El cargador de claves acepta PEM de Ed25519 o bytes de clave publica en bruto para
la verificacion EdDSA. Los JWT deben tener un `exp` valido y un `sub` no vacio (de
512 bytes como maximo). La validacion de la expiracion admite cinco segundos de
desfase de reloj. Si se proporciona, `iss` debe ser no vacio y de 2048 bytes como
maximo. Renovar un token con el mismo sujeto firmado, el mismo emisor y la misma
clave configurada conserva la identidad de su llamante.

Este verificador de la Entrada HTTP **no** aplica una audiencia ni una lista de
emisores permitidos. La clave configurada es su autoridad de confianza; usa una
clave dedicada a esa autoridad. Un emisor firmado contribuye a la identidad de
reintento, pero no establece una lista de emisores permitidos. La autenticacion
Bearer es obligatoria incluso cuando se configura la verificacion de firma de
webhooks; la firma del webhook es una comprobacion adicional.

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
- **CORS Deshabilitado por Defecto**: `cors_origins` por defecto es una lista vacia, lo que significa que CORS esta deshabilitado; agregue origenes especificos para habilitar el acceso entre origenes. Un literal `"*"` en `cors_origins` es **rechazado al arranque** — el servidor de Entrada HTTP se negara a iniciar con un origen comodin. (Anadido en la auditoria posterior a v1.13.0; consulta `SECURITY_AUDIT.md` M1.)
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

Crear un agente manejador de webhook en [`webhook_handler.symbi`](../agents/webhook_handler.symbi):

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
  -H "Idempotency-Key: 72d6a833-b825-4b22-b50c-206337d77f7c" \
  -d '{
    "type": "security_alert",
    "message": "Suspicious login detected",
    "source": "slack",
    "severity": "high",
    "user": "admin@company.com"
  }'
```

Elige y conserva un UUID nuevo para cada tarea prevista. Cada envio HTTP requiere
exactamente una cabecera `Idempotency-Key`; reintenta con el mismo ID, la misma URI
y la misma carga util JSON. Reutilizar el ID de este ejemplo para un trabajo
distinto se rechazara. Los emisores de webhooks deben conservar un UUID estable por
entrega, o usar un adaptador que asigne su identidad de entrega a un UUID estable
antes del envio. El servidor no infiere la identidad a partir de la salida del
modelo ni genera un sustituto cuando falta la cabecera.

### Estados de reintento

Los IDs ocupan un unico dominio HTTP por proyecto canonico. La reclamacion duradera
vincula al llamante verificado, la URI de la peticion, la entrada JSON y el codigo
fuente y los ajustes de destino en los que se confia. Los IDs de agente registrados
pueden cambiar al reiniciar sin que eso cree una peticion distinta. Un servidor SDK
autonomo debe conservar su `AgentId` configurado entre reinicios. Cambiar el codigo
fuente, el destino, el llamante o la carga util bajo un ID existente rechaza el
trabajo; no se devuelve a otro llamante ningun contenido de la cache ni referencia
de auditoria.

| Estado HTTP | `status` del cuerpo | Significado |
|---|---|---|
| 200 por defecto | `completed` | El resultado original se persistio; `replayed` identifica una respuesta guardada. |
| 422 | `failed` | Se persistio un fallo terminal con la evidencia completa registrada; los reintentos lo devuelven. |
| 409 | `in_progress` | Otro propietario tiene el ID; esta peticion no inicia ningun trabajo. |
| 409 | `unresolved` | La ejecucion original necesita reconciliacion; incluye su referencia de auditoria cuando esta disponible. |
| 409 | `reconciled` | Devuelve una valoracion de operador firmada por separado; el ID original no puede volver a ejecutarse. |
| 409 | `conflict` | El ID esta vinculado a otro llamante o a otra peticion. |
| 400 | `invalid_invocation_id` | Cabecera UUID ausente, repetida o invalida. |
| 503 | `unavailable` | El almacenamiento de invocaciones requerido no pudo autorizar la ejecucion. |

Las respuestas de estado de invocacion incluyen las cabeceras `Idempotency-Key`,
`Idempotency-Replayed` y `Cache-Control: no-store`. El formato de exito configurado
sigue aplicandose a los resultados completados; no puede convertir desenlaces sin
resolver o en conflicto en respuestas correctas. Los origenes CORS configurados
permiten y exponen las cabeceras de invocacion.

Un propietario retenido mantiene la reclamacion durante la preparacion, la
ejecucion, la limpieza y el guardado del resultado. La desconexion del cliente
cancela el trabajo de ese propietario, pero no libera el ID para volver a
ejecutarlo. La perdida del proceso antes de persistir el resultado deja una
reclamacion sin resolver. Los resultados guardados se verifican contra su auditoria
firmada original antes de devolverse; la recuperacion no vuelve a ejecutar el
proveedor ni el ejecutor.

Las credenciales compartidas estaticas representan a un solo llamante. Un llamante
JWT queda vinculado a la clave configurada, al emisor firmado y al sujeto; los
campos de expiracion y renovacion no lo cambian. Rotar las credenciales o el
material de clave hace que un ID existente entre en conflicto en lugar de crear
silenciosamente una segunda tarea. Las reclamaciones abarcan todo el proyecto a
traves de los listeners HTTP, asi que usa UUID aleatorios nuevos y conserva el
almacen junto con su evidencia de auditoria. Consulta
[identidades de invocacion persistentes](/invocation-idempotency) para conocer los
limites del almacenamiento.

### Respuesta Esperada

Cada peticion de razonamiento devuelve su propio resultado, incluso cuando hay otra
invocacion del mismo agente activa. La antigua respuesta de entrega con
`execution_started` / `message_id` ya no se usa en esta ruta. Las respuestas
correctas incluyen la referencia publica de auditoria de la ejecucion,
`invocation_id`, `replayed`, `total_usage` y la instantanea compartida de
`budget`. Valores ilustrativos:

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

`tool_runs` resume las observaciones de herramientas correlacionadas, incluidas las
denegaciones y los fallos de validacion. Su presencia no demuestra que un efecto se
haya ejecutado, y `status: completed` puede acompanar a una respuesta de rechazo por
politica. Usa los argumentos normalizados exactos, la decision y los registros de
efecto del diario protegido para verificarlo. Los fallos de auditoria o de limpieza
obligatorias devuelven un error, aunque ya se hubiera producido un efecto previo. El
valor de `audit.path` es una ruta en el host del runtime, no una URL de descarga.

## Invocacion LLM con Herramientas ToolClad

Cada peticion HTTP de razonamiento inicia una invocacion gobernada independiente,
incluso cuando el agente registrado ya tiene otra invocacion activa.

### Como funciona

1. Con un runtime adjunto, se resuelve el agente desde el registro de confianza. Se
   congelan su codigo fuente, su sandbox y sus ajustes de recursos seleccionados; se
   rechazan los agentes inexistentes, las selecciones ambiguas y los niveles de
   seguridad que no coinciden, antes de la inferencia. Un servidor SDK autonomo usa
   el agente y el ejecutor genericos que tenga configurados de forma explicita.
2. El prompt de sistema se construye unicamente a partir del codigo fuente del
   agente seleccionado. El `system_prompt` opcional proporcionado por el llamante
   sigue teniendo limite de longitud y quedando registrado; no aporta autoridad de
   politica, de principal ni de sandbox. El mensaje de usuario se construye a partir
   de la carga util de la peticion.
3. Se descubren las herramientas ToolClad del proyecto congelado y se abre un diario
   privado firmado obligatorio. El bucle ORGA permite hasta 15 iteraciones. Los
   plazos registrados y los seleccionados por el agente endurecen los limites del
   bucle y de las herramientas; el limite por herramienta es de 120 segundos por
   defecto.
4. Las llamadas propuestas se preparan y normalizan antes de Cedar. Las aprobaciones
   exactas obligatorias, la auditoria requerida y la autorizacion de un solo uso
   preceden a los efectos. Los IDs de llamada duplicados o vacios se rechazan; los
   resultados deben correlacionarse con las llamadas realmente preparadas.
5. Se espera la limpieza del worker y el registro terminal en el diario. Las
   respuestas correctas incluyen la respuesta final, los desenlaces de las
   herramientas, los metadatos de proveedor y modelo, y la referencia `audit`. La
   cancelacion conserva la propiedad de la limpieza; los errores de almacenamiento
   obligatorio o de limpieza no pueden producir en silencio un resultado correcto.

Consulta [llamadas preparadas](/prepared-calls) y
[auditoria de ejecuciones](/run-audit). Los limites de recursos por invocacion no
establecen una admision agregada de peticiones.

### Deteccion automatica de proveedor

El cliente LLM se inicializa a partir de variables de entorno al iniciar el servidor. Gana el primer proveedor cuya clave API este establecida, en este orden:

| Variable de entorno | Proveedor | Sobrescritura de modelo | Sobrescritura de URL base |
|---------|----------|----------------|-------------------|
| `OPENROUTER_API_KEY` | OpenRouter | `OPENROUTER_MODEL` (por defecto: `anthropic/claude-sonnet-4`) | `OPENROUTER_BASE_URL` |
| `OPENAI_API_KEY` | OpenAI | `CHAT_MODEL` (por defecto: `gpt-4o`) | `OPENAI_BASE_URL` |
| `ANTHROPIC_API_KEY` | Anthropic | `ANTHROPIC_MODEL` (por defecto: `claude-sonnet-4-20250514`) | `ANTHROPIC_BASE_URL` |

Sin un proveedor de inferencia configurado, las peticiones de razonamiento devuelven un error. Los endpoints locales configurados por el operador siguen estando soportados.

### Campos de entrada

El cuerpo JSON del webhook se interpreta de la siguiente manera cuando se toma la ruta LLM:

- `prompt` o `message` — se usa como el mensaje de usuario. Si no esta presente ninguno, toda la carga util se imprime de forma legible y se pasa como la descripcion de la tarea.
- `system_prompt` — prompt de sistema opcional proporcionado por el llamante que se agrega al prompt de sistema derivado del DSL. Limitado a 4096 bytes y registrado. Tratelo como una superficie de inyeccion de prompts: siempre aplique autenticacion cuando exponga este endpoint a llamantes no confiables.

### Formato normalizado de llamada a herramientas

El cliente LLM normaliza la llamada a funciones de OpenAI/OpenRouter a la misma forma de bloque de contenido usada por la API de Mensajes de Anthropic. Independientemente del proveedor, cada bloque de contenido de respuesta es `{"type": "text", "text": "..."}` o `{"type": "tool_use", "id": "...", "name": "...", "input": {...}}`, y `stop_reason` es `"end_turn"` o `"tool_use"`.

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
- **Desenlaces de Invocacion**: Devuelve los estados de reintento explicitos descritos arriba; el trabajo sin resolver nunca se reporta como completado.
- **Errores del Servidor**: Los fallos de runtime sin clasificar devuelven un estado configurable con un mensaje publico generico.

## Monitoreo y Observabilidad

### Registro de Auditoria

Cuando `audit_enabled` es true, el modulo registra informacion estructurada sobre todas las peticiones:

```log
INFO HTTP Input: Received request with 5 headers
INFO Agent webhook_handler is running, dispatching via communication bus
INFO Runtime execution dispatched for agent webhook_handler: message_id=… latency=3ms
```

Cuando se usa la ruta de invocacion LLM, lineas adicionales trazan el bucle ORGA:

```log
INFO Agent webhook_handler is not running, using LLM invocation path
INFO Invoking LLM for agent webhook_handler: provider=Anthropic model=… tools=4 …
INFO ORGA ACT: executing tool 'nmap_scan' (id=…) for agent webhook_handler
INFO Tool 'nmap_scan' executed successfully
INFO ORGA loop iteration 1 for agent webhook_handler: executed 1 tool(s), continuing
INFO LLM invocation completed for agent webhook_handler: latency=4821ms tool_runs=1 response_len=…
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
- [Bucle de Razonamiento (ORGA)](reasoning-loop.md)
- [Contratos de Herramientas ToolClad](toolclad.md)
- [Documentacion del Runtime de Agentes](../crates/runtime/README.md)

Una invocacion retenida con una resolucion del operador devuelve HTTP 409 y
`status: "reconciled"`, su referencia de auditoria original y un recibo de
`resolution` firmado por separado. No devuelve un resultado correcto fabricado ni
vuelve a ejecutarse. Consulta la
[reconciliacion por el operador](/invocation-reconciliation).
