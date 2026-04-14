# Módulo de Entrada HTTP

O módulo de Entrada HTTP fornece um servidor webhook que permite que sistemas externos invoquem agentes Symbiont através de requisições HTTP. Este módulo permite integração com serviços externos, webhooks e APIs expondo agentes através de endpoints HTTP.

## Visão Geral

O módulo de Entrada HTTP consiste em:

- **Servidor HTTP**: Um servidor web baseado em Axum que escuta requisições HTTP recebidas
- **Autenticação**: Suporte para autenticação baseada em Bearer token e JWT
- **Roteamento de Requisições**: Regras de roteamento flexíveis para direcionar requisições para agentes específicos
- **Controle de Resposta**: Formatação de resposta configurável e códigos de status
- **Recursos de Segurança**: Suporte CORS, limites de tamanho de requisição e registro de auditoria
- **Gerenciamento de Concorrência**: Limitação de taxa de requisições integrada e controle de concorrência
- **Invocação de LLM com ToolClad**: Quando o agente alvo não está ativamente em execução no barramento de comunicação do runtime, o webhook pode invocar o agente sob demanda através de um provedor de LLM configurado, usando um loop de chamada de ferramentas no estilo ORGA apoiado por manifestos ToolClad

O módulo é compilado condicionalmente com a flag de recurso `http-input` e integra-se perfeitamente com o runtime de agentes Symbiont.

## Configuração

O módulo de Entrada HTTP é configurado usando a estrutura [`HttpInputConfig`](../crates/runtime/src/http_input/config.rs):

### Configuração Básica

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

### Campos de Configuração

| Campo | Tipo | Padrão | Descrição |
|-------|------|--------|-----------|
| `bind_address` | `String` | `"127.0.0.1"` | Endereço IP para vincular o servidor HTTP |
| `port` | `u16` | `8081` | Número da porta para escutar |
| `path` | `String` | `"/webhook"` | Endpoint de caminho HTTP |
| `agent` | `AgentId` | Novo ID | Agente padrão para invocar para requisições |
| `auth_header` | `Option<String>` | `None` | Bearer token para autenticação |
| `jwt_public_key_path` | `Option<String>` | `None` | Caminho para arquivo de chave pública JWT |
| `max_body_bytes` | `usize` | `65536` | Tamanho máximo do corpo da requisição (64 KB) |
| `concurrency` | `usize` | `10` | Máximo de requisições concorrentes |
| `routing_rules` | `Option<Vec<AgentRoutingRule>>` | `None` | Regras de roteamento de requisições |
| `response_control` | `Option<ResponseControlConfig>` | `None` | Configuração de formatação de resposta |
| `forward_headers` | `Vec<String>` | `[]` | Cabeçalhos para encaminhar aos agentes |
| `cors_origins` | `Vec<String>` | `[]` | Origens CORS permitidas (vazio = CORS desabilitado) |
| `audit_enabled` | `bool` | `true` | Habilitar registro de auditoria de requisições |

### Regras de Roteamento de Agentes

Rotear requisições para diferentes agentes baseado nas características da requisição:

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

### Controle de Resposta

Personalizar respostas HTTP com [`ResponseControlConfig`](../crates/runtime/src/http_input/config.rs):

```rust
use symbiont_runtime::http_input::ResponseControlConfig;

let response_control = ResponseControlConfig {
    default_status: 200,
    agent_output_to_json: true,
    error_status: 500,
    echo_input_on_error: false,
};
```

## Recursos de Segurança

### Autenticação

O módulo de Entrada HTTP suporta múltiplos métodos de autenticação:

#### Autenticação com Bearer Token

Configurar um bearer token estático:

```rust
let config = HttpInputConfig {
    auth_header: Some("Bearer your-secret-token".to_string()),
    ..Default::default()
};
```

#### Integração com Armazenamento de Segredos

Usar referências de segredos para segurança aprimorada:

```rust
let config = HttpInputConfig {
    auth_header: Some("vault://webhook/auth_token".to_string()),
    ..Default::default()
};
```

#### Autenticação JWT (EdDSA)

Configurar autenticação baseada em JWT com chaves públicas Ed25519:

```rust
let config = HttpInputConfig {
    jwt_public_key_path: Some("/path/to/jwt/ed25519-public.pem".to_string()),
    ..Default::default()
};
```

O verificador JWT carrega uma chave pública Ed25519 do arquivo PEM especificado e valida tokens `Authorization: Bearer <jwt>` recebidos. Apenas o algoritmo **EdDSA** é aceito -- HS256, RS256 e outros algoritmos são rejeitados.

#### Endpoint de Saúde

O módulo de Entrada HTTP não expõe seu próprio endpoint `/health`. Verificações de saúde estão disponíveis através da API HTTP principal em `/api/v1/health` ao executar `symbi up`, que inicia o runtime completo incluindo o servidor de API:

