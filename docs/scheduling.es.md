---
layout: default
title: Guía de Programación
description: "Programación de tareas basada en cron de nivel de producción para agentes de IA de Symbiont"
nav_exclude: true
---

# Guía de Programación

## 🌐 Otros idiomas
{: .no_toc}

[English](scheduling.md) | [中文简体](scheduling.zh-cn.md) | **Español** | [Português](scheduling.pt.md) | [日本語](scheduling.ja.md) | [Deutsch](scheduling.de.md)

---

## Descripción General

El sistema de programación de Symbiont proporciona ejecución de tareas basada en cron de nivel de producción para agentes de IA. El sistema soporta:

- **Programaciones cron**: Sintaxis cron tradicional para tareas recurrentes
- **Trabajos de una sola ejecución**: Ejecutar una vez en un momento específico
- **Patrón de latido (heartbeat)**: Ciclos continuos de evaluación-acción-pausa para agentes de monitoreo
- **Aislamiento de sesión**: Contextos de agente efímeros, compartidos o completamente aislados
- **Enrutamiento de entrega**: Múltiples canales de salida (Stdout, LogFile, Webhook, Slack, Email, Custom)
- **Aplicación de políticas**: Verificaciones de seguridad y cumplimiento antes de la ejecución
- **Robustez para producción**: Jitter, límites de concurrencia, colas de mensajes no entregados y verificación de AgentPin

## Arquitectura

El sistema de programación está construido sobre tres componentes principales:

```
┌─────────────────────┐
│   CronScheduler     │  Bucle de ticks en segundo plano (intervalos de 1 segundo)
│   (Tick Loop)       │  Selección de trabajos y orquestación de ejecución
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│   SqliteJobStore    │  Almacenamiento persistente de trabajos
│   (Job Storage)     │  Soporte de transacciones, gestión de estado
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│DefaultAgentScheduler│  Entorno de ejecución de agentes
│ (Execution Engine)  │  Gestión del ciclo de vida de AgentContext
└─────────────────────┘
```

### CronScheduler

El `CronScheduler` es el punto de entrada principal. Gestiona:

- Bucle de ticks en segundo plano ejecutándose a intervalos de 1 segundo
- Selección de trabajos basada en el siguiente tiempo de ejecución
- Control de concurrencia e inyección de jitter
- Recolección de métricas y monitoreo de salud
- Apagado gracioso con seguimiento de trabajos en curso

### SqliteJobStore

El `SqliteJobStore` proporciona persistencia durable de trabajos con:

- Transacciones ACID para actualizaciones de estado de trabajos
- Seguimiento del ciclo de vida de trabajos (Active, Paused, Completed, Failed, DeadLetter)
- Historial de ejecuciones con pista de auditoría
- Capacidades de consulta para filtrar por estado, ID de agente, etc.

### DefaultAgentScheduler

El `DefaultAgentScheduler` ejecuta los agentes programados:

- Crea instancias de `AgentContext` aisladas o compartidas
- Gestiona el ciclo de vida de la sesión (crear, ejecutar, destruir)
- Enruta la entrega a los canales configurados
- Aplica puertas de política antes de la ejecución

## Sintaxis DSL

### Estructura del Bloque de Programación

Los bloques de programación se definen en archivos DSL de Symbiont:

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

### Sintaxis Cron

Sintaxis cron estándar con cinco campos:

```
┌───────────── minuto (0-59)
│ ┌─────────── hora (0-23)
│ │ ┌───────── día del mes (1-31)
│ │ │ ┌─────── mes (1-12)
│ │ │ │ ┌───── día de la semana (0-6, Domingo = 0)
│ │ │ │ │
* * * * *
```

**Ejemplos:**

```symbiont
# Todos los días a las 9 AM
cron: "0 9 * * *"

# Cada lunes a las 6 PM
cron: "0 18 * * 1"

# Cada 15 minutos
cron: "*/15 * * * *"

# Primer día de cada mes a medianoche
cron: "0 0 1 * *"
```

### Trabajos de Una Sola Ejecución (Sintaxis At)

Para trabajos que se ejecutan una sola vez en un momento específico:

