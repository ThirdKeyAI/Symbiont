<img src="logo-hz.png" alt="Symbi">

[English](README.md) | [中文简体](README.zh-cn.md) | [Español](README.es.md) | **Português** | [日本語](README.ja.md) | [Deutsch](README.de.md)

[![Build](https://img.shields.io/github/actions/workflow/status/thirdkeyai/symbiont/docker-build.yml?branch=main)](https://github.com/thirdkeyai/symbiont/actions)
[![Crates.io](https://img.shields.io/crates/v/symbi)](https://crates.io/crates/symbi)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Docs](https://img.shields.io/badge/docs-online-brightgreen)](https://docs.symbiont.dev)

---

## 🚀 O que é Symbiont?

**Symbi** é um **framework de agentes nativo do Rust com confiança zero** para construir agentes de IA autônomos e conscientes de políticas.
Ele corrige as maiores falhas de frameworks existentes como LangChain e AutoGPT ao focar em:

* **Segurança em primeiro lugar**: trilhas de auditoria criptográficas, políticas aplicadas e sandboxing.
* **Confiança zero**: todas as entradas são tratadas como não confiáveis por padrão.
* **Conformidade de nível empresarial**: projetado para indústrias regulamentadas (HIPAA, SOC2, finanças).

Agentes Symbiont colaboram com segurança com humanos, ferramentas e LLMs — sem sacrificar segurança ou desempenho.

---

## ⚡ Por que Symbiont?

| Recurso      | Symbiont                           | LangChain      | AutoGPT   |
| ------------ | ---------------------------------- | -------------- | --------- |
| Linguagem    | Rust (segurança, desempenho)      | Python         | Python    |
| Segurança    | Confiança zero, auditoria cripto   | Mínima         | Nenhuma   |
| Motor Políticas | DSL integrado                   | Limitado       | Nenhum    |
| Implantação  | REPL, Docker, HTTP API            | Scripts Python | Hacks CLI |
| Trilhas Auditoria | Logs criptográficos            | Não            | Não       |

---

## 🏁 Início Rápido

### Pré-requisitos

* Docker (recomendado) ou Rust 1.82+
* Nenhum banco de dados vetorial externo necessário (LanceDB embutido; Qdrant opcional para implantações em escala)

### Executar com Contêineres Pré-construídos

```bash
# Analisar arquivo DSL de agente
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest dsl parse /workspace/agent.dsl

# Executar MCP Server
docker run --rm -p 8080:8080 ghcr.io/thirdkeyai/symbi:latest mcp

# Shell de desenvolvimento interativo
docker run --rm -it -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest bash
```

### Construir a partir do Código-fonte

```bash
# Construir ambiente de desenvolvimento
docker build -t symbi:latest .
docker run --rm -it -v $(pwd):/workspace symbi:latest bash

# Construir binário unificado
cargo build --release

# Executar REPL
cargo run -- repl

# Analisar DSL e executar MCP
cargo run -- dsl parse my_agent.dsl
cargo run -- mcp --port 8080
```

---

## 🔧 Recursos Principais

* ✅ **Gramática DSL** – Define agentes declarativamente com políticas de segurança integradas.
* ✅ **Runtime de Agentes** – Agendamento de tarefas, gerenciamento de recursos e controle do ciclo de vida.
* 🔒 **Sandboxing** – Isolamento Docker Tier-1 para execução de agentes.
* 🔒 **Segurança SchemaPin** – Verificação criptográfica de ferramentas e esquemas.
* 🔒 **Gerenciamento de Segredos** – Integração HashiCorp Vault / OpenBao, armazenamento criptografado AES-256-GCM.
* 📊 **Engine RAG** – Busca vetorial (LanceDB embutido) com recuperação híbrida semântica + palavra-chave. Backend Qdrant opcional para implantações em escala.
* 🧩 **Integração MCP** – Suporte nativo para ferramentas do Protocolo de Contexto de Modelo.
* 📡 **API HTTP Opcional** – Interface REST controlada por recursos para integração externa.

---

## 📐 Exemplo de DSL Symbiont

```symbiont
metadata {
    version = "1.0.0"
    author = "Your Name"
    description = "Data analysis agent"
}

agent analyze_data(input: DataSet) -> Result {
    capabilities = ["data_analysis", "visualization"]
    
    policy data_privacy {
        allow: read(input) if input.anonymized == true
        deny: store(input) if input.contains_pii == true
        audit: all_operations
    }
    
    with memory = "persistent", requires = "approval" {
        if (llm_check_safety(input)) {
            result = analyze(input);
            return result;
        } else {
            return reject("Safety check failed");
        }
    }
}
```

---

## 🔒 Modelo de Segurança

* **Confiança Zero** – todas as entradas de agentes são não confiáveis por padrão.
* **Execução Sandboxed** – contenção baseada em Docker para processos.
* **Log de Auditoria** – logs criptograficamente à prova de adulteração.
* **Controle de Segredos** – backends Vault/OpenBao, armazenamento local criptografado, namespaces de agentes.

---

## 📚 Documentação

* [Primeiros Passos](https://docs.symbiont.dev/getting-started)
* [Guia do DSL](https://docs.symbiont.dev/dsl-guide)
* [Arquitetura do Runtime](https://docs.symbiont.dev/runtime-architecture)
* [Modelo de Segurança](https://docs.symbiont.dev/security-model)
* [Referência da API](https://docs.symbiont.dev/api-reference)

---

## 🎯 Casos de Uso

* **Desenvolvimento e Automação**

  * Geração e refatoração segura de código.
  * Implantação de agentes IA com políticas aplicadas.
  * Gerenciamento de conhecimento com busca semântica.

* **Empresas e Indústrias Regulamentadas**

  * Saúde (processamento compatível com HIPAA).
  * Finanças (fluxos de trabalho prontos para auditoria).
  * Governo (manuseio de contexto classificado).
  * Jurídico (análise confidencial de documentos).

---

## 📄 Licença

* **Edição Community**: Licença Apache 2.0
* **Edição Enterprise**: Licença comercial necessária

Entre em contato com [ThirdKey](https://thirdkey.ai) para licenciamento empresarial.

---

*Symbiont permite colaboração segura entre agentes IA e humanos através de aplicação inteligente de políticas, verificação criptográfica e trilhas de auditoria abrangentes.*


<div align="right">
  <img src="symbi-trans.png" alt="Logo Symbi" width="120">
</div>