```bash
# Verificação de saúde via o servidor de API principal (porta padrão 8080)
curl http://127.0.0.1:8080/api/v1/health
# => {"status": "ok"}
```

Se você precisar de probes de saúde especificamente para o servidor de Entrada HTTP, redirecione seu load balancer para o endpoint de saúde da API principal.

### Controles de Segurança

- **Apenas Loopback por Padrão**: `bind_address` padrão é `127.0.0.1` -- o servidor só aceita conexões locais a menos que configurado explicitamente de outra forma
- **CORS Desabilitado por Padrão**: `cors_origins` padrão é uma lista vazia, significando que CORS está desabilitado; adicione origens específicas para habilitar acesso cross-origin
- **Limites de Tamanho de Requisição**: Tamanho máximo configurável do corpo previne esgotamento de recursos
- **Limites de Concorrência**: Semáforo integrado controla processamento de requisições concorrentes
- **Registro de Auditoria**: Registro estruturado de todas as requisições recebidas quando habilitado
- **Resolução de Segredos**: Integração com Vault e armazenamentos de segredos baseados em arquivo

## Exemplo de Uso

### Iniciando o Servidor de Entrada HTTP

```rust
use symbiont_runtime::http_input::{HttpInputConfig, start_http_input};
use symbiont_runtime::secrets::SecretsConfig;
use std::sync::Arc;

// Configurar o servidor de entrada HTTP
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

// Opcional: Configurar segredos
let secrets_config = SecretsConfig::default();

// Iniciar o servidor
start_http_input(config, Some(runtime), Some(secrets_config)).await?;
```

### Exemplo de Definição de Agente

Criar um agente manipulador de webhook em [`webhook_handler.dsl`](../agents/webhook_handler.dsl):

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

### Exemplo de Requisição HTTP

Enviar uma requisição webhook para acionar o agente:

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

### Resposta Esperada

O formato da resposta depende de como o agente foi invocado.

**Dispatch do runtime** -- o agente alvo está `Running` no barramento de comunicação e a mensagem foi entregue para processamento assíncrono:

```json
{
  "status": "execution_started",
  "agent_id": "webhook_handler",
  "message_id": "01H...",
  "latency_ms": 3,
  "timestamp": "2024-01-15T10:30:00Z"
}
```