```symbiont
schedule {
  name: "deployment-check"
  agent: "health-checker"
  at: "2026-02-15T14:30:00Z"  # Marca de tiempo ISO 8601

  delivery: ["webhook"]
  webhook_url: "https://ops.example.com/hooks/deployment"
}
```

### Patrón de Latido (Heartbeat)

Para agentes de monitoreo continuo que evalúan → actúan → duermen:

```symbiont
schedule {
  name: "system-monitor"
  agent: "heartbeat-agent"
  cron: "*/5 * * * *"  # Despertar cada 5 minutos

  heartbeat: {
    enabled: true
    context_mode: "ephemeral_with_summary"
    max_iterations: 100  # Límite de seguridad
  }
}
```

El agente de latido sigue este ciclo:

1. **Evaluación**: Evaluar el estado del sistema (por ejemplo, verificar métricas, registros)
2. **Acción**: Tomar acción correctiva si es necesario (por ejemplo, reiniciar servicio, alertar a operaciones)
3. **Pausa**: Esperar hasta el siguiente tick programado

## Comandos CLI

El comando `symbi cron` proporciona gestión completa del ciclo de vida:

### Listar Trabajos

```bash
# Listar todos los trabajos
symbi cron list

# Filtrar por estado
symbi cron list --status active
symbi cron list --status paused

# Filtrar por agente
symbi cron list --agent "reporter-agent"

# Salida en JSON
symbi cron list --format json
```

### Agregar Trabajo

```bash
# Desde archivo DSL
symbi cron add --file agent.symbi --schedule "daily-report"

# Definición en línea (JSON)
symbi cron add --json '{
  "name": "quick-task",
  "agent_id": "agent-123",
  "cron_expr": "0 * * * *"
}'
```

### Eliminar Trabajo

```bash
# Por ID de trabajo
symbi cron remove <job-id>

# Por nombre
symbi cron remove --name "daily-report"

# Eliminación forzada (omitir confirmación)
symbi cron remove <job-id> --force
```

### Pausar/Reanudar

```bash
# Pausar trabajo (detiene la programación, preserva el estado)
symbi cron pause <job-id>

# Reanudar trabajo pausado
symbi cron resume <job-id>
```

### Estado

```bash
# Detalles del trabajo con próximo tiempo de ejecución
symbi cron status <job-id>

# Incluir los últimos 10 registros de ejecución
symbi cron status <job-id> --history 10

# Modo observación (actualización automática cada 5s)
symbi cron status <job-id> --watch
```

### Ejecutar Ahora

```bash
# Activar ejecución inmediata (omite la programación)
symbi cron run <job-id>

# Con entrada personalizada
symbi cron run <job-id> --input "Check production database"
```

### Historial

```bash
# Ver historial de ejecuciones de un trabajo
symbi cron history <job-id>

# Últimas 20 ejecuciones
symbi cron history <job-id> --limit 20

# Filtrar por estado
symbi cron history <job-id> --status failed

# Exportar a CSV
symbi cron history <job-id> --format csv > runs.csv
```

## Patrón de Latido (Heartbeat)

### HeartbeatContextMode

Controla cómo persiste el contexto entre iteraciones del heartbeat:

```rust
pub enum HeartbeatContextMode {
    /// Contexto nuevo en cada iteración, resumen adjunto al historial de ejecuciones
    EphemeralWithSummary,

    /// Contexto compartido entre todas las iteraciones (la memoria se acumula)
    SharedPersistent,

    /// Contexto nuevo en cada iteración, sin resumen (sin estado)
    FullyEphemeral,
}
```

**EphemeralWithSummary (predeterminado)**:
- Nuevo `AgentContext` por iteración
- Resumen de la iteración anterior adjunto al contexto
- Previene el crecimiento ilimitado de memoria
- Mantiene continuidad para acciones relacionadas

**SharedPersistent**:
- Un solo `AgentContext` reutilizado en todas las iteraciones
- Historial completo de conversación preservado
- Mayor uso de memoria
- Ideal para agentes que necesitan contexto profundo (por ejemplo, sesiones de depuración)

**FullyEphemeral**:
- Nuevo `AgentContext` por iteración, sin traspaso
- Menor huella de memoria
- Ideal para verificaciones independientes (por ejemplo, sondas de salud de API)

