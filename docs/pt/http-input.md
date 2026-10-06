# Módulo de Entrada HTTP

O módulo de Entrada HTTP fornece um servidor webhook que permite que sistemas externos invoquem agentes Symbiont através de requisições HTTP. Este módulo permite integração com serviços externos, webhooks e APIs expondo agentes através de endpoints HTTP.

## Visão Geral

Nesta branch, cada requisição HTTP de raciocínio é executada de forma
independente, mesmo que o agente registrado já esteja ativo. O código-fonte
registrado e a camada de segurança selecionam um executor de ferramentas
congelado antes da inferência. Limites de CPU, de memória e de tempo de execução
restringem essa invocação. Os workers governados também compartilham a CPU, a
memória e o pool de workers configurados do supervisor com as execuções do
agendador e da CLI que usam o mesmo diretório de estado privado; veja
[orçamentos compartilhados](/shared-budgets). As respostas bem-sucedidas incluem
`audit` com `run_id`, `path` e `public_key`. Falhas no armazenamento obrigatório
interrompem os efeitos seguintes, e as requisições descartadas mantêm a
responsabilidade pela limpeza. Veja [auditoria de execução](/run-audit) e o
[guia da branch](/containment-branch-guide).

O módulo de Entrada HTTP consiste em:

- **Servidor HTTP**: Um servidor web baseado em Axum que escuta requisições HTTP recebidas
- **Autenticação**: Suporte para autenticação baseada em Bearer token e JWT
- **Roteamento de Requisições**: Regras de roteamento flexíveis para direcionar requisições para agentes específicos
- **Controle de Resposta**: Formatação de resposta configurável e códigos de status
- **Recursos de Segurança**: Suporte CORS, limites de tamanho de requisição e registro de auditoria
- **Gerenciamento de Concorrência**: Limitação de taxa de requisições integrada e controle de concorrência
- **Invocação de LLM com ToolClad**: Cada requisição invoca o agente registrado de forma independente através do provedor de LLM configurado e do loop governado de chamada de ferramentas ORGA, inclusive quando outra invocação está ativa

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

O carregador de chaves aceita PEM Ed25519 ou bytes brutos de chave pública para a
verificação EdDSA. Os JWTs devem ter um `exp` válido e um `sub` não vazio (de no
máximo 512 bytes). A validação de expiração tolera cinco segundos de desvio de
relógio. Se fornecido, `iss` deve ser não vazio e ter no máximo 2.048 bytes.
Renovar um token com o mesmo subject assinado, o mesmo emissor e a mesma chave
configurada preserva a identidade do chamador.

Este verificador de Entrada HTTP **não** aplica uma allowlist de audience ou de
emissor. A chave configurada é a sua autoridade de confiança; use uma chave
dedicada a essa autoridade. Um emissor assinado contribui para a identidade em
retentativas, mas não estabelece uma allowlist de emissores. A autenticação
Bearer é obrigatória mesmo quando a verificação de assinatura de webhook está
configurada; a assinatura do webhook é uma verificação adicional.

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
- **CORS Desabilitado por Padrão**: `cors_origins` padrão é uma lista vazia, significando que CORS está desabilitado; adicione origens específicas para habilitar acesso cross-origin. Um `"*"` literal em `cors_origins` é **rejeitado no startup** — o servidor de Entrada HTTP se recusará a iniciar com uma origem curinga. (Adicionado na auditoria pós-v1.13.0; veja `SECURITY_AUDIT.md` M1.)
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

Criar um agente manipulador de webhook em [`webhook_handler.symbi`](../agents/webhook_handler.symbi):

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
  -H "Idempotency-Key: 72d6a833-b825-4b22-b50c-206337d77f7c" \
  -d '{
    "type": "security_alert",
    "message": "Suspicious login detected",
    "source": "slack",
    "severity": "high",
    "user": "admin@company.com"
  }'
