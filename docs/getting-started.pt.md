---
layout: default
title: Começando
description: "Guia de início rápido para Symbiont"
nav_exclude: true
---

# Começando
{: .no_toc }

## 🌐 Outros idiomas
{: .no_toc}

[English](getting-started.md) | [中文简体](getting-started.zh-cn.md) | [Español](getting-started.es.md) | **Português** | [日本語](getting-started.ja.md) | [Deutsch](getting-started.de.md)

---

Este guia irá orientá-lo na configuração do Symbi e na criação do seu primeiro agente de IA.
{: .fs-6 .fw-300 }

## Índice
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Pré-requisitos

Antes de começar com o Symbi, certifique-se de ter o seguinte instalado:

### Dependências Obrigatórias

- **Docker** (para desenvolvimento containerizado)
- **Rust 1.88+** (se compilando localmente)
- **Git** (para clonar o repositório)

### Dependências Opcionais

- **SchemaPin Go CLI** (para verificação de ferramentas)

> **Nota:** A busca vetorial é integrada. O Symbi inclui o [LanceDB](https://lancedb.com/) como banco de dados vetorial embutido -- nenhum serviço externo é necessário.

---

## Instalação

### Opção 1: Docker (Recomendado)

A maneira mais rápida de começar é usando Docker:

```bash
# Clonar o repositório
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont

# Construir o container unificado symbi
docker build -t symbi:latest .

# Ou usar container pré-construído
docker pull ghcr.io/thirdkeyai/symbi:latest

# Executar o ambiente de desenvolvimento
docker run --rm -it -v $(pwd):/workspace symbi:latest bash
```

### Opção 2: Instalação Local

Para desenvolvimento local:

```bash
# Clonar o repositório
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont

# Instalar dependências do Rust e compilar
cargo build --release

# Executar testes para verificar a instalação
cargo test
```

### Verificar Instalação

Testar se tudo está funcionando corretamente:

```bash
# Testar o analisador DSL
cd crates/dsl && cargo run && cargo test

# Testar o sistema de runtime
cd ../runtime && cargo test

# Executar agentes de exemplo
cargo run --example basic_agent
cargo run --example full_system

# Testar o CLI unificado symbi
cd ../.. && cargo run -- dsl --help
cargo run -- mcp --help

# Testar com container Docker
docker run --rm symbi:latest --version
docker run --rm -v $(pwd):/workspace symbi:latest dsl parse --help
docker run --rm symbi:latest mcp --help
```

---

## Inicialização de Projeto

A maneira mais rápida de iniciar um novo projeto Symbiont é `symbi init`:

```bash
symbi init
```

Isso inicia um assistente interativo que o guia por:
- **Seleção de perfil**: `minimal`, `assistant`, `dev-agent` ou `multi-agent`
- **Modo SchemaPin**: `tofu` (Trust-On-First-Use), `strict` ou `disabled`
- **Camada de sandbox**: `tier0` (nenhuma), `tier1` (Docker) ou `tier2` (gVisor)

### Modo não interativo

Para CI/CD ou configurações por script:

```bash
symbi init --profile assistant --schemapin tofu --sandbox tier1 --no-interact
```

### Perfis

| Perfil | O que cria |
|--------|-----------|
| `minimal` | `symbiont.toml` + política Cedar padrão |
| `assistant` | + agente assistente governado individual |
| `dev-agent` | + agente CliExecutor com políticas de segurança |
| `multi-agent` | + agentes coordenador/worker com políticas inter-agente |

### Importando do catálogo

Importe agentes pré-construídos junto com qualquer perfil:

```bash
symbi init --profile minimal --no-interact
symbi init --catalog assistant,dev
```

Listar agentes disponíveis no catálogo:

```bash
symbi init --catalog list
```

Após a inicialização, valide e inicie:

```bash
symbi dsl -f agents/assistant.dsl   # validar seu agente
symbi up                             # iniciar o runtime
```

---

## Seu Primeiro Agente

Vamos criar um agente simples de análise de dados para entender os conceitos básicos do Symbi.

### 1. Criar Definição do Agente

Crie um novo arquivo `my_agent.dsl`:

```rust
metadata {
    version = "1.0.0"
    author = "your-name"
    description = "My first Symbi agent"
}

agent greet_user(name: String) -> String {
    capabilities = ["greeting", "text_processing"]

    policy safe_greeting {
        allow: read(name) if name.length <= 100
        deny: store(name) if name.contains_sensitive_data
        audit: all_operations with signature
    }

    with memory = "ephemeral", privacy = "low" {
        if (validate_name(name)) {
            greeting = format_greeting(name);
            audit_log("greeting_generated", greeting.metadata);
            return greeting;
        } else {
            return "Hello, anonymous user!";
        }
    }
}
```

### 2. Executar o Agente

```bash
# Analisar e validar a definição do agente
cargo run -- dsl parse my_agent.dsl

# Executar o agente no runtime
cd crates/runtime && cargo run --example basic_agent -- --agent ../../my_agent.dsl
```

---

## Entendendo o DSL

O DSL do Symbi tem vários componentes principais:

### Bloco de Metadados

```rust
metadata {
    version = "1.0.0"
    author = "developer"
    description = "Agent description"
}
```

Fornece informações essenciais sobre o seu agente para documentação e gerenciamento do runtime.

### Definição do Agente

```rust
agent agent_name(parameter: Type) -> ReturnType {
    capabilities = ["capability1", "capability2"]
    // implementação do agente
}
```

Define a interface, capacidades e comportamento do agente.

### Definições de Política

```rust
policy policy_name {
    allow: action_list if condition
    deny: action_list if condition
    audit: operation_type with audit_method
}
```

Políticas de segurança declarativas que são aplicadas em tempo de execução.

### Contexto de Execução

```rust
with memory = "persistent", privacy = "high" {
    // implementação do agente
}
```

Especifica a configuração de runtime para gerenciamento de memória e requisitos de privacidade.

---

## Próximos Passos

### Explorar Exemplos

O repositório inclui vários agentes de exemplo:

```bash
# Exemplo de agente básico
cd crates/runtime && cargo run --example basic_agent

# Demonstração completa do sistema
cd crates/runtime && cargo run --example full_system

# Exemplo de contexto e memória
cd crates/runtime && cargo run --example context_example

# Agente potenciado por RAG
cd crates/runtime && cargo run --example rag_example
```

### Habilitar Recursos Avançados

#### API HTTP (Opcional)

```bash
# Habilitar o recurso de API HTTP
cd crates/runtime && cargo build --features http-api

# Executar com endpoints de API
cd crates/runtime && cargo run --features http-api --example full_system
```

**Principais Endpoints da API:**
- `GET /api/v1/health` - Verificação de saúde e status do sistema
- `GET /api/v1/agents` - Listar todos os agentes ativos com status de execução em tempo real
- `GET /api/v1/agents/{id}/status` - Obter métricas detalhadas de execução do agente
- `POST /api/v1/workflows/execute` - Executar fluxos de trabalho

**Novos Recursos de Gerenciamento de Agentes:**
- Monitoramento de processos em tempo real e verificações de saúde
- Capacidades de desligamento gracioso para agentes em execução
- Métricas de execução abrangentes e rastreamento de uso de recursos
- Suporte para diferentes modos de execução (efêmero, persistente, agendado, orientado a eventos)

#### Inferência LLM em Nuvem

Conecte a provedores de LLM em nuvem via OpenRouter:

```bash
# Habilitar inferência em nuvem
cargo build --features cloud-llm

# Definir chave de API e modelo
export OPENROUTER_API_KEY="sk-or-..."
export OPENROUTER_MODEL="google/gemini-2.0-flash-001"  # opcional
```

#### Modo Agente Autônomo

Linha única para agentes cloud-native com inferência LLM e acesso a ferramentas Composio:

```bash
cargo build --features standalone-agent
# Habilita: cloud-llm + composio
```

#### Primitivas de Raciocínio Avançado

Habilite curadoria de ferramentas, detecção de loops travados, pré-busca de contexto e convenções com escopo:

```bash
cargo build --features orga-adaptive
```

Veja o [guia orga-adaptive](/orga-adaptive) para documentação completa.

#### Motor de Políticas Cedar

Autorização formal com a linguagem de políticas Cedar:

```bash
cargo build --features cedar
```

#### Banco de Dados Vetorial (Integrado)

O Symbi inclui o LanceDB como banco de dados vetorial embutido sem configuração. A busca semântica e o RAG funcionam imediatamente -- nenhum serviço separado para iniciar:

```bash
# Executar agente com capacidades RAG (a busca vetorial funciona automaticamente)
cd crates/runtime && cargo run --example rag_example

# Testar gerenciamento de contexto com busca avançada
cd crates/runtime && cargo run --example context_example
```

> **Opção enterprise:** Para equipes que precisam de um banco de dados vetorial dedicado, o Qdrant está disponível como backend opcional com feature gate. Defina `SYMBIONT_VECTOR_BACKEND=qdrant` e `QDRANT_URL` para utilizá-lo.

**Recursos de Gerenciamento de Contexto:**
- **Busca Multi-Modal**: Modos de busca por palavra-chave, temporal, similaridade e híbrido
- **Cálculo de Importância**: Algoritmo de pontuação sofisticado considerando padrões de acesso, recência e feedback do usuário
- **Controle de Acesso**: Integração com motor de políticas e controles de acesso com escopo de agente
- **Arquivamento Automático**: Políticas de retenção com armazenamento comprimido e limpeza
- **Compartilhamento de Conhecimento**: Compartilhamento seguro de conhecimento entre agentes com pontuações de confiança

#### Referência de Feature Flags

| Feature | Descrição | Padrão |
|---------|-----------|--------|
| `keychain` | Integração com chaveiro do SO para segredos | Sim |
| `vector-lancedb` | Backend vetorial embutido LanceDB | Sim |
| `vector-qdrant` | Backend vetorial distribuído Qdrant | Não |
| `embedding-models` | Modelos de embedding locais via Candle | Não |
| `http-api` | API REST com Swagger UI | Não |
| `http-input` | Servidor de webhook com autenticação JWT | Não |
| `cloud-llm` | Inferência LLM em nuvem (OpenRouter) | Não |
| `composio` | Integração de ferramentas Composio MCP | Não |
| `standalone-agent` | Combo Cloud LLM + Composio | Não |
| `cedar` | Motor de políticas Cedar | Não |
| `orga-adaptive` | Primitivas de raciocínio avançado | Não |
| `cron` | Agendamento cron persistente | Não |
| `native-sandbox` | Sandboxing nativo de processos | Não |
| `metrics` | Métricas/rastreamento OpenTelemetry | Não |
| `interactive` | Prompts interativos para `symbi init` (dialoguer) | Sim |
| `full` | Todos os recursos exceto enterprise | Não |

```bash
# Compilar com features específicas
cargo build --features "cloud-llm,orga-adaptive,cedar"

# Compilar com tudo
cargo build --features full
```

---

## Configuração

### Variáveis de Ambiente

Configure seu ambiente para performance ideal:

```bash
# Configuração básica
export SYMBI_LOG_LEVEL=info
export SYMBI_RUNTIME_MODE=development

# A busca vetorial funciona automaticamente com o backend LanceDB integrado.
# Para usar o Qdrant em vez disso (opcional, enterprise):
# export SYMBIONT_VECTOR_BACKEND=qdrant
# export QDRANT_URL=http://localhost:6333

# Integração MCP (opcional)
export MCP_SERVER_URLS="http://localhost:8080"
```

### Configuração de Runtime

Crie um arquivo de configuração `symbi.toml`:

```toml
[runtime]
max_agents = 1000
memory_limit_mb = 512
execution_timeout_seconds = 300

[security]
default_sandbox_tier = "docker"
audit_enabled = true
policy_enforcement = "strict"

[vector_db]
enabled = true
backend = "lancedb"              # padrão; também suporta "qdrant"
collection_name = "symbi_knowledge"
# url = "http://localhost:6333"  # necessário apenas quando backend = "qdrant"
```

---

## Problemas Comuns

### Problemas com Docker

**Problema**: Construção do Docker falha com erros de permissão
```bash
# Solução: Garantir que o daemon do Docker está rodando e o usuário tem permissões
sudo systemctl start docker
sudo usermod -aG docker $USER
```

**Problema**: Container sai imediatamente
```bash
# Solução: Verificar logs do Docker
docker logs <container_id>
```

### Problemas de Construção com Rust

**Problema**: Construção do Cargo falha com erros de dependência
```bash
# Solução: Atualizar Rust e limpar cache de construção
rustup update
cargo clean
cargo build
```

**Problema**: Dependências do sistema em falta
```bash
# Ubuntu/Debian
sudo apt-get update
sudo apt-get install build-essential pkg-config libssl-dev

# macOS
brew install pkg-config openssl
```

### Problemas de Runtime

**Problema**: Agente falha ao iniciar
```bash
# Verificar sintaxe da definição do agente
cargo run -- dsl parse your_agent.dsl

# Habilitar logging de debug
RUST_LOG=debug cd crates/runtime && cargo run --example basic_agent
```

---

## Obtendo Ajuda

### Documentação

- **[Guia DSL](/dsl-guide)** - Referência completa do DSL
- **[Arquitetura de Runtime](/runtime-architecture)** - Detalhes da arquitetura do sistema
- **[Modelo de Segurança](/security-model)** - Documentação de segurança e políticas

### Suporte da Comunidade

- **Issues**: [GitHub Issues](https://github.com/thirdkeyai/symbiont/issues)
- **Discussões**: [GitHub Discussions](https://github.com/thirdkeyai/symbiont/discussions)
- **Documentação**: [Referência Completa da API](https://docs.symbiont.dev/api-reference)

### Modo de Debug

Para solução de problemas, habilite logging detalhado:

```bash
# Habilitar logging de debug
export RUST_LOG=symbi=debug

# Executar com saída detalhada
cd crates/runtime && cargo run --example basic_agent 2>&1 | tee debug.log
```

---

## O Que Vem a Seguir?

Agora que você tem o Symbi rodando, explore estes tópicos avançados:

1. **[Guia DSL](/dsl-guide)** - Aprenda recursos avançados do DSL
2. **[Guia do Loop de Raciocínio](/reasoning-loop)** - Entenda o ciclo ORGA
3. **[Raciocínio Avançado (orga-adaptive)](/orga-adaptive)** - Curadoria de ferramentas, detecção de loops travados, pré-hidratação
4. **[Arquitetura de Runtime](/runtime-architecture)** - Entenda os internos do sistema
5. **[Modelo de Segurança](/security-model)** - Implemente políticas de segurança
6. **[Contribuindo](/contributing)** - Contribua para o projeto

Pronto para construir algo incrível? Comece com nossos [projetos de exemplo](https://github.com/thirdkeyai/symbiont/tree/main/crates/runtime/examples) ou mergulhe na [especificação completa](/specification).