### Ejemplo de Agente Heartbeat

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
  cron: "*/10 * * * *"  # Cada 10 minutos

  heartbeat: {
    enabled: true
    context_mode: "ephemeral_with_summary"
    max_iterations: 50
  }

  delivery: ["log_file", "slack"]
  slack_channel: "#ops-alerts"
}
```

## Aislamiento de Sesión

### Modos de Sesión

```rust
pub enum SessionIsolationMode {
    /// Contexto efímero con traspaso de resumen (predeterminado)
    EphemeralWithSummary,

    /// Contexto persistente compartido entre todas las ejecuciones
    SharedPersistent,

    /// Completamente efímero, sin traspaso de estado
    FullyEphemeral,
}
```

**Configuración:**

```symbiont
schedule {
  name: "data-pipeline"
  agent: "etl-agent"
  cron: "0 2 * * *"

  # Contexto nuevo por ejecución, resumen de la ejecución anterior incluido
  session_mode: "ephemeral_with_summary"
}
```

### Ciclo de Vida de la Sesión

Para cada ejecución programada:

1. **Pre-ejecución**: Verificar límites de concurrencia, aplicar jitter
2. **Creación de sesión**: Crear `AgentContext` basado en `session_mode`
3. **Puerta de política**: Evaluar condiciones de política
4. **Ejecución**: Ejecutar agente con entrada y contexto
5. **Entrega**: Enrutar salida a los canales configurados
6. **Limpieza de sesión**: Destruir o persistir contexto según el modo
7. **Post-ejecución**: Actualizar registro de ejecución, recolectar métricas

## Enrutamiento de Entrega

### Canales Soportados

```rust
pub enum DeliveryChannel {
    Stdout,           // Imprimir en consola
    LogFile,          // Escribir al archivo de registro específico del trabajo
    Webhook,          // HTTP POST a URL
    Slack,            // Webhook o API de Slack
    Email,            // Correo electrónico SMTP
    Custom(String),   // Canal definido por el usuario
}
```

### Ejemplos de Configuración

**Canal único:**

```symbiont
schedule {
  name: "backup"
  agent: "backup-agent"
  cron: "0 3 * * *"
  delivery: ["log_file"]
}
```

**Múltiples canales:**

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

**Entrega por webhook:**

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

### Trait DeliveryRouter

Los canales de entrega personalizados implementan:

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

## Aplicación de Políticas

### PolicyGate

El `PolicyGate` evalúa políticas específicas de programación antes de la ejecución:

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

### Condiciones de Política

```symbiont
schedule {
  name: "production-deploy"
  agent: "deploy-agent"
  cron: "0 0 * * 0"  # Domingo a medianoche

  policy {
    # Requerir aprobación humana antes de la ejecución
    require_approval: true

    # Tiempo máximo de ejecución antes de terminación forzada
    max_runtime: "30m"

    # Requerir capacidades específicas
    require_capabilities: ["deployment", "production_write"]

    # Aplicación de ventana de tiempo (UTC)
    allowed_hours: {
      start: "00:00"
      end: "04:00"
    }

    # Restricciones de entorno
    allowed_environments: ["staging", "production"]

    # Verificación de AgentPin requerida
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

## Robustez para Producción

### Jitter

Previene la estampida (thundering herd) cuando múltiples trabajos comparten una programación:

```rust
pub struct CronSchedulerConfig {
    pub max_jitter_seconds: u64,  // Retraso aleatorio de 0-N segundos
    // ...
}
```

**Ejemplo:**

```toml
[scheduler]
max_jitter_seconds = 30  # Distribuir inicios de trabajos en una ventana de 30 segundos
```

### Concurrencia por Trabajo

Limitar ejecuciones concurrentes por trabajo para prevenir el agotamiento de recursos:

```symbiont
schedule {
  name: "data-sync"
  agent: "sync-agent"
  cron: "*/5 * * * *"

  max_concurrent: 2  # Permitir máximo 2 ejecuciones concurrentes
}
```

Si un trabajo ya se está ejecutando al máximo de concurrencia, el programador omite el tick.

### Cola de Mensajes No Entregados (Dead-Letter Queue)

Los trabajos que exceden `max_retries` pasan al estado `DeadLetter` para revisión manual:

```symbiont
schedule {
  name: "flaky-job"
  agent: "unreliable-agent"
  cron: "0 * * * *"

  max_retries: 3  # Después de 3 fallos, mover a dead-letter
}
```

**Recuperación:**

```bash
# Listar trabajos en dead-letter
symbi cron list --status dead_letter

# Revisar razones de fallo
symbi cron history <job-id> --status failed

# Restablecer trabajo a activo después de corregir
symbi cron reset <job-id>
```

### Verificación de AgentPin

Verificar criptográficamente la identidad del agente antes de la ejecución:

```symbiont
schedule {
  name: "secure-task"
  agent: "trusted-agent"
  cron: "0 * * * *"

  agent_pin_jwt: "${AGENT_PIN_JWT}"  # JWT ES256 de agentpin-cli

  policy {
    require_agent_pin: true
  }
}
```

El programador verifica:
1. Firma JWT usando ES256 (ECDSA P-256)
2. El ID del agente coincide con el claim `iss`
3. El ancla de dominio coincide con el origen esperado
4. La expiración (`exp`) es válida

Los fallos activan el evento de auditoría `SecurityEventType::AgentPinVerificationFailed`.

## Endpoints de la API HTTP

### Gestión de Programación

**POST /api/v1/schedule**
Crear un nuevo trabajo programado.

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
Listar todos los trabajos (filtrable por estado, ID de agente).

```bash
curl "http://localhost:8080/api/v1/schedule?status=active&agent_id=reporter"
```

**GET /api/v1/schedule/{job_id}**
Obtener detalles del trabajo.

```bash
curl http://localhost:8080/api/v1/schedule/job-123
```

**PUT /api/v1/schedule/{job_id}**
Actualizar trabajo (expresión cron, entrega, etc.).

```bash
curl -X PUT http://localhost:8080/api/v1/schedule/job-123 \
  -H "Content-Type: application/json" \
  -d '{"cron_expr": "0 */2 * * *"}'
```

**DELETE /api/v1/schedule/{job_id}**
Eliminar trabajo.

```bash
curl -X DELETE http://localhost:8080/api/v1/schedule/job-123
```

**POST /api/v1/schedule/{job_id}/pause**
Pausar trabajo.

```bash
curl -X POST http://localhost:8080/api/v1/schedule/job-123/pause
```

**POST /api/v1/schedule/{job_id}/resume**
Reanudar trabajo pausado.

```bash
curl -X POST http://localhost:8080/api/v1/schedule/job-123/resume
```

**POST /api/v1/schedule/{job_id}/run**
Activar ejecución inmediata.

```bash
curl -X POST http://localhost:8080/api/v1/schedule/job-123/run \
  -H "Content-Type: application/json" \
  -d '{"input": "Run with custom input"}'
```

**GET /api/v1/schedule/{job_id}/history**
Obtener historial de ejecuciones.

```bash
curl "http://localhost:8080/api/v1/schedule/job-123/history?limit=20&status=failed"
```

**GET /api/v1/schedule/{job_id}/next_run**
Obtener el próximo tiempo de ejecución programado.

```bash
curl http://localhost:8080/api/v1/schedule/job-123/next_run
```

### Monitoreo de Salud

**GET /api/v1/health/scheduler**
Salud y métricas del programador.

```bash
curl http://localhost:8080/api/v1/health/scheduler
```

**Respuesta:**

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

## Ejemplos del SDK

### SDK de JavaScript

```javascript
import { SymbiontClient } from '@symbiont/sdk-js';

const client = new SymbiontClient({
  baseUrl: 'http://localhost:8080',
  apiKey: process.env.SYMBI_API_KEY
});

// Crear trabajo programado
const job = await client.schedule.create({
  name: 'daily-backup',
  agentId: 'backup-agent',
  cronExpr: '0 2 * * *',
  sessionMode: 'ephemeral_with_summary',
  delivery: ['webhook'],
  webhookUrl: 'https://backup.example.com/notify'
});

console.log(`Created job: ${job.id}`);

// Listar trabajos activos
const activeJobs = await client.schedule.list({ status: 'active' });
console.log(`Active jobs: ${activeJobs.length}`);

// Obtener estado del trabajo
const status = await client.schedule.getStatus(job.id);
console.log(`Next run: ${status.next_run}`);

// Activar ejecución inmediata
await client.schedule.runNow(job.id, { input: 'Backup database' });

// Pausar trabajo
await client.schedule.pause(job.id);

// Ver historial
const history = await client.schedule.getHistory(job.id, { limit: 10 });
history.forEach(run => {
  console.log(`Run ${run.id}: ${run.status} (${run.execution_time_ms}ms)`);
});

// Reanudar trabajo
await client.schedule.resume(job.id);

// Eliminar trabajo
await client.schedule.delete(job.id);
```

### SDK de Python

```python
from symbiont import SymbiontClient

client = SymbiontClient(
    base_url='http://localhost:8080',
    api_key=os.environ['SYMBI_API_KEY']
)

# Crear trabajo programado
job = client.schedule.create(
    name='hourly-metrics',
    agent_id='metrics-agent',
    cron_expr='0 * * * *',
    session_mode='ephemeral_with_summary',
    delivery=['slack', 'log_file'],
    slack_channel='#metrics'
)

print(f"Created job: {job.id}")

# Listar trabajos para un agente específico
jobs = client.schedule.list(agent_id='metrics-agent')
print(f"Found {len(jobs)} jobs for metrics-agent")

# Obtener detalles del trabajo
details = client.schedule.get(job.id)
print(f"Cron: {details.cron_expr}")
print(f"Next run: {details.next_run}")

# Actualizar expresión cron
client.schedule.update(job.id, cron_expr='*/30 * * * *')

# Activar ejecución inmediata
run = client.schedule.run_now(job.id, input='Generate metrics report')
print(f"Run ID: {run.id}")

# Pausar durante mantenimiento
client.schedule.pause(job.id)
print("Job paused for maintenance")

# Ver fallos recientes
history = client.schedule.get_history(
    job.id,
    status='failed',
    limit=5
)
for run in history:
    print(f"Failed run {run.id}: {run.error_message}")

# Reanudar después del mantenimiento
client.schedule.resume(job.id)

# Verificar salud del programador
health = client.schedule.health()
print(f"Scheduler status: {health.status}")
print(f"Active jobs: {health.active_jobs}")
print(f"In-flight jobs: {health.in_flight_jobs}")
```

## Configuración

### CronSchedulerConfig

```rust
pub struct CronSchedulerConfig {
    /// Intervalo de tick en segundos (predeterminado: 1)
    pub tick_interval_seconds: u64,

    /// Jitter máximo para prevenir estampida (predeterminado: 0)
    pub max_jitter_seconds: u64,

    /// Límite global de concurrencia (predeterminado: 10)
    pub max_concurrent_jobs: usize,

    /// Habilitar recolección de métricas (predeterminado: true)
    pub enable_metrics: bool,

    /// Umbral de reintentos para dead-letter (predeterminado: 3)
    pub default_max_retries: u32,

    /// Tiempo de espera para apagado gracioso (predeterminado: 30s)
    pub shutdown_timeout_seconds: u64,
}
```

### Configuración TOML

```toml
[scheduler]
tick_interval_seconds = 1
max_jitter_seconds = 30
max_concurrent_jobs = 20
enable_metrics = true
default_max_retries = 3
shutdown_timeout_seconds = 60

[scheduler.delivery]
# Configuración de webhook
webhook_timeout_seconds = 30
webhook_retry_attempts = 3

# Configuración de Slack
slack_api_token = "${SLACK_API_TOKEN}"
slack_default_channel = "#ops"

# Configuración de correo electrónico
smtp_host = "smtp.example.com"
smtp_port = 587
smtp_username = "${SMTP_USER}"
smtp_password = "${SMTP_PASS}"
email_from = "symbiont@example.com"
```

### Variables de Entorno

```bash
# Configuración del programador
SYMBI_SCHEDULER_MAX_JITTER=30
SYMBI_SCHEDULER_MAX_CONCURRENT=20

# Configuración de entrega
SYMBI_SLACK_TOKEN=xoxb-...
SYMBI_WEBHOOK_AUTH_HEADER="Bearer secret-token"

# Verificación de AgentPin
SYMBI_AGENTPIN_REQUIRED=true
SYMBI_AGENTPIN_DOMAIN=agent.example.com
```

## Observabilidad

### Métricas (compatibles con Prometheus)

```
# Ejecuciones totales
symbiont_cron_runs_total{job_name="daily-report",status="succeeded"} 450

# Ejecuciones fallidas
symbiont_cron_runs_total{job_name="daily-report",status="failed"} 5

# Histograma de tiempo de ejecución
symbiont_cron_execution_duration_seconds{job_name="daily-report"} 1.234

# Indicador de trabajos en curso
symbiont_cron_in_flight_jobs 3

# Trabajos en dead-letter
symbiont_cron_dead_letter_total{job_name="flaky-job"} 2
```

### Eventos de Auditoría

Todas las acciones del programador emiten eventos de seguridad:

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

Consultar el registro de auditoría:

```bash
symbi audit query --type CronJobFailed --since "2026-02-01" --limit 50
```

## Mejores Prácticas

1. **Usar jitter para programaciones compartidas**: Prevenir que múltiples trabajos inicien simultáneamente
2. **Establecer límites de concurrencia**: Proteger contra el agotamiento de recursos
3. **Monitorear la cola de dead-letter**: Revisar y corregir trabajos fallidos regularmente
4. **Usar EphemeralWithSummary**: Previene el crecimiento ilimitado de memoria en heartbeats de larga duración
5. **Habilitar verificación de AgentPin**: Verificar criptográficamente la identidad del agente
6. **Configurar enrutamiento de entrega**: Usar canales apropiados para diferentes tipos de trabajos
7. **Establecer puertas de política**: Aplicar ventanas de tiempo, aprobaciones y verificaciones de capacidades
8. **Usar patrón de heartbeat para monitoreo**: Ciclos continuos de evaluación-acción-pausa
9. **Probar programaciones en staging**: Validar expresiones cron y lógica de trabajos antes de producción
10. **Exportar métricas**: Integrar con Prometheus/Grafana para visibilidad operativa

## Solución de Problemas

### El Trabajo No Se Ejecuta

1. Verificar el estado del trabajo: `symbi cron status <job-id>`
2. Verificar la expresión cron: Usar [crontab.guru](https://crontab.guru/)
3. Verificar la salud del programador: `curl http://localhost:8080/api/v1/health/scheduler`
4. Revisar los registros: `symbi logs --filter scheduler --level debug`

### El Trabajo Falla Repetidamente

1. Ver historial: `symbi cron history <job-id> --status failed`
2. Verificar mensajes de error en los registros de ejecución
3. Verificar la configuración y capacidades del agente
4. Probar el agente fuera del programador: `symbi run <agent-id> --input "test"`
5. Verificar puertas de política: Asegurar que las ventanas de tiempo y capacidades coincidan

### Trabajo en Dead-Letter

1. Listar trabajos en dead-letter: `symbi cron list --status dead_letter`
2. Revisar el patrón de fallos: `symbi cron history <job-id>`
3. Corregir la causa raíz (código del agente, permisos, dependencias externas)
4. Restablecer el trabajo: `symbi cron reset <job-id>`

### Alto Uso de Memoria

1. Verificar el modo de sesión: Cambiar a `ephemeral_with_summary` o `fully_ephemeral`
2. Reducir iteraciones de heartbeat: Disminuir `max_iterations`
3. Monitorear el tamaño del contexto: Revisar la verbosidad de salida del agente
4. Habilitar archivado de contexto: Configurar políticas de retención

## Migración desde v0.9.0

La versión v1.0.0 agrega características de robustez para producción. Actualice sus definiciones de trabajos:

```diff
 schedule {
   name: "my-job"
   agent: "my-agent"
   cron: "0 * * * *"
+
+  # Agregar límite de concurrencia
+  max_concurrent: 2
+
+  # Agregar AgentPin para verificación de identidad
+  agent_pin_jwt: "${AGENT_PIN_JWT}"
+
+  policy {
+    require_agent_pin: true
+  }
 }
```

Actualizar configuración:

```diff
 [scheduler]
 tick_interval_seconds = 1
+ max_jitter_seconds = 30
+ default_max_retries = 3
+ shutdown_timeout_seconds = 60
```

Sin cambios de API incompatibles. Todos los trabajos de v0.9.0 continúan funcionando.
