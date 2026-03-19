layout: default
title: Guia de Agendamento
description: "Agendamento de tarefas baseado em cron de nível de produção para agentes de IA do Symbiont"
nav_exclude: true
---

# Guia de Agendamento

## Outros idiomas

[English](scheduling.md) | [中文简体](scheduling.zh-cn.md) | [Español](scheduling.es.md) | ## Visão Geral

O sistema de agendamento do Symbiont oferece execução de tarefas baseada em cron de nível de produção para agentes de IA. O sistema suporta:

- **Agendamentos cron**: Sintaxe cron tradicional para tarefas recorrentes
- **Tarefas únicas**: Execução única em um horário específico
- **Padrão heartbeat**: Ciclos contínuos de avaliação-ação-espera para agentes de monitoramento
- **Isolamento de sessão**: Contextos de agente efêmeros, compartilhados ou totalmente isolados
- **Roteamento de entrega**: Múltiplos canais de saída (Stdout, LogFile, Webhook, Slack, Email, Custom)
- **Aplicação de políticas**: Verificações de segurança e conformidade antes da execução
- **Robustez para produção**: Jitter, limites de concorrência, filas de dead-letter e verificação AgentPin

## Arquitetura

O sistema de agendamento é construído sobre três componentes principais:

```
┌─────────────────────┐
│   CronScheduler     │  Loop de tick em segundo plano (intervalos de 1 segundo)
│   (Tick Loop)       │  Seleção de tarefas e orquestração de execução
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│   SqliteJobStore    │  Armazenamento persistente de tarefas
│   (Job Storage)     │  Suporte a transações, gerenciamento de estado
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│DefaultAgentScheduler│  Runtime de execução de agentes
│ (Execution Engine)  │  Gerenciamento do ciclo de vida do AgentContext
└─────────────────────┘
```

### CronScheduler

O `CronScheduler` é o ponto de entrada principal. Ele gerencia:

- Loop de tick em segundo plano executando em intervalos de 1 segundo
- Seleção de tarefas baseada no próximo horário de execução
- Controle de concorrência e injeção de jitter
- Coleta de métricas e monitoramento de saúde
- Encerramento gracioso com rastreamento de tarefas em andamento

### SqliteJobStore

O `SqliteJobStore` fornece persistência durável de tarefas com:

- Transações ACID para atualizações de estado das tarefas
- Rastreamento do ciclo de vida das tarefas (Active, Paused, Completed, Failed, DeadLetter)
- Histórico de execuções com trilha de auditoria
- Capacidades de consulta para filtragem por status, ID do agente, etc.

### DefaultAgentScheduler

O `DefaultAgentScheduler` executa agentes agendados:

- Cria instâncias de `AgentContext` isoladas ou compartilhadas
- Gerencia o ciclo de vida da sessão (criar, executar, destruir)
- Roteia entregas para os canais configurados
- Aplica portões de política antes da execução

## Sintaxe DSL

### Estrutura do Bloco Schedule

Blocos de agendamento são definidos em arquivos DSL do Symbiont:

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

### Sintaxe Cron

Sintaxe cron estendida com seis campos (segundos primeiro, campo opcional sétimo para ano):

```
┌─────────────── segundo (0-59)
│ ┌───────────── minuto (0-59)
│ │ ┌─────────── hora (0-23)
│ │ │ ┌───────── dia do mês (1-31)
│ │ │ │ ┌─────── mês (1-12)
│ │ │ │ │ ┌───── dia da semana (0-6, Domingo = 0)
│ │ │ │ │ │
* * * * * *
```

**Exemplos:**

```symbiont
# Todo dia às 9h
cron: "0 0 9 * * *"

# Toda segunda-feira às 18h
cron: "0 0 18 * * 1"

# A cada 15 minutos
cron: "0 */15 * * * *"

# Primeiro dia de cada mês à meia-noite
cron: "0 0 0 1 * *"
```

### Tarefas Únicas (Sintaxe At)

Para tarefas que executam uma única vez em um horário específico:

```symbiont
schedule {
  name: "deployment-check"
  agent: "health-checker"
  at: "2026-02-15T14:30:00Z"  # Timestamp ISO 8601

  delivery: ["webhook"]
  webhook_url: "https://ops.example.com/hooks/deployment"
}
```

### Padrão Heartbeat

Para agentes de monitoramento contínuo que avaliam -> agem -> dormem:

```symbiont
schedule {
  name: "system-monitor"
  agent: "heartbeat-agent"
  cron: "0 */5 * * * *"  # Acordar a cada 5 minutos

  heartbeat: {
    enabled: true
    context_mode: "ephemeral_with_summary"
    max_iterations: 100  # Limite de segurança
  }
}
```