```

Escolha e guarde um UUID novo para cada tarefa pretendida. Todo envio HTTP exige
exatamente um cabeçalho `Idempotency-Key`; repita a tentativa com o mesmo ID, a
mesma URI e o mesmo payload JSON. Reutilizar o ID deste exemplo para um trabalho
diferente será recusado. Os remetentes de webhook devem manter um UUID estável por
entrega, ou usar um adaptador que mapeie a identidade de entrega deles para um
UUID estável antes do envio. O servidor não infere identidade a partir da saída do
modelo nem gera um substituto quando o cabeçalho está ausente.

### Estados de retentativa

Os IDs ocupam um domínio HTTP por projeto canônico. A reivindicação durável
vincula o chamador verificado, a URI da requisição, a entrada JSON e o
código-fonte/as configurações confiáveis do alvo. Os IDs de agentes registrados
podem mudar na reinicialização sem criar uma requisição diferente. Um servidor SDK
autônomo deve preservar o `AgentId` configurado entre reinicializações. Alterar o
código-fonte, o alvo, o chamador ou o payload sob um ID existente recusa o
trabalho; nenhum conteúdo de cache ou referência de auditoria é retornado a um
chamador diferente.

| Status HTTP | `status` do corpo | Significado |
|---|---|---|
| 200 por padrão | `completed` | Resultado original persistido; `replayed` identifica uma resposta salva. |
| 422 | `failed` | Uma falha terminal com evidências completas e rastreadas foi persistida; as retentativas a retornam. |
| 409 | `in_progress` | Outro dono detém o ID; esta requisição não inicia trabalho algum. |
| 409 | `unresolved` | A execução original precisa de reconciliação; inclui a referência de auditoria dela quando disponível. |
| 409 | `reconciled` | Retorna uma avaliação do operador assinada separadamente; o ID original não pode executar de novo. |
| 409 | `conflict` | O ID está vinculado a outro chamador ou a outra requisição. |
| 400 | `invalid_invocation_id` | Cabeçalho UUID ausente, repetido ou inválido. |
| 503 | `unavailable` | O armazenamento de invocações obrigatório não pôde autorizar a execução. |

As respostas de estado de invocação incluem os cabeçalhos `Idempotency-Key`,
`Idempotency-Replayed` e `Cache-Control: no-store`. A formatação de sucesso
configurada continua se aplicando aos resultados concluídos; ela não pode
transformar desfechos não resolvidos ou conflitantes em respostas bem-sucedidas.
As origens CORS configuradas permitem e expõem os cabeçalhos de invocação.

Um dono retido mantém a reivindicação durante a preparação, a execução, a limpeza
e a gravação do resultado. A desconexão do cliente cancela o trabalho desse dono,
mas não libera o ID para executar de novo. A perda do processo antes da
persistência do resultado deixa uma reivindicação não resolvida. Os resultados
salvos são verificados contra a auditoria assinada original antes do retorno; a
recuperação não executa o provedor nem o executor novamente.

Credenciais estáticas compartilhadas representam um único chamador. Um chamador
JWT vincula a chave configurada, o emissor assinado e o subject; os campos de
expiração/renovação não o alteram. Rotacionar credenciais ou material de chave faz
um ID existente entrar em conflito, em vez de criar silenciosamente uma segunda
tarefa. As reivindicações valem para todo o projeto, em todos os listeners HTTP,
então use UUIDs aleatórios novos e preserve o armazenamento junto com as
evidências de auditoria dele. Veja
[identidades persistentes de invocação](/invocation-idempotency) para os limites
de armazenamento.

### Resposta Esperada

Cada requisição de raciocínio retorna o próprio resultado, inclusive quando outra
invocação do mesmo agente está ativa. A antiga resposta de entrega
`execution_started`/`message_id` não é mais usada nesta rota. As respostas
bem-sucedidas incluem a referência pública de auditoria da execução,
`invocation_id`, `replayed`, `total_usage` e o snapshot do `budget` compartilhado.
Valores ilustrativos:

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

`tool_runs` resume as observações de ferramentas correlacionadas, incluindo
negações e falhas de validação. A presença dele não prova que um efeito foi
executado, e `status: completed` pode acompanhar uma resposta de recusa por
política. Use os argumentos normalizados exatos, a decisão e os registros de
efeito do journal protegido para verificação. Falhas na auditoria obrigatória ou
na limpeza retornam erro, mesmo que um efeito anterior já tenha ocorrido. O
`audit.path` é um caminho no host do runtime, não uma URL de download.

## Invocação de LLM com Ferramentas ToolClad

Cada requisição HTTP de raciocínio inicia uma invocação governada independente,
inclusive quando o agente registrado já tem outra invocação ativa.

### Como funciona

1. Com um runtime anexado, resolva o agente a partir do registro confiável.
   Congele o código-fonte, o sandbox e as configurações de recursos selecionados;
   rejeite agentes ausentes, seleções ambíguas e camadas de segurança
   incompatíveis antes da inferência. Um servidor SDK autônomo usa o
   agente/executor genérico configurado explicitamente nele.
2. Construa o system prompt apenas a partir do código-fonte do agente
   selecionado. Um `system_prompt` opcional fornecido pelo chamador continua com
   limite de tamanho e registrado; ele não fornece autoridade de política, de
   principal nem de sandbox. Construa a mensagem de usuário a partir do payload da
   requisição.
3. Descubra as ferramentas ToolClad no projeto congelado e abra um journal
   assinado, privado e obrigatório. O loop ORGA permite até 15 iterações. Os
   prazos registrados e os selecionados pelo agente restringem os limites do loop
   e das ferramentas; o limite padrão por ferramenta é de 120 segundos.
4. Prepare e normalize as chamadas propostas antes do Cedar. Aprovações exatas
   obrigatórias, auditoria obrigatória e autorização de uso único precedem os
   efeitos. IDs de chamada duplicados ou vazios são rejeitados; os resultados
   precisam se correlacionar com as chamadas efetivamente preparadas.
5. Aguarde a limpeza do worker e o registro terminal no journal. As respostas
   bem-sucedidas incluem a resposta final, os desfechos das ferramentas, os
   metadados de provedor/modelo e a referência `audit`. O cancelamento mantém a
   responsabilidade pela limpeza; erros no armazenamento obrigatório ou na limpeza
   não podem produzir silenciosamente um resultado bem-sucedido.

Veja [chamadas preparadas](/prepared-calls) e [auditoria de execução](/run-audit).
Os limites de recursos por invocação não estabelecem a admissão agregada de
requisições.

### Detecção automática de provedor

O cliente LLM é inicializado a partir de variáveis de ambiente na inicialização do servidor. O primeiro provedor cuja chave de API está definida vence, nesta ordem:

| Variável de ambiente | Provedor | Override de modelo | Override de URL base |
|----------------------|----------|--------------------|----------------------|
| `OPENROUTER_API_KEY` | OpenRouter | `OPENROUTER_MODEL` (padrão: `anthropic/claude-sonnet-4`) | `OPENROUTER_BASE_URL` |
| `OPENAI_API_KEY` | OpenAI | `CHAT_MODEL` (padrão: `gpt-4o`) | `OPENAI_BASE_URL` |
| `ANTHROPIC_API_KEY` | Anthropic | `ANTHROPIC_MODEL` (padrão: `claude-sonnet-4-20250514`) | `ANTHROPIC_BASE_URL` |

Sem um provedor de inferência configurado, as requisições de raciocínio retornam um erro. Endpoints locais configurados pelo operador continuam suportados.

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
- **Desfechos de Invocação**: Retorna os estados explícitos de retentativa acima; trabalho não resolvido nunca é reportado como concluído.
- **Erros do Servidor**: Falhas de runtime não classificadas retornam um status configurável com uma mensagem pública genérica.

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

Uma invocação retida com uma resolução do operador retorna HTTP 409 e
`status: "reconciled"`, a referência de auditoria original dela e um recibo de
`resolution` assinado separadamente. Ela não retorna um resultado bem-sucedido
fabricado nem executa novamente. Veja
[reconciliação pelo operador](/invocation-reconciliation).