**Invocação de LLM** -- o agente não está em execução e foi executado sob demanda através do provedor de LLM configurado (veja [Invocação de LLM com Ferramentas ToolClad](#invocação-de-llm-com-ferramentas-toolclad) abaixo). A resposta inclui o texto final e um resumo de quaisquer chamadas de ferramentas que foram executadas:

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

## Invocação de LLM com Ferramentas ToolClad

Quando o runtime está anexado mas o agente roteado **não está no estado `Running`**, o manipulador de webhook recorre a um caminho de invocação de LLM sob demanda. Isso é útil para agentes que executam por requisição em vez de como listeners de longa duração.

### Como funciona

1. O manipulador de webhook chama `scheduler.get_agent_status()` para verificar se o agente está ativamente em execução. Mensagens para agentes não em execução não são despachadas através do barramento de comunicação, já que `send_message` as descartaria silenciosamente.
2. Se o agente não está em execução, o manipulador constrói um system prompt a partir de quaisquer arquivos `.dsl` encontrados no diretório `agents/`, anexa um `system_prompt` opcional fornecido pelo chamador (com limite de tamanho e registrado), e constrói uma mensagem de usuário a partir do payload da requisição.
3. Manifestos ToolClad no diretório `tools/` são carregados e expostos ao LLM como ferramentas de function-calling. Tipos customizados de `toolclad.toml` são aplicados.
4. O manipulador executa um loop de chamada de ferramentas **ORGA** (Observe-Reason-Gate-Act), até 15 iterações:
   - O LLM propõe zero ou mais chamadas `tool_use`.
   - Cada chamada de ferramenta é validada pelo ToolClad e executada em um pool de threads bloqueante com um **timeout de 120 segundos por ferramenta**.
   - Pares `(tool_name, input)` duplicados dentro de uma única iteração são deduplicados para evitar execução redundante de ferramentas não idempotentes.
   - Resultados de ferramentas são realimentados ao LLM como mensagens `tool_result`.
   - O loop termina quando o LLM produz uma resposta de texto final ou o limite de iterações é atingido.
5. A resposta final, a lista de execuções de ferramentas e metadados de provedor/modelo são retornados ao chamador.

### Detecção automática de provedor

O cliente LLM é inicializado a partir de variáveis de ambiente na inicialização do servidor. O primeiro provedor cuja chave de API está definida vence, nesta ordem:

| Variável de ambiente | Provedor | Override de modelo | Override de URL base |
|----------------------|----------|--------------------|----------------------|
| `OPENROUTER_API_KEY` | OpenRouter | `OPENROUTER_MODEL` (padrão: `anthropic/claude-sonnet-4`) | `OPENROUTER_BASE_URL` |
| `OPENAI_API_KEY` | OpenAI | `CHAT_MODEL` (padrão: `gpt-4o`) | `OPENAI_BASE_URL` |
| `ANTHROPIC_API_KEY` | Anthropic | `ANTHROPIC_MODEL` (padrão: `claude-sonnet-4-20250514`) | `ANTHROPIC_BASE_URL` |

Se nenhuma chave de API estiver definida, o caminho de invocação de LLM é desabilitado e requisições para agentes não em execução retornam um erro.

### Campos de entrada

O corpo JSON do webhook é interpretado da seguinte forma quando o caminho de LLM é tomado:

- `prompt` ou `message` -- usado como a mensagem de usuário. Se nenhum estiver presente, o payload inteiro é formatado e passado como a descrição da tarefa.
- `system_prompt` -- system prompt opcional fornecido pelo chamador, anexado ao system prompt derivado do DSL. Limitado a 4096 bytes e registrado. Trate como uma superfície de prompt-injection: sempre aplique autenticação ao expor este endpoint a chamadores não confiáveis.

### Formato normalizado de chamada de ferramentas

O cliente LLM normaliza o function calling do OpenAI/OpenRouter para o mesmo formato de bloco de conteúdo usado pela API de Messages da Anthropic. Independentemente do provedor, cada bloco de conteúdo de resposta é `{"type": "text", "text": "..."}` ou `{"type": "tool_use", "id": "...", "name": "...", "input": {...}}`, e `stop_reason` é `"end_turn"` ou `"tool_use"`.

## Padrões de Integração

### Endpoints de Webhook

Configurar diferentes agentes para diferentes fontes de webhook:

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

### Integração com Gateway de API

Usar como serviço backend atrás de um gateway de API:

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

### Integração com Verificação de Saúde

O módulo de Entrada HTTP não inclui um endpoint de saúde dedicado. Use o endpoint de saúde da API principal (`/api/v1/health`) para integração com load balancers e monitoramento. Veja a seção [Endpoint de Saúde](#endpoint-de-saúde) acima para detalhes.

## Tratamento de Erros

O módulo de Entrada HTTP fornece tratamento de erros abrangente:

- **Erros de Autenticação**: Retorna `401 Unauthorized` para tokens inválidos
- **Limitação de Taxa**: Retorna `429 Too Many Requests` quando limites de concorrência são excedidos
- **Erros de Payload**: Retorna `400 Bad Request` para JSON malformado
- **Erros de Agente**: Retorna status de erro configurável com detalhes do erro
- **Erros do Servidor**: Retorna `500 Internal Server Error` para falhas de runtime

## Monitoramento e Observabilidade

### Registro de Auditoria

Quando `audit_enabled` é true, o módulo registra informações estruturadas sobre todas as requisições:

```log
INFO HTTP Input: Received request with 5 headers
INFO Agent webhook_handler is running, dispatching via communication bus
INFO Runtime execution dispatched for agent webhook_handler: message_id=… latency=3ms
```

Quando o caminho de invocação de LLM é usado, linhas adicionais rastreiam o loop ORGA:

```log
INFO Agent webhook_handler is not running, using LLM invocation path
INFO Invoking LLM for agent webhook_handler: provider=Anthropic model=… tools=4 …
INFO ORGA ACT: executing tool 'nmap_scan' (id=…) for agent webhook_handler
INFO Tool 'nmap_scan' executed successfully
INFO ORGA loop iteration 1 for agent webhook_handler: executed 1 tool(s), continuing
INFO LLM invocation completed for agent webhook_handler: latency=4821ms tool_runs=1 response_len=…
```

### Integração de Métricas

O módulo integra-se com o sistema de métricas do runtime Symbiont para fornecer:

- Contagem e taxa de requisições
- Distribuições de tempo de resposta
- Taxas de erro por tipo
- Contagens de conexões ativas
- Utilização de concorrência

## Melhores Práticas

1. **Segurança**: Sempre usar autenticação em ambientes de produção
2. **Limitação de Taxa**: Configurar limites de concorrência apropriados baseados na sua infraestrutura
3. **Monitoramento**: Habilitar registro de auditoria e integrar com sua stack de monitoramento
4. **Tratamento de Erros**: Configurar respostas de erro apropriadas para seu caso de uso
5. **Design de Agentes**: Projetar agentes para lidar com formatos de entrada específicos de webhook
6. **Limites de Recursos**: Definir limites razoáveis de tamanho de corpo para prevenir esgotamento de recursos

## Veja Também

- [Guia de Introdução](getting-started.md)
- [Guia DSL](dsl-guide.md)
- [Referência da API](api-reference.md)
- [Loop de Raciocínio (ORGA)](reasoning-loop.md)
- [Contratos de Ferramentas ToolClad](toolclad.md)
- [Documentação do Runtime de Agentes](../crates/runtime/README.md)
