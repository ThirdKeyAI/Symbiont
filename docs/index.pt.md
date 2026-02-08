---
layout: default
title: Início
description: "Symbiont: Linguagem de programação nativa de IA e framework de agentes com privacidade em primeiro lugar"
nav_exclude: true
---

# Documentação do Symbiont
{: .fs-9 }

Linguagem de programação nativa de IA e framework de agentes com privacidade em primeiro lugar para desenvolvimento de software autônomo e consciente de políticas.
{: .fs-6 .fw-300 }

[Começar agora](#getting-started){: .btn .btn-primary .fs-5 .mb-4 .mb-md-0 .mr-2 }
[Ver no GitHub](https://github.com/thirdkeyai/symbiont){: .btn .fs-5 .mb-4 .mb-md-0 }

---

## 🌐 Outros idiomas
{: .no_toc}

[English](index.md) | [中文简体](index.zh-cn.md) | [Español](index.es.md) | **Português** | [日本語](index.ja.md) | [Deutsch](index.de.md)

---

## O que é o Symbiont?

O Symbiont representa a próxima evolução no desenvolvimento de software — onde agentes de IA e desenvolvedores humanos colaboram de forma segura, transparente e eficaz. Ele permite que desenvolvedores construam agentes autônomos e conscientes de políticas que podem colaborar com segurança com humanos, outros agentes e modelos de linguagem grandes, enquanto aplicam segurança de confiança zero, privacidade de dados e comportamento verificável.

### Principais Características

- **🛡️ Design Focado em Segurança**: Sandbox multi-camadas com Docker e gVisor
- **📋 Programação Consciente de Políticas**: Políticas de segurança declarativas com aplicação em tempo de execução
- **🔐 Gestão de Segredos Empresariais**: Integração com HashiCorp Vault e backends de arquivos criptografados
- **🔑 Auditabilidade Criptográfica**: Log completo de operações com assinaturas Ed25519
- **🧠 Gestão Inteligente de Contexto**: Sistemas de conhecimento aprimorados com RAG e busca vetorial
- **🔗 Integração Segura de Ferramentas**: Protocolo MCP com verificação criptográfica
- **⚡ Alto Desempenho**: Implementação nativa em Rust para cargas de trabalho de produção

---

## Primeiros Passos

### Instalação Rápida

```bash
# Clonar o repositório
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont

# Construir container symbi unificado
docker build -t symbi:latest .

# Ou usar container pré-construído
docker pull ghcr.io/thirdkeyai/symbi:latest

# Testar o sistema
cargo test

# Testar o CLI unificado
docker run --rm symbi:latest --version
docker run --rm -v $(pwd):/workspace symbi:latest dsl parse --help
docker run --rm symbi:latest mcp --help
```

### Seu Primeiro Agente

```rust
metadata {
    version = "1.0.0"
    author = "developer"
    description = "Simple analysis agent"
}

agent analyze_data(input: DataSet) -> Result {
    capabilities = ["data_analysis"]
    
    policy secure_analysis {
        allow: read(input) if input.anonymized == true
        deny: store(input) if input.contains_pii == true
        audit: all_operations with signature
    }
    
    with memory = "ephemeral", privacy = "high" {
        if (validate_input(input)) {
            result = process_data(input);
            audit_log("analysis_completed", result.metadata);
            return result;
        } else {
            return reject("Invalid input data");
        }
    }
}
```

---

## Visão Geral da Arquitetura

```mermaid
graph TB
    A[Camada de Governança e Políticas] --> B[Motor Central Rust]
    B --> C[Framework de Agentes]
    B --> D[Motor DSL Tree-sitter]
    B --> E[Sandbox Multi-camadas]
    E --> F[Docker - Baixo Risco]
    E --> G[gVisor - Médio/Alto Risco]
    B --> I[Trilha de Auditoria Criptográfica]
    
    subgraph "Contexto e Conhecimento"
        J[Gestor de Contexto]
        K[Base de Dados Vetorial]
        L[Motor RAG]
    end
    
    subgraph "Integrações Seguras"
        M[Cliente MCP]
        N[Verificação de Ferramentas]
        O[Motor de Políticas]
    end
    
    C --> J
    C --> M
    J --> K
    J --> L
    M --> N
    M --> O
```

---

## Casos de Uso

### Desenvolvimento e Pesquisa
- Geração segura de código e testes automatizados
- Experimentos de colaboração multi-agente
- Desenvolvimento de sistemas de IA conscientes do contexto

### Aplicações Críticas de Privacidade
- Processamento de dados de saúde com controles de privacidade
- Automação de serviços financeiros com capacidades de auditoria
- Sistemas governamentais e de defesa com recursos de segurança

---

## Status do Projeto

### v1.0.0 Lançado

O Symbiont v1.0.0 é a primeira versão estável, oferecendo um framework completo de agentes de IA com capacidades de nível de produção:

- **Agendamento**: Execução de tarefas baseada em cron com isolamento de sessão, roteamento de entrega e filas de mensagens mortas
- **Isolamento de Sessão**: Contextos de agente efêmeros, compartilhados ou totalmente isolados
- **Roteamento de Entrega**: Saída para Stdout, LogFile, Webhook, Slack, Email ou canais personalizados
- **Aplicação de Políticas**: Verificações de segurança e conformidade com janelas de tempo e verificação de capacidades
- **Identidade AgentPin**: Verificação criptográfica de identidade de agentes via ES256 JWTs
- **Observabilidade**: Métricas compatíveis com Prometheus, eventos de auditoria estruturados e endpoints de saúde

### 🔮 Recursos Planejados
- Suporte RAG multi-modal (imagens, áudio, dados estruturados)
- Síntese de conhecimento e colaboração entre agentes
- Redes federadas de agentes com confiança entre domínios
- Otimização de desempenho e cache inteligente

---

## Comunidade

- **Documentação**: Guias abrangentes e referências de API
- [Referência da API](api-reference.md)
- [Guia de Agendamento](scheduling.md)
- [Módulo de Entrada HTTP](http-input.md)
- **Problemas**: [GitHub Issues](https://github.com/thirdkeyai/symbiont/issues)
- **Discussões**: [GitHub Discussions](https://github.com/thirdkeyai/symbiont/discussions)
- **Licença**: Software de código aberto da ThirdKey

---

## Próximos Passos

<div class="grid grid-cols-1 md:grid-cols-3 gap-6 mt-8">
  <div class="card">
    <h3>🚀 Começar</h3>
    <p>Siga nosso guia de introdução para configurar seu primeiro ambiente Symbiont.</p>
    <a href="/getting-started" class="btn btn-outline">Guia de Início Rápido</a>
  </div>
  
  <div class="card">
    <h3>📖 Aprender o DSL</h3>
    <p>Domine o DSL do Symbiont para construir agentes conscientes de políticas.</p>
    <a href="/dsl-guide" class="btn btn-outline">Documentação DSL</a>
  </div>
  
  <div class="card">
    <h3>🏗️ Arquitetura</h3>
    <p>Compreenda o sistema de tempo de execução e o modelo de segurança.</p>
    <a href="/runtime-architecture" class="btn btn-outline">Guia de Arquitetura</a>
  </div>
</div>