O agente heartbeat segue este ciclo:

1. **Avaliação**: Avaliar o estado do sistema (ex.: verificar métricas, logs)
2. **Ação**: Tomar ação corretiva se necessário (ex.: reiniciar serviço, alertar operações)
3. **Espera**: Aguardar até o próximo tick agendado

## Comandos CLI

O comando `symbi cron` fornece gerenciamento completo do ciclo de vida:

### Listar Tarefas

```bash
# Listar todas as tarefas
symbi cron list

# Filtrar por status
symbi cron list --status active
symbi cron list --status paused

# Filtrar por agente
symbi cron list --agent "reporter-agent"

# Saída em JSON
symbi cron list --format json
```

### Adicionar Tarefa

```bash
# A partir de arquivo DSL
symbi cron add --file agent.symbi --schedule "daily-report"

# Definição inline (JSON)
symbi cron add --json '{
  "name": "quick-task",
  "agent_id": "agent-123",
  "cron_expr": "0 0 * * * *"
}'
```

### Remover Tarefa

```bash
# Por ID da tarefa
symbi cron remove <job-id>

# Por nome
symbi cron remove --name "daily-report"

# Remoção forçada (pular confirmação)
symbi cron remove <job-id> --force
```

### Pausar/Retomar

```bash
# Pausar tarefa (para agendamento, preserva estado)
symbi cron pause <job-id>

# Retomar tarefa pausada
symbi cron resume <job-id>
```

### Status

```bash
# Detalhes da tarefa com próximo horário de execução
symbi cron status <job-id>

# Incluir os últimos 10 registros de execução
symbi cron status <job-id> --history 10

# Modo observação (atualização automática a cada 5s)
symbi cron status <job-id> --watch
```

### Executar Agora

```bash
# Disparar execução imediata (ignora agendamento)
symbi cron run <job-id>

# Com entrada personalizada
symbi cron run <job-id> --input "Check production database"
```

### Histórico

```bash
# Ver histórico de execução de uma tarefa
symbi cron history <job-id>

# Últimas 20 execuções
symbi cron history <job-id> --limit 20

# Filtrar por status
symbi cron history <job-id> --status failed

# Exportar para CSV
symbi cron history <job-id> --format csv > runs.csv
```

## Padrão Heartbeat

### HeartbeatContextMode

Controla como o contexto persiste entre iterações do heartbeat:

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

**EphemeralWithSummary (padrão)**:
- Novo `AgentContext` por iteração
- Resumo da iteração anterior anexado ao contexto
- Previne crescimento ilimitado de memória
- Mantém continuidade para ações relacionadas

**SharedPersistent**:
- Um único `AgentContext` reutilizado em todas as iterações
- Histórico completo de conversação preservado
- Maior uso de memória
- Melhor para agentes que precisam de contexto profundo (ex.: sessões de depuração)

**FullyEphemeral**:
- Novo `AgentContext` por iteração, sem transferência
- Menor consumo de memória
- Melhor para verificações independentes (ex.: probes de saúde de API)

