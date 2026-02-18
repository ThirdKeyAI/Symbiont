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

## Seu Primeiro Agente

Vamos criar um agente simples de análise de dados para entender os conceitos básicos do Symbi.

### 1. Criar Definição do Agente

Criar um novo arquivo `my_agent.dsl`:

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
- `GET /api/v1/agents` - Listar todos os agentes ativos
- `POST /api/v1/workflows/execute` - Executar fluxos de trabalho

#### Banco de Dados Vetorial (integrado)

O Symbi inclui o LanceDB como banco de dados vetorial embutido sem configuração. A busca semântica e o RAG funcionam imediatamente -- nenhum serviço separado para iniciar:

```bash
# Executar agente com capacidades RAG (a busca vetorial funciona automaticamente)
cd crates/runtime && cargo run --example rag_example
```

> **Opção enterprise:** Para equipes que precisam de um banco de dados vetorial dedicado, o Qdrant está disponível como backend opcional com feature gate. Defina `SYMBIONT_VECTOR_BACKEND=qdrant` e `QDRANT_URL` para utilizá-lo.

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

Criar um arquivo de configuração `symbi.toml`:

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
- **Documentação**: [Referência Completa da API](https://docs.symbiont.platform)

### Modo de Debug

Para solução de problemas, habilitar logging detalhado:

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
2. **[Arquitetura de Runtime](/runtime-architecture)** - Entenda os internos do sistema
3. **[Modelo de Segurança](/security-model)** - Implemente políticas de segurança
4. **[Contribuindo](/contributing)** - Contribua para o projeto

Pronto para construir algo incrível? Comece com nossos [projetos de exemplo](https://github.com/thirdkeyai/symbiont/tree/main/crates/runtime/examples) ou mergulhe na [especificação completa](/specification).