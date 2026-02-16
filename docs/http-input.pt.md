---
layout: default
title: Módulo de Entrada HTTP
description: "Módulo de entrada HTTP para integração webhook com agentes Symbiont"
nav_exclude: true
---

# Módulo de Entrada HTTP

## 🌐 Outros idiomas
{: .no_toc}

[English](http-input.md) | [中文简体](http-input.zh-cn.md) | [Español](http-input.es.md) | **Português** | [日本語](http-input.ja.md) | [Deutsch](http-input.de.md)

---

O módulo de Entrada HTTP fornece um servidor webhook que permite que sistemas externos invoquem agentes Symbiont através de requisições HTTP. Este módulo permite integração com serviços externos, webhooks e APIs expondo agentes através de endpoints HTTP.

## Visão Geral

O módulo de Entrada HTTP consiste em:

- **Servidor HTTP**: Um servidor web baseado em Axum que escuta requisições HTTP recebidas
- **Autenticação**: Suporte para autenticação baseada em Bearer token e JWT
- **Roteamento de Requisições**: Regras de roteamento flexíveis para direcionar requisições para agentes específicos
- **Controle de Resposta**: Formatação de resposta configurável e códigos de status
- **Recursos de Segurança**: Suporte CORS, limites de tamanho de requisição e registro de auditoria
- **Gerenciamento de Concorrência**: Limitação de taxa de requisições integrada e controle de concorrência

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
|-------|------|---------|-------------|
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

#### Autenticação JWT

Configurar autenticação baseada em JWT:

```rust
let config = HttpInputConfig {
    jwt_public_key_path: Some("/path/to/jwt/public.key".to_string()),
    ..Default::default()
};
```

### Controles de Segurança

- **Limites de Tamanho de Requisição**: Tamanho máximo configurável do corpo previne esgotamento de recursos
- **Limites de Concorrência**: Semáforo integrado controla processamento de requisições concorrentes
- **Suporte CORS**: Cabeçalhos CORS opcionais para aplicações baseadas em navegador
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

O servidor retorna uma resposta JSON com a saída do agente:

```json
{
  "status": "execution_started",
  "agent_id": "webhook_handler",
  "timestamp": "2024-01-15T10:30:00Z"
}
```

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

### Endpoint de Verificação de Saúde

O servidor automaticamente fornece capacidades de verificação de saúde para balanceadores de carga e sistemas de monitoramento.

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
INFO Would invoke agent webhook_handler with input data
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

- [Guia de Introdução](getting-started.pt.md)
- [Guia DSL](dsl-guide.pt.md)
- [Referência da API](api-reference.pt.md)
- [Documentação do Runtime de Agentes](../crates/runtime/README.md)