### Exemplo de Agente Heartbeat

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
  cron: "0 */10 * * * *"  # A cada 10 minutos

  heartbeat: {
    enabled: true
    context_mode: "ephemeral_with_summary"
    max_iterations: 50
  }

  delivery: ["log_file", "slack"]
  slack_channel: "#ops-alerts"
}
```

## Isolamento de Sessão

### Modos de Sessão

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

**Configuração:**

```symbiont
schedule {
  name: "data-pipeline"
  agent: "etl-agent"
  cron: "0 0 2 * * *"

  # Contexto novo por execução, resumo da execução anterior incluído
  session_mode: "ephemeral_with_summary"
}
```

### Ciclo de Vida da Sessão

Para cada execução agendada:

1. **Pré-execução**: Verificar limites de concorrência, aplicar jitter
2. **Criação de sessão**: Criar `AgentContext` baseado no `session_mode`
3. **Portão de política**: Avaliar condições de política
4. **Execução**: Executar agente com entrada e contexto
5. **Entrega**: Rotear saída para os canais configurados
6. **Limpeza de sessão**: Destruir ou persistir contexto baseado no modo
7. **Pós-execução**: Atualizar registro de execução, coletar métricas

## Roteamento de Entrega

### Canais Suportados

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

### Exemplos de Configuração

**Canal único:**

```symbiont
schedule {
  name: "backup"
  agent: "backup-agent"
  cron: "0 0 3 * * *"
  delivery: ["log_file"]
}
```

**Múltiplos canais:**

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

**Entrega via webhook:**

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

### Trait DeliveryRouter

Canais de entrega personalizados implementam:

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

## Aplicação de Políticas

### PolicyGate

O `PolicyGate` avalia políticas específicas de agendamento antes da execução:

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

### Condições de Política

```symbiont
schedule {
  name: "production-deploy"
  agent: "deploy-agent"
  cron: "0 0 0 * * 0"  # Domingo à meia-noite

  policy {
    # Exigir aprovação humana antes da execução
    require_approval: true

    # Tempo máximo de execução antes do encerramento forçado
    max_runtime: "30m"

    # Exigir capacidades específicas
    require_capabilities: ["deployment", "production_write"]

    # Aplicação de janela de horário (UTC)
    allowed_hours: {
      start: "00:00"
      end: "04:00"
    }

    # Restrições de ambiente
    allowed_environments: ["staging", "production"]

    # Verificação AgentPin obrigatória
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

## Robustez para Produção

### Jitter

Previne efeito manada (thundering herd) quando múltiplas tarefas compartilham um agendamento:

```rust
pub struct CronSchedulerConfig {
    pub max_jitter_seconds: u64,  // Random delay 0-N seconds
    // ...
}
```

**Exemplo:**

```toml
[scheduler]
max_jitter_seconds = 30  # Distribuir início das tarefas em uma janela de 30 segundos
```

### Concorrência Por Tarefa

Limitar execuções concorrentes por tarefa para prevenir esgotamento de recursos:

```symbiont
schedule {
  name: "data-sync"
  agent: "sync-agent"
  cron: "0 */5 * * * *"

  max_concurrent: 2  # Permitir no máximo 2 execuções concorrentes
}
```

Se uma tarefa já está em execução na concorrência máxima, o agendador pula o tick.

### Fila de Dead-Letter

Tarefas que excedem `max_retries` são movidas para o status `DeadLetter` para revisão manual:

```symbiont
schedule {
  name: "flaky-job"
  agent: "unreliable-agent"
  cron: "0 0 * * * *"

  max_retries: 3  # Após 3 falhas, mover para dead-letter
}
```

**Recuperação:**

```bash
# Listar tarefas em dead-letter
symbi cron list --status dead_letter

# Revisar motivos de falha
symbi cron history <job-id> --status failed

# Resetar tarefa para ativa após correção
symbi cron reset <job-id>
```

### Verificação AgentPin

Verificar criptograficamente a identidade do agente antes da execução:

```symbiont
schedule {
  name: "secure-task"
  agent: "trusted-agent"
  cron: "0 0 * * * *"

  agent_pin_jwt: "${AGENT_PIN_JWT}"  # JWT ES256 do agentpin-cli

  policy {
    require_agent_pin: true
  }
}
```

O agendador verifica:
1. Assinatura JWT usando ES256 (ECDSA P-256)
2. ID do agente corresponde à claim `iss`
3. Âncora de domínio corresponde à origem esperada
4. Expiração (`exp`) é válida

Falhas disparam o evento de auditoria `SecurityEventType::AgentPinVerificationFailed`.

## Endpoints da API HTTP

### Gerenciamento de Agendamento

**POST /api/v1/schedule**
Criar uma nova tarefa agendada.

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
Listar todas as tarefas (filtrável por status, ID do agente).

```bash
curl "http://localhost:8080/api/v1/schedule?status=active&agent_id=reporter"
```

**GET /api/v1/schedule/{job_id}**
Obter detalhes da tarefa.

```bash
curl http://localhost:8080/api/v1/schedule/job-123
```

**PUT /api/v1/schedule/{job_id}**
Atualizar tarefa (expressão cron, entrega, etc.).

```bash
curl -X PUT http://localhost:8080/api/v1/schedule/job-123 \
  -H "Content-Type: application/json" \
  -d '{"cron_expr": "0 0 */2 * * *"}'
```

**DELETE /api/v1/schedule/{job_id}**
Excluir tarefa.

```bash
curl -X DELETE http://localhost:8080/api/v1/schedule/job-123
```

**POST /api/v1/schedule/{job_id}/pause**
Pausar tarefa.

```bash
curl -X POST http://localhost:8080/api/v1/schedule/job-123/pause
```

**POST /api/v1/schedule/{job_id}/resume**
Retomar tarefa pausada.

```bash
curl -X POST http://localhost:8080/api/v1/schedule/job-123/resume
```

**POST /api/v1/schedule/{job_id}/run**
Disparar execução imediata.

```bash
curl -X POST http://localhost:8080/api/v1/schedule/job-123/run \
  -H "Content-Type: application/json" \
  -d '{"input": "Run with custom input"}'
```

**GET /api/v1/schedule/{job_id}/history**
Obter histórico de execução.

```bash
curl "http://localhost:8080/api/v1/schedule/job-123/history?limit=20&status=failed"
```

**GET /api/v1/schedule/{job_id}/next_run**
Obter próximo horário de execução agendado.

```bash
curl http://localhost:8080/api/v1/schedule/job-123/next_run
```

### Monitoramento de Saúde

**GET /api/v1/health/scheduler**
Saúde e métricas do agendador.

```bash
curl http://localhost:8080/api/v1/health/scheduler
```

**Resposta:**

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

## Exemplos de SDK

### SDK JavaScript

```javascript
import { SymbiontClient } from '@symbiont/sdk-js';

const client = new SymbiontClient({
  baseUrl: 'http://localhost:8080',
  apiKey: process.env.SYMBI_API_KEY
});

// Criar tarefa agendada
const job = await client.schedule.create({
  name: 'daily-backup',
  agentId: 'backup-agent',
  cronExpr: '0 0 2 * * *',
  sessionMode: 'ephemeral_with_summary',
  delivery: ['webhook'],
  webhookUrl: 'https://backup.example.com/notify'
});

console.log(`Created job: ${job.id}`);

// Listar tarefas ativas
const activeJobs = await client.schedule.list({ status: 'active' });
console.log(`Active jobs: ${activeJobs.length}`);

// Obter status da tarefa
const status = await client.schedule.getStatus(job.id);
console.log(`Next run: ${status.next_run}`);

// Disparar execução imediata
await client.schedule.runNow(job.id, { input: 'Backup database' });

// Pausar tarefa
await client.schedule.pause(job.id);

// Ver histórico
const history = await client.schedule.getHistory(job.id, { limit: 10 });
history.forEach(run => {
  console.log(`Run ${run.id}: ${run.status} (${run.execution_time_ms}ms)`);
});

// Retomar tarefa
await client.schedule.resume(job.id);

// Excluir tarefa
await client.schedule.delete(job.id);
```

### SDK Python

```python
from symbiont import SymbiontClient

client = SymbiontClient(
    base_url='http://localhost:8080',
    api_key=os.environ['SYMBI_API_KEY']
)

# Criar tarefa agendada
job = client.schedule.create(
    name='hourly-metrics',
    agent_id='metrics-agent',
    cron_expr='0 0 * * * *',
    session_mode='ephemeral_with_summary',
    delivery=['slack', 'log_file'],
    slack_channel='#metrics'
)

print(f"Created job: {job.id}")

# Listar tarefas para agente específico
jobs = client.schedule.list(agent_id='metrics-agent')
print(f"Found {len(jobs)} jobs for metrics-agent")

# Obter detalhes da tarefa
details = client.schedule.get(job.id)
print(f"Cron: {details.cron_expr}")
print(f"Next run: {details.next_run}")

# Atualizar expressão cron
client.schedule.update(job.id, cron_expr='0 */30 * * * *')

# Disparar execução imediata
run = client.schedule.run_now(job.id, input='Generate metrics report')
print(f"Run ID: {run.id}")

# Pausar durante manutenção
client.schedule.pause(job.id)
print("Job paused for maintenance")

# Ver falhas recentes
history = client.schedule.get_history(
    job.id,
    status='failed',
    limit=5
)
for run in history:
    print(f"Failed run {run.id}: {run.error_message}")

# Retomar após manutenção
client.schedule.resume(job.id)

# Verificar saúde do agendador
health = client.schedule.health()
print(f"Scheduler status: {health.status}")
print(f"Active jobs: {health.active_jobs}")
print(f"In-flight jobs: {health.in_flight_jobs}")
```

## Configuração

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

### Configuração TOML

```toml
[scheduler]
tick_interval_seconds = 1
max_jitter_seconds = 30
max_concurrent_jobs = 20
enable_metrics = true
default_max_retries = 3
shutdown_timeout_seconds = 60

[scheduler.delivery]
# Configurações de webhook
webhook_timeout_seconds = 30
webhook_retry_attempts = 3

# Configurações do Slack
slack_api_token = "${SLACK_API_TOKEN}"
slack_default_channel = "#ops"

# Configurações de email
smtp_host = "smtp.example.com"
smtp_port = 587
smtp_username = "${SMTP_USER}"
smtp_password = "${SMTP_PASS}"
email_from = "symbiont@example.com"
```

### Variáveis de Ambiente

```bash
# Configurações do agendador
SYMBI_SCHEDULER_MAX_JITTER=30
SYMBI_SCHEDULER_MAX_CONCURRENT=20

# Configurações de entrega
SYMBI_SLACK_TOKEN=xoxb-...
SYMBI_WEBHOOK_AUTH_HEADER="Bearer secret-token"

# Verificação AgentPin
SYMBI_AGENTPIN_REQUIRED=true
SYMBI_AGENTPIN_DOMAIN=agent.example.com
```

## Observabilidade

### Métricas (compatíveis com Prometheus)

```
# Total de execuções
symbiont_cron_runs_total{job_name="daily-report",status="succeeded"} 450

# Execuções falhas
symbiont_cron_runs_total{job_name="daily-report",status="failed"} 5

# Histograma de tempo de execução
symbiont_cron_execution_duration_seconds{job_name="daily-report"} 1.234

# Gauge de tarefas em andamento
symbiont_cron_in_flight_jobs 3

# Tarefas em dead-letter
symbiont_cron_dead_letter_total{job_name="flaky-job"} 2
```

### Eventos de Auditoria

Todas as ações do agendador emitem eventos de segurança:

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

Consultar log de auditoria:

```bash
symbi audit query --type CronJobFailed --since "2026-02-01" --limit 50
```

## Melhores Práticas

1. **Use jitter para agendamentos compartilhados**: Previne que múltiplas tarefas iniciem simultaneamente
2. **Defina limites de concorrência**: Protege contra esgotamento de recursos
3. **Monitore a fila de dead-letter**: Revise e corrija tarefas com falha regularmente
4. **Use EphemeralWithSummary**: Previne crescimento ilimitado de memória em heartbeats de longa duração
5. **Habilite verificação AgentPin**: Verifique criptograficamente a identidade do agente
6. **Configure roteamento de entrega**: Use canais apropriados para diferentes tipos de tarefa
7. **Defina portões de política**: Aplique janelas de horário, aprovações e verificações de capacidade
8. **Use o padrão heartbeat para monitoramento**: Ciclos contínuos de avaliação-ação-espera
9. **Teste agendamentos em staging**: Valide expressões cron e lógica de tarefas antes da produção
10. **Exporte métricas**: Integre com Prometheus/Grafana para visibilidade operacional

## Solução de Problemas

### Tarefa Não Executando

1. Verifique o status da tarefa: `symbi cron status <job-id>`
2. Verifique a expressão cron: Use [crontab.guru](https://crontab.guru/)
3. Verifique a saúde do agendador: `curl http://localhost:8080/api/v1/health/scheduler`
4. Revise os logs: `symbi logs --filter scheduler --level debug`

### Tarefa Falhando Repetidamente

1. Veja o histórico: `symbi cron history <job-id> --status failed`
2. Verifique mensagens de erro nos registros de execução
3. Verifique a configuração e capacidades do agente
4. Teste o agente fora do agendador: `symbi run <agent-id> --input "test"`
5. Verifique portões de política: Assegure que janelas de horário e capacidades correspondam

### Tarefa em Dead-Letter

1. Liste tarefas em dead-letter: `symbi cron list --status dead_letter`
2. Revise o padrão de falha: `symbi cron history <job-id>`
3. Corrija a causa raiz (código do agente, permissões, dependências externas)
4. Resete a tarefa: `symbi cron reset <job-id>`

### Alto Uso de Memória

1. Verifique o modo de sessão: Mude para `ephemeral_with_summary` ou `fully_ephemeral`
2. Reduza iterações do heartbeat: Diminua `max_iterations`
3. Monitore o tamanho do contexto: Revise a verbosidade da saída do agente
4. Habilite arquivamento de contexto: Configure políticas de retenção

## Migração do v0.9.0

A versão v1.0.0 adiciona recursos de robustez para produção. Atualize suas definições de tarefa:

```diff
 schedule {
   name: "my-job"
   agent: "my-agent"
   cron: "0 0 * * * *"
+
+  # Adicionar limite de concorrência
+  max_concurrent: 2
+
+  # Adicionar AgentPin para verificação de identidade
+  agent_pin_jwt: "${AGENT_PIN_JWT}"
+
+  policy {
+    require_agent_pin: true
+  }
 }
```

Atualize a configuração:

```diff
 [scheduler]
 tick_interval_seconds = 1
+ max_jitter_seconds = 30
+ default_max_retries = 3
+ shutdown_timeout_seconds = 60
```

Sem alterações de API que quebrem compatibilidade. Todas as tarefas da v0.9.0 continuam funcionando.
